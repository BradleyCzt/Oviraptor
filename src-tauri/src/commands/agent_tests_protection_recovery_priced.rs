fn priced_recovery_input() -> NativeRecoveryPricingInput {
    use crate::recovery_finance::Money;
    NativeRecoveryPricingInput {
        schema_version: 1,
        max_budget_usd: "5".into(),
        price_revision: "fixture-v2".into(),
        uncached_input: Money::from_nanos(2_000_000_000).unwrap(),
        cached_input: Money::from_nanos(500_000_000).unwrap(),
        output: Money::from_nanos(4_000_000_000).unwrap(),
        per_request: Money::from_nanos(10_000_000).unwrap(),
    }
}
fn priced_recovery_original_tables(db: &rusqlite::Connection) -> Vec<(String, Vec<String>)> {
    super::tests::application_table_snapshot(db)
        .into_iter()
        .filter(|(name, _)| name != "native_protection_recovery_priced_approvals")
        .collect()
}

#[test]
fn priced_recovery_actual_paid_source_freezes_exact_terms_without_birth_or_new_sdk() {
    let (f, selected, scope) = protection_recovery_approval_fixture("priced-approval");
    let pricing = priced_recovery_input();
    let mut db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let proposal =
        native_priced_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope, &pricing)
            .unwrap();
    assert_eq!(
        super::tests::application_table_snapshot(&db),
        before,
        "pricing preview is read-only"
    );
    assert_eq!(proposal["schemaVersion"], 2);
    assert_eq!(proposal["financialTerms"]["maxNanoUsd"], "5000000000");
    assert_eq!(
        proposal["financialTerms"]["pricing"]["modelBindingHash"],
        crate::agent_runtime::store::stable_hash(&proposal["modelBinding"].to_string())
    );
    assert_eq!(
        proposal["pricingBasis"],
        "approved_rate_card_not_provider_dollar_invoice"
    );
    let originals = priced_recovery_original_tables(&db);
    let request = Uuid::new_v4().to_string();
    let hash = proposal["proposalHash"].as_str().unwrap();
    let mut unaffordable = pricing.clone();
    unaffordable.per_request = crate::recovery_finance::Money::parse_usd("5.000000001").unwrap();
    assert_eq!(
        native_priced_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope, &unaffordable)
            .unwrap_err(),
        "protection_recovery_fixed_call_exceeds_budget"
    );
    assert_eq!(
        native_priced_recovery_approve_on(
            &f.h.db_path, &selected, &scope, &unaffordable, &request, hash,
            PRICED_RECOVERY_CONSENT
        ).unwrap_err(),
        "protection_recovery_fixed_call_exceeds_budget"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    // Exact equality is not itself an overrun. A later executable consumer
    // must still prove and reserve the complete input/output charge.
    let mut fixed_only = pricing.clone();
    fixed_only.per_request = crate::recovery_finance::Money::parse_usd("5").unwrap();
    native_priced_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope, &fixed_only)
        .unwrap();
    let receipt = native_priced_recovery_approve_on(
        &f.h.db_path,
        &selected,
        &scope,
        &pricing,
        &request,
        hash,
        PRICED_RECOVERY_CONSENT,
    )
    .unwrap();
    assert_eq!(receipt["schemaVersion"], 2);
    assert_eq!(receipt["proposal"], proposal);
    assert_eq!(receipt["executionAllowed"], false);
    assert_eq!(receipt["targetRequestsGranted"], 0);
    assert_eq!(receipt["status"], "approved_pending_priced_fresh_birth");
    if std::env::var("OVIRAPTOR_PRICED_RECOVERY_PUBLIC_FIXTURE").as_deref() == Ok("1") {
        // Public receipt from this disposable localhost fixture only; the
        // signature/private issuer key are never part of the emitted value.
        println!("PRICED_RECOVERY_PUBLIC_APPROVAL={receipt}");
    }
    assert_eq!(
        priced_recovery_original_tables(&db),
        originals,
        "no old approval, Root, balance, clock or draft writes"
    );
    let after = super::tests::application_table_snapshot(&db);
    drop(db);
    let reopened = native_protection_recovery_open(&f.h.db_path, false).unwrap();
    let tx = reopened.unchecked_transaction().unwrap();
    assert_eq!(
        native_priced_recovery_saved_on(&tx, &request).unwrap(),
        Some(receipt.clone())
    );
    assert_eq!(
        native_protection_recovery_saved_on(&tx, &request).unwrap(),
        None,
        "v1 cannot discover or upgrade v2"
    );
    assert_eq!(native_priced_recovery_for_source_on(&tx, &selected).unwrap(), Some(receipt.clone()));
    for field in 0..4 {
        let mut wrong = selected.clone();
        match field {
            0 => wrong.scan_id.push_str("-other"),
            1 => wrong.attempt_number += 1,
            2 => wrong.root_run_id.push_str("-other"),
            _ => wrong.protection_receipt_id.push_str("-other"),
        }
        assert_eq!(native_priced_recovery_for_source_on(&tx, &wrong).unwrap(), None);
    }
    assert_eq!(super::tests::application_table_snapshot(&tx), after);
    tx.commit().unwrap();
    assert_eq!(
        native_priced_recovery_approve_on(
            &f.h.db_path,
            &selected,
            &scope,
            &pricing,
            &request,
            hash,
            PRICED_RECOVERY_CONSENT
        )
        .unwrap(),
        receipt
    );
    assert_eq!(super::tests::application_table_snapshot(&reopened), after);
    let mut changed = pricing.clone();
    changed.price_revision = "different-v2".into();
    assert_eq!(
        native_priced_recovery_approve_on(
            &f.h.db_path,
            &selected,
            &scope,
            &changed,
            &request,
            hash,
            PRICED_RECOVERY_CONSENT
        )
        .unwrap_err(),
        "protection_recovery_priced_approval_conflict"
    );
    assert_eq!(super::tests::application_table_snapshot(&reopened), after);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);

    // Cold-edit the complete canonical price/proposal/column hashes while
    // keeping the original signature. Restore the exact schema before reading,
    // so this exercises signature binding rather than only schema rejection.
    let writer = db::open(&f.h.db_path).unwrap();
    let tx = writer.unchecked_transaction().unwrap();
    let mut forged = receipt.clone();
    forged["proposal"]["pricingInput"]["output"] = json!("5000000000");
    forged["proposal"]["financialTerms"]["pricing"]["output"] = json!("5000000000");
    let mut proposal = forged["proposal"].clone();
    proposal.as_object_mut().unwrap().remove("proposalHash");
    let changed_hash = crate::agent_runtime::store::stable_hash(&proposal.to_string());
    forged["proposal"]["proposalHash"] = json!(changed_hash);
    forged["proposalHash"] = json!(changed_hash);
    forged["approvalId"] = json!(native_priced_recovery_approval_id(&changed_hash));
    tx.execute_batch("DROP TRIGGER protection_recovery_priced_approval_no_update")
        .unwrap();
    tx.execute("UPDATE native_protection_recovery_priced_approvals SET approval_id=?1,proposal_hash=?2,approval_json=?3 WHERE request_id=?4",
        params![forged["approvalId"].as_str().unwrap(),changed_hash,forged.to_string(),request]).unwrap();
    tx.execute_batch(include_str!("native_protection_recovery_priced_schema.sql"))
        .unwrap();
    assert_eq!(
        native_priced_recovery_saved_on(&tx, &request).unwrap_err(),
        "protection_recovery_approval_signature_invalid"
    );
    assert_eq!(
        native_priced_recovery_for_source_on(&tx, &selected).unwrap_err(),
        "protection_recovery_approval_signature_invalid"
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&writer), after);
}

