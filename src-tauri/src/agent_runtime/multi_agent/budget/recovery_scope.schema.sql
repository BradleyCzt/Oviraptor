-- Dedicated recovery scan totals. No upgrade of existing Root budgets.
CREATE TABLE IF NOT EXISTS agent_recovery_scope_owners(
 owner_id TEXT PRIMARY KEY NOT NULL,scan_id TEXT NOT NULL UNIQUE,
 original_contract_json TEXT NOT NULL,original_contract_hash TEXT NOT NULL,
 limits_json TEXT NOT NULL,max_approved_usd_nanos INTEGER NOT NULL CHECK(max_approved_usd_nanos>=0)
);
CREATE TABLE IF NOT EXISTS agent_recovery_scope_root_bindings(
 root_run_id TEXT PRIMARY KEY NOT NULL,owner_id TEXT NOT NULL REFERENCES agent_recovery_scope_owners(owner_id),
 original_birth_hash TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS agent_recovery_scope_tickets(
 ticket_id TEXT PRIMARY KEY NOT NULL,owner_id TEXT NOT NULL REFERENCES agent_recovery_scope_owners(owner_id),
 root_run_id TEXT NOT NULL REFERENCES agent_recovery_scope_root_bindings(root_run_id),
 original_source_hash TEXT NOT NULL,reserved_json TEXT NOT NULL,reserved_approved_usd_nanos INTEGER NOT NULL CHECK(reserved_approved_usd_nanos>=0),
 UNIQUE(owner_id,original_source_hash)
);
CREATE TABLE IF NOT EXISTS agent_recovery_scope_entries(
 ticket_id TEXT NOT NULL REFERENCES agent_recovery_scope_tickets(ticket_id),
 dimension TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN('reserve','release','consume','forfeit')),
 amount INTEGER NOT NULL CHECK(amount>=0),original_source_hash TEXT NOT NULL,
 PRIMARY KEY(ticket_id,dimension,kind)
);
CREATE TABLE IF NOT EXISTS agent_recovery_scope_terminals(
 ticket_id TEXT PRIMARY KEY NOT NULL REFERENCES agent_recovery_scope_tickets(ticket_id),
 phase TEXT NOT NULL CHECK(phase IN('known','unknown')),original_source_hash TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS agent_recovery_scope_stops(
 owner_id TEXT NOT NULL REFERENCES agent_recovery_scope_owners(owner_id),ticket_id TEXT NOT NULL,
 cause TEXT NOT NULL CHECK(cause IN('unknown_cost','approved_limit_exceeded')),
 PRIMARY KEY(owner_id,ticket_id)
);
CREATE TRIGGER IF NOT EXISTS recovery_scope_owners_update BEFORE UPDATE ON agent_recovery_scope_owners BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_owners_delete BEFORE DELETE ON agent_recovery_scope_owners BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_root_bindings_update BEFORE UPDATE ON agent_recovery_scope_root_bindings BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_root_bindings_delete BEFORE DELETE ON agent_recovery_scope_root_bindings BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_tickets_update BEFORE UPDATE ON agent_recovery_scope_tickets BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_tickets_delete BEFORE DELETE ON agent_recovery_scope_tickets BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_entries_update BEFORE UPDATE ON agent_recovery_scope_entries BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_entries_delete BEFORE DELETE ON agent_recovery_scope_entries BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_terminals_update BEFORE UPDATE ON agent_recovery_scope_terminals BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_terminals_delete BEFORE DELETE ON agent_recovery_scope_terminals BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_stops_update BEFORE UPDATE ON agent_recovery_scope_stops BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_stops_delete BEFORE DELETE ON agent_recovery_scope_stops BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_owners_no_replace BEFORE INSERT ON agent_recovery_scope_owners WHEN EXISTS(SELECT 1 FROM agent_recovery_scope_owners WHERE owner_id=NEW.owner_id OR scan_id=NEW.scan_id) BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_root_bindings_no_replace BEFORE INSERT ON agent_recovery_scope_root_bindings WHEN EXISTS(SELECT 1 FROM agent_recovery_scope_root_bindings WHERE root_run_id=NEW.root_run_id) BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_tickets_no_replace BEFORE INSERT ON agent_recovery_scope_tickets WHEN EXISTS(SELECT 1 FROM agent_recovery_scope_tickets WHERE ticket_id=NEW.ticket_id OR (owner_id=NEW.owner_id AND original_source_hash=NEW.original_source_hash)) BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_entries_no_replace BEFORE INSERT ON agent_recovery_scope_entries WHEN EXISTS(SELECT 1 FROM agent_recovery_scope_entries WHERE ticket_id=NEW.ticket_id AND dimension=NEW.dimension AND kind=NEW.kind) BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_terminals_no_replace BEFORE INSERT ON agent_recovery_scope_terminals WHEN EXISTS(SELECT 1 FROM agent_recovery_scope_terminals WHERE ticket_id=NEW.ticket_id) BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS recovery_scope_stops_no_replace BEFORE INSERT ON agent_recovery_scope_stops WHEN EXISTS(SELECT 1 FROM agent_recovery_scope_stops WHERE owner_id=NEW.owner_id AND ticket_id=NEW.ticket_id) BEGIN SELECT RAISE(ABORT,'recovery_scope_immutable'); END;
