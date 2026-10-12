#[test]
fn signed_withdrawal_real_prepare_supervisor_issues_once_and_cold_fact_cannot_be_resigned() {
    use crate::agent_runtime::multi_agent::attempts;
    let _real = RealSpecialistTransport::enter();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let mut f = root_tick_fixture("signed-withdrawal", &format!("http://127.0.0.1:{port}/v1"));
    drop(f.parent.take());
    let session = multi_agent_prepare(&mut f.context).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 3);
    let db = db::open(&f.context.db_path).unwrap();
    let declaration = crate::agent_runtime::web_mode::root::read(&db, &session.lease.root_run_id).unwrap().unwrap();
    assert!(declaration.lease_expiry_supervision().is_some());
    let protection = declaration.protection_supervision().expect("new ordinary Root must freeze the original protection contract at birth");
    assert_eq!(protection["source"], "original_signed_protection_response_receipt");
    assert_eq!(protection["executionGrant"], false);
    let decoded: crate::agent_runtime::web_mode::root::NewRootModeDeclaration = serde_json::from_value(serde_json::to_value(&declaration).unwrap()).unwrap();
    assert_eq!(decoded.clone().with_protection_supervision().unwrap_err(), "web_mode_private_creation_missing");
    assert_eq!(decoded.with_lease_expiry_supervision().unwrap_err(), "web_mode_private_creation_missing");
    expire_worker_deadline(&db, &session.executor.run_id);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        session.supervisor.check().unwrap();
        let n: i64 = db
            .query_row(
                "SELECT count(*) FROM agent_worker_expiry_issuance_seals WHERE root_run_id=?1",
                [&session.lease.root_run_id],
                |r| r.get(0),
            )
            .unwrap();
        if n == 1 {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let worker = attempts::current(&db, &session.lease, &session.executor.assignment_id).unwrap();
    assert_eq!(worker.state, "expired");
    let before = super::tests::application_table_snapshot(&db);
    let issuer = original_withdrawal_issuer_on(&db, &session.lease)
        .unwrap()
        .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    issuer.verify_issued(&tx, &worker.id).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
    drop(tx);
    let actor = session.lease.clone();
    drop(session);
    let reopened = db::open(&f.context.db_path).unwrap();
    let issuer = original_withdrawal_issuer_on(&reopened, &actor)
        .unwrap()
        .unwrap();
    let tx = reopened.unchecked_transaction().unwrap();
    issuer.verify_issued(&tx, &worker.id).unwrap();
    drop(tx);
    assert_eq!(super::tests::application_table_snapshot(&reopened), before);
    for sql in ["UPDATE agent_worker_expiry_issuance_seals SET signature=zeroblob(32)", "DELETE FROM agent_worker_expiry_issuance_seals", "INSERT OR REPLACE INTO agent_worker_expiry_issuance_seals SELECT * FROM agent_worker_expiry_issuance_seals"] {
        assert!(reopened.execute_batch(sql).is_err());
        assert_eq!(super::tests::application_table_snapshot(&reopened),before);
    }
    let tx = reopened.unchecked_transaction().unwrap();
    tx.execute_batch("DROP TRIGGER worker_expiry_seal_no_delete; DELETE FROM agent_worker_expiry_issuance_seals;").unwrap();
    tx.execute_batch(include_str!(
        "../agent_runtime/multi_agent/attempts/withdrawal_receipt/schema.sql"
    ))
    .unwrap();
    attempts::withdrawal_receipt::verify_original(&tx, &worker.id).unwrap();
    let unsigned = super::tests::application_table_snapshot(&tx);
    assert_eq!(
        issuer.verify_issued(&tx, &worker.id).unwrap_err(),
        "withdrawal_issuer_original_seal_missing"
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), unsigned);
    drop(tx);
    // Cold DB can recompute public SHA, not the private issuance signature.
    let tx = reopened.unchecked_transaction().unwrap();
    tx.execute_batch("DROP TRIGGER worker_expiry_receipt_no_update")
        .unwrap();
    let raw: String = tx
        .query_row(
            "SELECT fact_json FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1",
            [&worker.id],
            |r| r.get(0),
        )
        .unwrap();
    // Replace an unobservable historical before-hash, preserving canonical shape.
    let altered = raw.replace("\"beforeWorker\":\"", "\"beforeWorker\":\"cold-");
    assert_ne!(altered, raw);
    tx.execute("UPDATE agent_worker_expiry_receipts SET fact_json=?1,fact_hash=?2 WHERE worker_attempt_id=?3", params![altered,crate::agent_runtime::store::stable_hash(&altered),worker.id]).unwrap();
    tx.execute_batch(include_str!(
        "../agent_runtime/multi_agent/attempts/withdrawal_receipt/schema.sql"
    ))
    .unwrap();
    attempts::withdrawal_receipt::verify_original(&tx, &worker.id).unwrap();
    let damaged = super::tests::application_table_snapshot(&tx);
    assert!(issuer.verify_issued(&tx, &worker.id).is_err());
    assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
    tx.execute_batch("DROP TRIGGER worker_expiry_seal_no_update")
        .unwrap();
    let message: String = tx
        .query_row(
            "SELECT binding_json FROM agent_worker_expiry_issuance_seals",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut recomputed: JsonValue = serde_json::from_str(&message).unwrap();
    recomputed["canonicalFact"] = altered.into();
    tx.execute(
        "UPDATE agent_worker_expiry_issuance_seals SET binding_json=?1",
        [recomputed.to_string()],
    )
    .unwrap();
    tx.execute_batch(include_str!(
        "../agent_runtime/multi_agent/attempts/withdrawal_receipt/schema.sql"
    ))
    .unwrap();
    let forged = super::tests::application_table_snapshot(&tx);
    assert_eq!(
        issuer.verify_issued(&tx, &worker.id).unwrap_err(),
        "withdrawal_issuer_signature_invalid"
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), forged);
    drop(tx);
    assert_eq!(seen.lock().unwrap().len(), 3);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn signed_withdrawal_valid_birth_without_supervision_protocol_stays_unsigned_readonly() {
    let f =
        root_tick_fixture_protocol("unsigned-original-contract", "http://127.0.0.1:9/v1", false);
    let db = db::open(&f.context.db_path).unwrap();
    let declaration = crate::agent_runtime::web_mode::root::read(&db, &f.actor.root_run_id).unwrap().unwrap();
    assert!(declaration.lease_expiry_supervision().is_none());
    let before = super::tests::application_table_snapshot(&db);
    assert!(original_withdrawal_issuer_on(&db, &f.actor)
        .unwrap()
        .is_none());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}

