#!/usr/bin/env python3
"""Explicit inventory/backup then hash-pinned upgrade of two Native receipt tables.

Not a legacy import, data cleanup, app bootstrap or receipt reconstruction.
Default action is read-only inventory with an online SQLite backup.
"""
import argparse
import json
import os
import sqlite3
import uuid
from pathlib import Path
from native_tick_migration_snapshot import canonical, file_hash, open_backup, open_database, snapshot
from native_tick_migration_schema import classify
from native_tick_migration_apply import apply


def inventory(database, backup_directory):
    database = Path(database).resolve(strict=True)
    destination = Path(backup_directory).resolve() / ('native-tick-' + uuid.uuid4().hex)
    destination.mkdir(mode=0o700, parents=True, exist_ok=False)
    backup_path = destination / 'database.sqlite3.backup'
    db = open_database(database)
    try:
        db.execute('BEGIN')
        before = snapshot(db)
        state = classify(before['schema'])
        out = sqlite3.connect(backup_path)
        try:
            db.backup(out)
        finally:
            out.close()
        os.chmod(backup_path, 0o600)
    finally:
        db.close()
    backup = open_backup(backup_path)
    try:
        backup.execute('BEGIN')
        if snapshot(backup) != before or backup.execute('PRAGMA integrity_check').fetchone()[0] != b'ok':
            raise ValueError('native_tick_backup_verification_failed')
    finally:
        backup.close()
    report = {'format': 'native-tick-migration-v1', 'source': str(database),
              'backup': str(backup_path), 'backup_sha256': file_hash(backup_path),
              'state': state, 'snapshot': before}
    manifest = destination / 'inventory.json'
    manifest.write_bytes(canonical(report))
    os.chmod(manifest, 0o600)
    return {'inventory': str(manifest), 'inventory_sha256': file_hash(manifest),
            'state': state, 'changed': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--database', type=Path)
    parser.add_argument('--backup-directory', type=Path)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--inventory', type=Path)
    parser.add_argument('--expected-sha256')
    args = parser.parse_args()
    if args.apply:
        if not args.inventory or not args.expected_sha256 or args.database or args.backup_directory:
            parser.error('--apply requires only --inventory and --expected-sha256')
        result = apply(args.inventory, args.expected_sha256)
    else:
        if not args.database or not args.backup_directory or args.inventory or args.expected_sha256:
            parser.error('inventory requires only --database and --backup-directory')
        result = inventory(args.database, args.backup_directory)
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    main()
