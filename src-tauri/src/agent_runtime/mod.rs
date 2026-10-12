//! Oviraptor-owned agent runtime.
//!
//! The orchestrator, budget, identity handles, coverage and terminal state live
//! here. An executor may only report facts; it can never
//! write the terminal state itself.

pub mod checkpoint;
pub mod contract;
pub(crate) mod controlled_executor;
pub(crate) mod deleted_scan_audit;
pub mod evidence_graph;
pub(crate) mod execution_owner;
pub mod model;
pub mod multi_agent;
pub mod reducer;
pub mod review;
pub mod role_config;
pub mod runtime_adapter;
pub(crate) mod single_projection;
pub mod secrets;
pub mod store;
pub mod target_requests;
pub(crate) mod target_protection;
pub(crate) mod web_mode;

#[cfg(test)]
mod tests;
