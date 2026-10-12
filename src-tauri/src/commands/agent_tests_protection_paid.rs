fn protection_assert_original_finish_phase_delta(
    f: &ProtectionOriginalFixture,
    db: &rusqlite::Connection,
    before: std::collections::BTreeMap<String, usize>,
) -> usize {
    let after = followup_original_root_phase_counts(
        db,
        &f.actor.root_run_id,
        &f.h.model_seen.lock().unwrap(),
    );
    let mut expected = before.clone();
    for phase in ["http-budget-consumption", "protection-changed"] {
        *expected.entry(phase.into()).or_default() += 1;
    }
    assert_eq!(after,expected,"finish must append exactly the original HTTP assessment and original protection assessment bills");
    let (publications,requests):(i64,i64)=db.query_row("SELECT count(*),sum(json_extract(p.fact_json,'$.event.usage.modelRequests')) FROM agent_root_tick_receipts p JOIN agent_root_tick_receipts q ON q.root_run_id=p.root_run_id AND q.call_id=p.call_id AND q.phase='request' WHERE p.root_run_id=?1 AND p.phase='publication' AND json_extract(q.fact_json,'$.request.basis.phase')='http-budget-consumption'",[&f.actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        (publications, requests),
        (1, 1),
        "HTTP assessment has its own known paid publication"
    );
    after.values().sum::<usize>() - before.values().sum::<usize>()
}

fn protection_paid_root_calls(f: &ProtectionOriginalFixture) -> usize {
    f.h.model_seen
        .lock()
        .unwrap()
        .iter()
        .filter(|w| w.contains("You are the Root Coordinator"))
        .count()
}
fn protection_paid_phases(db: &rusqlite::Connection, root: &str, phase: &str) -> i64 {
    db.query_row("SELECT count(*) FROM agent_root_tick_receipts p JOIN agent_root_tick_receipts q ON q.root_run_id=p.root_run_id AND q.call_id=p.call_id AND q.phase='request' WHERE p.root_run_id=?1 AND p.phase=?2 AND json_extract(q.fact_json,'$.request.basis.phase')='protection-changed'",params![root,phase],|r|r.get(0)).unwrap()
}

#[test]
fn protection_paid_actual_finish_assesses_original_receipt_once_and_full_reentry_is_zero() {
    let mut f = ProtectionOriginalFixture::new("protection-paid-once");
    let db = db::open(&f.h.db_path).unwrap();
    let calls = protection_paid_root_calls(&f);
    let cost = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap()
    .consumed;
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    assert_eq!(protection_paid_root_calls(&f), calls + 1);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        cost + 1
    );
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "request"),
        1
    );
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "publication"),
        1
    );
    let inputs: Vec<JsonValue> =
        f.h.model_seen
            .lock()
            .unwrap()
            .iter()
            .filter(|w| w.contains("You are the Root Coordinator"))
            .map(|w| {
                let body: JsonValue =
                    serde_json::from_str(w.split_once("\r\n\r\n").unwrap().1).unwrap();
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap()
            })
            .filter(|v: &JsonValue| v["basis"]["phase"] == "protection-changed")
            .collect();
    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs[0]["permittedSuggestion"],
        "assess:protection_changed"
    );
    assert_eq!(
        inputs[0]["basis"]["changedFact"]["semantic"]["receiptId"],
        f.receipt
    );
    assert_eq!(
        inputs[0]["basis"]["changedFact"]["semantic"]["closedAuthorVerified"],
        true
    );
    assert_eq!(
        inputs[0]["basis"]["changedFact"]["semantic"]["executionGrant"],
        false
    );
    assert_eq!(
        inputs[0]["basis"]["changedFact"]["semantic"]["targetRequestsGranted"],
        0
    );
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let replay =
        native_coordinator_protection_feedback(&f.h.context, &f.actor, &f.receipt).unwrap();
    assert!(replay.replayed);
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    let tx = db.unchecked_transaction().unwrap();
    tx.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1",[&f.actor.root_run_id]).unwrap();
    tx.commit().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(native_coordinator_protection_feedback(&f.h.context, &f.actor, &f.receipt).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    f.assert_closed_read_only(&db, true);
}

#[test]
fn protection_paid_actual_wrong_step_keeps_bill_without_publication_or_target_dispatch() {
    let reply=proposal_model_response(&json!({"schemaVersion":1,"observed":["closed protection fact"],"missing":[],"suggestions":["dispatch:web_executor"],"costNotes":[],"risks":[]}).to_string());
    let mut f =
        ProtectionOriginalFixture::with_protection_reply("protection-paid-wrong-step", Some(reply));
    let db = db::open(&f.h.db_path).unwrap();
    let calls = protection_paid_root_calls(&f);
    let cost = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap()
    .consumed;
    let error = multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome)
        .unwrap_err();
    assert_eq!(error, "root_protection_step_not_bounded");
    assert_eq!(protection_paid_root_calls(&f), calls + 1);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        cost + 1
    );
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "publication"),
        0
    );
    f.assert_closed_read_only(&db, true);
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        native_coordinator_protection_feedback(&f.h.context, &f.actor, &f.receipt)
            .err()
            .unwrap(),
        error
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(protection_paid_root_calls(&f), calls + 1);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

