#[path = "collaboration_events/client_delivery_schema.rs"]
pub(crate) mod client_delivery_schema;
#[path = "collaboration_events/client_grant_schema.rs"]
pub(crate) mod client_grant_schema;
use rusqlite::Connection;
#[path = "collaboration_events/web_creation_schema.rs"]
pub(crate) mod web_creation_schema;
#[path = "collaboration_events/closure_schema.rs"]
pub(crate) mod closure_schema;
#[path = "collaboration_events/dispatch_schema.rs"]
pub(crate) mod dispatch_schema;
#[path = "collaboration_events/source_creation_schema.rs"]
pub(crate) mod source_creation_schema;

#[path = "collaboration_events/pump.rs"]
mod pump;
#[path = "collaboration_events/signal.rs"]
mod signal;

pub(crate) use pump::{start_event_pump, EventPumpControl};
pub(crate) use signal::install_commit_notifications;

#[cfg(test)]
#[path = "collaboration_events/tests_support.rs"]
mod tests_support;

const COLLABORATION_EVENT_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agent_collaboration_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL DEFAULT '',
    attempt_number INTEGER NOT NULL DEFAULT 0,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS agent_collaboration_homogeneous_guidance_insert
AFTER INSERT ON agent_human_directive_gated BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  SELECT scan_id,attempt_number,'homogeneous_guidance',NEW.directive_id,'homogeneous_guidance',json_object('state',NEW.state)
  FROM agent_homogeneous_tasks WHERE task_id=NEW.parent_task_id;
END;
CREATE TRIGGER IF NOT EXISTS agent_collaboration_homogeneous_guidance_update
AFTER UPDATE ON agent_human_directive_gated WHEN OLD.state<>NEW.state OR OLD.user_feedback_zh<>NEW.user_feedback_zh BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  SELECT scan_id,attempt_number,'homogeneous_guidance',NEW.directive_id,'homogeneous_guidance',json_object('state',NEW.state)
  FROM agent_homogeneous_tasks WHERE task_id=NEW.parent_task_id;
END;
CREATE TRIGGER IF NOT EXISTS agent_collaboration_homogeneous_report
AFTER INSERT ON agent_worker_reports BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  SELECT t.scan_id,t.attempt_number,'worker_report',NEW.report_id,'worker_report',
    json_object('assignmentId',NEW.assignment_id,'sliceId',NEW.slice_id,'workerId',NEW.worker_id)
  FROM agent_assignments a JOIN agent_homogeneous_tasks t ON t.root_run_id=a.coordinator_run_id
  WHERE a.id=NEW.assignment_id AND a.scheduling_model='homogeneous_workers';
END;
CREATE TRIGGER IF NOT EXISTS agent_collaboration_homogeneous_decision
AFTER INSERT ON agent_homogeneous_decision_audit BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  SELECT scan_id,attempt_number,'homogeneous_decision',CAST(NEW.audit_id AS TEXT),'homogeneous_decision',
    json_object('decisionId',NEW.decision_id,'action',NEW.action,'stopReason',NEW.stop_reason)
  FROM agent_homogeneous_tasks WHERE task_id=NEW.parent_task_id;
END;
CREATE INDEX IF NOT EXISTS idx_agent_collaboration_events_scan
    ON agent_collaboration_events(scan_id,attempt_number,sequence);
CREATE INDEX IF NOT EXISTS idx_agent_collaboration_events_entity
    ON agent_collaboration_events(event_type,entity_id,sequence DESC);
CREATE INDEX IF NOT EXISTS idx_agent_collaboration_events_entity_identity
    ON agent_collaboration_events(entity_id);

-- Durable queue preferences are distinct from model delivery and scan results.
-- Added through ensure_schema for existing databases as well as new databases.
CREATE TABLE IF NOT EXISTS agent_directive_queue_actions (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    directive_id TEXT NOT NULL UNIQUE REFERENCES agent_user_directives(id) ON DELETE CASCADE,
    family TEXT NOT NULL CHECK(family IN ('information_disclosure','error_handling',
      'authentication_session','authorization','input_reflection_xss','hidden_interface_discovery','business_flow')),
    receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json) AND json_type(receipt_json)='object'),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS agent_directive_queue_action_immutable
BEFORE UPDATE ON agent_directive_queue_actions
BEGIN
  SELECT RAISE(ABORT,'directive_queue_action_immutable');
END;

