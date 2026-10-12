#[path = "multi_agent/mapper_executor_adapter.rs"]
mod mapper_executor_adapter;
#[path = "multi_agent/mapper_expired_delivery_writer.rs"]
mod mapper_expired_delivery_writer;
#[path = "multi_agent/mapper_loop_request.rs"]
pub(crate) mod mapper_loop_request;
#[path = "multi_agent/mapper_evidence_lookup.rs"]
pub(crate) mod mapper_evidence_lookup;
#[path = "multi_agent/client_delivery_writer.rs"]
mod client_delivery_writer;
#[path = "multi_agent/client_grant_writer.rs"]
mod client_grant_writer;
#[path = "multi_agent/client_sdk_dispatch_writer.rs"]
mod client_sdk_dispatch_writer;
struct MultiAgentSession {
    supervisor: crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor,
    lease: crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    mapper: crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    executor: crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
}

include!("multi_agent/findings.rs");
include!("multi_agent/child_transport.rs");
include!("multi_agent/client_side_input.rs");
include!("multi_agent/client_side_capture.rs");
include!("multi_agent/client_side_source_proof.rs");
include!("multi_agent/client_side_execution.rs");
include!("multi_agent/input_parser_input.rs");
include!("multi_agent/input_parser_replay.rs");
include!("multi_agent/input_parser_execution.rs");
include!("multi_agent/business_logic_execution.rs");
include!("multi_agent/child_accounting.rs");
include!("multi_agent/finalization.rs");
include!("multi_agent/review_snapshot.rs");
include!("multi_agent/review_requests.rs");
include!("multi_agent/review_delivery.rs");
include!("multi_agent/readonly_assessment.rs");
include!("multi_agent/mapper_expired_delivery.rs");
include!("multi_agent/prepare.rs");
include!("multi_agent/mapper_historical_projection.rs");
include!("multi_agent/gap_proposal.rs");
include!("multi_agent/gap_investigation.rs");
include!("multi_agent/gap_delivery.rs");
include!("multi_agent/gap_validation.rs");
include!("multi_agent/executor_finished_replay.rs");
include!("multi_agent/execution.rs");
include!("multi_agent/review_validation.rs");
include!("multi_agent/review.rs");
