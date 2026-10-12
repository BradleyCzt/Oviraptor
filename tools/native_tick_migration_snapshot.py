"""Private, lossless inventory; never logs database values or executes saved SQL."""
import hashlib
import json
import sqlite3
import struct
from pathlib import Path


def quote(name):
    return '"' + name.replace('"', '""') + '"'


def file_hash(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def open_database(path, writable=False):
    path = Path(path).resolve(strict=True)
    try:
        db = sqlite3.connect(path.as_uri() + ('?mode=rw' if writable else '?mode=ro'),
                             uri=True, isolation_level=None, timeout=10)
        db.execute('SELECT count(*) FROM sqlite_master').fetchone()
    except sqlite3.OperationalError:
        # A closed WAL database has no -shm/-wal; some SQLite builds (macOS's
        # system libsqlite3) cannot open it mode=ro. Reopen read-write only so
        # SQLite may create its own WAL index; query_only below still forbids
        # every write and the live source may keep changing, so no immutable.
        if writable:
            raise
        db = sqlite3.connect(path.as_uri() + '?mode=rw', uri=True, isolation_level=None, timeout=10)
    db.text_factory = bytes
    db.execute('PRAGMA foreign_keys=ON')
    if not writable:
        db.execute('PRAGMA query_only=ON')
    return db


def open_backup(path):
    """Read a closed, private, hash-pinned backup file without sidecar files.

    The online backup copies the source's WAL header. Some SQLite builds (for
    example macOS's system libsqlite3) refuse a mode=ro open of a WAL database
    whose -shm/-wal do not exist, others create them. immutable=1 reads only the
    backup file itself, never takes locks and never writes next to it.
    """
    path = Path(path).resolve(strict=True)
    db = sqlite3.connect(path.as_uri() + '?mode=ro&immutable=1', uri=True, isolation_level=None)
    db.text_factory = bytes
    db.execute('PRAGMA foreign_keys=ON')
    db.execute('PRAGMA query_only=ON')
    return db


def catalogue(db):
    return [[v.decode() if isinstance(v, bytes) else v for v in row]
            for row in db.execute('SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name')]


def snapshot(db):
    schema = catalogue(db)
    tables = {}
    for kind, name, _, _ in schema:
        if kind != 'table':
            continue
        columns = list(db.execute('PRAGMA table_info(' + quote(name) + ')'))
        names = [c[1].decode() for c in columns]
        without_rowid = next(r[4] for r in db.execute('PRAGMA table_list')
                             if r[0] == b'main' and r[1].decode() == name)
        selected = names if without_rowid else ['rowid', *names]
        order = [c[1].decode() for c in sorted(columns, key=lambda c: c[5]) if c[5]] if without_rowid else ['rowid']
        expressions = ','.join('typeof(' + quote(c) + '),' + quote(c) for c in selected)
        digest = hashlib.sha256()
        count = 0
        for row in db.execute('SELECT ' + expressions + ' FROM ' + quote(name) + ' ORDER BY ' + ','.join(map(quote, order))):
            count += 1
            for i in range(0, len(row), 2):
                kind, value = row[i:i+2]
                raw = (b'' if value is None else struct.pack('>q', value) if kind == b'integer'
                       else struct.pack('>d', value) if kind == b'real' else value)
                digest.update(kind + struct.pack('>Q', len(raw)) + raw)
        tables[name] = {'count': count, 'columns': selected, 'sha256': digest.hexdigest()}
    return {'schema': schema, 'tables': tables,
            'foreign_key_check': [[v.decode() if isinstance(v, bytes) else v for v in row]
                                  for row in db.execute('PRAGMA foreign_key_check')]}


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()
