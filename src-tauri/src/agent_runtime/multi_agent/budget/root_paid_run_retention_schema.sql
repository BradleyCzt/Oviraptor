-- Kept out of root_schema.sql on purpose: that file is the self-contained
-- trusted definition of the original control (the offline tick migration builds
-- it alone in memory), while this guard needs the agent_runs table.
-- Tick/timeline FKs now pin the permanent control (so a closed paid task can
-- be archived), which also stopped protecting a still-open paid Root run from
-- a direct DELETE/cascade. An open paid Root's run, events and worker sources
-- are the only basis for verifying its in-flight cost: never delete them before
-- the original exit. Closed Roots keep using the audited archive deletion.
CREATE TRIGGER IF NOT EXISTS root_paid_open_run_retained BEFORE DELETE ON agent_runs
 WHEN OLD.role='coordinator' AND OLD.root_run_id=OLD.id AND OLD.status<>'terminal'
  AND EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=OLD.id)
 BEGIN SELECT RAISE(ABORT,'root_paid_open_run_retained'); END;
