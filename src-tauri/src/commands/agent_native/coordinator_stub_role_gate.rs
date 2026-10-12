// Upload/Concurrency remain unimplemented (U21). A paid Root choice of
// `dispatch:upload` / `dispatch:concurrency` therefore gets the same stable
// not-implemented codes as schedule_child: no child, grant, capability,
// trigger or SDK call is written, and success is never faked.
const NATIVE_UPLOAD_DISPATCH_STEP: &str = "dispatch:upload";
const NATIVE_CONCURRENCY_DISPATCH_STEP: &str = "dispatch:concurrency";
const NATIVE_UPLOAD_ROLE_NOT_IMPLEMENTED: &str = "upload_role_not_implemented";
const NATIVE_CONCURRENCY_ROLE_NOT_IMPLEMENTED: &str = "concurrency_role_not_implemented";

/// U147: single map from a stub role to its stable not-implemented code. Upload and
/// Concurrency are the only two unimplemented roles (U21/U26/U134); every other role
/// returns None so a new stub can never silently reuse a neighbour's code.
#[cfg_attr(not(test), allow(dead_code))]
fn native_stub_role_not_implemented_code(
    role: crate::agent_runtime::contract::AgentRole,
) -> Option<&'static str> {
    use crate::agent_runtime::contract::AgentRole;
    match role {
        AgentRole::Upload => Some(NATIVE_UPLOAD_ROLE_NOT_IMPLEMENTED),
        AgentRole::Concurrency => Some(NATIVE_CONCURRENCY_ROLE_NOT_IMPLEMENTED),
        _ => None,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn native_coordinator_prepare_upload(
    _context: &AgentRunContext,
    _actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    decision: &NativeCoordinatorTickReceipt,
    _task: &JsonValue,
) -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
    if decision.saved.local_step.is_some()
        || !native_root_paid_suggestion_selected(decision, NATIVE_UPLOAD_DISPATCH_STEP)
    {
        return Err("root_did_not_select_upload".into());
    }
    Err(NATIVE_UPLOAD_ROLE_NOT_IMPLEMENTED.into())
}

#[cfg_attr(not(test), allow(dead_code))]
fn native_coordinator_prepare_concurrency(
    _context: &AgentRunContext,
    _actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    decision: &NativeCoordinatorTickReceipt,
    _task: &JsonValue,
) -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
    if decision.saved.local_step.is_some()
        || !native_root_paid_suggestion_selected(decision, NATIVE_CONCURRENCY_DISPATCH_STEP)
    {
        return Err("root_did_not_select_concurrency".into());
    }
    Err(NATIVE_CONCURRENCY_ROLE_NOT_IMPLEMENTED.into())
}
