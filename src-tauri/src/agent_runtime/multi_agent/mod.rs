//! Durable multi-agent orchestration primitives.
//!
//! The commands layer owns model execution; this module owns the invariants that
//! make several model loops one auditable job: coordinator fencing, scoped user
//! directives, typed mailbox delivery and deterministic child-run scheduling.
pub mod assignment;
pub(crate) mod client_side;
pub(crate) mod input_parser;
pub(crate) mod attempts;
pub mod budget;
pub(crate) mod budget_gaps;
pub mod contract_owner;
mod coordinator_heartbeat;
pub mod directive;
pub mod lease;
pub mod mailbox;
pub(crate) mod homogeneous;
pub mod scheduler;
pub mod source;
pub(crate) mod source_coverage;
pub(crate) mod source_coverage_contract;
pub(crate) mod source_coverage_decisions;
pub(crate) mod source_coverage_reviewer;
pub(crate) mod source_decisions;
pub(crate) mod source_phases;
pub(crate) mod source_plan_contract;
pub mod source_review;
pub(crate) mod source_review_contract;
pub(crate) mod source_review_projection;
pub(crate) mod source_review_subject;
pub(crate) mod source_reviewer;
pub mod source_rounds;
pub mod specialist;
pub(crate) mod supervision_ticket;
pub(crate) mod supervisor;
pub(crate) mod parent_invocation_owner;
