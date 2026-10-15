fn late_web_model_response() -> crate::agent_runtime::model::gateway::ModelResponse {
    crate::agent_runtime::model::gateway::ModelResponse {
        text: "original saved model response".into(),
        usage: crate::agent_runtime::store::UsageDelta {
            input_tokens: 8,
            output_tokens: 2,
            total_tokens: 10,
            model_requests: 1,
            ..Default::default()
        },
        usage_reported: true,
        finish_reason: "stop".into(),
        ..Default::default()
    }
}

#[test]
fn assignment_attempt_late_web_model_bill_after_takeover_stays_with_original_worker() {
    use crate::agent_runtime::multi_agent::budget;
    for phase in ["received", "uncertain", "unsent"] {
        for different_root in [false, true] {
            let (root, context, _, _) = http_journal_fixture("http://127.0.0.1:9/", 0);
            let call =
                native_model_budget_admission(&context, 1, "original-model-wire-hash".into())
                    .unwrap()
                    .unwrap();
            let db = db::open(&context.db_path).unwrap();
            let original_worker: String = db
                .query_row(
                    "SELECT id FROM agent_assignment_attempts WHERE child_run_id=?1",
                    [&context.run.as_ref().unwrap().run_id],
                    |r| r.get(0),
                )
                .unwrap();
            let replacement = replace_target_cost_coordinator(&db, &call.lease, different_root);
            let before = super::tests::application_table_snapshot(&db);
            let response = late_web_model_response();
            let unresolved = native_model_terminal_receipt(
                &context,
                &call,
                phase,
                (phase == "received").then_some(&response),
                "original_transport_fact",
            )
            .expect("original model bill must survive Coordinator takeover");
            assert_eq!(unresolved, phase == "uncertain");
            let after = super::tests::application_table_snapshot(&db);
            for (table, values) in &before {
                if !matches!(
                    table.as_str(),
                    "agent_web_model_journal" | "agent_budget_entries"
                ) {
                    assert_eq!(
                        after
                            .iter()
                            .find(|(name, _)| name == table)
                            .map(|(_, rows)| rows),
                        Some(values),
                        "{table}"
                    );
                }
            }
            let requests = budget::balance(
                &db,
                &call.lease.root_run_id,
                Some(&call.assignment),
                "model_requests",
            )
            .unwrap();
            let expected = match phase {
                "received" => (0, 1, 0),
                "uncertain" => (0, 0, 1),
                _ => (1, 0, 0),
            };
            assert_eq!(
                (requests.reserved, requests.consumed, requests.indeterminate),
                expected
            );
            assert!(db.query_row("SELECT NOT EXISTS(SELECT 1 FROM agent_budget_entries WHERE assignment_id=?1 AND lease_attempt_id<>?2)",
                params![call.assignment, original_worker], |r|r.get::<_,bool>(0)).unwrap());
            if different_root {
                assert_eq!(
                    budget::balance(&db, &replacement.root_run_id, None, "model_requests").unwrap(),
                    budget::Balance::default()
                );
            }
            let saved = super::tests::application_table_snapshot(&db);
            assert!(native_model_budget_admission(
                &context,
                2,
                "must-not-dispatch-after-cost".into()
            )
            .is_err());
            assert!(native_model_terminal_receipt(
                &context,
                &call,
                phase,
                (phase == "received").then_some(&response),
                "original_transport_fact"
            )
            .is_err());
            assert_eq!(super::tests::application_table_snapshot(&db), saved);
            drop(db);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn assignment_attempt_web_model_bill_rechecks_original_role_and_lane_after_last_write() {
    for damaged in ["role", "lane", "orchestration_policy"] {
        let (root, context, _, _) = http_journal_fixture("http://127.0.0.1:9/", 0);
        let call = native_model_budget_admission(&context, 1, "original-model-wire-hash".into())
            .unwrap()
            .unwrap();
        let db = db::open(&context.db_path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER damage_model_owner AFTER INSERT ON agent_budget_entries
            WHEN NEW.dimension='model_requests' AND NEW.kind='consume' BEGIN
            UPDATE agent_runs SET {damaged}='foreign-owner' WHERE assignment_id=NEW.assignment_id; END;")).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            native_model_terminal_receipt(
                &context,
                &call,
                "received",
                Some(&late_web_model_response()),
                ""
            )
            .is_err(),
            "{damaged}: model bill accepted changed original owner"
        );
        assert_eq!(
            super::tests::application_table_snapshot(&db),
            before,
            "{damaged}"
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_web_model_terminal_receipt_keeps_one_original_proof_through_final_journal_write(
) {
    for phase in ["received", "uncertain", "unsent"] {
        let (root, context, _, _) = http_journal_fixture("http://127.0.0.1:9/", 0);
        let call = native_model_budget_admission(&context, 1, "original-model-wire-hash".into())
            .unwrap()
            .unwrap();
        let db = db::open(&context.db_path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER replace_model_role AFTER INSERT ON agent_web_model_journal
            WHEN NEW.phase='{phase}' BEGIN
            UPDATE agent_runs SET role='evidence_reviewer',lane='review' WHERE id=NEW.child_run_id;
            UPDATE agent_assignments SET role='evidence_reviewer',lane='review' WHERE id=NEW.assignment_id; END;")).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let response = late_web_model_response();
        assert!(
            native_model_terminal_receipt(
                &context,
                &call,
                phase,
                (phase == "received").then_some(&response),
                "original-fact"
            )
            .is_err(),
            "{phase}: final journal write replaced the original role proof"
        );
        assert_eq!(
            super::tests::application_table_snapshot(&db),
            before,
            "{phase}"
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_real_model_takeover_keeps_original_cost_but_returns_no_executable_result() {
    use crate::agent_runtime::multi_agent::budget;
    let owner = std::sync::Arc::new(std::sync::Mutex::new(
        None::<(
            PathBuf,
            crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        )>,
    ));
    let handler_owner = owner.clone();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let (path, old) = handler_owner.lock().unwrap().clone().unwrap();
        let db = db::open(&path).unwrap();
        replace_target_cost_coordinator(&db, &old, false);
        (200, "application/json", model_round(&[], 10))
    }));
    let (root, mut context, _, _) = real_http_transport_fixture("http://127.0.0.1:9/", 0);
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let db = db::open(&context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let (old, assignment) = budget::target::child_owner(&tx, &context.run.as_ref().unwrap().run_id)
        .unwrap()
        .unwrap();
    tx.rollback().unwrap();
    *owner.lock().unwrap() = Some((context.db_path.clone(), old.clone()));
    let client = AgentModelClient::new(
        agent_model_profile(&context.environment, None).unwrap(),
        &[],
    );
    let result = native_model_transport(
            &context,
            &client,
            vec![serde_json::json!({"role":"user","content":"bounded local fixture"})],
            &[],
            1
        );
    assert!(
        result.is_err(),
        "known late model result escaped transport under a replaced Coordinator"
    );
    let diagnostic = match &result {
        Err(NativeModelRoundFailure::Model(error)) => error.code().to_string(),
        Err(NativeModelRoundFailure::Outcome(outcome)) => {
            format!("{}: {}", outcome.terminal_code(), outcome.detail())
        }
        Ok(_) => "unexpected executable result".into(),
    };
    assert_eq!(seen.lock().unwrap().len(), 1, "transport result: {diagnostic}");
    let b = budget::balance(&db, &old.root_run_id, Some(&assignment), "model_requests").unwrap();
    assert_eq!(b.reserved, 0);
    assert_eq!(b.consumed + b.indeterminate, 1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_web_model_journal WHERE phase IN ('received','uncertain')",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert!(native_model_transport(&context, &client, vec![], &[], 2).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