#[test]
fn priced_recovery_precision_protocol_model_and_scope_changes_refuse_before_any_write() {
    let (f, selected, scope) = protection_recovery_approval_fixture("priced-negatives");
    let pricing = priced_recovery_input();
    let mut db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    for text in ["5.0000000001", "5e0", "+5", "05", "5 ", "0", "6"] {
        let mut bad = pricing.clone();
        bad.max_budget_usd = text.into();
        assert!(
            native_priced_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope, &bad)
                .is_err(),
            "{text}"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    let mut future = pricing.clone();
    future.schema_version = 2;
    assert!(
        native_priced_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope, &future)
            .is_err()
    );
    let proposal =
        native_priced_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope, &pricing)
            .unwrap();
    let request = Uuid::new_v4().to_string();
    let hash = proposal["proposalHash"].as_str().unwrap();
    assert!(
        native_priced_recovery_approve_on(
            &f.h.db_path,
            &selected,
            &scope,
            &pricing,
            &request,
            hash,
            RECOVERY_EXPLICIT_CONSENT
        )
        .is_err(),
        "v1 consent cannot approve v2 pricing"
    );
    let mut changed = pricing.clone();
    changed.per_request = crate::recovery_finance::Money::from_nanos(11_000_000).unwrap();
    assert_eq!(
        native_priced_recovery_approve_on(
            &f.h.db_path,
            &selected,
            &scope,
            &changed,
            &request,
            hash,
            PRICED_RECOVERY_CONSENT
        )
        .unwrap_err(),
        "protection_recovery_priced_proposal_changed"
    );
    let mut wrong_model = scope.clone();
    wrong_model.model.push_str("-changed");
    assert!(native_priced_recovery_approve_on(
        &f.h.db_path,
        &selected,
        &wrong_model,
        &pricing,
        &request,
        hash,
        PRICED_RECOVERY_CONSENT
    )
    .is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn priced_recovery_extra_schema_trigger_refuses_before_any_approval_or_business_write() {
    let (f, selected, scope) = protection_recovery_approval_fixture("priced-writer-fault");
    let pricing = priced_recovery_input();
    let mut db = db::open(&f.h.db_path).unwrap();
    let proposal =
        native_priced_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope, &pricing)
            .unwrap();
    for body in [
        "SELECT RAISE(IGNORE);",
        "UPDATE projects SET name='forbidden';",
    ] {
        db.execute_batch(&format!("CREATE TRIGGER priced_approval_fault BEFORE INSERT ON native_protection_recovery_priced_approvals BEGIN {body} END;")).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert_eq!(
            native_priced_recovery_approve_on(
                &f.h.db_path,
                &selected,
                &scope,
                &pricing,
                &Uuid::new_v4().to_string(),
                proposal["proposalHash"].as_str().unwrap(),
                PRICED_RECOVERY_CONSENT
            )
            .unwrap_err(),
            "protection_recovery_priced_schema_changed"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        db.execute_batch("DROP TRIGGER priced_approval_fault")
            .unwrap();
    }
}

#[test]
fn priced_recovery_explicit_cancel_preserves_paid_history_and_cannot_reapprove_or_reseal() {
    let (f, selected, scope) = protection_recovery_approval_fixture("priced-cancel");
    let pricing = priced_recovery_input();
    let mut db = db::open(&f.h.db_path).unwrap();
    let proposal =
        native_priced_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope, &pricing)
            .unwrap();
    let request = Uuid::new_v4().to_string();
    let hash = proposal["proposalHash"].as_str().unwrap();
    let approval = native_priced_recovery_approve_on(
        &f.h.db_path,
        &selected,
        &scope,
        &pricing,
        &request,
        hash,
        PRICED_RECOVERY_CONSENT,
    )
    .unwrap();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let target = f.h.site_seen.lock().unwrap().len();
    let before = super::tests::application_table_snapshot(&db);
    let originals = |db: &rusqlite::Connection| {
        super::tests::application_table_snapshot(db)
            .into_iter()
            .filter(|(name, _)| name != "native_protection_recovery_priced_cancellations")
            .collect::<Vec<_>>()
    };
    let old = originals(&db);
    let id = approval["approvalId"].as_str().unwrap();
    for (wrong_request, wrong_id, consent) in [
        (
            request.clone(),
            id.to_string(),
            "approve_exact_new_scope_preserve_old_stop_and_costs",
        ),
        (
            request.clone(),
            "a".repeat(64),
            PRICED_RECOVERY_CANCEL_CONSENT,
        ),
        (
            Uuid::new_v4().to_string(),
            id.to_string(),
            PRICED_RECOVERY_CANCEL_CONSENT,
        ),
    ] {
        assert!(
            native_priced_recovery_cancel_on(&f.h.db_path, &wrong_request, &wrong_id, consent)
                .is_err()
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    let cancellation = native_priced_recovery_cancel_on(
        &f.h.db_path,
        &request,
        id,
        PRICED_RECOVERY_CANCEL_CONSENT,
    )
    .unwrap();
    assert_eq!(cancellation["authorityRevoked"], true);
    assert_eq!(cancellation["executionAllowed"], false);
    assert_eq!(cancellation["targetRequestsGranted"], 0);
    assert_eq!(
        originals(&db),
        old,
        "original approval and all old roots/fees stay byte-for-byte"
    );
    let after = super::tests::application_table_snapshot(&db);
    drop(db);
    let reopened = db::open(&f.h.db_path).unwrap();
    let tx = reopened.unchecked_transaction().unwrap();
    assert_eq!(
        native_priced_recovery_saved_on(&tx, &request).unwrap(),
        Some(approval.clone())
    );
    assert_eq!(
        native_priced_recovery_cancel_saved_on(&tx, &request).unwrap(),
        Some(cancellation.clone())
    );
    assert_eq!(native_priced_recovery_for_source_on(&tx, &selected).unwrap(), Some(approval.clone()));
    assert_eq!(super::tests::application_table_snapshot(&tx), after);
    tx.commit().unwrap();
    assert_eq!(
        native_priced_recovery_cancel_on(
            &f.h.db_path,
            &request,
            id,
            PRICED_RECOVERY_CANCEL_CONSENT
        )
        .unwrap(),
        cancellation
    );
    assert_eq!(
        native_priced_recovery_approve_on(
            &f.h.db_path,
            &selected,
            &scope,
            &pricing,
            &request,
            hash,
            PRICED_RECOVERY_CONSENT
        )
        .unwrap_err(),
        "protection_recovery_priced_approval_revoked"
    );
    assert_eq!(native_priced_recovery_approve_on(
        &f.h.db_path, &selected, &scope, &pricing, &Uuid::new_v4().to_string(), hash,
        PRICED_RECOVERY_CONSENT
    ).unwrap_err(), "protection_recovery_priced_approval_revoked",
        "a new window UUID cannot replace a revoked source approval");
    assert_eq!(super::tests::application_table_snapshot(&reopened), after);
    let tx = reopened.unchecked_transaction().unwrap();
    // Cold SQL cannot substitute the approval signature into the distinct revoke domain.
    tx.execute_batch("DROP TRIGGER protection_recovery_priced_cancellation_no_update")
        .unwrap();
    tx.execute("UPDATE native_protection_recovery_priced_cancellations SET signature=(SELECT signature FROM native_protection_recovery_priced_approvals WHERE request_id=?1) WHERE request_id=?1",[&request]).unwrap();
    let trigger: String=rusqlite::Connection::open_in_memory().and_then(|expected|{
        expected.execute_batch(include_str!("native_protection_recovery_priced_schema.sql"))?;
        expected.query_row("SELECT sql FROM sqlite_master WHERE name='protection_recovery_priced_cancellation_no_update'",[],|r|r.get(0))
    }).unwrap();
    tx.execute_batch(&trigger).unwrap();
    assert_eq!(
        native_priced_recovery_cancel_saved_on(&tx, &request).unwrap_err(),
        "protection_recovery_priced_cancellation_signature_invalid"
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&reopened), after);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), target);
}

