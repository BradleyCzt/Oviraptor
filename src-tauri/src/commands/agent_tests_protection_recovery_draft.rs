#[test]
fn protection_recovery_draft_ordinary_root_publisher_cannot_expand_approved_scope() {
    let (f, _, _, request, _) = protection_recovery_draft_fixture("recovery-root-route");
    let draft = native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap();
    let scan = draft["scanId"].as_str().unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let work_dir = f.h.db_path.parent().unwrap().join("recovery-denied-frontend");
    let assert_frontend_denied = || {
        let result = launch_frontend_recon_producer(
            f.h.db_path.clone(), scan.to_string(), 1,
            work_dir.join("unused-helper"), work_dir.clone(),
            vec![("recovery".into(), f.h.context.target_url.clone())],
            vec![], String::new(), false, false, OsString::from("unused-runtime"),
            AgentBudgetSettings::from_json(&serde_json::json!({})), 1,
            work_dir.join("unused.log"), None,
            serde_json::from_value::<FrontendReconConfig>(serde_json::json!({})).unwrap(),
        );
        assert_eq!(result.err().unwrap(), "protection_recovery_requires_dedicated_frontend_budget");
        assert!(!work_dir.exists());
    };
    assert_frontend_denied();

    for attempt in [1, 2] {
        assert_eq!(
            persist_frozen_web_execution_plan(
                &f.h.db_path,
                scan,
                attempt,
                &f.h.context.target_url,
                &f.h.context.execution_plan
            )
            .unwrap_err(),
            "protection_recovery_requires_dedicated_root_budget"
        );
    }
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    // Only a disposable fixture database loses the relation. The signed
    // action still prevents the ordinary route; no authority is backfilled.
    db.execute_batch("DROP TRIGGER protection_recovery_draft_no_delete")
        .unwrap();
    db.execute(
        "DELETE FROM main.native_protection_recovery_drafts WHERE scan_id=?1",
        [scan],
    )
    .unwrap();
    db.execute_batch(include_str!("native_protection_recovery_schema.sql"))
        .unwrap();
    let damaged = super::tests::application_table_snapshot(&db);
    assert_frontend_denied();
    assert_eq!(
        persist_frozen_web_execution_plan(
            &f.h.db_path,
            scan,
            1,
            &f.h.context.target_url,
            &f.h.context.execution_plan
        )
        .unwrap_err(),
        "protection_recovery_requires_dedicated_root_budget"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), damaged);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

