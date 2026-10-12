// U178: drive a minted InputParser child through the readonly SDK input surface
// and deliver its strict parameter assessment. The surface is built and bound
// before any provider call; the shared transport re-verifies it and sends no
// tools; delivery re-checks the surface and the strict output schema and rolls
// back with zero writes on any violation. Wired into production by U180.
fn multi_agent_input_parser_readonly(
    context: &AgentRunContext,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<(), String> {
    let db = db::open(&context.db_path)?;
    if input_parser_completed_original_replay(&db, lease, child, &context.target_dir)? {
        native_coordinator_original_expired_feedback(context,lease,child)?;
        return Ok(());
    }
    let task = input_parser_build_surface(&db, lease, child, &context.target_dir)?;
    let input = serde_json::to_value(&task).map_err(|e| e.to_string())?;
    let result = multi_agent_child_round_transport(
        context,
        lease,
        child,
        "你是独立 InputParser 只读输入面专家。只分析已冻结、已授权、绑定本 Root 本 scan 的请求元数据（方法/路径/已观察查询参数名/是否有请求体）。严禁调用任何工具、浏览器或目标，严禁发起任何目标请求，严禁臆造未观察到的参数。输出严格 JSON：summary、parameters（每项 name/location/typeAssumption/brokerVariantTemplateId；name 必须来自已观察参数、location 为 query、brokerVariantTemplateId 只能取受控变体模板）、gaps（必须含 missing_live_parameter_probe）。只给类型假设与受控变体建议，这不是影响证明，不能 confirmed，不输出 candidates。",
        input,
    );
    let (text, usage) = match result {
        Ok(value) => value,
        Err(code) => return Err(failed_specialist_error(&db, lease, child, &code)),
    };
    #[cfg(test)]
    if INPUT_PARSER_EXPIRE_RECEIVED_ONCE.with(|flag|flag.replace(false)) {
        let tx = db.unchecked_transaction().map_err(|e|e.to_string())?;
        let original = crate::agent_runtime::multi_agent::specialist::received_for_local_delivery(&tx,lease,child)?;
        if original.text != text || original.usage != usage || !original.rejection.is_empty() {
            return Err("test_original_input_parser_received_mismatch".into());
        }
        tx.commit().map_err(|e|e.to_string())?;
        let changed = db.execute("UPDATE agent_assignment_attempts SET expires_at=datetime('now','localtime','+1 second') WHERE child_run_id=?1 AND assignment_id=?2",params![child.run_id,child.assignment_id]).map_err(|e|e.to_string())?;
        if changed!=1 {return Err("test_original_input_parser_worker_missing".into());}
        let deadline = Instant::now()+Duration::from_secs(5);
        while crate::agent_runtime::multi_agent::attempts::current(&db,lease,&child.assignment_id)?.state != "expired" {
            context.supervision.as_ref().ok_or("test_original_supervisor_missing")?.check()?;
            if Instant::now()>=deadline {return Err("test_original_input_parser_expiry_timeout".into());}
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    deliver_readonly_assessment(
        &db,
        lease,
        child,
        &usage,
        &json!({"summary": text, "inputParserTask": task}),
        Some(&context.target_dir),
    )?;
    native_coordinator_original_expired_feedback(context,lease,child)?;
    Ok(())
}


// U180: production entry for the InputParser readonly flow, wired after the Executor
// settles (same place as ClientSide). Real dispatch is gated behind
// `native_root_role_prepare_implemented(InputParser)`, which U181 flipped to true (the
// only flipped role). When dispatch is disabled this returns before any paid Root tick,
// so fixture behavior and cost are unchanged. The flow is: deterministic trigger predicate -> frozen-frame paid Root
// tick selecting `dispatch:input_parser` -> validated fixed-grant mint (both retained
// floors preserved) -> readonly SDK input surface -> delivery. Every failure is the
// zero-write refusal proven in U176/U177/U179.
fn multi_agent_input_parser(
    context: &AgentRunContext,
    session: &MultiAgentSession,
) -> Result<(), String> {
    use crate::agent_runtime::contract::AgentRole;
    session.supervisor.check()?;
    // Only the prepared production pipeline (a real paid SPA/API Mapper) dispatches
    // InputParser, mirroring the ClientSide standalone guard.
    if session.mapper.role != AgentRole::SpaApiMapper {
        return Ok(());
    }
    // Honest gate FIRST: when the flag is false (U181 flipped it for InputParser; the
    // test-only forced-off hook simulates false), do nothing here — no trigger
    // evaluation, no paid Root tick, no cost, no write.
    if !input_parser_dispatch_enabled() {
        return Ok(());
    }
    let db = db::open(&context.db_path)?;
    // Deterministic Rust predicate: not eligible => no dispatch, no paid tick, no cost.
    if !native_input_parser_trigger_decision(&db, &session.lease.root_run_id, &context.target_dir)?
        .eligible
    {
        return Ok(());
    }
    // Paid original-Root tick over the U175 frozen frame; the Root must select
    // `dispatch:input_parser` (an empty/local step is a deferral, not a dispatch).
    let decision = native_coordinator_input_parser_feedback(context, &session.lease)?;
    if decision.saved.local_step.is_some()
        || !native_root_paid_suggestion_selected(&decision, NATIVE_INPUT_PARSER_DISPATCH_STEP)
    {
        return Ok(());
    }
    session.supervisor.check()?;
    // Gate + validated mint of the readonly fixed-grant child (both retained floors kept).
    let child = native_coordinator_prepare_input_parser(context, &session.lease, &decision, &json!({}))?;
    session.supervisor.check()?;
    // Readonly SDK input surface + strict delivery (U178/U179). Any failure is zero-write.
    multi_agent_input_parser_readonly(context, &session.lease, &child)?;
    session.supervisor.check()
}

// Dispatch uses the real implemented-role gate in production and positive
// tests alike. The only test hook can close that gate; it cannot open it.
#[cfg(test)]
thread_local! {
    static INPUT_PARSER_TEST_FLAG_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(test)]
fn input_parser_test_flag_forced_off() -> bool {
    INPUT_PARSER_TEST_FLAG_OFF.with(|cell| cell.get())
}
#[cfg(not(test))]
fn input_parser_test_flag_forced_off() -> bool {
    false
}
fn input_parser_dispatch_enabled() -> bool {
    native_root_role_prepare_implemented(crate::agent_runtime::contract::AgentRole::InputParser)
        && !input_parser_test_flag_forced_off()
}

// U181: test-only simulation of `native_root_role_prepare_implemented(InputParser) ==
// false` (the pre-U181 production state), used by the honest flag-false negative.
#[cfg(test)]
struct InputParserProductionFlagOff;
#[cfg(test)]
impl InputParserProductionFlagOff {
    fn enter() -> Self {
        INPUT_PARSER_TEST_FLAG_OFF.with(|cell| cell.set(true));
        Self
    }
}
#[cfg(test)]
impl Drop for InputParserProductionFlagOff {
    fn drop(&mut self) {
        INPUT_PARSER_TEST_FLAG_OFF.with(|cell| cell.set(false));
    }
}

#[cfg(test)]
thread_local! {
    static INPUT_PARSER_EXPIRE_RECEIVED_ONCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