#[test]
fn priced_recovery_cancel_extra_trigger_refuses_before_history_or_revocation_writes() {
    let (f, selected, scope) = protection_recovery_approval_fixture("priced-cancel-trigger");
    let pricing = priced_recovery_input();
    let mut db = db::open(&f.h.db_path).unwrap();
    let proposal =
        native_priced_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope, &pricing)
            .unwrap();
    let request = Uuid::new_v4().to_string();
    let approval = native_priced_recovery_approve_on(
        &f.h.db_path,
        &selected,
        &scope,
        &pricing,
        &request,
        proposal["proposalHash"].as_str().unwrap(),
        PRICED_RECOVERY_CONSENT,
    )
    .unwrap();
    let sdk = f.h.model_seen.lock().unwrap().len();
    for fault in [
        "SELECT RAISE(IGNORE)",
        "UPDATE projects SET name='forbidden-priced-cancel'",
    ] {
        db.execute_batch(&format!("CREATE TRIGGER injected_priced_cancel BEFORE INSERT ON native_protection_recovery_priced_cancellations BEGIN {fault}; END;")).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert_eq!(
            native_priced_recovery_cancel_on(
                &f.h.db_path,
                &request,
                approval["approvalId"].as_str().unwrap(),
                PRICED_RECOVERY_CANCEL_CONSENT
            )
            .unwrap_err(),
            "protection_recovery_priced_cancellation_schema_changed"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        db.execute_batch("DROP TRIGGER injected_priced_cancel")
            .unwrap();
    }
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
}

