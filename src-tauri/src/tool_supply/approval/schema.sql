-- Additive task-external decisions; no default trust or approval rows.
CREATE TABLE IF NOT EXISTS tool_administrator_decisions (
 decision_id TEXT PRIMARY KEY NOT NULL CHECK(length(decision_id)=36),
 kind TEXT NOT NULL CHECK(kind IN ('approve','revoke')),
 approval_id TEXT NOT NULL CHECK(length(approval_id)=36),
 administrator_id TEXT NOT NULL,
 canonical_envelope TEXT NOT NULL,
 envelope_hash TEXT NOT NULL CHECK(length(envelope_hash)=64),
 canonical_manifest TEXT NOT NULL,
 created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
 CHECK((kind='approve' AND approval_id=decision_id) OR (kind='revoke' AND approval_id<>decision_id))
);
CREATE UNIQUE INDEX IF NOT EXISTS tool_administrator_one_approval ON tool_administrator_decisions(approval_id) WHERE kind='approve';
CREATE UNIQUE INDEX IF NOT EXISTS tool_administrator_one_revocation ON tool_administrator_decisions(approval_id) WHERE kind='revoke';
CREATE TRIGGER IF NOT EXISTS tool_administrator_no_update BEFORE UPDATE ON tool_administrator_decisions BEGIN SELECT RAISE(ABORT,'tool_administrator_decision_immutable'); END;
CREATE TRIGGER IF NOT EXISTS tool_administrator_no_delete BEFORE DELETE ON tool_administrator_decisions BEGIN SELECT RAISE(ABORT,'tool_administrator_decision_immutable'); END;
CREATE TRIGGER IF NOT EXISTS tool_administrator_no_replace BEFORE INSERT ON tool_administrator_decisions WHEN EXISTS(SELECT 1 FROM tool_administrator_decisions WHERE decision_id=NEW.decision_id OR (kind=NEW.kind AND approval_id=NEW.approval_id)) BEGIN SELECT RAISE(ABORT,'tool_administrator_decision_immutable'); END;
