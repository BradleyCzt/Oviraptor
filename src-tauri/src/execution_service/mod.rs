//! Internal execution mechanisms shared by domain adapters, with no UI or IPC.
//!
//! Host processes and approved offline parsing have separate entry points and
//! guarantees. Neither entry point establishes project/run/attempt authority.
//! Existing callers retain policy, approvals, leases, invocation ownership,
//! cancellation checks, durable receipts and task-state publication.

pub(crate) mod host_process;
pub(crate) mod offline_parser;

/// A bounded, redacted diagnostic record, not a task-state or authority event.
/// Domain journals bind it to the existing scope and persist before notifying.
#[derive(Debug)]
pub(crate) struct OutputEvent {
    pub stream: &'static str,
    pub stream_sequence: i64,
    pub message: String,
    pub gap: bool,
}

#[cfg(test)]
mod tests;
