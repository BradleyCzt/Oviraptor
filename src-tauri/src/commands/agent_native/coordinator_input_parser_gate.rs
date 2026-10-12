// InputParser has a ReadOnlyAnalysis contract (U14/U20), a frozen Root frame
// (U175), paid Root feedback (U176), a real fixed-grant child mint (U177,
// `native_coordinator_mint_input_parser_child`), the readonly SDK input surface (U178)
// and the failure zero-write proof (U179); U180 wired it after the Executor and U181
// flipped `native_root_role_prepare_implemented(InputParser)` to true. The not_ready
// branch below is kept: whenever the flag is false, a paid Root choice of
// `dispatch:input_parser` gets a stable not-ready gate: no child, grant, capability,
// trigger or SDK call is written, and success is never faked.
const NATIVE_INPUT_PARSER_DISPATCH_STEP: &str = "dispatch:input_parser";
const NATIVE_INPUT_PARSER_DISPATCH_NOT_READY: &str = "input_parser_dispatch_not_ready";

#[cfg_attr(not(test), allow(dead_code))]
fn native_coordinator_prepare_input_parser(
    _context: &AgentRunContext,
    _actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    decision: &NativeCoordinatorTickReceipt,
    _task: &JsonValue,
) -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
    if decision.saved.local_step.is_some()
        || !native_root_paid_suggestion_selected(decision, NATIVE_INPUT_PARSER_DISPATCH_STEP)
    {
        return Err("root_did_not_select_input_parser".into());
    }
    // U139: explicit !implemented branch (true path still not_ready — no fake success).
    // U180: `input_parser_dispatch_enabled()` is `native_root_role_prepare_implemented`
    // in production (true since U181) OR a test-only override; U181 adds a test-only
    // forced-off hook that simulates the flag being false.
    if !input_parser_dispatch_enabled() {
        // U95: named reserved try_produce (still no child/SDK write).
        let _ = native_try_produce_input_parser_output_frame("prepare");
        // U146: blockers are source-visible to the gate; while any remain, stay not_ready.
        let _ = native_root_prepare_a_blockers_remaining(
            crate::agent_runtime::contract::AgentRole::InputParser,
        );
        return Err(NATIVE_INPUT_PARSER_DISPATCH_NOT_READY.into());
    }
    // U177: the implemented arm mints the real fixed-grant child (reachable since U181).
    native_coordinator_mint_input_parser_child(_context, _actor, decision)
}
