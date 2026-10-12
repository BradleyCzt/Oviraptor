//! New append-only accounting; existing summary rows are never backfilled.
//! Audit IDs outlive explicit task deletion. Live authority always requires a
//! still-bound run and assignment; these rows cannot resurrect deleted work.
pub(crate) const SCHEMA: &str = concat!(
    r#"
CREATE TABLE IF NOT EXISTS agent_budget_limits (
 root_run_id TEXT NOT NULL,
 dimension TEXT NOT NULL,
 hard_limit INTEGER CHECK(hard_limit IS NULL OR hard_limit>=0),
 PRIMARY KEY(root_run_id,dimension)
);
CREATE TABLE IF NOT EXISTS agent_budget_entries (
 entry_id TEXT PRIMARY KEY,
 root_run_id TEXT NOT NULL,
 assignment_id TEXT NOT NULL,
 lease_attempt_id TEXT NOT NULL,
 dimension TEXT NOT NULL CHECK(dimension IN (
 'model_input_tokens','model_cached_tokens','model_output_tokens','model_requests',
 'target_requests','browser_actions','controlled_writes','upload_bytes','concurrency_batches','wall_time_ms')),
 kind TEXT NOT NULL CHECK(kind IN ('reserve','consume','release','forfeit','reconcile')),
 amount INTEGER NOT NULL CHECK(amount>0),
 idempotency_key TEXT NOT NULL,
 source_id TEXT NOT NULL,
 created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
 UNIQUE(root_run_id,idempotency_key),
 FOREIGN KEY(root_run_id,dimension) REFERENCES agent_budget_limits(root_run_id,dimension)
);
CREATE TABLE IF NOT EXISTS agent_budget_clock_origins (
 root_run_id TEXT PRIMARY KEY,
 started_at TEXT NOT NULL
);
CREATE TRIGGER IF NOT EXISTS budget_clock_no_update BEFORE UPDATE ON agent_budget_clock_origins
 BEGIN SELECT RAISE(ABORT,'budget_clock_immutable'); END;
CREATE TRIGGER IF NOT EXISTS budget_clock_no_delete BEFORE DELETE ON agent_budget_clock_origins
 BEGIN SELECT RAISE(ABORT,'budget_clock_immutable'); END;
CREATE INDEX IF NOT EXISTS idx_budget_entries_assignment
 ON agent_budget_entries(root_run_id,assignment_id,dimension);
CREATE TRIGGER IF NOT EXISTS budget_entry_no_update BEFORE UPDATE ON agent_budget_entries
 BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END;
CREATE TRIGGER IF NOT EXISTS budget_entry_no_delete BEFORE DELETE ON agent_budget_entries
 BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END;
CREATE TRIGGER IF NOT EXISTS budget_limit_no_update BEFORE UPDATE ON agent_budget_limits
 BEGIN SELECT RAISE(ABORT,'budget_limit_immutable'); END;
CREATE TRIGGER IF NOT EXISTS budget_limit_no_delete BEFORE DELETE ON agent_budget_limits
 BEGIN SELECT RAISE(ABORT,'budget_limit_immutable'); END;
"#,
    include_str!("elapsed_fact_schema.sql"),
    include_str!("immutable_unique_guards.sql"),
    include_str!("web_model_schema.sql"),
    include_str!("root_schema.sql"),
    include_str!("root_cache_provenance_schema.sql"),
    include_str!("root_cache_capture_schema.sql"),
    include_str!("root_paid_run_retention_schema.sql"),
    include_str!("root_definition_schema.sql"),
    include_str!("financial_root_definition_schema.sql"),
    include_str!("financial_continuation_schema.sql"),
    include_str!("model_facts_schema.sql"),
    include_str!("single_exit_schema.sql"),
    include_str!("single_projection_schema.sql")
);
