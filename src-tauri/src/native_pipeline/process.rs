//! Compatibility surface for existing Native callers. Process ownership and
//! bounded capture live in execution_service; scan journals remain here.
//! This host-process path has never been the approved offline-parser sandbox.

#[cfg(unix)]
pub(crate) use crate::execution_service::host_process::capture_pipe;
pub(crate) use crate::execution_service::host_process::{interruptible, run_observed, OwnedChild};
pub use crate::execution_service::host_process::{run, BoundedRun, ProcessError, ProcessLimits};

#[path = "process_log/mod.rs"]
pub(crate) mod log;
#[path = "process_output.rs"]
pub(crate) mod output;
