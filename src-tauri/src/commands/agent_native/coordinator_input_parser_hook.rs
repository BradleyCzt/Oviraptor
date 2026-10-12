// U176: the frozen, hash-bound InputParser trigger frame (U175) becomes one
// independently paid original-Root advisory fact. Rust admits only that frame
// kind, re-verifies it before and after the paid call, and accepts only the
// frame's own fixed step (`dispatch:input_parser`) or a deferral. Nothing is
// minted, granted or dispatched here (U177 owns the child mint).
#[cfg_attr(not(test), allow(dead_code))]
fn native_coordinator_input_parser_feedback(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<NativeCoordinatorTickReceipt, String> {
    let root = native_coordinator_root_context(context, actor);
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, &root, actor)?;
    let frame = NativeCoordinatorFrame::input_parser_trigger(&tx, actor, &root.target_dir)?;
    frame.verify(&tx, &root, actor)?;
    native_coordinator_tick_authority(&tx, &root, actor)?;
    tx.commit().map_err(|e| e.to_string())?;
    let decision = native_coordinator_tick_for_frame(&root, actor, &frame)?;
    if !decision.summary.as_json()["suggestions"].as_array().is_some_and(|items| {
        items.iter().all(|item| item.as_str() == Some(frame.step()))
    }) {
        return Err("root_input_parser_step_not_bounded".into());
    }
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, &root, actor)?;
    frame.verify(&tx, &root, actor)?;
    decision.tick.require_executable(&tx)?;
    if decision.tick.published(&tx, &decision.saved)? != Some(decision.event_sequence) {
        return Err("root_input_parser_publication_changed".into());
    }
    // No child/capability/grant/tool/network effect follows publication.
    tx.commit().map_err(|e| e.to_string())?;
    Ok(decision)
}
