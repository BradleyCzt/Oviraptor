-- Original paid Worker response -> one immutable offline tool invocation.
-- This is an execution receipt, not a new supervisor action or intention.
CREATE TABLE IF NOT EXISTS agent_homogeneous_parser_calls (
  invocation_id TEXT PRIMARY KEY,
  root_run_id TEXT NOT NULL,
  assignment_id TEXT NOT NULL,
  child_run_id TEXT NOT NULL,
  assignment_attempt INTEGER NOT NULL,
  fact_json TEXT NOT NULL,
  fact_hash TEXT NOT NULL,
  signature BLOB NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
  UNIQUE(assignment_id, assignment_attempt)
);
CREATE TABLE IF NOT EXISTS agent_homogeneous_parser_results (
  invocation_id TEXT PRIMARY KEY,
  root_run_id TEXT NOT NULL,
  assignment_id TEXT NOT NULL,
  child_run_id TEXT NOT NULL,
  fact_json TEXT NOT NULL,
  fact_hash TEXT NOT NULL,
  signature BLOB NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS homogeneous_parser_call_no_update BEFORE UPDATE ON agent_homogeneous_parser_calls
BEGIN SELECT RAISE(ABORT,'homogeneous_parser_call_immutable'); END;
CREATE TRIGGER IF NOT EXISTS homogeneous_parser_call_no_delete BEFORE DELETE ON agent_homogeneous_parser_calls
BEGIN SELECT RAISE(ABORT,'homogeneous_parser_call_immutable'); END;
CREATE TRIGGER IF NOT EXISTS homogeneous_parser_call_no_replace BEFORE INSERT ON agent_homogeneous_parser_calls
WHEN EXISTS(SELECT 1 FROM agent_homogeneous_parser_calls WHERE invocation_id=NEW.invocation_id OR (assignment_id=NEW.assignment_id AND assignment_attempt=NEW.assignment_attempt))
BEGIN SELECT RAISE(ABORT,'homogeneous_parser_call_immutable'); END;
CREATE TRIGGER IF NOT EXISTS homogeneous_parser_result_no_update BEFORE UPDATE ON agent_homogeneous_parser_results
BEGIN SELECT RAISE(ABORT,'homogeneous_parser_result_immutable'); END;
CREATE TRIGGER IF NOT EXISTS homogeneous_parser_result_no_delete BEFORE DELETE ON agent_homogeneous_parser_results
BEGIN SELECT RAISE(ABORT,'homogeneous_parser_result_immutable'); END;
CREATE TRIGGER IF NOT EXISTS homogeneous_parser_result_no_replace BEFORE INSERT ON agent_homogeneous_parser_results
WHEN EXISTS(SELECT 1 FROM agent_homogeneous_parser_results WHERE invocation_id=NEW.invocation_id)
BEGIN SELECT RAISE(ABORT,'homogeneous_parser_result_immutable'); END;
