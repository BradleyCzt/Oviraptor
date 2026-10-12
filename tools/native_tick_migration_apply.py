"""Explicit, backed-up atomic schema upgrade; never called by app startup."""
import json
import hashlib
from pathlib import Path
from native_tick_migration_snapshot import file_hash, open_backup, open_database, quote, snapshot
from native_tick_migration_writer import install
from native_tick_migration_schema import TABLES, classify, require_scope, SCHEMA_PATH, managed_statements


def postconditions(db, before):
    after = snapshot(db)
    if after['tables'] != before['tables'] or after['foreign_key_check']:
        raise ValueError('native_tick_original_rows_changed')
    if [r for r in after['schema'] if r[2] not in TABLES] != [r for r in before['schema'] if r[2] not in TABLES]:
        raise ValueError('native_tick_other_schema_changed')
    if classify(after['schema']) != 'current':
        raise ValueError('native_tick_guards_not_preserved')
    return after


def apply(inventory, expected_sha256):
    inventory = Path(inventory).resolve(strict=True)
    raw = inventory.read_bytes()
    if hashlib.sha256(raw).hexdigest() != expected_sha256:
        raise ValueError('native_tick_inventory_changed')
    report = json.loads(raw)
    if report['format'] != 'native-tick-migration-v1':
        raise ValueError('native_tick_inventory_format')
    if file_hash(report['backup']) != report['backup_sha256']:
        raise ValueError('native_tick_backup_changed')
    backup = open_backup(report['backup'])
    try:
        backup.execute('BEGIN')
        if snapshot(backup) != report['snapshot'] or backup.execute('PRAGMA integrity_check').fetchone()[0] != b'ok':
            raise ValueError('native_tick_backup_not_original')
    finally:
        backup.close()
    db = open_database(report['source'], writable=True)
    try:
        db.execute('PRAGMA synchronous=FULL')
        db.execute('BEGIN IMMEDIATE')
        before = snapshot(db)
        state = classify(before['schema'])
        if before != report['snapshot']:
            if state == 'current':
                postconditions(db, report['snapshot'])
                db.rollback()
                return {'state': 'current', 'changed': False}
            raise ValueError('native_tick_database_changed_since_inventory')
        if state != 'original_run_identity':
            db.rollback()
            return {'state': state, 'changed': False}
        if before['foreign_key_check']:
            raise ValueError('native_tick_original_foreign_key_violation')
        require_scope(db)
        install(db)
        # Guards are atomically recreated from trusted source, with FK enforcement ON.
        # TEMP copies never lose SQLite values, original physical rowids or JSON bytes.
        for i, table in enumerate(TABLES):
            db.execute('CREATE TEMP TABLE migration_' + str(i) + ' AS SELECT rowid AS original_rowid,* FROM ' + quote(table))
            db.execute('DROP TABLE ' + quote(table))
        for statement in managed_statements(SCHEMA_PATH.read_text()):
            db.execute(statement)
        for i, table in enumerate(TABLES):
            columns = before['tables'][table]['columns']
            db.execute('INSERT INTO ' + quote(table) + '(' + ','.join(map(quote, columns)) + ') SELECT original_rowid,'
                       + ','.join(map(quote, columns[1:])) + ' FROM temp.migration_' + str(i))
        postconditions(db, before)
        db.commit()
        return {'state': 'current', 'changed': True}
    finally:
        # Closing an interrupted/unconfirmed transaction rolls it back. Never repair.
        db.close()
