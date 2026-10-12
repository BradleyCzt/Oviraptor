CREATE TABLE IF NOT EXISTS agent_frozen_tool_supply_plans (
 plan_id TEXT PRIMARY KEY NOT NULL CHECK(length(plan_id)=36),
 root_run_id TEXT NOT NULL UNIQUE,
 scan_id TEXT NOT NULL,
 attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
 native_plan_text_hash TEXT NOT NULL CHECK(length(native_plan_text_hash)=64),
 fact_json TEXT NOT NULL,
 fact_hash TEXT NOT NULL CHECK(length(fact_hash)=64),
 signature BLOB NOT NULL CHECK(length(signature)=32),
 created_at TEXT NOT NULL DEFAULT(datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS frozen_tool_supply_no_update BEFORE UPDATE ON agent_frozen_tool_supply_plans BEGIN SELECT RAISE(ABORT,'frozen_tool_supply_plan_immutable'); END;
CREATE TRIGGER IF NOT EXISTS frozen_tool_supply_no_delete BEFORE DELETE ON agent_frozen_tool_supply_plans BEGIN SELECT RAISE(ABORT,'frozen_tool_supply_plan_immutable'); END;
CREATE TRIGGER IF NOT EXISTS frozen_tool_supply_no_replace BEFORE INSERT ON agent_frozen_tool_supply_plans WHEN EXISTS(SELECT 1 FROM agent_frozen_tool_supply_plans WHERE plan_id=NEW.plan_id OR root_run_id=NEW.root_run_id) BEGIN SELECT RAISE(ABORT,'frozen_tool_supply_plan_immutable'); END;