#[test]
fn priced_recovery_two_actual_connections_cancel_one_original_approval_once() {
    let (f, selected, scope) = protection_recovery_approval_fixture("priced-cancel-concurrent");
    let pricing = priced_recovery_input();
    let mut db = db::open(&f.h.db_path).unwrap();
    let proposal =
        native_priced_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope, &pricing)
            .unwrap();
    let request = Uuid::new_v4().to_string();
    let approval = native_priced_recovery_approve_on(
        &f.h.db_path,
        &selected,
        &scope,
        &pricing,
        &request,
        proposal["proposalHash"].as_str().unwrap(),
        PRICED_RECOVERY_CONSENT,
    )
    .unwrap();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let target = f.h.site_seen.lock().unwrap().len();
    let before = super::tests::application_table_snapshot(&db)
        .into_iter()
        .filter(|(name, _)| name != "native_protection_recovery_priced_cancellations")
        .collect::<Vec<_>>();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let id = approval["approvalId"].as_str().unwrap();
    let replies = std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..2 {
            let barrier = barrier.clone();
            let path = &f.h.db_path;
            let request = &request;
            workers.push(scope.spawn(move || {
                barrier.wait();
                native_priced_recovery_cancel_on(path, request, id, PRICED_RECOVERY_CANCEL_CONSENT)
            }));
        }
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(replies[0], replies[1]);
    assert_eq!(db.query_row("SELECT count(*) FROM native_protection_recovery_priced_cancellations WHERE request_id=?1",[&request],|r|r.get::<_,i64>(0)).unwrap(),1);
    let after = super::tests::application_table_snapshot(&db)
        .into_iter()
        .filter(|(name, _)| name != "native_protection_recovery_priced_cancellations")
        .collect::<Vec<_>>();
    assert_eq!(before, after);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), target);
}

