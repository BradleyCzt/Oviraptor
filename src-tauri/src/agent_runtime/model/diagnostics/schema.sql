-- SDK diagnostic metadata only. No prompt/response/reasoning/credentials/payload.
-- Never a dispatch, financial receipt or execution authority.
CREATE TABLE IF NOT EXISTS native_sdk_log_owners (
 owner_id TEXT PRIMARY KEY,
 domain TEXT NOT NULL CHECK(domain IN ('root','specialist','source_round')),
 dispatch_key TEXT NOT NULL,
 scan_id TEXT NOT NULL,
 attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
 root_run_id TEXT NOT NULL,
 run_id TEXT NOT NULL,
 assignment_id TEXT,
 lease_attempt_id TEXT NOT NULL,
 worker_id TEXT,
 round_number INTEGER NOT NULL CHECK(round_number>0),
 request_hash TEXT NOT NULL CHECK(length(request_hash)=64),
 identity_hash TEXT NOT NULL CHECK(length(identity_hash)=64),
 created_at TEXT NOT NULL,
 UNIQUE(domain,dispatch_key)
);
CREATE TABLE IF NOT EXISTS native_sdk_log_rows (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 owner_id TEXT NOT NULL REFERENCES native_sdk_log_owners(owner_id),
 ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 1 AND 6),
 stage TEXT NOT NULL CHECK(stage IN ('prepared','sent','response_received','cost_saved','validated','terminal')),
 cost_phase TEXT NOT NULL DEFAULT '' CHECK(cost_phase IN ('','received','uncertain','unsent')),
 terminal_state TEXT NOT NULL DEFAULT '' CHECK(terminal_state IN ('','returned','withheld','uncertain','unsent','failed')),
 created_at TEXT NOT NULL,
 UNIQUE(owner_id,ordinal), UNIQUE(owner_id,stage)
);
CREATE INDEX IF NOT EXISTS native_sdk_log_scope ON native_sdk_log_owners(scan_id,attempt_number,owner_id);
CREATE INDEX IF NOT EXISTS native_sdk_log_cursor ON native_sdk_log_rows(owner_id,sequence);
CREATE TRIGGER IF NOT EXISTS native_sdk_log_owner_no_update BEFORE UPDATE ON native_sdk_log_owners BEGIN SELECT RAISE(ABORT,'native_sdk_log_owner_immutable'); END;
CREATE TRIGGER IF NOT EXISTS native_sdk_log_owner_no_delete BEFORE DELETE ON native_sdk_log_owners BEGIN SELECT RAISE(ABORT,'native_sdk_log_owner_immutable'); END;
CREATE TRIGGER IF NOT EXISTS native_sdk_log_owner_no_replace BEFORE INSERT ON native_sdk_log_owners
 WHEN EXISTS(SELECT 1 FROM native_sdk_log_owners WHERE owner_id=NEW.owner_id OR (domain=NEW.domain AND dispatch_key=NEW.dispatch_key))
 BEGIN SELECT RAISE(ABORT,'native_sdk_log_owner_immutable'); END;
CREATE TRIGGER IF NOT EXISTS native_sdk_log_row_no_update BEFORE UPDATE ON native_sdk_log_rows BEGIN SELECT RAISE(ABORT,'native_sdk_log_row_immutable'); END;
CREATE TRIGGER IF NOT EXISTS native_sdk_log_row_no_delete BEFORE DELETE ON native_sdk_log_rows BEGIN SELECT RAISE(ABORT,'native_sdk_log_row_immutable'); END;
CREATE TRIGGER IF NOT EXISTS native_sdk_log_row_no_replace BEFORE INSERT ON native_sdk_log_rows
 WHEN EXISTS(SELECT 1 FROM native_sdk_log_rows WHERE sequence=NEW.sequence OR (owner_id=NEW.owner_id AND (ordinal=NEW.ordinal OR stage=NEW.stage)))
 BEGIN SELECT RAISE(ABORT,'native_sdk_log_row_immutable'); END;

-- A failed write is a separate safe gap fact, never a fabricated SDK stage.
CREATE TABLE IF NOT EXISTS native_sdk_log_gaps (
 gap_id INTEGER PRIMARY KEY AUTOINCREMENT,
 owner_id TEXT NOT NULL REFERENCES native_sdk_log_owners(owner_id),
 after_ordinal INTEGER NOT NULL CHECK(after_ordinal BETWEEN 0 AND 6),
 failed_stage TEXT NOT NULL CHECK(failed_stage IN ('prepared','sent','response_received','cost_saved','validated','terminal')),
 code TEXT NOT NULL CHECK(code='stage_write_failed'),
 created_at TEXT NOT NULL,
 UNIQUE(owner_id)
);
CREATE TRIGGER IF NOT EXISTS native_sdk_log_gap_no_update BEFORE UPDATE ON native_sdk_log_gaps BEGIN SELECT RAISE(ABORT,'native_sdk_log_gap_immutable'); END;
CREATE TRIGGER IF NOT EXISTS native_sdk_log_gap_no_delete BEFORE DELETE ON native_sdk_log_gaps BEGIN SELECT RAISE(ABORT,'native_sdk_log_gap_immutable'); END;
CREATE TRIGGER IF NOT EXISTS native_sdk_log_gap_no_replace BEFORE INSERT ON native_sdk_log_gaps
 WHEN EXISTS(SELECT 1 FROM native_sdk_log_gaps WHERE gap_id=NEW.gap_id OR owner_id=NEW.owner_id)
 BEGIN SELECT RAISE(ABORT,'native_sdk_log_gap_immutable'); END;

-- Additive logical Mapper ownership; original owner family/cursors stay unchanged.
CREATE TABLE IF NOT EXISTS native_sdk_log_mapper_owners (
 owner_id TEXT PRIMARY KEY REFERENCES native_sdk_log_owners(owner_id),
 logical_domain TEXT NOT NULL CHECK(logical_domain='mapper_loop'),
 identity_hash TEXT NOT NULL CHECK(length(identity_hash)=64)
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS native_sdk_log_mapper_no_update BEFORE UPDATE ON native_sdk_log_mapper_owners BEGIN SELECT RAISE(ABORT,'native_sdk_log_mapper_immutable'); END;
CREATE TRIGGER IF NOT EXISTS native_sdk_log_mapper_no_delete BEFORE DELETE ON native_sdk_log_mapper_owners BEGIN SELECT RAISE(ABORT,'native_sdk_log_mapper_immutable'); END;
CREATE TRIGGER IF NOT EXISTS native_sdk_log_mapper_no_replace BEFORE INSERT ON native_sdk_log_mapper_owners WHEN EXISTS(SELECT 1 FROM native_sdk_log_mapper_owners WHERE owner_id=NEW.owner_id) BEGIN SELECT RAISE(ABORT,'native_sdk_log_mapper_immutable'); END;