#[test]
fn signed_withdrawal_new_contract_refuses_unsigned_start_before_any_application_write() {
    let mut f = root_tick_fixture("signed-no-downgrade", "http://127.0.0.1:9/v1");
    drop(f.parent.take());
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    match crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor::start(&f.context.db_path, &f.actor) {
        Ok(_) => panic!("new signed contract must not start unsigned"),
        Err(error) => assert_eq!(error, "worker_supervisor_original_signed_issuer_required"),
    }
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}

#[test]
fn signed_withdrawal_prepare_bad_or_missing_birth_refuses_before_c_fees_and_sdk() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|request| {
        (200, "application/json", changed_fact_response(&request, true))
    }));
    for (tag, damage, expected) in [
        ("missing", "DROP TRIGGER root_mode_birth_no_delete; DELETE FROM agent_root_mode_birth_seals", "withdrawal_issuer_required_birth_seal_missing"),
        ("bad", "DROP TRIGGER root_mode_birth_no_update; UPDATE agent_root_mode_birth_seals SET signature=zeroblob(32)", "root_mode_birth_signature_invalid"),
    ] {
        let mut f = root_tick_fixture(&format!("signed-preflight-{tag}"), &format!("http://127.0.0.1:{port}/v1"));
        drop(f.parent.take());
        let db = db::open(&f.context.db_path).unwrap();
        db.execute_batch(damage).unwrap();
        db.execute_batch(include_str!("../agent_runtime/web_mode/root/birth_seal.sql")).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        match multi_agent_prepare(&mut f.context) {
            Ok(_) => panic!("damaged original birth must not prepare"),
            Err(error) => assert_eq!(error, expected),
        }
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert!(seen.lock().unwrap().is_empty());
    }
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn signed_withdrawal_actual_received_saved_delivery_preserves_original_seal() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{
            attempts, parent_invocation_owner::ParentInvocationOwner, scheduler,
            supervisor::WorkerSupervisor,
        },
    };
    let _real = RealSpecialistTransport::enter();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response("saved original signed mapper result"),
        )
    }));
    let mut f = root_tick_fixture("signed-saved", &format!("http://127.0.0.1:{port}/v1"));
    drop(f.parent.take());
    let db = db::open(&f.context.db_path).unwrap();
    let child = scheduler::schedule_child(
        &db,
        &f.actor,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "signed-saved-original",
        &json!({"objective":"original readonly result"}),
        1,
        &["evidence.read".into(), "mailbox.write".into()],
        8000,
        1,
    )
    .unwrap();
    scheduler::start_child_or_release(&db, &f.actor, &child).unwrap();
    let issuer = original_withdrawal_issuer_on(&db, &f.actor)
        .unwrap()
        .unwrap();
    let owner = ParentInvocationOwner::claim(
        &f.context.db_path,
        &f.actor.scan_id,
        f.actor.attempt_number,
        &f.actor.root_run_id,
    )
    .unwrap();
    let service =
        WorkerSupervisor::start_owned_signed(&f.context.db_path, &f.actor, owner, issuer).unwrap();
    f.context.supervision = Some(service.ticket());
    let (text, usage) = multi_agent_child_round_transport(
        &f.context,
        &f.actor,
        &child,
        "readonly",
        json!({"fixed":true}),
    )
    .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    db.execute("UPDATE agent_assignment_attempts SET expires_at=datetime('now','localtime','+1 second') WHERE child_run_id=?1",[&child.run_id]).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        service.check().unwrap();
        if attempts::current(&db, &f.actor, &child.assignment_id)
            .unwrap()
            .state
            == "expired"
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let worker = attempts::current(&db, &f.actor, &child.assignment_id).unwrap();
    let issuer = original_withdrawal_issuer_on(&db, &f.actor)
        .unwrap()
        .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    issuer.verify_issued(&tx, &worker.id).unwrap();
    drop(tx);
    let tx = db.unchecked_transaction().unwrap();
    let unsettled = super::tests::application_table_snapshot(&tx);
    assert!(NativeCoordinatorFrame::lease_closed_mapper(&tx, &f.context, &f.actor, &child).is_err());
    assert_eq!(super::tests::application_table_snapshot(&tx), unsettled);
    drop(tx);
    let payload = json!({"summary":text});
    complete_readonly_assessment(&db, &f.actor, &child, &usage, &payload).unwrap();
    let saved = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    issuer.verify_issued(&tx, &worker.id).unwrap();
    let sdk_exit = native_coordinator_lease_original_sdk_exit(&f.context, &f.actor, &child).unwrap();
    assert!(native_coordinator_lease_original_sdk_exit(&f.context, &f.actor, &child).is_err());
    let frame = NativeCoordinatorFrame::lease_closed_mapper(&tx, &f.context, &f.actor, &child).unwrap();
    assert_eq!(frame.kind, "lease-expired");
    frame.verify(&tx, &f.context, &f.actor).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&tx), saved);
    drop(tx);
    drop(sdk_exit);
    for damage in [
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='evidence_summary'",
        "UPDATE agent_messages SET delivery_attempts=2 WHERE kind='evidence_summary'",
        "UPDATE agent_messages SET dedup_key='wrong' WHERE kind='evidence_summary'",
        "UPDATE agent_messages SET artifact_refs_json='[\"cold-artifact\"]' WHERE kind='evidence_summary'",
        "UPDATE agent_messages SET payload_json='{}' WHERE kind='evidence_summary'",
        "UPDATE agent_runs SET used_cached_tokens=used_cached_tokens+1 WHERE role='spa_api_mapper'",
        "UPDATE agent_assignments SET reserved_tokens=1 WHERE role='spa_api_mapper'",
        "DELETE FROM agent_collaboration_events WHERE entity_type='mailbox_message'",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'",
        "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1",
        "UPDATE agent_runs SET cancel_requested_at='2000-01-01 00:00:00' WHERE role='spa_api_mapper'",
        "DROP TRIGGER worker_expiry_seal_no_delete; DELETE FROM agent_worker_expiry_issuance_seals",
    ] {
        let tx = db.unchecked_transaction().unwrap();
        tx.execute_batch(damage).unwrap();
        // Restore only DDL in this disposable transaction; never repair facts.
        tx.execute_batch(include_str!("../agent_runtime/multi_agent/attempts/withdrawal_receipt/schema.sql")).unwrap();
        let damaged = super::tests::application_table_snapshot(&tx);
        assert!(NativeCoordinatorFrame::lease_closed_mapper(&tx, &f.context, &f.actor, &child).is_err(), "{damage}");
        assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
        drop(tx);
        assert_eq!(super::tests::application_table_snapshot(&db), saved);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
    let readonly = rusqlite::Connection::open_with_flags(&f.context.db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let tx = readonly.unchecked_transaction().unwrap();
    let reopened_frame = NativeCoordinatorFrame::lease_closed_mapper(&tx, &f.context, &f.actor, &child).unwrap();
    assert_eq!(reopened_frame.fact(), frame.fact());
    reopened_frame.verify(&tx, &f.context, &f.actor).unwrap();
    drop(tx);
    // Only the disposable fixture's original SDK inode is moved and restored.
    let canonical = fs::canonicalize(&f.context.db_path).unwrap();
    let mut directory_name = canonical.file_name().unwrap().to_os_string();
    directory_name.push(".invocations");
    let key = serde_json::to_vec(&(&f.actor.scan_id, f.actor.attempt_number, "specialist-sdk", &child.run_id)).unwrap();
    let inode = canonical.with_file_name(directory_name).join(format!("{:x}.lock", sha2::Sha256::digest(key)));
    let backup = inode.with_extension("fixture-backup");
    fs::rename(&inode, &backup).unwrap();
    assert_eq!(native_coordinator_lease_original_sdk_exit(&f.context, &f.actor, &child).unwrap_err(), "root_lease_closed_original_sdk_exit_missing");
    assert!(!inode.exists());
    assert_eq!(super::tests::application_table_snapshot(&db), saved);
    fs::rename(&backup, &inode).unwrap();
    let original_exit = native_coordinator_lease_original_sdk_exit(&f.context, &f.actor, &child).unwrap();
    drop(original_exit);
    complete_readonly_assessment(&db, &f.actor, &child, &usage, &payload).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), saved);
    assert_eq!(seen.lock().unwrap().len(), 1);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(service);
}
