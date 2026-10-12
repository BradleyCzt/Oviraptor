// Current positive queue fixtures use the signed creator and original Root financial identity.
#[test]
fn directive_action_real_executor_child_resolves_the_coordinator_inbox() {
    use crate::agent_runtime::multi_agent::{directive, scheduler};
    let _real = RealSpecialistTransport::enter();
    let (harness, session) = fresh_directive_closure_session("directive-priority-child");
    let context = &harness.context;
    let root_id = session.lease.root_run_id.clone();
    assert_ne!(context.run.as_ref().unwrap().run_id, root_id);
    assert_eq!(context.run.as_ref().unwrap().run_id, session.executor.run_id);
    let connection = db::open(&harness.db_path).unwrap();
    let id = confirm_queue_directive(&connection, &session.lease, "prioritize authorization");
    let mut inbox = take_human_directives(context).unwrap();
    assert_eq!(inbox.lease.as_ref().unwrap().root_run_id, root_id);
    let mut queue = priority_test_queue();
    apply_human_queue_actions(context, &mut inbox, &mut queue).unwrap();
    assert!(inbox.items.is_empty());
    assert_eq!(queue[0], "family:authorization");
    scheduler::finish_child(&connection, &session.lease, &session.executor, true, "test completed").unwrap();
    // Only completed local queue rules are replayed; this does not adopt paid work.
    let mut next = session.lease.clone();
    drop(session);
    connection.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='next-owner' WHERE root_run_id=?1", [&root_id]).unwrap();
    next.lease_epoch += 1;
    next.fencing_token = "next-owner".into();
    assert_eq!(directive::apply_queue_actions(&connection, &next, &priority_test_queue()).unwrap().1, queue);
    let status: String = connection.query_row("SELECT status FROM agent_user_directives WHERE id=?1", [&id], |row| row.get(0)).unwrap();
    assert_eq!(status, "completed");
    drop(connection);
    let _ = std::fs::remove_dir_all(harness.root);
}

#[test]
fn directive_action_native_request_uses_the_committed_priority_not_only_chat_text() {
    let _real = RealSpecialistTransport::enter();
    let mut harness = fresh_multi_production_harness("directive-priority-native");
    harness.context.execution_plan.max_turns = 2;
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (200, "application/json", fresh_directive_closure_wire_response(&request)
            .unwrap_or_else(|| model_round(&[], 10)))
    }));
    harness.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    freeze_fresh_multi_production_harness(&mut harness);
    let mut session = multi_agent_prepare(&mut harness.context).unwrap();
    let connection = db::open(&harness.db_path).unwrap();
    let id = confirm_queue_directive(&connection, &session.lease, "请优先检查权限控制");
    let outcome = NativeAgentBackend.execute(&harness.context);
    let wire = seen.lock().unwrap().clone();
    let root_calls = wire.iter().filter(|request| fresh_multi_wire_system(request).contains("You are the Root Coordinator")).count();
    assert_eq!(root_calls, 5, "existing business-logic assessment plus HumanDirective and feedback rounds");
    assert_eq!(wire.len(), root_calls + 3, "one Mapper, two actual WebExecutor requests: {outcome:?}");
    let recorded: i64 = connection.query_row("SELECT COUNT(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'", [&session.lease.root_run_id], |row| row.get(0)).unwrap();
    assert_eq!(recorded, root_calls as i64, "every physical Root call retains its original receipt");
    assert_eq!(crate::agent_runtime::multi_agent::budget::balance(&connection, &session.lease.root_run_id, Some(""), "model_requests").unwrap().consumed, root_calls as i64);
    let requests = wire.iter().filter(|request| {
        let system = fresh_multi_wire_system(request);
        !system.contains("You are the Root Coordinator") && !system.contains("SPA/API Mapper")
    }).collect::<Vec<_>>();
    assert_eq!(requests.len(), 2, "{outcome:?}");
    for request in requests.iter() {
        let request: JsonValue = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let messages = request["messages"].as_array().unwrap();
        let state = messages.iter().filter_map(|message| message["content"].as_str())
            .filter_map(|text| serde_json::from_str::<JsonValue>(text).ok())
            .find(|value| value.get("pendingQueue").is_some()).unwrap();
        assert_eq!(state["pendingQueue"][0], "family:authorization");
        assert!(!messages.iter().any(|message| message["content"].as_str().is_some_and(|text| text.contains(&id))),
            "an applied scheduling directive must not masquerade as unexecuted human model context");
    }
    let (status, delivery): (String, Option<String>) = connection.query_row(
        "SELECT status,json_extract(payload_json,'$.modelDelivery.state') FROM agent_user_directives WHERE id=?1", [&id],
        |row| Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    assert_eq!(status, "completed");
    assert!(delivery.is_none());
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM agent_directive_queue_actions", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 1);
    multi_agent_finish_execution(&harness.context, &mut session, &outcome).unwrap();
    drop(session);
    drop(connection);
    let _ = std::fs::remove_dir_all(harness.root);
}
