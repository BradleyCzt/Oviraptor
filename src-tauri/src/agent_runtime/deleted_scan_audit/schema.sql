-- Original task-scoped physical sources. Never an executable runtime history.
CREATE TABLE IF NOT EXISTS native_deleted_scan_audits (
 scan_id TEXT PRIMARY KEY,
 version INTEGER NOT NULL CHECK(version=1),
 audit_json TEXT NOT NULL CHECK(json_valid(audit_json)),
 audit_hash TEXT NOT NULL CHECK(length(audit_hash)=64),
 deleted_at TEXT NOT NULL
);
CREATE TRIGGER IF NOT EXISTS deleted_audit_no_update BEFORE UPDATE ON native_deleted_scan_audits
 BEGIN SELECT RAISE(ABORT,'deleted_audit_immutable'); END;
CREATE TRIGGER IF NOT EXISTS deleted_audit_no_delete BEFORE DELETE ON native_deleted_scan_audits
 BEGIN SELECT RAISE(ABORT,'deleted_audit_immutable'); END;
CREATE TRIGGER IF NOT EXISTS deleted_audit_no_replace BEFORE INSERT ON native_deleted_scan_audits
 WHEN EXISTS(SELECT 1 FROM native_deleted_scan_audits WHERE scan_id=NEW.scan_id)
 BEGIN SELECT RAISE(ABORT,'deleted_audit_immutable'); END;

-- Independent archive identity pins all physical sources, including workers.
-- It references permanent financial controls, never live task/run authority.
CREATE TABLE IF NOT EXISTS native_deleted_scan_anchors (
 root_run_id TEXT PRIMARY KEY REFERENCES agent_root_budget_attempts(root_run_id) ON DELETE RESTRICT,
 control_id TEXT NOT NULL REFERENCES agent_root_budget_attempts(id) ON DELETE RESTRICT,
 scan_id TEXT NOT NULL,
 audit_hash TEXT NOT NULL CHECK(length(audit_hash)=64),
 deleted_at TEXT NOT NULL
);
CREATE TRIGGER IF NOT EXISTS deleted_anchor_no_update BEFORE UPDATE ON native_deleted_scan_anchors
 BEGIN SELECT RAISE(ABORT,'deleted_anchor_immutable'); END;
CREATE TRIGGER IF NOT EXISTS deleted_anchor_no_delete BEFORE DELETE ON native_deleted_scan_anchors
 BEGIN SELECT RAISE(ABORT,'deleted_anchor_immutable'); END;
CREATE TRIGGER IF NOT EXISTS deleted_anchor_no_replace BEFORE INSERT ON native_deleted_scan_anchors
 WHEN EXISTS(SELECT 1 FROM native_deleted_scan_anchors WHERE root_run_id=NEW.root_run_id)
 BEGIN SELECT RAISE(ABORT,'deleted_anchor_immutable'); END;

-- Additive v2 identity-aware archives. Original v1 rows and hashes are untouched.
CREATE TABLE IF NOT EXISTS native_deleted_scan_audits_v2 (
 scan_id TEXT PRIMARY KEY,
 version INTEGER NOT NULL CHECK(version=2),
 audit_json TEXT NOT NULL CHECK(json_valid(audit_json)),
 audit_hash TEXT NOT NULL CHECK(length(audit_hash)=64),
 deleted_at TEXT NOT NULL
);
CREATE TRIGGER IF NOT EXISTS deleted_audit_v2_no_update BEFORE UPDATE ON native_deleted_scan_audits_v2
 BEGIN SELECT RAISE(ABORT,'deleted_audit_immutable'); END;
CREATE TRIGGER IF NOT EXISTS deleted_audit_v2_no_delete BEFORE DELETE ON native_deleted_scan_audits_v2
 BEGIN SELECT RAISE(ABORT,'deleted_audit_immutable'); END;
CREATE TRIGGER IF NOT EXISTS deleted_audit_v2_no_replace BEFORE INSERT ON native_deleted_scan_audits_v2
 WHEN EXISTS(SELECT 1 FROM native_deleted_scan_audits_v2 WHERE scan_id=NEW.scan_id)
   OR EXISTS(SELECT 1 FROM native_deleted_scan_audits WHERE scan_id=NEW.scan_id)
 BEGIN SELECT RAISE(ABORT,'deleted_audit_immutable'); END;
CREATE TRIGGER IF NOT EXISTS deleted_audit_v1_v2_conflict BEFORE INSERT ON native_deleted_scan_audits
 WHEN EXISTS(SELECT 1 FROM native_deleted_scan_audits_v2 WHERE scan_id=NEW.scan_id)
 BEGIN SELECT RAISE(ABORT,'deleted_audit_immutable'); END;
