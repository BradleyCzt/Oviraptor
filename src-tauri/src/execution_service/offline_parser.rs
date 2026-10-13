//! Adapter for the existing fixed offline container mechanism.
//!
//! The caller must already hold and revalidate its original approval, frozen
//! inventory, scoped worker lease and invocation receipt. A Request alone is
//! not such authority. No fallback to host_process is offered here.
use crate::tool_supply::container_parser::{self, Receipt, Request};

pub(crate) fn execute(request: &Request, cancelled: &dyn Fn() -> bool) -> Result<Receipt, String> {
    #[cfg(not(test))]
    {
        container_parser::execute_guarded(request, cancelled)
    }
    // The test harness executable has its own main. Keep the existing test
    // route through the same real fixed runtime. Cross-process guardian and
    // SIGKILL verification still uses tools/test_nest_sandbox_guardian.py.
    #[cfg(test)]
    {
        container_parser::Runtime::inspect()?.execute(request, cancelled)
    }
}