fn protection_recovery_draft_fixture(
    tag: &str,
) -> (
    ProtectionOriginalFixture,
    NativeProtectionRecoverySelection,
    NativeProtectionRecoveryScope,
    String,
    JsonValue,
) {
    let (f, selection, mut scope) = protection_recovery_approval_fixture(tag);
    scope.urls = vec![
        format!("{}/", f.h.context.target_url.trim_end_matches('/')),
        format!("{}?selected=alpha,beta;gamma", f.h.context.target_url),
    ];
    scope.urls.sort();
    let mut db = db::open(&f.h.db_path).unwrap();
    let proposal =
        native_protection_recovery_preview_on(&mut db, &f.h.db_path, &selection, &scope).unwrap();
    let request = Uuid::new_v4().to_string();
    let approval = native_protection_recovery_approve_on(
        &f.h.db_path,
        &selection,
        &scope,
        &request,
        proposal["proposalHash"].as_str().unwrap(),
        RECOVERY_EXPLICIT_CONSENT,
    )
    .unwrap();
    (f, selection, scope, request, approval)
}
fn protection_recovery_draft_assert_original_rows(
    before: &[(String, Vec<String>)],
    db: &rusqlite::Connection,
) {
    let after = super::tests::application_table_snapshot(db);
    for (table, rows) in before {
        let current = &after.iter().find(|(name, _)| name == table).unwrap().1;
        if matches!(
            table.as_str(),
            "sentinel_scans"
                | "sentinel_targets"
                | "sentinel_scan_contexts"
                | "native_web_mode_drafts"
                | "native_web_mode_actions"
                | "native_protection_recovery_drafts"
                | "native_protection_recovery_scope_budgets"
        ) {
            for row in rows {
                assert!(current.contains(row), "changed original row in {table}");
            }
        } else {
            assert_eq!(rows, current, "unexpected write to {table}");
        }
    }
}
#[test]
fn protection_recovery_draft_actual_paid_approval_unique_literal_scope_and_generic_start_closed() {
    let (f, selected, scope, request, approval) =
        protection_recovery_draft_fixture("recovery-draft-literal");
    let mut db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let value = native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap();
    let scan = value["scanId"].as_str().unwrap();
    assert_ne!(scan, selected.scan_id);
    assert_eq!(value["approval"], approval);
    assert_eq!(value["executionAllowed"], false);
    assert_eq!(value["identitiesBound"], false);
    let mut q = db
        .prepare("SELECT url FROM sentinel_targets WHERE scan_id=?1 ORDER BY url")
        .unwrap();
    let urls = q
        .query_map([scan], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    drop(q);
    assert_eq!(urls, scope.urls);
    protection_recovery_draft_assert_original_rows(&before, &db);
    let staged = super::tests::application_table_snapshot(&db);
    assert_eq!(
        native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap(),
        value
    );
    assert_eq!(super::tests::application_table_snapshot(&db), staged);
    let ro = native_protection_recovery_open(&f.h.db_path, false).unwrap();
    let snapshot = ro.unchecked_transaction().unwrap();
    assert_eq!(
        native_protection_recovery_draft_saved_on(&snapshot, &request).unwrap(),
        Some(value.clone())
    );
    assert_eq!(
        crate::agent_runtime::web_mode::action::read_hash(&snapshot, scan)
            .unwrap()
            .unwrap()
            .len(),
        64
    );
    snapshot.commit().unwrap();
    drop(ro);
    for mode in [
        WebStartMode::Confirm,
        WebStartMode::Resume,
        WebStartMode::Retry,
    ] {
        let tx = db.transaction().unwrap();
        assert_eq!(
            prepare_web_startup_in(&tx, &f.h.root, scan, mode)
                .err()
                .unwrap(),
            "protection_recovery_requires_dedicated_fresh_birth"
        );
        tx.rollback().unwrap();
    }
    assert_eq!(super::tests::application_table_snapshot(&db), staged);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
#[test]
fn protection_recovery_draft_source_change_and_late_writer_fault_never_create_partial_task() {
    let (f, selected, _scope, request, _approval) =
        protection_recovery_draft_fixture("recovery-draft-rollback");
    let db = db::open(&f.h.db_path).unwrap();
    let old: String = db
        .query_row(
            "SELECT current_checkpoint FROM sentinel_scans WHERE id=?1",
            [&selected.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    db.execute(
        "UPDATE sentinel_scans SET current_checkpoint='changed-after-approval' WHERE id=?1",
        [&selected.scan_id],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap_err(),
        "protection_recovery_proposal_changed"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    db.execute(
        "UPDATE sentinel_scans SET current_checkpoint=?1 WHERE id=?2",
        params![old, selected.scan_id],
    )
    .unwrap();
    db.execute_batch("CREATE TRIGGER recovery_draft_fixture_fault BEFORE INSERT ON native_protection_recovery_drafts BEGIN SELECT RAISE(ABORT,'fixture-draft-relation-fault'); END;").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap_err(),
        "protection_recovery_draft_schema_changed"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    db.execute_batch("DROP TRIGGER recovery_draft_fixture_fault; CREATE TRIGGER recovery_draft_fixture_target_fault BEFORE INSERT ON sentinel_targets BEGIN SELECT RAISE(ABORT,'fixture-target-insert-fault'); END;").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let error = native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap_err();
    assert!(error.contains("fixture-target-insert-fault"), "{error}");
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    db.execute_batch("DROP TRIGGER recovery_draft_fixture_target_fault;")
        .unwrap();
    assert!(native_protection_recovery_create_draft_on(&f.h.db_path, &request).is_ok());
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn protection_recovery_draft_cold_mode_action_reclassification_and_unsigned_policy_refused() {
    let (f, _selected, _scope, request, _approval) =
        protection_recovery_draft_fixture("recovery-draft-cold-provenance");
    let value = native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap();
    let scan = value["scanId"].as_str().unwrap();
    let mut db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    for sql in [
        "UPDATE native_protection_recovery_drafts SET proposal_hash=proposal_hash",
        "DELETE FROM native_protection_recovery_drafts",
        "INSERT OR REPLACE INTO native_protection_recovery_drafts(rowid,approval_id,request_id,scan_id,draft_id,source_scan_id,proposal_hash,draft_json,signature) SELECT rowid,printf('%064d',1),'different-request','different-scan','different-draft',source_scan_id,proposal_hash,draft_json,signature FROM native_protection_recovery_drafts",
    ] {
        db.pragma_update(None, "recursive_triggers", "OFF").unwrap();
        assert!(db.execute(sql, []).unwrap_err().to_string().contains("protection_recovery_draft_immutable"));
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    for operation in ["mode", "action_kind", "policy"] {
        let tx = db.transaction().unwrap();
        let trigger = match operation {
            "mode" => "web_mode_draft_immutable_update",
            "action_kind" => "web_mode_action_no_update",
            _ => "protection_recovery_draft_no_update",
        };
        let ddl: String = tx
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
                [trigger],
                |r| r.get(0),
            )
            .unwrap();
        tx.execute_batch(&format!("DROP TRIGGER {trigger}"))
            .unwrap();
        match operation {
            "mode" => {
                tx.execute(
                    "UPDATE native_web_mode_drafts SET mode='single' WHERE scan_id=?1",
                    [scan],
                )
                .unwrap();
            }
            "action_kind" => {
                let text: String = tx
                    .query_row(
                        "SELECT action_json FROM native_web_mode_actions WHERE scan_id=?1",
                        [scan],
                        |r| r.get(0),
                    )
                    .unwrap();
                let mut action: crate::agent_runtime::web_mode::action::NewWebAction =
                    serde_json::from_str(&text).unwrap();
                action.kind = "gap_followup".into();
                let forged = serde_json::to_string(&action).unwrap();
                tx.execute("UPDATE native_web_mode_actions SET action_json=?1,action_hash=?2 WHERE scan_id=?3",params![forged,crate::agent_runtime::store::stable_hash(&forged),scan]).unwrap();
                tx.execute("INSERT INTO agent_gap_followups(scan_id,request_id,request_hash,source_scan_id,assessment_message_id,source_hash,source_preview_json) VALUES(?1,?2,?3,?4,'fixture',?5,'{}')",params![scan,action.request_id,action.request_hash,action.source_scan_id,action.source_hash]).unwrap();
            }
            _ => {
                let mut forged = value.clone();
                forged["policyJson"] = json!(format!("{} ", value["policyJson"].as_str().unwrap()));
                tx.execute("UPDATE native_protection_recovery_drafts SET draft_json=?1 WHERE request_id=?2",params![forged.to_string(),request]).unwrap();
                tx.execute(
                    "UPDATE sentinel_scan_contexts SET policy_json=?1 WHERE scan_id=?2",
                    params![forged["policyJson"].as_str(), scan],
                )
                .unwrap();
            }
        }
        tx.execute_batch(&ddl).unwrap();
        let damaged = super::tests::application_table_snapshot(&tx);
        for _ in 0..2 {
            if operation == "action_kind" {
                assert_eq!(
                    crate::agent_runtime::web_mode::action::read_hash(&tx, scan).unwrap_err(),
                    "web_mode_recovery_action_kind_changed"
                );
            } else {
                assert_eq!(
                    native_protection_recovery_draft_saved_on(&tx, &request).unwrap_err(),
                    if operation == "mode" {
                        "protection_recovery_draft_scope_changed"
                    } else {
                        "protection_recovery_draft_signature_invalid"
                    }
                );
            }
            assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
        }
        tx.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    // Mixing a second provenance also fails even when the original action stays.
    let tx = db.transaction().unwrap();
    let approval = &value["approval"];
    tx.execute("INSERT INTO agent_gap_followups(scan_id,request_id,request_hash,source_scan_id,assessment_message_id,source_hash,source_preview_json) VALUES(?1,?2,?3,?4,'fixture',?5,'{}')",params![scan,request,approval["approvalId"].as_str(),approval["proposal"]["selection"]["scanId"].as_str(),approval["proposalHash"].as_str()]).unwrap();
    assert_eq!(
        crate::agent_runtime::web_mode::action::read_hash(&tx, scan).unwrap_err(),
        "web_mode_recovery_action_conflicting_relation"
    );
    tx.rollback().unwrap();
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
#[test]
fn protection_recovery_draft_concurrent_same_approval_keeps_one_scan_and_relation() {
    let (f, _selected, _scope, request, _approval) =
        protection_recovery_draft_fixture("recovery-draft-concurrent");
    let db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let path = f.h.db_path.clone();
            let request = request.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                native_protection_recovery_create_draft_on(&path, &request)
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert!(results.iter().any(Result::is_ok));
    let saved = native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap();
    for result in results {
        match result {
            Ok(value) => assert_eq!(value, saved),
            Err(error) => assert!(error.starts_with("native_invocation_not_owned:"), "{error}"),
        }
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_protection_recovery_drafts",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    protection_recovery_draft_assert_original_rows(&before, &db);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
