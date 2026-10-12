#[test]
fn protection_recovery_scope_budget_preserves_independent_zero_unbounded_and_shared_total() {
    let (f, selected, mut scope) = protection_recovery_approval_fixture("recovery-scope-budget");
    scope.urls = vec![
        format!("{}/", f.h.context.target_url.trim_end_matches('/')),
        format!("{}?exact=a,b", f.h.context.target_url),
    ];
    scope.urls.sort();
    scope.limits[0] = Some(0);
    scope.limits[1] = None;
    scope.limits[2] = Some(37);
    scope.max_budget_usd = 3.25;
    let mut db = db::open(&f.h.db_path).unwrap();
    let proposal =
        native_protection_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope).unwrap();
    let request = Uuid::new_v4().to_string();
    native_protection_recovery_approve_on(
        &f.h.db_path,
        &selected,
        &scope,
        &request,
        proposal["proposalHash"].as_str().unwrap(),
        RECOVERY_EXPLICIT_CONSENT,
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let draft = native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap();
    let budget = &draft["scopeBudget"];
    assert_eq!(budget["limits"], json!(scope.limits));
    assert_eq!(budget["limits"][0], 0);
    assert_eq!(budget["limits"][1], JsonValue::Null);
    assert_eq!(budget["limits"][2], 37);
    assert_eq!(
        budget["dimensions"],
        json!(crate::agent_runtime::multi_agent::budget::DIMENSIONS)
    );
    assert_eq!(budget["urls"], json!(scope.urls));
    assert_eq!(budget["maxBudgetUsd"], 3.25);
    assert_eq!(budget["deadlineUtc"], scope.deadline_utc);
    assert_eq!(
        budget["allocationPolicy"],
        "one_shared_scan_total_across_all_roots"
    );
    assert_eq!(
        budget["wallTimePolicy"],
        "scan_birth_clock_clipped_by_original_deadline"
    );
    assert_eq!(budget["executionAllowed"], false);
    assert_eq!(budget["targetRequestsGranted"], 0);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_protection_recovery_scope_budgets",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    protection_recovery_draft_assert_original_rows(&before, &db);
    let after = super::tests::application_table_snapshot(&db);
    assert_eq!(
        native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap(),
        draft
    );
    let ro = native_protection_recovery_open(&f.h.db_path, false).unwrap();
    let tx = ro.unchecked_transaction().unwrap();
    assert_eq!(
        native_protection_recovery_draft_saved_on(&tx, &request).unwrap(),
        Some(draft.clone())
    );
    tx.commit().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), after);
    cancel_draft_scan_in(&db, draft["scanId"].as_str().unwrap()).unwrap();
    let tx = ro.unchecked_transaction().unwrap();
    assert_eq!(
        native_protection_recovery_draft_saved_on(&tx, &request).unwrap(),
        Some(draft)
    );
    tx.commit().unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn protection_recovery_scope_budget_immutable_and_cold_hash_recomputation_cannot_repair() {
    let (f, _, _, request, _) = protection_recovery_draft_fixture("recovery-scope-budget-cold");
    let draft = native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    db.execute_batch("PRAGMA recursive_triggers=OFF").unwrap();
    for sql in ["UPDATE native_protection_recovery_scope_budgets SET contract_hash=contract_hash",
        "DELETE FROM native_protection_recovery_scope_budgets",
        "INSERT OR REPLACE INTO native_protection_recovery_scope_budgets(rowid,owner_id,scan_id,draft_id,contract_json,contract_hash) SELECT rowid,printf('%064d',1),'other-scan','other-draft',contract_json,contract_hash FROM native_protection_recovery_scope_budgets"] {
        assert!(db.execute(sql,[]).is_err());
    }
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    let tx = db.unchecked_transaction().unwrap();
    tx.execute_batch("DROP TRIGGER protection_recovery_scope_budget_no_update")
        .unwrap();
    let mut forged = draft["scopeBudget"].clone();
    forged["limits"][4] = json!(9999);
    let text = forged.to_string();
    tx.execute(
        "UPDATE native_protection_recovery_scope_budgets SET contract_json=?1,contract_hash=?2",
        params![text, crate::agent_runtime::store::stable_hash(&text)],
    )
    .unwrap();
    tx.execute_batch(include_str!("native_protection_recovery_schema.sql"))
        .unwrap();
    let damaged = super::tests::application_table_snapshot(&tx);
    for _ in 0..2 {
        assert_eq!(
            native_protection_recovery_draft_saved_on(&tx, &request).unwrap_err(),
            "protection_recovery_scope_budget_binding_changed"
        );
    }
    assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}