fn protection_paid_publication_fault_case(ignore: bool) {
    let mut f = ProtectionOriginalFixture::new(if ignore {
        "protection-paid-ignore"
    } else {
        "protection-paid-abort"
    });
    let db = db::open(&f.h.db_path).unwrap();
    let raise = if ignore {
        "RAISE(IGNORE)"
    } else {
        "RAISE(ABORT,'protected_publication_fault')"
    };
    db.execute_batch(&format!("CREATE TRIGGER protection_paid_publication_fault BEFORE INSERT ON agent_root_tick_receipts WHEN NEW.phase='publication' AND EXISTS(SELECT 1 FROM agent_root_tick_receipts r WHERE r.root_run_id=NEW.root_run_id AND r.call_id=NEW.call_id AND r.phase='request' AND json_extract(r.fact_json,'$.request.basis.phase')='protection-changed') BEGIN SELECT {raise}; END;")).unwrap();
    let calls = protection_paid_root_calls(&f);
    let cost = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap()
    .consumed;
    assert!(
        multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome)
            .is_err()
    );
    assert_eq!(protection_paid_root_calls(&f), calls + 1);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        cost + 1
    );
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "publication"),
        0
    );
    f.assert_closed_read_only(&db, true);
    let paid = super::tests::application_table_snapshot(&db);
    for _ in 0..2 {
        assert!(
            native_coordinator_protection_feedback(&f.h.context, &f.actor, &f.receipt).is_err()
        );
        assert_eq!(super::tests::application_table_snapshot(&db), paid);
        assert_eq!(protection_paid_root_calls(&f), calls + 1);
    }
    db.execute_batch("DROP TRIGGER protection_paid_publication_fault")
        .unwrap();
    let costs = client_delivery_rows(&db, "agent_budget_entries");
    let decision =
        native_coordinator_protection_feedback(&f.h.context, &f.actor, &f.receipt).unwrap();
    assert!(decision.replayed);
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "publication"),
        1
    );
    assert_eq!(client_delivery_rows(&db, "agent_budget_entries"), costs);
    let before = super::tests::application_table_snapshot(&db);
    native_coordinator_protection_feedback(&f.h.context, &f.actor, &f.receipt).unwrap();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(protection_paid_root_calls(&f), calls + 1);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
#[test]
fn protection_paid_actual_publication_abort_replays_original_bill_without_second_sdk() {
    protection_paid_publication_fault_case(false);
}
#[test]
fn protection_paid_actual_publication_ignore_replays_original_bill_without_second_sdk() {
    protection_paid_publication_fault_case(true);
}

#[test]
fn protection_paid_actual_sdk_file_drift_keeps_bill_and_restored_source_publishes_without_resend() {
    let saved = std::sync::Arc::new(std::sync::Mutex::new(None::<(std::path::PathBuf, Vec<u8>)>));
    let captured = saved.clone();
    let boundary = std::sync::Arc::new(move |directory: &Path| {
        let path = directory.join(AGENT_HTTP_DIRECTORY).join("0001.body");
        let original = fs::read(&path).unwrap();
        let mut saved = captured.lock().unwrap();
        assert!(saved.is_none(), "only one actual paid protection SDK");
        fs::write(
            &path,
            [original.as_slice(), b"cold-sdk-boundary-damage"].concat(),
        )
        .unwrap();
        *saved = Some((path, original));
    });
    let mut f = ProtectionOriginalFixture::with_protection_boundary(
        "protection-paid-drift",
        None,
        Some(boundary),
    );
    let db = db::open(&f.h.db_path).unwrap();
    let calls = protection_paid_root_calls(&f);
    let phases = followup_original_root_phase_counts(
        &db,
        &f.actor.root_run_id,
        &f.h.model_seen.lock().unwrap(),
    );
    let cost = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap()
    .consumed;
    assert!(
        multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome)
            .is_err()
    );
    let added = protection_assert_original_finish_phase_delta(&f, &db, phases);
    assert_eq!(protection_paid_root_calls(&f), calls + added);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        cost + i64::try_from(added).unwrap()
    );
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "publication"),
        0
    );
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    let paid = super::tests::application_table_snapshot(&db);
    for _ in 0..2 {
        assert!(
            native_coordinator_protection_feedback(&f.h.context, &f.actor, &f.receipt).is_err()
        );
        assert_eq!(super::tests::application_table_snapshot(&db), paid);
        assert_eq!(protection_paid_root_calls(&f), calls + added);
    }
    let (path, original) = saved.lock().unwrap().take().unwrap();
    fs::write(path, original).unwrap();
    f.assert_closed_read_only(&db, true);
    let costs = client_delivery_rows(&db, "agent_budget_entries");
    assert!(
        native_coordinator_protection_feedback(&f.h.context, &f.actor, &f.receipt)
            .unwrap()
            .replayed
    );
    assert_eq!(client_delivery_rows(&db, "agent_budget_entries"), costs);
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "publication"),
        1
    );
    let before = super::tests::application_table_snapshot(&db);
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(protection_paid_root_calls(&f), calls + added);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
