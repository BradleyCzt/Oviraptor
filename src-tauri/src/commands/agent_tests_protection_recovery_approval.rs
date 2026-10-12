fn protection_recovery_approval_fixture(
    tag: &str,
) -> (
    ProtectionOriginalFixture,
    NativeProtectionRecoverySelection,
    NativeProtectionRecoveryScope,
) {
    let mut f = ProtectionOriginalFixture::with_original_target_owner(tag);
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    drop(f.session.take());
    drop(f.original_target_owner.take());
    // Use the real pause/finalizer after actual paid protected execution.
    assert_eq!(
        request_sentinel_pause(&f.h.db_path, &f.actor.scan_id).unwrap(),
        f.actor.attempt_number
    );
    assert!(finish_sentinel_pause(&f.h.db_path, &f.actor.scan_id, f.actor.attempt_number).unwrap());
    let db = db::open(&f.h.db_path).unwrap();
    // This is the independently selected NEW model, not an old-model rewrite.
    let settings = json!({"modelProfiles":[{"id":"new-recovery-profile","llm":f.h.context.environment.llm,
        "apiKey":"new-fixture-key","apiBase":f.h.context.environment.api_base,"deployment":"cloud"}],
        "activeModelProfileId":"new-recovery-profile","agentProxyEnabled":false});
    assert!(
        db.execute(
            "UPDATE config_profiles SET settings_json=?1",
            [settings.to_string()]
        )
        .unwrap()
            > 0
    );
    let selection = NativeProtectionRecoverySelection {
        scan_id: f.actor.scan_id.clone(),
        attempt_number: f.actor.attempt_number,
        root_run_id: f.actor.root_run_id.clone(),
        protection_receipt_id: f.receipt.clone(),
    };
    let model = agent_model_profile(&model_runtime_env(&sentinel_settings(&db)).unwrap(), None)
        .unwrap()
        .model;
    let scope = NativeProtectionRecoveryScope {
        project_id: 9001,
        task_name: "Explicit fresh recovery task".into(),
        urls: vec![f.h.context.target_url.clone()],
        allowed_methods: vec!["GET".into()],
        auth_session_ids: vec![],
        auth_session_scope_id: String::new(),
        model,
        proxy_binding_hash: None,
        orchestration_mode: "multi".into(),
        scan_mode: "standard".into(),
        closure: "proof".into(),
        skill_ids: vec![],
        instruction: "Inspect the explicitly approved target only".into(),
        max_budget_usd: 5.0,
        limits: [
            Some(60000),
            Some(60000),
            Some(60000),
            Some(20),
            Some(80),
            Some(0),
            Some(0),
            Some(0),
            Some(0),
            Some(600000),
        ],
        deadline_utc: (chrono::Utc::now() + chrono::Duration::hours(1))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    };
    (f, selection, scope)
}
fn protection_recovery_original_tables(db: &rusqlite::Connection) -> Vec<(String, Vec<String>)> {
    super::tests::application_table_snapshot(db)
        .into_iter()
        .filter(|(name, _)| name != "native_protection_recovery_approvals")
        .collect()
}
const RECOVERY_EXPLICIT_CONSENT: &str = "approve_exact_new_scope_preserve_old_stop_and_costs";

