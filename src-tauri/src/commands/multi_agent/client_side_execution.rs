// Dedicated production ClientSide worker. It reads supplier facts, no new target I/O.
fn multi_agent_client_side_readonly(
    context: &AgentRunContext,
    session: &MultiAgentSession,
) -> Result<(), String> {
    session.supervisor.check()?;
    let db = db::open(&context.db_path)?;
    let existing = if crate::agent_runtime::web_mode::root::read(&db,&session.lease.root_run_id)?.is_some_and(|d|d.lease_expiry_supervision().is_some()) {
        let mut q = db.prepare("SELECT a.id,a.child_run_id,a.trigger_code FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE (a.coordinator_run_id=?1 OR r.root_run_id=?1) AND (a.role='client_side' OR r.role='client_side')").map_err(|e|e.to_string())?;
        let rows = q.query_map([&session.lease.root_run_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
        if rows.len()>1 {return Err("client_side_original_assignment_conflict".into());}
        if rows.iter().any(|(_,_,trigger)|trigger!="client_side_frozen_observations_ready") {return Err("client_side_original_trigger_conflict".into());}
        rows.into_iter().next().map(|(assignment_id,run_id,_)|crate::agent_runtime::multi_agent::scheduler::ScheduledChild {assignment_id,run_id,role:crate::agent_runtime::contract::AgentRole::ClientSide})
    } else {None};
    let (child,task) = if let Some(child) = existing {
        let original = crate::agent_runtime::multi_agent::client_side::assignment(&db,&session.lease,&child)?;
        client_side_verify_frozen_artifacts(&original,&db,&session.lease,&context.target_dir)?;
        (child,serde_json::to_value(original).map_err(|e|e.to_string())?)
    } else {
        let Some(task) = client_side_frozen_http_task(&db,&session.lease,&context.target_dir)? else {return Ok(());};
        (client_grant_writer::prepare(context,&session.lease,&task)?,task)
    };
    let result=multi_agent_child_round_transport(context,&session.lease,&child,
        "你是独立ClientSide配置事实专家。只分析冻结本地配置观察，不调用工具、浏览器或目标。输出严格JSON：summary、observationRefs、gaps、candidates。引用已有观察id；candidates=[]；gaps包含missing_browser_validation。配置缺失不是影响证明，不能confirmed。",task.clone());
    let (text, usage) = match result {
        Ok(value) => value,
        Err(code) if code.starts_with("client_side_sdk_dispatch_") => return Err(code),
        Err(code) => return Err(failed_specialist_error(&db, &session.lease, &child, &code)),
    };
    #[cfg(test)]
    if CLIENT_EXPIRE_RECEIVED_ONCE.with(|flag|flag.replace(false)) {
        let tx = db.unchecked_transaction().map_err(|e|e.to_string())?;
        let original = crate::agent_runtime::multi_agent::specialist::received_for_local_delivery(&tx,&session.lease,&child)?;
        if original.text != text || original.usage != usage || !original.rejection.is_empty() {return Err("test_original_client_received_mismatch".into());}
        tx.commit().map_err(|e|e.to_string())?;
        let changed = db.execute("UPDATE agent_assignment_attempts SET expires_at=datetime('now','localtime','+1 second') WHERE child_run_id=?1 AND assignment_id=?2",params![child.run_id,child.assignment_id]).map_err(|e|e.to_string())?;
        if changed!=1 {return Err("test_original_client_worker_missing".into());}
        let deadline = Instant::now()+Duration::from_secs(5);
        while crate::agent_runtime::multi_agent::attempts::current(&db,&session.lease,&child.assignment_id)?.state != "expired" {
            session.supervisor.check()?;
            if Instant::now()>=deadline {return Err("test_original_client_expiry_timeout".into());}
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    session.supervisor.check()?;
    deliver_readonly_assessment(
        &db,
        &session.lease,
        &child,
        &usage,
        &json!({"summary":text,"clientSideTask":task}),
        Some(&context.target_dir),
    )?;
    session.supervisor.check()?;
    native_coordinator_original_expired_feedback(context,&session.lease,&child)?;
    // The normal prepared pipeline has an original paid independent Mapper.
    // Standalone readonly supplier contexts do not fabricate Root SDK history.
    if session.mapper.role == crate::agent_runtime::contract::AgentRole::SpaApiMapper {
        native_coordinator_client_feedback(context, session, &child)?;
    }
    session.supervisor.check()
}

#[cfg(test)]
thread_local! {static CLIENT_EXPIRE_RECEIVED_ONCE: std::cell::Cell<bool> = const {std::cell::Cell::new(false)};}
