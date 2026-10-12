CREATE TABLE IF NOT EXISTS agent_homogeneous_execution_facts (
 id TEXT PRIMARY KEY, root_run_id TEXT NOT NULL, assignment_id TEXT NOT NULL,
 attempt INTEGER NOT NULL CHECK(attempt>0), kind TEXT NOT NULL CHECK(kind IN ('target','review')),
 fact_json TEXT NOT NULL, fact_hash TEXT NOT NULL, signature BLOB NOT NULL,
 UNIQUE(assignment_id,attempt,kind));
CREATE TRIGGER IF NOT EXISTS homogeneous_execution_fact_immutable_update BEFORE UPDATE ON agent_homogeneous_execution_facts BEGIN SELECT RAISE(ABORT,'homogeneous_execution_fact_immutable'); END;
CREATE TRIGGER IF NOT EXISTS homogeneous_execution_fact_immutable_delete BEFORE DELETE ON agent_homogeneous_execution_facts BEGIN SELECT RAISE(ABORT,'homogeneous_execution_fact_immutable'); END;
CREATE TRIGGER IF NOT EXISTS homogeneous_execution_fact_immutable_replace BEFORE INSERT ON agent_homogeneous_execution_facts WHEN EXISTS(SELECT 1 FROM agent_homogeneous_execution_facts WHERE id=NEW.id OR (assignment_id=NEW.assignment_id AND attempt=NEW.attempt AND kind=NEW.kind)) BEGIN SELECT RAISE(ABORT,'homogeneous_execution_fact_immutable'); END;
