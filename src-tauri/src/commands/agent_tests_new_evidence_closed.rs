#[test]
fn new_evidence_original_web_settlement_live_sdk_and_summary_writer_fault_preserve_original_costs()
{
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_root_fixture("ne-finance-fault", &[]);
    let db = db::open(&f.h.db_path).unwrap();
    let child = &f.session.as_ref().unwrap().executor;
    let actor = f.lease();
    let calls = f.calls();
    let before = receipt_database_snapshot(&db);
    let active = crate::agent_runtime::execution_owner::claim_native_invocation(
        &f.h.db_path,
        &actor.scan_id,
        actor.attempt_number,
        crate::agent_runtime::multi_agent::budget::web_lifetime::WEB_MODEL_INVOCATION_KIND,
        &child.run_id,
    )
    .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert!(settle_original_web_child_usage(&tx, actor, child)
        .err()
        .unwrap()
        .starts_with("web_model_transport_not_idle:"));
    drop(tx);
    assert_eq!(receipt_database_snapshot(&db), before);
    drop(active);
    // The original provider bill exists before this local summary attempt.
    // IGNORE must roll back every tentative vector/aggregate transition.
    db.execute_batch("CREATE TEMP TRIGGER ne_summary_fault BEFORE UPDATE OF budget_settled_at ON agent_assignments BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        settle_original_web_child_usage(&tx, actor, child)
            .err()
            .unwrap(),
        "child_budget_settlement_replayed_or_stale"
    );
    drop(tx);
    assert_eq!(receipt_database_snapshot(&db), before);
    assert_eq!(f.calls(), calls);
}