-- An assessment request owns one frozen assignment and one model dispatch.
-- `executing`/`uncertain` must never be retried automatically: the provider
-- may have billed a request whose result was lost before the local commit.
CREATE TABLE IF NOT EXISTS agent_directive_proposals (
    directive_id TEXT PRIMARY KEY REFERENCES agent_user_directives(id) ON DELETE CASCADE,
    assignment_id TEXT NOT NULL UNIQUE REFERENCES agent_assignments(id),
    child_run_id TEXT NOT NULL UNIQUE REFERENCES agent_runs(id),
    request_message_id TEXT NOT NULL REFERENCES agent_messages(id),
    state TEXT NOT NULL DEFAULT 'prepared'
      CHECK(state IN ('prepared','executing','uncertain','received','completed','failed')),
    response_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(response_json)),
    usage_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(usage_json)),
    error_code TEXT NOT NULL DEFAULT '',
    result_message_id TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS agent_directive_proposal_binding_immutable
BEFORE UPDATE OF directive_id,assignment_id,child_run_id,request_message_id ON agent_directive_proposals
BEGIN
  SELECT RAISE(ABORT,'directive_proposal_binding_immutable');
END;
CREATE TRIGGER IF NOT EXISTS agent_directive_proposal_response_immutable
BEFORE UPDATE OF response_json,usage_json ON agent_directive_proposals
WHEN OLD.state<>'executing' OR NEW.state<>'received'
BEGIN
  SELECT RAISE(ABORT,'directive_proposal_response_immutable');
END;
-- Replace only the superseded guard; v2 adds a proven pre-dispatch cancellation.
DROP TRIGGER IF EXISTS agent_directive_proposal_state_guard;
CREATE TRIGGER IF NOT EXISTS agent_directive_proposal_state_guard_v2
BEFORE UPDATE OF state ON agent_directive_proposals
WHEN NOT ((OLD.state='prepared' AND NEW.state='executing')
  OR (OLD.state='prepared' AND NEW.state='failed' AND NEW.error_code='proposal_task_ended_before_dispatch'
    AND EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
      WHERE a.id=NEW.assignment_id AND r.id=NEW.child_run_id AND a.state='cancelled'
      AND a.started_at='' AND r.status='terminal' AND r.terminal_state='cancelled'))
  OR (OLD.state='executing' AND NEW.state IN ('uncertain','received'))
  OR (OLD.state='received' AND NEW.state IN ('completed','failed')))
BEGIN
  SELECT RAISE(ABORT,'directive_proposal_state_conflict');
END;
CREATE TRIGGER IF NOT EXISTS agent_directive_task_closure_immutable
BEFORE UPDATE OF payload_json ON agent_user_directives
WHEN json_extract(OLD.payload_json,'$.taskClosure') IS NOT NULL
  AND json_extract(OLD.payload_json,'$.taskClosure') IS NOT json_extract(NEW.payload_json,'$.taskClosure')
BEGIN
  SELECT RAISE(ABORT,'directive_task_closure_immutable');
END;
CREATE TRIGGER IF NOT EXISTS agent_collaboration_directive_proposal
AFTER UPDATE ON agent_directive_proposals
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  SELECT scan_id,attempt_number,'user_directive',id,'user_directive',
    json_object('proposalState',NEW.state) FROM agent_user_directives WHERE id=NEW.directive_id;
END;

CREATE TRIGGER IF NOT EXISTS agent_directive_local_reconciliation_immutable
BEFORE UPDATE OF payload_json ON agent_user_directives
WHEN json_extract(OLD.payload_json,'$.localReconciliation') IS NOT NULL
  AND json_extract(OLD.payload_json,'$.localReconciliation') IS NOT json_extract(NEW.payload_json,'$.localReconciliation')
BEGIN
  SELECT RAISE(ABORT,'directive_local_reconciliation_immutable');
END;

-- A Web attempt can record a host boundary without gaining any host scope.
-- The only representable state is non-executable; any future HostVerifier
-- must use a separate, independently approved contract and attempt.
CREATE TABLE IF NOT EXISTS agent_host_boundary_candidates (
    id TEXT PRIMARY KEY,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL CHECK(attempt_number > 0),
    root_run_id TEXT NOT NULL,
    target_key TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK(source_kind IN ('human_directive','observed_web_evidence')),
    source_id TEXT NOT NULL,
    source_draft_id TEXT NOT NULL DEFAULT '',
    evidence_fact_refs_json TEXT NOT NULL DEFAULT '[]'
        CHECK(json_valid(evidence_fact_refs_json) AND json_type(evidence_fact_refs_json)='array'),
    summary_redacted TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'recorded_only' CHECK(status='recorded_only'),
    execution_eligible INTEGER NOT NULL DEFAULT 0 CHECK(execution_eligible=0),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(source_kind,source_id)
);
CREATE INDEX IF NOT EXISTS idx_agent_host_boundary_scope
    ON agent_host_boundary_candidates(scan_id,attempt_number,created_at);
CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_host_boundary_draft
    ON agent_host_boundary_candidates(source_draft_id) WHERE source_draft_id<>'';
