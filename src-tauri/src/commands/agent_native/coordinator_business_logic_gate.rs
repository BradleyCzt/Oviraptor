// BusinessLogic has a frozen readonly SDK/output assessment protocol. Its output
// frame does not authorize a new child: the independent prepare contract is still
// missing. A paid Root choice of
// `dispatch:business_logic` therefore gets a stable not-ready gate: no child,
// grant, capability, trigger or SDK call is written, and success is never faked.
const NATIVE_BUSINESS_LOGIC_DISPATCH_STEP: &str = "dispatch:business_logic";
const NATIVE_BUSINESS_LOGIC_DISPATCH_NOT_READY: &str = "business_logic_dispatch_not_ready";

#[cfg_attr(not(test), allow(dead_code))]
fn native_coordinator_prepare_business_logic(
    _context: &AgentRunContext,
    _actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    decision: &NativeCoordinatorTickReceipt,
    _task: &JsonValue,
) -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
    if decision.saved.local_step.is_some()
        || !native_root_paid_suggestion_selected(decision, NATIVE_BUSINESS_LOGIC_DISPATCH_STEP)
    {
        return Err("root_did_not_select_business_logic".into());
    }
    // U139: explicit !implemented branch (true path still not_ready — no fake success).
    if !native_root_role_prepare_implemented(
        crate::agent_runtime::contract::AgentRole::BusinessLogic,
    ) {
        // U95: named reserved try_produce (still no child/SDK write).
        let _ = native_try_produce_business_logic_output_frame("prepare");
        // U146: blockers are source-visible to the gate; while any remain, stay not_ready.
        let _ = native_root_prepare_a_blockers_remaining(
            crate::agent_runtime::contract::AgentRole::BusinessLogic,
        );
        return Err(NATIVE_BUSINESS_LOGIC_DISPATCH_NOT_READY.into());
    }
    // Future (a): build ScheduledChild here. Never fake Ok until then.
    let _ = native_try_produce_business_logic_output_frame("prepare");
    Err(NATIVE_BUSINESS_LOGIC_DISPATCH_NOT_READY.into())
}
