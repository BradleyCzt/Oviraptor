// Frozen bounded local reasoning; actual dispatch remains with Rust policy.
fn native_coordinator_tick(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<NativeCoordinatorTickReceipt, String> {
    native_coordinator_tick_with_frame(context, actor, None)
}
fn native_coordinator_tick_with_frame(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&NativeCoordinatorFrame>,
) -> Result<NativeCoordinatorTickReceipt, String> {
    // P1-1：只有显式 homogeneous_workers 的 Root 会在无帧轮次里 mint 同质主管帧。
    let minted = match frame {
        None => native_homogeneous_frame_for_root(context, actor)?,
        Some(_) => None,
    };
    let frame = frame.or(minted.as_ref());
    let original = native_coordinator_request_for_frame(context, actor, frame)?;
    let original_basis = original.basis.clone();
    if original_basis.get("localDeliberation").is_none() {
        return native_coordinator_tick_once(context, actor, frame, original, &original_basis, &[]);
    }
    native_run_root_controlled_adapter(NativeRootControlledAdapter {
        context,
        actor,
        frame,
        original_basis,
        first: Some(original),
        history: Vec::new(),
        registered: crate::agent_runtime::multi_agent::budget::root::model::tick::local::schemas()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect(),
        calls_used: 0,
        next_tool: 0,
        local: None,
        results: Vec::new(),
        original_error: None,
    })
}
fn native_coordinator_verify_local_history(
    db: &rusqlite::Connection,
    history: &[NativeCoordinatorTickReceipt],
) -> Result<(), String> {
    for receipt in history {
        receipt.tick.require_executable(db)?;
        if receipt.saved.local_step.is_none()
            || receipt.tick.published(db, &receipt.saved)? != Some(receipt.event_sequence)
        {
            return Err("root_local_original_history_changed".into());
        }
    }
    Ok(())
}