CREATE TRIGGER IF NOT EXISTS agent_host_boundary_immutable
BEFORE UPDATE ON agent_host_boundary_candidates
BEGIN
  SELECT RAISE(ABORT,'host_boundary_candidate_immutable');
END;
CREATE TRIGGER IF NOT EXISTS agent_collaboration_host_boundary_insert
AFTER INSERT ON agent_host_boundary_candidates
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(NEW.scan_id,NEW.attempt_number,'host_boundary_candidate',NEW.id,'host_boundary_candidate',
    json_object('status',NEW.status,'sourceKind',NEW.source_kind));
END;

CREATE TRIGGER IF NOT EXISTS agent_collaboration_request_review_insert
AFTER INSERT ON agent_request_reviews
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(NEW.scan_id,NEW.attempt_number,'request_review',NEW.id,'request_review',
    json_object('disposition',NEW.disposition,'executionUnlocked',json('false')));
END;

CREATE TRIGGER IF NOT EXISTS agent_collaboration_administrative_closure_insert
AFTER INSERT ON native_web_administrative_closures
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(NEW.scan_id,NEW.attempt_number,'administrative_closure',NEW.id,'administrative_closure',
    json_object('executionSettled',json('false'),'automaticReplayAllowed',json('false')));
END;

CREATE TRIGGER IF NOT EXISTS agent_collaboration_closure_handoff_insert
AFTER INSERT ON native_web_closure_handoffs
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  SELECT NEW.source_scan_id,attempt_number,'closure_handoff',NEW.request_id,'closure_handoff',
    json_object('scanId',NEW.scan_id,'closureId',NEW.closure_id,
      'executionGranted',json('false'),'sourceExecutionSettled',json('false'))
  FROM native_web_administrative_closures WHERE scan_id=NEW.source_scan_id AND id=NEW.closure_id;
END;

CREATE TRIGGER IF NOT EXISTS agent_collaboration_draft_insert
AFTER INSERT ON agent_directive_drafts
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(NEW.scan_id,NEW.attempt_number,'directive_draft',NEW.id,'directive_draft',
    json_object('status',NEW.status,'revision',NEW.revision,'validationResult',NEW.validation_result));
END;
CREATE TRIGGER IF NOT EXISTS agent_collaboration_draft_update
AFTER UPDATE OF status,revision,validation_result,coordinator_decision,confirmed_directive_id ON agent_directive_drafts
WHEN OLD.status<>NEW.status OR OLD.revision<>NEW.revision
  OR OLD.validation_result<>NEW.validation_result OR OLD.coordinator_decision<>NEW.coordinator_decision
  OR OLD.confirmed_directive_id<>NEW.confirmed_directive_id
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(NEW.scan_id,NEW.attempt_number,'directive_draft',NEW.id,'directive_draft',
    json_object('status',NEW.status,'revision',NEW.revision,'validationResult',NEW.validation_result));
END;

CREATE TRIGGER IF NOT EXISTS agent_collaboration_directive_insert
AFTER INSERT ON agent_user_directives
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(NEW.scan_id,NEW.attempt_number,'user_directive',NEW.id,'user_directive',
    json_object('status',NEW.status,'sourceDraftId',NEW.source_draft_id));
END;
CREATE TRIGGER IF NOT EXISTS agent_collaboration_directive_update
AFTER UPDATE OF status,claim_run_id,rejection_code ON agent_user_directives
WHEN OLD.status<>NEW.status OR OLD.claim_run_id<>NEW.claim_run_id OR OLD.rejection_code<>NEW.rejection_code
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(NEW.scan_id,NEW.attempt_number,'user_directive',NEW.id,'user_directive',
    json_object('status',NEW.status,'sourceDraftId',NEW.source_draft_id,'rejectionCode',NEW.rejection_code));
END;

-- Delivery receipts have their own trigger so existing databases acquire it
-- without replacing their already-installed directive status trigger.
CREATE TRIGGER IF NOT EXISTS agent_collaboration_directive_delivery
AFTER UPDATE OF payload_json ON agent_user_directives
WHEN json_extract(OLD.payload_json,'$.modelDelivery') IS NOT json_extract(NEW.payload_json,'$.modelDelivery')
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(NEW.scan_id,NEW.attempt_number,'user_directive',NEW.id,'user_directive',
    json_object('status',NEW.status,'deliveryState',json_extract(NEW.payload_json,'$.modelDelivery.state')));
END;

