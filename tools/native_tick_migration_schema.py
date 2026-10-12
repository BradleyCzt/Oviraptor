"""Recognize exactly the two reviewed Native receipt schemas, with every guard."""
import re
import sqlite3
from pathlib import Path
from native_tick_migration_snapshot import catalogue, quote

SCHEMA_PATH = Path(__file__).resolve().parents[1] / 'src-tauri/src/agent_runtime/multi_agent/budget/root/model/tick/schema.sql'
TABLES = ('agent_root_tick_receipts', 'agent_root_tick_timeline_receipts')
GUARDS = ('root_tick_no_update', 'root_tick_no_delete', 'root_tick_no_replace',
          'root_tick_timeline_no_update', 'root_tick_timeline_no_delete', 'root_tick_timeline_no_replace')


def statements(sql):
    pending = ''
    for line in sql.splitlines(True):
        pending += line
        if sqlite3.complete_statement(pending):
            yield pending
            pending = ''
    if pending.strip():
        raise ValueError('native_tick_trusted_ddl_incomplete')


def managed_statements(sql):
    """Only the two receipt tables and their six guards are this tool's scope.

    The trusted tick DDL also defines other Root tables (for example terminal
    proofs). Those are created by the app itself and are outside the private
    writer's authority, so they are skipped here; anything unrecognized fails
    closed, and every managed table/guard must be present exactly once.
    """
    seen = []
    for statement in statements(sql):
        body = ' '.join(line for line in statement.splitlines() if not line.lstrip().startswith('--')).strip()
        if not body:
            continue
        table = re.match(r'CREATE TABLE IF NOT EXISTS (\w+)\s*\(', body)
        trigger = re.match(r'CREATE TRIGGER IF NOT EXISTS (\w+)\s.*?\bON (\w+)\b', body, re.S)
        if table:
            name, managed_object = table.group(1), table.group(1) in TABLES
        elif trigger:
            name = trigger.group(1)
            managed_object = trigger.group(2) in TABLES
            if managed_object and name not in GUARDS:
                raise ValueError('native_tick_trusted_ddl_unrecognized')
        else:
            raise ValueError('native_tick_trusted_ddl_unrecognized')
        if managed_object:
            seen.append(name)
            yield statement
    if sorted(seen) != sorted((*TABLES, *GUARDS)):
        raise ValueError('native_tick_trusted_ddl_incomplete')


def managed(schema):
    return [r for r in schema if r[2] in TABLES]


def expected(old=False):
    sql = SCHEMA_PATH.read_text()
    if old:
        sql = sql.replace('REFERENCES agent_root_budget_attempts(root_run_id)', 'REFERENCES agent_runs(id)')
    db = sqlite3.connect(':memory:')
    db.text_factory = bytes
    db.executescript(sql)
    result = managed(catalogue(db))
    db.close()
    return result


def classify(schema):
    actual = managed(schema)
    if not actual:
        return 'not_applicable'
    if actual == expected():
        return 'current'
    if actual == expected(old=True):
        return 'original_run_identity'
    raise ValueError('native_tick_schema_unrecognized')


def require_scope(db):
    control = 'agent_root_budget_attempts'
    trusted = sqlite3.connect(':memory:')
    trusted.text_factory = bytes
    trusted.executescript((SCHEMA_PATH.parents[3] / 'root_schema.sql').read_text())
    try:
        if [r for r in catalogue(db) if r[2] == control] != [r for r in catalogue(trusted) if r[2] == control]:
            raise ValueError('native_tick_original_control_guards_unrecognized')
    finally:
        trusted.close()
    for kind, name, _, _ in catalogue(db):
        if kind == 'table' and name not in TABLES:
            if any(r[2].decode() in TABLES for r in db.execute('PRAGMA foreign_key_list(' + quote(name) + ')')):
                raise ValueError('native_tick_external_foreign_key')
    if not db.execute("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='agent_root_budget_attempts'").fetchone()[0]:
        raise ValueError('native_tick_original_financial_control_missing')
    for table in TABLES:
        if db.execute('SELECT EXISTS(SELECT 1 FROM ' + quote(table) + ' r WHERE NOT EXISTS('
                      'SELECT 1 FROM agent_root_budget_attempts b WHERE b.root_run_id=r.root_run_id AND b.id=r.lease_attempt_id))').fetchone()[0]:
            raise ValueError('native_tick_original_financial_identity_mismatch')
