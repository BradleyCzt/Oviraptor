#[test]
fn root_human_original_confirmation_damage_refuses_without_any_new_sdk_or_write() {
    for damage in [
        "DELETE FROM agent_directive_human_reviews",
        "UPDATE agent_user_directives SET confirmed_hash='foreign-confirmation'",
        "UPDATE agent_user_directives SET thread_key='foreign-thread'",
    ] {
        let f = human_root_producer(0);
        let db = db::open(&f.f.h.db_path).unwrap();
        let actor = &f.f.session.as_ref().unwrap().lease;
        confirm_queue_directive(&db, actor, "prioritize authorization");
        let mut inbox = take_human_directives(&f.f.h.context).unwrap();
        db.execute_batch(damage).unwrap();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        let mut queue = priority_test_queue();
        assert!(
            apply_assessed_human_queue_actions(&f.f.h.context, &mut inbox, &mut queue).is_err(),
            "{damage}"
        );
        assert_eq!(queue, priority_test_queue());
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        assert!(f.human.lock().unwrap().is_empty());
        assert!(f.web.lock().unwrap().is_empty());
        assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 4);
    }
}

#[test]
fn root_human_actual_paid_assessment_queue_withdrawal_rolls_back_without_refunding_bill() {
    let f = human_root_producer(0);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    confirm_queue_directive(&db, actor, "prioritize authorization");
    let mut inbox = take_human_directives(&f.f.h.context).unwrap();
    let paid = native_coordinator_human_assessments(&f.f.h.context, actor).unwrap();
    assert_eq!(paid.len(), 1);
    assert_eq!(f.human.lock().unwrap().len(), 1);
    db.execute_batch("CREATE TRIGGER withdraw_paid_human_consumer AFTER INSERT ON agent_directive_queue_actions BEGIN UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE role='web_executor'; END;").unwrap();
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let original = inbox.items.clone();
    let mut queue = priority_test_queue();
    assert!(apply_assessed_human_queue_actions(&f.f.h.context, &mut inbox, &mut queue).is_err());
    assert_eq!(queue, priority_test_queue());
    assert_eq!(inbox.items, original);
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(f.human.lock().unwrap().len(), 1);
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 5);
    assert_eq!(root_tick_count(&db, &actor.root_run_id, "publication"), 4);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &actor.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        4
    );
    assert!(f.web.lock().unwrap().is_empty());
}

#[test]
fn root_human_actual_inflight_consumer_cancellation_stops_transport_keeps_unknown_invoice() {
    let f = human_root_producer(4);
    let db = db::open(&f.f.h.db_path).unwrap();
    let actor = &f.f.session.as_ref().unwrap().lease;
    let id = confirm_queue_directive(&db, actor, "prioritize authorization");
    let child = f.f.h.context.run.as_ref().unwrap().run_id.clone();
    // The provider accepts the in-flight Human Root request, the consumer is
    // cancelled at that same instant, then the reply is held for two seconds.
    // Measure from that instant so prior Root ticks (slower when the suite runs
    // in parallel) cannot be mistaken for waiting on the provider.
    let cancelled_at = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mark = cancelled_at.clone();
    *f.f.boundary.lock().unwrap() = Some(Box::new(move |db| {
        db.execute(
            "UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1",
            [&child],
        )
        .unwrap();
        *mark.lock().unwrap() = Some(std::time::Instant::now());
    }));
    let outcome = NativeAgentBackend.execute(&f.f.h.context);
    let returned_at = std::time::Instant::now();
    let cancelled_at = cancelled_at
        .lock()
        .unwrap()
        .expect("the in-flight Human Root request reached the provider and was cancelled");
    assert!(
        returned_at.saturating_duration_since(cancelled_at) < Duration::from_millis(1500),
        "cancelled Root waited for the two-second provider: {outcome:?}"
    );
    assert_eq!(f.human.lock().unwrap().len(), 1);
    assert!(f.web.lock().unwrap().is_empty());
    let root = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &actor.root_run_id,
        Some(""),
        "model_requests",
    )
    .unwrap();
    assert_eq!((root.consumed, root.indeterminate), (3, 1));
    assert_eq!(root_tick_count(&db, &actor.root_run_id, "publication"), 3);
    let state:(String,i64)=db.query_row("SELECT status,(SELECT COUNT(*) FROM agent_directive_queue_actions) FROM agent_user_directives WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(state, ("accepted".into(), 0));
    // Keep the original endpoint alive until its deliberately late reply exits.
    let until = std::time::Instant::now() + Duration::from_secs(3);
    while f.f.h.model_seen.lock().unwrap().len() < 5 {
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(f.f.h.site_seen.lock().unwrap().is_empty());
}
