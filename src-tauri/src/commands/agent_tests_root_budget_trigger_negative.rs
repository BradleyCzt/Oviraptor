include!("agent_tests_supervision_capacity_concurrency.rs");

#[test]
fn root_budget_trigger_v9_insufficient_followup_capacity_refuses_new_executor() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|wire| {
        (200,"application/json",changed_fact_response(&wire,true))
    }));
    let mut f = root_tick_fixture_protocol_limits("v9-low-followup-capacity",&format!("http://127.0.0.1:{port}/v1"),true,(60_000,6));
    drop(f.parent.take());
    assert_eq!(multi_agent_prepare(&mut f.context).err().unwrap(),"multi_agent_executor_budget_unavailable");
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(seen.lock().unwrap().len(),3,"only original Root/Mapper/Root calls, no additional capacity bought");
    assert_eq!(crate::agent_runtime::web_mode::root::read(&db,&f.actor.root_run_id).unwrap().unwrap().bootstrap_dispatch().unwrap()["schemaVersion"],9);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='web_executor'",[&f.actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_http_request_claims WHERE scan_id=?1",[&f.context.scan_id],|r|r.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn root_budget_trigger_v6_missing_sidecar_cannot_downgrade_http_gate() {
    let _real = RealSpecialistTransport::enter();
    let f = budget_trigger_fixture(0);
    let db = db::open(&f.h.db_path).unwrap();
    let s = f.session.as_ref().unwrap();
    let original = super::tests::application_table_snapshot(&db);
    let tx = rusqlite::Transaction::new_unchecked(&db,rusqlite::TransactionBehavior::Immediate).unwrap();
    // Simulate imported corruption only in this temporary transaction; rollback
    // restores the original signed declaration and immutable trigger.
    tx.execute_batch("DROP TRIGGER root_mode_immutable_delete").unwrap();
    assert_eq!(tx.execute("DELETE FROM agent_root_mode_definitions WHERE root_run_id=?1",[&s.lease.root_run_id]).unwrap(),1);
    let broken = super::tests::application_table_snapshot(&tx);
    assert_eq!(crate::agent_runtime::multi_agent::budget::supervision_capacity::require_http_headroom(&tx,&s.lease,&s.executor.assignment_id,&s.executor.run_id).unwrap_err(),"web_mode_root_missing");
    assert_eq!(crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&tx,&s.lease.root_run_id).unwrap_err(),"web_mode_root_missing");
    assert!(super::tests::application_table_snapshot(&tx)==broken);
    tx.rollback().unwrap();
    assert!(super::tests::application_table_snapshot(&db)==original);
    assert_eq!(f.h.model_seen.lock().unwrap().len(),3);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn root_budget_trigger_actual_entry_defer_and_forbidden_advisory_stop_before_web_sdk_or_target() {
    let _real = RealSpecialistTransport::enter();
    for (reply, code) in [
        (1, "root_budget_allocation_deferred"),
        (2, "root_budget_step_not_bounded"),
    ] {
        let f = budget_trigger_fixture(reply);
        let db = db::open(&f.h.db_path).unwrap();
        let root = &f.session.as_ref().unwrap().lease.root_run_id;
        let outcome = run_native_agent(&f.h.context);
        assert!(outcome.detail().contains(code), "{}", outcome.detail());
        if reply == 1 {
            assert!(matches!(outcome, AgentTargetOutcome::Incomplete(_)));
        }
        assert_eq!(f.h.model_seen.lock().unwrap().len(), 4);
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 0);
        assert_eq!(root_tick_count(&db, root, "publication"), 3);
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::balance(
                &db,
                root,
                Some(""),
                "model_requests"
            )
            .unwrap()
            .consumed,
            3
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_assignments WHERE role='web_executor'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        let before = web_mode_test_rows(&db);
        assert_eq!(
            native_coordinator_budget_before_executor(&f.h.context).unwrap_err(),
            code
        );
        web_mode_assert_rows(&db, &before);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), 4);
    }
}
#[test]
fn root_budget_trigger_actual_unknown_sdk_keeps_debt_and_original_worker_without_retry() {
    let _real = RealSpecialistTransport::enter();
    let f = budget_trigger_fixture(3);
    let db = db::open(&f.h.db_path).unwrap();
    let s = f.session.as_ref().unwrap();
    let held = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
        &db,
        "SELECT rowid,id,child_run_id,state,reserved_tokens,reserved_requests,budget_settled_at,lease_epoch,fencing_token FROM agent_assignments WHERE id=?1",
        [&s.executor.assignment_id],
    )
    .unwrap();
    let outcome = run_native_agent(&f.h.context);
    assert_eq!(
        outcome.terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED
    );
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 4);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 0);
    let balance = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &s.lease.root_run_id,
        Some(""),
        "model_requests",
    )
    .unwrap();
    assert_eq!(balance.consumed, 2);
    assert_eq!(balance.indeterminate, 1);
    assert!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &s.lease.root_run_id,
            Some(""),
            "model_input_tokens"
        )
        .unwrap()
        .indeterminate
            > 0
    );
    // The entry heartbeat may renew timestamps, but does not settle/refund this grant.
    assert!(db
        .query_row(
            "SELECT state='running' AND budget_settled_at='' FROM agent_assignments WHERE id=?1",
            [&s.executor.assignment_id],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
        &db,
        "SELECT rowid,id,child_run_id,state,reserved_tokens,reserved_requests,budget_settled_at,lease_epoch,fencing_token FROM agent_assignments WHERE id=?1",
        [&s.executor.assignment_id]
    ).unwrap()==held);
    let before = web_mode_test_rows(&db);
    assert!(native_coordinator_budget_before_executor(&f.h.context).is_err());
    web_mode_assert_rows(&db, &before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 4);
}
#[test]
fn root_budget_trigger_original_authority_corruption_refuses_zero_write_zero_sdk() {
    let _real = RealSpecialistTransport::enter();
    for corruption in 0..8 {
        let f = budget_trigger_fixture(0);
        let db = db::open(&f.h.db_path).unwrap();
        let id = &f.session.as_ref().unwrap().executor.assignment_id;
        let mut context = f.h.context.clone();
        match corruption {
            0 => {
                db.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE assignment_id=?1",[id]).unwrap();
            }
            1 => {
                db.execute("DELETE FROM agent_lane_leases WHERE assignment_id=?1", [id])
                    .unwrap();
            }
            2 => {
                db.execute(
                    "UPDATE agent_assignments SET reserved_tokens=reserved_tokens+1 WHERE id=?1",
                    [id],
                )
                .unwrap();
            }
            3 => {
                db.execute("UPDATE agent_runs SET reserved_tokens=reserved_tokens+1 WHERE assignment_id=?1",[id]).unwrap();
            }
            4 => {
                db.execute("UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.rootDecision.eventSequence',999999) WHERE id=?1",[id]).unwrap();
            }
            5 => {
                db.execute("UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.objective','unauthorized wider scope') WHERE id=?1",[id]).unwrap();
            }
            6 => {
                context.supervision = None;
            }
            _ => {
                context.target_url.push_str("/outside-original");
            }
        }
        let before = web_mode_test_rows(&db);
        assert!(
            native_coordinator_budget_before_executor(&context).is_err(),
            "corruption {corruption}"
        );
        web_mode_assert_rows(&db, &before);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), 3);
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 0);
    }
}
#[test]
fn root_budget_trigger_paid_scope_revocation_stops_publication_and_replays_only_after_original_restore(
) {
    let _real = RealSpecialistTransport::enter();
    let f = budget_trigger_fixture(0);
    let id = f.session.as_ref().unwrap().executor.assignment_id.clone();
    let revoked = id.clone();
    *f.boundary.lock().unwrap() = Some(Box::new(move |db| {
        db.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE assignment_id=?1",[revoked]).unwrap();
    }));
    assert!(native_coordinator_budget_before_executor(&f.h.context).is_err());
    let db = db::open(&f.h.db_path).unwrap();
    let root = &f.session.as_ref().unwrap().lease.root_run_id;
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 4);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 0);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, root, Some(""), "model_requests")
            .unwrap()
            .consumed,
        3
    );
    assert_eq!(root_tick_count(&db, root, "publication"), 2);
    let before = web_mode_test_rows(&db);
    assert!(native_coordinator_budget_before_executor(&f.h.context).is_err());
    web_mode_assert_rows(&db, &before);
    db.execute(
        "UPDATE agent_capability_leases SET revoked_at='' WHERE assignment_id=?1",
        [id],
    )
    .unwrap();
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 4);
    assert_eq!(root_tick_count(&db, root, "publication"), 3);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 0);
}