#[test]
fn new_evidence_original_web_finish_expired_c_settles_only_original_fee_without_renewal() {
    let _real = RealSpecialistTransport::enter();
    let mut f = input_parser_root_fixture("ne-expired-finance", &[]);
    let db = db::open(&f.h.db_path).unwrap();
    let context = f.h.context.clone();
    let child = f.session.as_ref().unwrap().executor.clone();
    let actor = f.lease().clone();
    let calls = f.calls();
    let active = crate::agent_runtime::execution_owner::claim_native_invocation(
        &context.db_path,
        &actor.scan_id,
        actor.attempt_number,
        "web-executor-original-execution",
        &child.run_id,
    )
    .unwrap();
    let before = receipt_database_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(settle_original_web_child_usage(&tx, &actor, &child)
        .err()
        .unwrap()
        .starts_with("web_executor_original_not_idle:"));
    drop(tx);
    assert_eq!(receipt_database_snapshot(&db), before);
    drop(active);
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1", [&f.root]).unwrap();
    let c: String = db
        .query_row(
            "SELECT lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",
            [&f.root],
            |r| r.get(0),
        )
        .unwrap();
    let outcome = AgentTargetOutcome::Incomplete(AgentStop::from_reason("expired original C"));
    assert!(multi_agent_finish_execution(&context, f.session.as_mut().unwrap(), &outcome).is_err());
    let tx = db.unchecked_transaction().unwrap();
    let usage = crate::agent_runtime::multi_agent::budget::web_lifetime::child_usage_original(
        &tx,
        &actor,
        &child.assignment_id,
        &child.run_id,
    )
    .unwrap();
    assert!(usage.model_requests > 0);
    let stored: (i64,i64,String) = tx.query_row("SELECT r.used_tokens,r.used_requests,a.budget_settled_at FROM agent_runs r JOIN agent_assignments a ON a.id=r.assignment_id WHERE r.id=?1", [&child.run_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        (stored.0, stored.1),
        (usage.total_tokens, usage.model_requests)
    );
    assert!(!stored.2.is_empty());
    tx.commit().unwrap();
    let paid = receipt_database_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let (repeated, _idle) = settle_original_web_child_usage(&tx, &actor, &child).unwrap();
    assert_eq!(repeated, usage);
    tx.commit().unwrap();
    drop(_idle);
    assert_eq!(receipt_database_snapshot(&db), paid);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        crate::agent_runtime::multi_agent::budget::model::settle_original_web_receipt(
            &tx,
            &actor,
            &child.assignment_id,
            &f.root
        )
        .is_err()
    );
    drop(tx);
    assert_eq!(receipt_database_snapshot(&db), paid);
    assert_eq!(
        db.query_row(
            "SELECT lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",
            [&f.root],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        c
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result'", [&child.assignment_id], |r|r.get::<_,i64>(0)).unwrap(), 0);
    assert_eq!(f.calls(), calls);
}

#[test]
fn new_evidence_protocol_fresh_root_freezes_without_native_mutation_or_old_root_upgrade() {
    use crate::agent_runtime::web_mode::root;
    for policy in ["single", "multi"] {
        for existing in [false, true] {
            let fixture = web_mode_fixture(policy, "https://mode.example.test/app");
            let plan = web_mode_test_plan(&fixture);
            let db = db::open(&fixture.path).unwrap();
            if existing {
                web_mode_test_record(&fixture, true, true).unwrap();
            }
            let before = web_mode_test_rows(&db);
            persist_frozen_web_execution_plan(
                &fixture.path,
                &fixture.scan,
                1,
                &fixture.target,
                &plan,
            )
            .unwrap();
            let id: String = db
                .query_row(
                    "SELECT id FROM agent_runs WHERE scan_id=?1 AND role='coordinator'",
                    [&fixture.scan],
                    |r| r.get(0),
                )
                .unwrap();
            let mode = root::read(&db, &id).unwrap().unwrap();
            assert_eq!(
                mode.new_evidence_supervision().is_some(),
                !existing && policy == "multi"
            );
            let text: String = db
                .query_row("SELECT plan_json FROM agent_runs WHERE id=?1", [&id], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(text, plan.as_json().to_string());
            if existing {
                web_mode_assert_rows(&db, &before);
                let raw = serde_json::to_string(&mode).unwrap();
                assert!(!raw.contains("newEvidenceSupervision"));
                let decoded: root::NewRootModeDeclaration = serde_json::from_str(&raw).unwrap();
                assert_eq!(serde_json::to_string(&decoded).unwrap(), raw);
                assert_eq!(
                    decoded.with_new_evidence_supervision().unwrap_err(),
                    "web_mode_private_creation_missing"
                );
            }
        }
    }
}

#[test]
fn new_evidence_closed_frame_real_delivery_reverifies_without_new_effects() {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_production_fixture("ne-closed");
    let db = db::open(&f.h.db_path).unwrap();
    let (id, _) = f.record(&db);
    let calls = f.calls();
    let frame = input_parser_frame_zero_write(&db, || {
        NativeCoordinatorFrame::new_closed_http_evidence(&db, &f.h.context, f.lease(), &id).unwrap()
    });
    assert_eq!(
        frame.semantic["closedAuthor"]["runId"],
        f.h.context.run.as_ref().unwrap().run_id
    );
    assert_eq!(
        native_supervision_frame_admission(frame.kind),
        Ok(())
    );
    input_parser_frame_zero_write(&db, || frame.verify(&db, &f.h.context, f.lease()).unwrap());
    let repeated = input_parser_frame_zero_write(&db, || {
        NativeCoordinatorFrame::new_closed_http_evidence(&db, &f.h.context, f.lease(), &id).unwrap()
    });
    assert_eq!(frame.fact(), repeated.fact());
    let mut forged = frame.clone();
    forged.semantic["closedAuthor"]["assignmentId"] = "another-assignment".into();
    assert_eq!(
        input_parser_frame_zero_write(&db, || forged.verify(&db, &f.h.context, f.lease())),
        Err("root_new_evidence_binding_changed".into())
    );
    let mut other = f.h.context.clone();
    other.target_dir = f.h.root.clone();
    assert_eq!(
        input_parser_frame_zero_write(&db, || NativeCoordinatorFrame::new_closed_http_evidence(
            &db,
            &other,
            f.lease(),
            &id
        )
        .err()),
        Some("followup_target_directory_invalid".into())
    );
    assert_eq!(calls, f.calls(), "read proof never calls SDK");
}

#[test]
fn new_evidence_closed_frame_received_http_without_delivery_refuses_before_sdk() {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_root_fixture("ne-open", &[]);
    let db = db::open(&f.h.db_path).unwrap();
    let (id, _) = f.record(&db);
    let calls = f.calls();
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    let raw =
        NativeCoordinatorFrame::new_http_evidence(&db, f.lease(), &f.h.context.target_dir, &id)
            .unwrap();
    let root = native_coordinator_root_context(&f.h.context, f.lease());
    assert_eq!(
        input_parser_frame_zero_write(&db, || native_coordinator_basis_for_frame_on(
            &db,
            &root,
            f.lease(),
            Some(&raw)
        )
        .err()),
        Some("root_new_evidence_author_not_closed".into())
    );

    assert_eq!(
        input_parser_frame_zero_write(&db, || NativeCoordinatorFrame::new_closed_http_evidence(
            &db,
            &f.h.context,
            f.lease(),
            &id
        )
        .err()),
        Some("root_new_evidence_author_not_closed".into())
    );
    assert_eq!(calls, f.calls());
}

#[test]
fn new_evidence_source_corrupt_multibyte_natural_key_refuses_without_panic_or_writes() {
    let (root, context, actor) = input_parser_frame_fixture();
    let artifact = input_parser_trigger_observe(
        &context,
        1,
        json!({"method":"GET","url":context.target_url,"identity":"anonymous"}),
        "ne-invalid-hash",
    );
    let db = db::open(&context.db_path).unwrap();
    let id = input_parser_trigger_node(&db, &artifact);
    db.execute(
        "UPDATE agent_evidence_nodes SET natural_key_hash=?1 WHERE id=?2",
        params!["汉".repeat(12), id],
    )
    .unwrap();
    assert_eq!(
        input_parser_frame_zero_write(&db, || NativeCoordinatorFrame::new_http_evidence(
            &db,
            &actor,
            &context.target_dir,
            &id
        )
        .err()),
        Some("root_new_evidence_identity_invalid".into())
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
