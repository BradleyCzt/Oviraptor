"""Temporary SQLite fault tests; not real model or installed-app acceptance."""
import json
import re
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from migrate_native_tick_identity import inventory
from native_tick_migration_apply import apply
from native_tick_migration_snapshot import open_database, snapshot
from native_tick_migration_schema import SCHEMA_PATH, TABLES


class NativeTickMigrationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.path = self.root / 'db.sqlite3'
        db = sqlite3.connect(self.path)
        db.executescript("""PRAGMA foreign_keys=ON;
        CREATE TABLE agent_runs(id TEXT PRIMARY KEY);
        CREATE TABLE assets(id INTEGER PRIMARY KEY,original BLOB);
        CREATE TABLE private_business(k TEXT PRIMARY KEY,value BLOB) WITHOUT ROWID;
        INSERT INTO agent_runs VALUES('root');
        INSERT INTO assets VALUES(765,X'00FF');
        INSERT INTO private_business VALUES('business',X'AA00FF');""")
        db.executescript((SCHEMA_PATH.parents[3] / 'root_schema.sql').read_text())
        db.execute("INSERT INTO agent_root_budget_attempts(id,root_run_id,contract_json) VALUES('control','root','{}')")
        db.commit()
        db.executescript(SCHEMA_PATH.read_text().replace('REFERENCES agent_root_budget_attempts(root_run_id)', 'REFERENCES agent_runs(id)'))
        db.execute("INSERT INTO agent_root_tick_receipts(rowid,call_id,root_run_id,lease_attempt_id,round,request_hash,basis_hash,phase,fact_json) VALUES(371,'call','root','control',1,?1,?1,'request',?2)", ('a'*64, ' { "original" : "原\\u0000 bytes" } '))
        db.execute("INSERT INTO agent_root_tick_timeline_receipts(rowid,call_id,root_run_id,lease_attempt_id,model_event_sequence,collaboration_sequence,scan_id,attempt_number,entity_id,payload_json,event_created_at) VALUES(992,'call','root','control',1,123,'scan',1,'event',?1,'original timestamp')", (' { "paid" : true } ',))
        db.commit()
        db.close()

    def tearDown(self):
        self.directory.cleanup()

    def original(self):
        db = open_database(self.path)
        try:
            db.execute('BEGIN')
            return snapshot(db)
        finally:
            db.close()

    def prepare(self):
        before = self.original()
        result = inventory(self.path, self.root / 'backups')
        self.assertEqual(self.original(), before)
        return before, result

    def upgrade(self, report):
        return apply(report['inventory'], report['inventory_sha256'])

    def test_original_rows_rowids_bytes_assets_and_every_guard_survive(self):
        before, report = self.prepare()
        self.assertEqual(self.upgrade(report), {'state': 'current', 'changed': True})
        self.assertEqual(self.original()['tables'], before['tables'])
        self.assertEqual(self.upgrade(report), {'state': 'current', 'changed': False})
        db = sqlite3.connect(self.path)
        db.execute('PRAGMA foreign_keys=ON')
        for table in TABLES:
            for sql in [f'UPDATE {table} SET root_run_id=root_run_id', f'DELETE FROM {table}', f'INSERT OR REPLACE INTO {table} SELECT * FROM {table}']:
                with self.assertRaises(sqlite3.IntegrityError):
                    db.execute(sql)
        db.execute("DELETE FROM agent_runs WHERE id='root'")
        with self.assertRaises(sqlite3.IntegrityError):
            db.execute("DELETE FROM agent_root_budget_attempts WHERE id='control'")
        db.rollback()
        db.close()
        _, fresh = self.prepare()
        self.assertEqual(self.upgrade(fresh), {'state': 'current', 'changed': False})

    def test_changed_inventory_or_backup_never_writes_source(self):
        before, report = self.prepare()
        with self.assertRaisesRegex(ValueError, 'inventory_changed'):
            apply(report['inventory'], '0'*64)
        data = json.loads(Path(report['inventory']).read_bytes())
        with Path(data['backup']).open('ab') as stream:
            stream.write(b'changed backup')
        with self.assertRaisesRegex(ValueError, 'backup_changed'):
            self.upgrade(report)
        self.assertEqual(self.original(), before)

    def test_changed_source_keeps_new_business_and_original_receipts(self):
        _, report = self.prepare()
        db = sqlite3.connect(self.path)
        db.execute("INSERT INTO assets VALUES(999,X'CAFE')")
        db.commit()
        db.close()
        changed = self.original()
        with self.assertRaisesRegex(ValueError, 'changed_since_inventory'):
            self.upgrade(report)
        self.assertEqual(self.original(), changed)

    def test_external_foreign_key_never_cascades_business(self):
        db = sqlite3.connect(self.path)
        db.executescript("CREATE TABLE private_fk(value TEXT REFERENCES agent_root_tick_timeline_receipts(call_id) ON DELETE CASCADE); INSERT INTO private_fk VALUES('call');")
        db.close()
        before, report = self.prepare()
        with self.assertRaisesRegex(ValueError, 'external_foreign_key'):
            self.upgrade(report)
        self.assertEqual(self.original(), before)

    def test_postcondition_failure_rolls_back_schema_and_all_original_data(self):
        before, report = self.prepare()
        with patch('native_tick_migration_apply.postconditions', side_effect=ValueError('original proof withheld')):
            with self.assertRaisesRegex(ValueError, 'original proof withheld'):
                self.upgrade(report)
        self.assertEqual(self.original(), before)
        self.assertTrue(self.upgrade(report)['changed'])

    def test_process_death_before_commit_rolls_back_then_same_inventory_succeeds(self):
        import multiprocessing
        import threading
        before, report = self.prepare()
        context = multiprocessing.get_context('fork')
        receive, send = context.Pipe(duplex=False)
        def child():
            import native_tick_migration_apply as upgrade
            original = upgrade.postconditions
            def barrier(db, previous):
                result = original(db, previous)
                send.send('verified before commit')
                threading.Event().wait()
                return result
            with patch('native_tick_migration_apply.postconditions', side_effect=barrier):
                self.upgrade(report)
        process = context.Process(target=child)
        process.start()
        try:
            self.assertTrue(receive.poll(10), 'child never reached verified transaction')
            self.assertEqual(receive.recv(), 'verified before commit')
        finally:
            process.kill()
            process.join(10)
            receive.close()
            send.close()
        self.assertFalse(process.is_alive())
        self.assertEqual(self.original(), before)
        self.assertTrue(self.upgrade(report)['changed'])

    def test_private_writer_refuses_other_business_write_and_rolls_back(self):
        before, report = self.prepare()
        def corrupt(db, _):
            db.execute("UPDATE assets SET original=X'CAFEBABE'")
        with patch('native_tick_migration_apply.postconditions', side_effect=corrupt):
            with self.assertRaises(sqlite3.DatabaseError):
                self.upgrade(report)
        self.assertEqual(self.original(), before)

    def test_weakened_original_guard_is_not_adopted(self):
        db = sqlite3.connect(self.path)
        db.execute('DROP TRIGGER root_tick_no_update')
        db.close()
        before = self.original()
        with self.assertRaisesRegex(ValueError, 'schema_unrecognized'):
            inventory(self.path, self.root / 'backups')
        self.assertEqual(self.original(), before)

    def test_wal_source_backup_verifies_without_sidecar_files(self):
        # The app database is WAL; its online backup keeps the WAL header. The
        # backup must verify on every SQLite build (macOS's system libsqlite3
        # refuses mode=ro without -shm) and nothing may be written beside it.
        db = sqlite3.connect(self.path)
        self.assertEqual(db.execute('PRAGMA journal_mode=WAL').fetchone()[0], 'wal')
        db.close()
        before, report = self.prepare()
        backup = Path(json.loads(Path(report['inventory']).read_bytes())['backup'])
        self.assertEqual(sorted(p.name for p in backup.parent.iterdir()), ['database.sqlite3.backup', 'inventory.json'])
        self.assertEqual(self.upgrade(report), {'state': 'current', 'changed': True})
        self.assertEqual(sorted(p.name for p in backup.parent.iterdir()), ['database.sqlite3.backup', 'inventory.json'])
        self.assertEqual(self.original()['tables'], before['tables'])

    def test_upgrade_never_creates_tick_tables_outside_its_scope(self):
        db = sqlite3.connect(self.path)
        db.execute('DROP TABLE agent_root_tick_terminal_proofs')
        db.close()
        _, report = self.prepare()
        self.assertTrue(self.upgrade(report)['changed'])
        db = sqlite3.connect(self.path)
        names = {r[0] for r in db.execute("SELECT name FROM sqlite_master")}
        db.close()
        self.assertNotIn('agent_root_tick_terminal_proofs', names)

    def test_unrecognized_or_incomplete_trusted_ddl_fails_closed(self):
        from native_tick_migration_schema import SCHEMA_PATH as trusted, managed_statements
        text = trusted.read_text()
        with self.assertRaisesRegex(ValueError, 'trusted_ddl_unrecognized'):
            list(managed_statements(text + '\nCREATE INDEX IF NOT EXISTS sneaky ON agent_root_tick_receipts(round);\n'))
        with self.assertRaisesRegex(ValueError, 'trusted_ddl_unrecognized'):
            list(managed_statements(text + "\nCREATE TRIGGER IF NOT EXISTS sneaky BEFORE INSERT ON agent_root_tick_receipts BEGIN SELECT 1; END;\n"))
        with self.assertRaisesRegex(ValueError, 'trusted_ddl_incomplete'):
            list(managed_statements(re.sub(r'CREATE TRIGGER IF NOT EXISTS root_tick_no_update .*?END;', '', text, count=1, flags=re.S)))


if __name__ == '__main__':
    unittest.main()