CREATE TRIGGER IF NOT EXISTS agent_collaboration_message_insert
AFTER INSERT ON agent_messages
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(
    COALESCE((SELECT scan_id FROM agent_runs WHERE id=NEW.root_run_id),(SELECT scan_id FROM agent_runs WHERE id=NEW.run_id),''),
    COALESCE((SELECT attempt_number FROM agent_runs WHERE id=NEW.root_run_id),(SELECT attempt_number FROM agent_runs WHERE id=NEW.run_id),0),
    'mailbox_message',NEW.id,'mailbox_message',
    json_object('kind',NEW.kind,'deliveredAt',NEW.delivered_at,'acknowledgedAt',NEW.acknowledged_at));
END;
CREATE TRIGGER IF NOT EXISTS agent_collaboration_message_update
AFTER UPDATE OF delivered_at,acknowledged_at,delivery_attempts ON agent_messages
WHEN OLD.delivered_at<>NEW.delivered_at OR OLD.acknowledged_at<>NEW.acknowledged_at
  OR OLD.delivery_attempts<>NEW.delivery_attempts
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(
    COALESCE((SELECT scan_id FROM agent_runs WHERE id=NEW.root_run_id),(SELECT scan_id FROM agent_runs WHERE id=NEW.run_id),''),
    COALESCE((SELECT attempt_number FROM agent_runs WHERE id=NEW.root_run_id),(SELECT attempt_number FROM agent_runs WHERE id=NEW.run_id),0),
    'mailbox_message',NEW.id,'mailbox_message',
    json_object('kind',NEW.kind,'deliveredAt',NEW.delivered_at,'acknowledgedAt',NEW.acknowledged_at));
END;

CREATE TRIGGER IF NOT EXISTS agent_collaboration_run_insert
AFTER INSERT ON agent_runs
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(NEW.scan_id,NEW.attempt_number,'agent_run',NEW.id,'agent_run',
    json_object('role',NEW.role,'status',NEW.status,'terminalState',NEW.terminal_state));
END;
CREATE TRIGGER IF NOT EXISTS agent_collaboration_run_update
AFTER UPDATE OF status,terminal_state,terminal_code,heartbeat_at ON agent_runs
WHEN OLD.status<>NEW.status OR OLD.terminal_state<>NEW.terminal_state OR OLD.terminal_code<>NEW.terminal_code
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(NEW.scan_id,NEW.attempt_number,'agent_run',NEW.id,'agent_run',
    json_object('role',NEW.role,'status',NEW.status,'terminalState',NEW.terminal_state));
END;

CREATE TRIGGER IF NOT EXISTS agent_collaboration_assignment_insert
AFTER INSERT ON agent_assignments
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(
    COALESCE((SELECT scan_id FROM agent_runs WHERE id=NEW.coordinator_run_id),''),
    COALESCE((SELECT attempt_number FROM agent_runs WHERE id=NEW.coordinator_run_id),0),
    'assignment',NEW.id,'assignment',json_object('role',NEW.role,'state',NEW.state));
END;
CREATE TRIGGER IF NOT EXISTS agent_collaboration_assignment_update
AFTER UPDATE OF state,child_run_id,failure_class,lease_epoch ON agent_assignments
WHEN OLD.state<>NEW.state OR OLD.child_run_id<>NEW.child_run_id
  OR OLD.failure_class<>NEW.failure_class OR OLD.lease_epoch<>NEW.lease_epoch
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(
    COALESCE((SELECT scan_id FROM agent_runs WHERE id=NEW.coordinator_run_id),''),
    COALESCE((SELECT attempt_number FROM agent_runs WHERE id=NEW.coordinator_run_id),0),
    'assignment',NEW.id,'assignment',json_object('role',NEW.role,'state',NEW.state));
END;

CREATE TRIGGER IF NOT EXISTS agent_collaboration_review_insert
AFTER INSERT ON agent_review_requests
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(
    COALESCE((SELECT scan_id FROM agent_runs WHERE id=NEW.root_run_id),''),
    COALESCE((SELECT attempt_number FROM agent_runs WHERE id=NEW.root_run_id),0),
    'review_gate',NEW.id,'review_gate',json_object('status',NEW.status,'candidateId',NEW.candidate_id));
END;
CREATE TRIGGER IF NOT EXISTS agent_collaboration_review_update
AFTER UPDATE OF status,decision_id ON agent_review_requests
WHEN OLD.status<>NEW.status OR COALESCE(OLD.decision_id,0)<>COALESCE(NEW.decision_id,0)
BEGIN
  INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
  VALUES(
    COALESCE((SELECT scan_id FROM agent_runs WHERE id=NEW.root_run_id),''),
    COALESCE((SELECT attempt_number FROM agent_runs WHERE id=NEW.root_run_id),0),
    'review_gate',NEW.id,'review_gate',json_object('status',NEW.status,'candidateId',NEW.candidate_id));
END;
"#;

pub(crate) fn ensure_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(COLLABORATION_EVENT_SCHEMA)
        .map_err(|error| format!("创建协作事件序列失败：{error}"))
}
