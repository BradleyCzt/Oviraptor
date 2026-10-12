// Real signed first WAF response and actual Root/Mapper/Web SDK receipts.
// Recovery readiness is a historical read; it grants no restored execution.
fn recovery_source_assert_read_only(
    f: &ProtectionOriginalFixture,
    db: &rusqlite::Connection,
    want_ok: bool,
) -> Option<JsonValue> {
    let check = |snapshot: &rusqlite::Connection| {
        let before = super::tests::application_table_snapshot(snapshot);
        let changes = snapshot.total_changes();
        let sdk = f.h.model_seen.lock().unwrap().len();
        let gets = f.h.site_seen.lock().unwrap().len();
        let result = native_recovery_source_at_on(snapshot, &f.h.db_path, &f.actor, &f.receipt);
        let fact = match result {
            Ok(proof) => {
                assert!(want_ok, "unsettled recovery source must be rejected");
                for rows in proof.verification_rows() {
                    rows.verify(snapshot).unwrap();
                }
                let fact = proof.bounded_fact();
                assert_eq!(fact["source"], "original_settled_recovery_source");
                assert_eq!(fact["readiness"], "ready");
                assert_eq!(fact["rootRunId"], f.actor.root_run_id);
                assert_eq!(fact["protectionReceiptId"], f.receipt);
                assert_eq!(fact["originalProtection"]["closedAuthorVerified"], true);
                for key in ["grant", "approval", "executionGrant"] {
                    assert_eq!(fact[key], false);
                }
                assert_eq!(fact["targetRequestsGranted"], 0);
                for dimension in crate::agent_runtime::multi_agent::budget::DIMENSIONS {
                    assert_eq!(fact["dimensions"][dimension]["reserved"], 0);
                    assert_eq!(fact["dimensions"][dimension]["indeterminate"], 0);
                }
                assert!(!fact["rootCalls"].as_array().unwrap().is_empty());
                for call in fact["rootCalls"].as_array().unwrap() {
                    assert_eq!(call["phase"], "received");
                    assert_eq!(call["originalSemanticAvailable"], true);
                    assert_eq!(call["semanticReplayAllowed"], false);
                }
                Some(fact)
            }
            Err(error) => {
                assert!(!want_ok, "actual settled recovery source rejected: {error}");
                None
            }
        };
        assert_eq!(super::tests::application_table_snapshot(snapshot), before);
        assert_eq!(snapshot.total_changes(), changes);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
        assert_eq!(f.h.site_seen.lock().unwrap().len(), gets);
        fact
    };
    if db.is_autocommit() {
        let snapshot = db.unchecked_transaction().unwrap();
        let fact = check(&snapshot);
        snapshot.commit().unwrap();
        fact
    } else {
        check(db)
    }
}

#[test]
fn recovery_source_actual_paid_protection_is_ready_without_writes_or_new_authority() {
    let mut f = ProtectionOriginalFixture::new("recovery-source-ready");
    let mut db = db::open(&f.h.db_path).unwrap();
    recovery_source_assert_read_only(&f, &db, false);
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    drop(f.session.take());
    let fact = recovery_source_assert_read_only(&f, &db, true).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    assert_eq!(
        native_recovery_preview_on(
            &mut db,
            &f.h.db_path,
            &f.actor.scan_id,
            f.actor.attempt_number,
            &f.actor.root_run_id,
            &f.receipt
        )
        .unwrap()["selectedSource"],
        fact
    );
    assert_eq!(
        native_recovery_preview_on(
            &mut db,
            &f.h.db_path,
            "wrong-scan",
            f.actor.attempt_number,
            &f.actor.root_run_id,
            &f.receipt
        )
        .unwrap_err(),
        "recovery_preview_original_scope_conflict"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    let reopened = rusqlite::Connection::open_with_flags(
        &f.h.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        recovery_source_assert_read_only(&f, &reopened, true).unwrap(),
        fact
    );
    // Expiry changes eligibility for execution, never repairs/renews the old C.
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(tx.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1",
        [&f.actor.root_run_id]).unwrap(), 1);
    assert!(
        crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&tx, &f.actor)
            .is_err()
    );
    assert_eq!(
        recovery_source_assert_read_only(&f, &tx, true).unwrap(),
        fact
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn recovery_source_actual_unknown_root_sdk_cannot_prepare_from_known_web_balance() {
    let mut reply: JsonValue = serde_json::from_str(&proposal_model_response(
        &json!({
            "schemaVersion":1,"observed":["original stopped response"],"missing":[],
            "suggestions":["assess:protection_changed"],"costNotes":[],"risks":[]
        })
        .to_string(),
    ))
    .unwrap();
    reply.as_object_mut().unwrap().remove("usage");
    let mut f = ProtectionOriginalFixture::with_protection_reply(
        "recovery-source-unknown",
        Some(reply.to_string()),
    );
    let db = db::open(&f.h.db_path).unwrap();
    let calls = protection_paid_root_calls(&f);
    let before_phases = followup_original_root_phase_counts(
        &db,
        &f.actor.root_run_id,
        &f.h.model_seen.lock().unwrap(),
    );
    assert_eq!(
        multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome)
            .unwrap_err(),
        "budget_indeterminate_requires_reconciliation"
    );
    drop(f.session.take());
    let after_phases = followup_original_root_phase_counts(
        &db,
        &f.actor.root_run_id,
        &f.h.model_seen.lock().unwrap(),
    );
    let mut expected_phases = before_phases;
    *expected_phases
        .entry("http-budget-consumption".into())
        .or_default() += 1;
    *expected_phases
        .entry("protection-changed".into())
        .or_default() += 1;
    assert_eq!(after_phases, expected_phases, "finish drains the one original HTTP bill before assessing original WAF protection; no other SDK phase may be appended");
    assert_eq!(protection_paid_root_calls(&f), calls + 2);
    let (http_publications,http_requests):(i64,i64)=db.query_row("SELECT count(*),sum(json_extract(p.fact_json,'$.event.usage.modelRequests')) FROM agent_root_tick_receipts p JOIN agent_root_tick_receipts q ON q.call_id=p.call_id AND q.root_run_id=p.root_run_id AND q.phase='request' WHERE p.root_run_id=?1 AND p.phase='publication' AND json_extract(q.fact_json,'$.request.basis.phase')='http-budget-consumption'",[&f.actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        (http_publications, http_requests),
        (1, 1),
        "the additional HTTP assessment is a known independent original paid invoice"
    );
    assert_eq!(
        protection_paid_phases(&db, &f.actor.root_run_id, "publication"),
        0
    );
    assert!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_input_tokens"
        )
        .unwrap()
        .indeterminate
            > 0
    );
    f.assert_closed_read_only(&db, true);
    for _ in 0..2 {
        let snapshot = db.unchecked_transaction().unwrap();
        let before = super::tests::application_table_snapshot(&snapshot);
        let error = native_recovery_source_at_on(&snapshot, &f.h.db_path, &f.actor, &f.receipt)
            .err()
            .expect("unknown original Root fee must reject preparation");
        assert_eq!(error, "budget_indeterminate_requires_reconciliation");
        assert_eq!(super::tests::application_table_snapshot(&snapshot), before);
        snapshot.commit().unwrap();
        recovery_source_assert_read_only(&f, &db, false);
    }
    assert_eq!(protection_paid_root_calls(&f), calls + 2);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