#[test]
fn protection_recovery_approval_approved_proxy_reference_keeps_credentials_out_of_receipt() {
    let (f, selected, mut scope) =
        protection_recovery_approval_fixture("recovery-approval-proxy-reference");
    let mut db = db::open(&f.h.db_path).unwrap();
    let proxy = "http://fixture-proxy-user:fixture-proxy-secret@127.0.0.1:9";
    let mut settings = sentinel_settings(&db);
    settings["agentProxyEnabled"] = json!(true);
    settings["authorizedProxyPool"] = json!([format!("ALL|{proxy}")]);
    assert!(
        db.execute(
            "UPDATE config_profiles SET settings_json=?1",
            [settings.to_string()]
        )
        .unwrap()
            > 0
    );
    scope.proxy_binding_hash = Some(crate::agent_runtime::store::stable_hash(proxy));
    let before = super::tests::application_table_snapshot(&db);
    let originals = protection_recovery_original_tables(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let proposal =
        native_protection_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope).unwrap();
    assert_eq!(
        proposal["modelBinding"]["proxyBinding"],
        json!(scope.proxy_binding_hash)
    );
    for secret in [
        proxy,
        "fixture-proxy-user",
        "fixture-proxy-secret",
        "new-fixture-key",
    ] {
        assert!(!proposal.to_string().contains(secret));
    }
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    for (binding, error) in [
        (
            proxy.to_string(),
            "protection_recovery_proxy_binding_invalid",
        ),
        ("0".repeat(64), "protection_recovery_proxy_not_approved"),
    ] {
        let mut wrong = scope.clone();
        wrong.proxy_binding_hash = Some(binding);
        assert_eq!(
            native_protection_recovery_preview_on(&mut db, &f.h.db_path, &selected, &wrong)
                .unwrap_err(),
            error
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    let request = Uuid::new_v4().to_string();
    let receipt = native_protection_recovery_approve_on(
        &f.h.db_path,
        &selected,
        &scope,
        &request,
        proposal["proposalHash"].as_str().unwrap(),
        RECOVERY_EXPLICIT_CONSENT,
    )
    .unwrap();
    let saved: String = db
        .query_row(
            "SELECT approval_json FROM native_protection_recovery_approvals WHERE request_id=?1",
            [&request],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved, receipt.to_string());
    for secret in [
        proxy,
        "fixture-proxy-user",
        "fixture-proxy-secret",
        "new-fixture-key",
    ] {
        assert!(!saved.contains(secret));
    }
    assert_eq!(protection_recovery_original_tables(&db), originals);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn protection_recovery_approval_actual_paid_pause_preview_and_durable_exact_receipt() {
    let (f, selected, scope) = protection_recovery_approval_fixture("recovery-approval-durable");
    let mut db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let originals = protection_recovery_original_tables(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let proposal =
        native_protection_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope).unwrap();
    assert_eq!(proposal["newScope"], json!(scope));
    assert_eq!(proposal["originalHistory"]["readiness"], "ready");
    assert_eq!(proposal["oldStopRetained"], true);
    assert_eq!(proposal["oldCostsRetained"], true);
    assert_eq!(proposal["executionAllowed"], false);
    assert_eq!(proposal["targetRequestsGranted"], 0);
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    let request = Uuid::new_v4().to_string();
    let hash = proposal["proposalHash"].as_str().unwrap();
    let receipt = native_protection_recovery_approve_on(
        &f.h.db_path,
        &selected,
        &scope,
        &request,
        hash,
        RECOVERY_EXPLICIT_CONSENT,
    )
    .unwrap();
    assert_eq!(receipt["proposal"], proposal);
    assert_eq!(receipt["status"], "approved_pending_fresh_birth");
    assert_eq!(receipt["executionAllowed"], false);
    assert_eq!(protection_recovery_original_tables(&db), originals);
    let after = super::tests::application_table_snapshot(&db);
    drop(db);
    let reopened = native_protection_recovery_open(&f.h.db_path, false).unwrap();
    let tx = reopened.unchecked_transaction().unwrap();
    assert_eq!(
        native_protection_recovery_saved_on(&tx, &request).unwrap(),
        Some(receipt.clone())
    );
    tx.commit().unwrap();
    assert_eq!(
        native_protection_recovery_approve_on(
            &f.h.db_path,
            &selected,
            &scope,
            &request,
            hash,
            RECOVERY_EXPLICIT_CONSENT
        )
        .unwrap(),
        receipt
    );
    assert_eq!(super::tests::application_table_snapshot(&reopened), after);
    let mut changed = scope.clone();
    changed.allowed_methods.push("HEAD".into());
    assert_eq!(
        native_protection_recovery_approve_on(
            &f.h.db_path,
            &selected,
            &changed,
            &request,
            hash,
            RECOVERY_EXPLICIT_CONSENT
        )
        .unwrap_err(),
        "protection_recovery_approval_conflict"
    );
    assert_eq!(super::tests::application_table_snapshot(&reopened), after);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn protection_recovery_approval_no_generic_confirmation_or_changed_scope_or_source() {
    let (f, selected, scope) = protection_recovery_approval_fixture("recovery-approval-negative");
    let mut db = db::open(&f.h.db_path).unwrap();
    let proposal =
        native_protection_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope).unwrap();
    let hash = proposal["proposalHash"].as_str().unwrap();
    let request = Uuid::new_v4().to_string();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    for consent in ["", "confirmed", "approve", "confirm_original_costs"] {
        assert_eq!(
            native_protection_recovery_approve_on(
                &f.h.db_path,
                &selected,
                &scope,
                &request,
                hash,
                consent
            )
            .unwrap_err(),
            "protection_recovery_explicit_approval_required"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    for (change, error) in [
        ("budget", "protection_recovery_proposal_changed"),
        ("method", "protection_recovery_proposal_changed"),
        ("model", "protection_recovery_selected_model_changed"),
        ("deadline", "protection_recovery_deadline_expired"),
        ("write", "protection_recovery_dimension_writer_unavailable"),
        ("project", "protection_recovery_project_conflict"),
    ] {
        let mut changed = scope.clone();
        match change {
            "budget" => changed.limits[4] = Some(81),
            "method" => changed.allowed_methods.push("HEAD".into()),
            "model" => changed.model = "unselected-model".into(),
            "deadline" => changed.deadline_utc = "2000-01-01T00:00:00Z".into(),
            "write" => changed.limits[6] = Some(1),
            "project" => changed.project_id = 9002,
            _ => unreachable!(),
        }
        assert_eq!(
            native_protection_recovery_approve_on(
                &f.h.db_path,
                &selected,
                &changed,
                &request,
                hash,
                RECOVERY_EXPLICIT_CONSENT
            )
            .unwrap_err(),
            error,
            "{change}"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    // A changed durable projection invalidates the original displayed choice.
    db.execute(
        "UPDATE sentinel_scans SET current_checkpoint='changed after preview' WHERE id=?1",
        [&selected.scan_id],
    )
    .unwrap();
    let changed = super::tests::application_table_snapshot(&db);
    for _ in 0..2 {
        assert_eq!(
            native_protection_recovery_approve_on(
                &f.h.db_path,
                &selected,
                &scope,
                &request,
                hash,
                RECOVERY_EXPLICIT_CONSENT
            )
            .unwrap_err(),
            "protection_recovery_proposal_changed"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), changed);
    }
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn protection_recovery_approval_concurrent_exact_request_commits_once() {
    let (f, selected, scope) = protection_recovery_approval_fixture("recovery-approval-concurrent");
    let mut db = db::open(&f.h.db_path).unwrap();
    let originals = protection_recovery_original_tables(&db);
    let proposal =
        native_protection_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope).unwrap();
    let hash = proposal["proposalHash"].as_str().unwrap().to_string();
    let request = Uuid::new_v4().to_string();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let jobs = (0..2)
        .map(|_| {
            let path = f.h.db_path.clone();
            let selected = selected.clone();
            let scope = scope.clone();
            let hash = hash.clone();
            let request = request.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                native_protection_recovery_approve_on(
                    &path,
                    &selected,
                    &scope,
                    &request,
                    &hash,
                    RECOVERY_EXPLICIT_CONSENT,
                )
            })
        })
        .collect::<Vec<_>>();
    let results = jobs
        .into_iter()
        .map(|job| job.join().unwrap().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results[0], results[1]);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_protection_recovery_approvals",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(protection_recovery_original_tables(&db), originals);
    let after = super::tests::application_table_snapshot(&db);
    assert_eq!(
        native_protection_recovery_approve_on(
            &f.h.db_path,
            &selected,
            &scope,
            &Uuid::new_v4().to_string(),
            &hash,
            RECOVERY_EXPLICIT_CONSENT
        )
        .unwrap_err(),
        "protection_recovery_approval_conflict"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), after);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn protection_recovery_approval_immutable_rowid_and_cold_rehashed_json_cannot_authorize() {
    let (f, selected, scope) = protection_recovery_approval_fixture("recovery-approval-cold");
    let mut db = db::open(&f.h.db_path).unwrap();
    let proposal =
        native_protection_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope).unwrap();
    let hash = proposal["proposalHash"].as_str().unwrap();
    let request = Uuid::new_v4().to_string();
    let receipt = native_protection_recovery_approve_on(
        &f.h.db_path,
        &selected,
        &scope,
        &request,
        hash,
        RECOVERY_EXPLICIT_CONSENT,
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    db.pragma_update(None, "recursive_triggers", "OFF").unwrap();
    for sql in [
        "UPDATE native_protection_recovery_approvals SET approval_json='{}'",
        "DELETE FROM native_protection_recovery_approvals",
        "INSERT OR REPLACE INTO native_protection_recovery_approvals(rowid,approval_id,request_id,source_scan_id,source_attempt_number,source_root_run_id,protection_receipt_id,proposal_hash,approval_json,signature) SELECT rowid,printf('%064d',2),'replacement','other',1,'other','other',printf('%064d',3),'{}',signature FROM native_protection_recovery_approvals",
    ] {
        assert!(db.execute(sql,[]).unwrap_err().to_string().contains("protection_recovery_approval_immutable"));
        assert_eq!(super::tests::application_table_snapshot(&db),before);
    }
    let tx = db.unchecked_transaction().unwrap();
    // Only this disposable fixture: emulate cold offline data damage, restore
    // the exact trigger before the reader. The original signature stays old.
    let ddl: String = tx
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='protection_recovery_approval_no_update'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    tx.execute_batch("DROP TRIGGER protection_recovery_approval_no_update")
        .unwrap();
    let mut forged = receipt.clone();
    forged["proposal"]["newScope"]["limits"][4] = json!(9999);
    forged["proposal"]
        .as_object_mut()
        .unwrap()
        .remove("proposalHash");
    let forged_hash = crate::agent_runtime::store::stable_hash(&forged["proposal"].to_string());
    forged["proposal"]["proposalHash"] = json!(forged_hash);
    forged["proposalHash"] = json!(forged_hash);
    forged["approvalId"] = json!(native_protection_recovery_approval_id(&forged_hash));
    assert_eq!(tx.execute("UPDATE native_protection_recovery_approvals SET approval_id=?1,proposal_hash=?2,approval_json=?3 WHERE request_id=?4",
        params![forged["approvalId"].as_str(),forged_hash,forged.to_string(),request]).unwrap(),1);
    tx.execute_batch(&ddl).unwrap();
    let damaged = super::tests::application_table_snapshot(&tx);
    for _ in 0..2 {
        assert_eq!(
            native_protection_recovery_saved_on(&tx, &request).unwrap_err(),
            "protection_recovery_approval_signature_invalid"
        );
        assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
    }
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(
        native_protection_recovery_saved_on(&snapshot, &request).unwrap(),
        Some(receipt)
    );
    snapshot.commit().unwrap();
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn protection_recovery_approval_original_target_owner_blocks_new_authority_until_exit() {
    let (f, selected, scope) =
        protection_recovery_approval_fixture("recovery-approval-target-owner");
    let mut db = db::open(&f.h.db_path).unwrap();
    let proposal =
        native_protection_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope).unwrap();
    assert_eq!(proposal["originalTargetExecutorsVerified"], true);
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let owner = claim_native_invocation(
        &f.h.db_path,
        &selected.scan_id,
        selected.attempt_number,
        "target",
        &f.h.context.target_url,
    )
    .unwrap();
    let request = Uuid::new_v4().to_string();
    let error = native_protection_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope)
        .unwrap_err();
    assert!(
        error.starts_with("recovery_history_target_not_idle:native_invocation_not_owned:"),
        "{error}"
    );
    let error = native_protection_recovery_approve_on(
        &f.h.db_path,
        &selected,
        &scope,
        &request,
        proposal["proposalHash"].as_str().unwrap(),
        RECOVERY_EXPLICIT_CONSENT,
    )
    .unwrap_err();
    assert!(
        error.starts_with("recovery_history_target_not_idle:native_invocation_not_owned:"),
        "{error}"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    drop(owner);
    // The verified proof owns the original outer inode through its lifetime.
    let snapshot = db.transaction().unwrap();
    let proof =
        native_protection_recovery_proposal_in(&snapshot, &f.h.db_path, &selected, &scope).unwrap();
    assert!(claim_native_invocation(
        &f.h.db_path,
        &selected.scan_id,
        selected.attempt_number,
        "target",
        &f.h.context.target_url
    )
    .is_err());
    proof
        .verify_on(&snapshot, &f.h.db_path, &selected, &scope)
        .unwrap();
    snapshot.commit().unwrap();
    assert!(claim_native_invocation(
        &f.h.db_path,
        &selected.scan_id,
        selected.attempt_number,
        "target",
        &f.h.context.target_url
    )
    .is_err());
    drop(proof);
    let receipt = native_protection_recovery_approve_on(
        &f.h.db_path,
        &selected,
        &scope,
        &request,
        proposal["proposalHash"].as_str().unwrap(),
        RECOVERY_EXPLICIT_CONSENT,
    )
    .unwrap();
    assert_eq!(receipt["executionAllowed"], false);
    assert_eq!(
        protection_recovery_original_tables(&db),
        before
            .into_iter()
            .filter(|(name, _)| name != "native_protection_recovery_approvals")
            .collect::<Vec<_>>()
    );
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn protection_recovery_approval_missing_original_target_inode_never_backfills_exit() {
    use sha2::{Digest, Sha256};
    let (f, selected, scope) =
        protection_recovery_approval_fixture("recovery-approval-missing-target-inode");
    let mut db = db::open(&f.h.db_path).unwrap();
    let proposal =
        native_protection_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope).unwrap();
    let canonical = fs::canonicalize(&f.h.db_path).unwrap();
    let mut directory_name = canonical.file_name().unwrap().to_os_string();
    directory_name.push(".invocations");
    let directory = canonical.with_file_name(directory_name);
    let key = serde_json::to_vec(&(
        &selected.scan_id,
        selected.attempt_number,
        "target",
        &f.h.context.target_url,
    ))
    .unwrap();
    let inode = directory.join(format!("{:x}.lock", Sha256::digest(key)));
    // Remove only this disposable fixture's already released original inode.
    assert!(inode.is_file());
    fs::remove_file(&inode).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let files = fs::read_dir(&directory).unwrap().count();
    for _ in 0..2 {
        assert_eq!(
            native_protection_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope)
                .unwrap_err(),
            "recovery_history_original_target_exit_missing"
        );
        assert_eq!(
            native_protection_recovery_approve_on(
                &f.h.db_path,
                &selected,
                &scope,
                &Uuid::new_v4().to_string(),
                proposal["proposalHash"].as_str().unwrap(),
                RECOVERY_EXPLICIT_CONSENT
            )
            .unwrap_err(),
            "recovery_history_original_target_exit_missing"
        );
        assert!(!inode.exists());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), files);
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