#[test]
fn priced_recovery_two_actual_connections_approve_one_source_with_distinct_requests() {
    let (f, selected, new_scope) = protection_recovery_approval_fixture("priced-approve-two-windows");
    let pricing = priced_recovery_input();
    let mut db = db::open(&f.h.db_path).unwrap();
    let proposal = native_priced_recovery_preview_on(&mut db, &f.h.db_path, &selected, &new_scope, &pricing).unwrap();
    let hash = proposal["proposalHash"].as_str().unwrap();
    let before = priced_recovery_original_tables(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let targets = f.h.site_seen.lock().unwrap().len();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let replies = std::thread::scope(|threads| {
        let mut workers = Vec::new();
        for _ in 0..2 {
            let barrier = barrier.clone();
            let path = &f.h.db_path;
            let selected = &selected;
            let new_scope = &new_scope;
            let pricing = &pricing;
            workers.push(threads.spawn(move || {
                let request = Uuid::new_v4().to_string();
                barrier.wait();
                native_priced_recovery_approve_on(path, selected, new_scope, pricing, &request, hash, PRICED_RECOVERY_CONSENT)
            }));
        }
        workers.into_iter().map(|w| w.join().unwrap()).collect::<Vec<_>>()
    });
    let successes = replies.iter().filter_map(|r| r.as_ref().ok()).collect::<Vec<_>>();
    assert_eq!(successes.len(), 1);
    assert_eq!(successes[0]["executionAllowed"], false);
    assert_eq!(successes[0]["targetRequestsGranted"], 0);
    let errors = replies.iter().filter_map(|r| r.as_ref().err().map(String::as_str)).collect::<Vec<_>>();
    assert_eq!(errors, vec!["protection_recovery_priced_source_already_approved"]);
    let saved = successes[0].clone();
    let original_request = saved["requestId"].as_str().unwrap();
    let replay_barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let replayed = std::thread::scope(|threads| {
        let mut workers = Vec::new();
        for _ in 0..2 {
            let barrier = replay_barrier.clone();
            let path = &f.h.db_path;
            let selected = &selected;
            let new_scope = &new_scope;
            let pricing = &pricing;
            workers.push(threads.spawn(move || {
                barrier.wait();
                native_priced_recovery_approve_on(path, selected, new_scope, pricing, original_request, hash, PRICED_RECOVERY_CONSENT).unwrap()
            }));
        }
        workers.into_iter().map(|w| w.join().unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(replayed, vec![saved.clone(), saved]);
    assert_eq!(db.query_row("SELECT count(*) FROM native_protection_recovery_priced_approvals", [], |r|r.get::<_,i64>(0)).unwrap(), 1);
    assert_eq!(priced_recovery_original_tables(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), targets);
}
