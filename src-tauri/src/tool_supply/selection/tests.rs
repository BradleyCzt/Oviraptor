use super::*;
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn request(approval_id: &str) -> Value {
    json!({"schemaVersion":1,"requestId":id(),"scope":{"scanId":"declared-new-scan","attemptNumber":1,"targetUrls":["https://authorized.example.test/"]},"selection":{"schemaVersion":1,"approvalIds":[approval_id]}})
}
#[test]
fn tool_selection_plan_parser_exact_none_empty_duplicates_unknown_and_order() {
    assert!(FrozenToolSelection::from_plan_value(None)
        .unwrap()
        .is_none());
    assert!(FrozenToolSelection::from_plan_value(Some(
        &json!({"schemaVersion":1,"approvalIds":[]})
    ))
    .unwrap()
    .is_none());
    let one = id();
    let valid = json!({"schemaVersion":1,"approvalIds":[one]});
    let parsed = FrozenToolSelection::from_plan_value(Some(&valid))
        .unwrap()
        .unwrap();
    assert_eq!(parsed.as_plan_value(), valid);
    for invalid in [
        json!({"schemaVersion":1,"approvalIds":[one,one]}),
        json!({"schemaVersion":1,"approvalIds":[one],"approved":true}),
        json!({"schemaVersion":2,"approvalIds":[one]}),
        json!({"schemaVersion":1,"approvalIds":["not-a-uuid"]}),
        Value::Null,
    ] {
        assert!(FrozenToolSelection::from_plan_value(Some(&invalid)).is_err());
    }
    let mut ids = (0..17).map(|_| id()).collect::<Vec<_>>();
    ids.sort();
    assert!(FrozenToolSelection::from_plan_value(Some(
        &json!({"schemaVersion":1,"approvalIds":ids})
    ))
    .is_err());
    let mut ids = vec![id(), id()];
    ids.sort();
    ids.reverse();
    assert!(FrozenToolSelection::from_plan_value(Some(
        &json!({"schemaVersion":1,"approvalIds":ids})
    ))
    .is_err());
}
#[test]
fn tool_selection_request_parser_rejects_unknown_empty_and_changed_scope_hash() {
    let base = request(&id());
    let original = DeclaredSelectionRequest::parse(&base).unwrap();
    assert_eq!(original.as_value(), base);
    let mut changed = base.clone();
    changed["scope"]["attemptNumber"] = json!(2);
    assert_ne!(
        DeclaredSelectionRequest::parse(&changed).unwrap().hash(),
        original.hash()
    );
    for invalid in [
        {
            let mut v = base.clone();
            v["approved"] = json!(true);
            v
        },
        {
            let mut v = base.clone();
            v["scope"]["authority"] = json!(true);
            v
        },
        {
            let mut v = base.clone();
            v["selection"]["approvalIds"] = json!([]);
            v
        },
        {
            let mut v = base.clone();
            v["scope"]["targetUrls"] = json!(["file:///etc/passwd"]);
            v
        },
        {
            let mut v = base.clone();
            v["scope"]["targetUrls"] = json!(["https://user:secret@example.test/"]);
            v
        },
    ] {
        assert!(DeclaredSelectionRequest::parse(&invalid).is_err());
    }
}
#[test]
fn tool_selection_prepared_source_is_actual_signed_inventory_but_not_scope_authority() {
    let root = std::env::temp_dir().join(format!("oviraptor-selection-{}", id()));
    std::fs::create_dir(&root).unwrap();
    let path = crate::db::initialize(&root).unwrap();
    let supply =
        crate::tool_supply::frozen_plan::tests::TestSupplyFixture::approve_on(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    let original = DeclaredSelectionRequest::parse(&request(supply.approval_id())).unwrap();
    let prepared = prepare_requested_on(&db, supply.configuration(), &original).unwrap();
    assert_eq!(prepared.fact()["scopeVerified"], false);
    assert_eq!(prepared.fact()["executionAllowed"], false);
    assert_eq!(prepared.declared_request().as_value(), original.as_value());
    assert_eq!(
        prepared.fact()["inventory"][0]["approvalId"],
        supply.approval_id()
    );
    let changes = db.total_changes();
    prepared
        .require_current_on(&db, supply.configuration(), &original)
        .unwrap();
    assert_eq!(db.total_changes(), changes);
    let mut changed = original.as_value();
    changed["scope"]["targetUrls"] = json!(["https://different.example.test/"]);
    let changed = DeclaredSelectionRequest::parse(&changed).unwrap();
    assert_eq!(
        prepared
            .require_current_on(&db, supply.configuration(), &changed)
            .unwrap_err(),
        "tool_selection_request_binding_changed"
    );
    supply.revoke_on(&path).unwrap();
    assert_eq!(
        prepared
            .require_current_on(&db, supply.configuration(), &original)
            .unwrap_err(),
        "tool_administrator_approval_revoked"
    );
    assert_eq!(db.total_changes(), changes);
    drop(db);
    drop(supply);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tool_selection_plan_preserves_explicit_empty_for_original_bytes_and_hash() {
    assert!(FrozenToolSelection::parse_preserving_empty(None)
        .unwrap()
        .is_none());
    let empty = json!({"schemaVersion":1,"approvalIds":[]});
    let saved = FrozenToolSelection::parse_preserving_empty(Some(&empty))
        .unwrap()
        .unwrap();
    assert!(saved.approval_ids().is_empty());
    assert_eq!(saved.as_plan_value().to_string(), empty.to_string());
    assert!(FrozenToolSelection::from_plan_value(Some(&empty))
        .unwrap()
        .is_none());
    let absent = json!({"schemaVersion":2});
    let explicit = json!({"schemaVersion":2,"toolSupply":saved.as_plan_value()});
    assert_ne!(
        store::stable_hash(&absent.to_string()),
        store::stable_hash(&explicit.to_string())
    );
    for invalid in [
        json!({"schemaVersion":1,"approvalIds":[],"approved":true}),
        json!({"schemaVersion":2,"approvalIds":[]}),
        Value::Null,
    ] {
        assert!(FrozenToolSelection::parse_preserving_empty(Some(&invalid)).is_err());
    }
}

#[test]
fn tool_selection_actual_fresh_draft_stage_preserves_policy_and_empty_is_pure() {
    let root = std::env::temp_dir().join(format!("oviraptor-selection-draft-{}", id()));
    std::fs::create_dir(&root).unwrap();
    let path = crate::db::initialize(&root).unwrap();
    let supply =
        crate::tool_supply::frozen_plan::tests::TestSupplyFixture::approve_on(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch(SCHEMA).unwrap();
    db.execute(
        "INSERT INTO projects(id,name,status) VALUES(9001,'selection fixture','active')",
        [],
    )
    .unwrap();
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let window = crate::agent_runtime::web_mode::draft::DraftCreationWindow::begin(&tx).unwrap();
    let scan = id();
    tx.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type,task_name) VALUES(?1,9001,'selection fixture','draft','draft','','web','selection fixture')",[&scan]).unwrap();
    tx.execute("INSERT INTO sentinel_scan_contexts(scan_id,environment,policy_json) VALUES(?1,'internal',?2)",rusqlite::params![scan,json!({"existingPolicy":{"preserve":true}}).to_string()]).unwrap();
    tx.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(9001,?1,'selection fixture','https://authorized.example.test/','queued')",[&scan]).unwrap();
    let fresh = crate::agent_runtime::web_mode::draft::register_new(
        window,
        &scan,
        crate::agent_runtime::web_mode::WebMode::Multi,
    )
    .unwrap();
    let before = tx.total_changes();
    assert!(stage_for_new_draft(&fresh, supply.configuration(), None)
        .unwrap()
        .is_none());
    assert_eq!(tx.total_changes(), before);
    let mut value = request(supply.approval_id());
    value["scope"]["scanId"] = json!(scan);
    let declared = DeclaredSelectionRequest::parse(&value).unwrap();
    let pending = stage_for_new_draft(&fresh, supply.configuration(), Some(&declared))
        .unwrap()
        .unwrap();
    assert_eq!(
        pending.fact()["signatureStage"],
        "pending_original_mode_policy_signature"
    );
    assert_eq!(pending.fact()["executionAllowed"], false);
    let policy: String = tx
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [&scan],
            |r| r.get(0),
        )
        .unwrap();
    let policy: Value = serde_json::from_str(&policy).unwrap();
    assert_eq!(policy["existingPolicy"], json!({"preserve":true}));
    assert_eq!(policy["toolSupplySelection"], *pending.fact());
    assert!(stage_for_new_draft(&fresh, supply.configuration(), Some(&declared)).is_err());
    drop(fresh);
    tx.commit().unwrap();
    assert!(db
        .execute(
            "UPDATE native_tool_selection_requests SET request_hash=request_hash",
            []
        )
        .is_err());
    drop(db);
    drop(supply);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tool_selection_actual_stage_trigger_refusal_and_borrowed_authorizer_rollback() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    for fault in ["request_trigger", "policy_trigger", "borrowed_authorizer"] {
        let root = std::env::temp_dir().join(format!("oviraptor-selection-fault-{}", id()));
        std::fs::create_dir(&root).unwrap();
        let path = crate::db::initialize(&root).unwrap();
        let supply =
            crate::tool_supply::frozen_plan::tests::TestSupplyFixture::approve_on(&path).unwrap();
        let db = Connection::open(&path).unwrap();
        db.execute_batch(SCHEMA).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,status) VALUES(9001,'selection fixture','active')",
            [],
        )
        .unwrap();
        let tx =
            rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
                .unwrap();
        let window =
            crate::agent_runtime::web_mode::draft::DraftCreationWindow::begin(&tx).unwrap();
        let scan = id();
        tx.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type,task_name) VALUES(?1,9001,'selection fixture','draft','draft','','web','selection fixture')",[&scan]).unwrap();
        tx.execute("INSERT INTO sentinel_scan_contexts(scan_id,environment,policy_json) VALUES(?1,'internal',?2)",rusqlite::params![scan,json!({"existingPolicy":{"preserve":true}}).to_string()]).unwrap();
        tx.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(9001,?1,'selection fixture','https://authorized.example.test/','queued')",[&scan]).unwrap();
        let fresh = crate::agent_runtime::web_mode::draft::register_new(
            window,
            &scan,
            crate::agent_runtime::web_mode::WebMode::Multi,
        )
        .unwrap();
        let mut value = request(supply.approval_id());
        value["scope"]["scanId"] = json!(scan);
        let declared = DeclaredSelectionRequest::parse(&value).unwrap();
        match fault {
            "request_trigger" => tx.execute_batch("CREATE TRIGGER selection_fault AFTER INSERT ON native_tool_selection_requests BEGIN UPDATE projects SET name='sidewrite' WHERE id=9001; END").unwrap(),
            "policy_trigger" => tx.execute_batch("CREATE TRIGGER selection_fault AFTER UPDATE ON sentinel_scan_contexts BEGIN UPDATE projects SET name='sidewrite' WHERE id=9001; END").unwrap(),
            _ => (),
        }
        let outside = outside_snapshot(&tx, &scan, &declared.request_id).unwrap();
        let policy: String = tx
            .query_row(
                "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
                [&scan],
                |r| r.get(0),
            )
            .unwrap();
        let inserts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        if fault == "borrowed_authorizer" {
            let observed = inserts.clone();
            tx.authorizer(Some(move |c: AuthContext<'_>| match c.action {
                AuthAction::Insert {
                    table_name: "native_tool_selection_requests",
                } => {
                    observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Authorization::Allow
                }
                AuthAction::Update {
                    table_name: "sentinel_scan_contexts",
                    ..
                } => Authorization::Deny,
                _ => Authorization::Allow,
            }))
            .unwrap();
        }
        let error = stage_for_new_draft(&fresh, supply.configuration(), Some(&declared))
            .err()
            .unwrap();
        if fault == "borrowed_authorizer" {
            assert!(error.contains("authorized"), "{error}");
            assert!(inserts.load(std::sync::atomic::Ordering::SeqCst) > 0);
            // The original INSERT reached SQLite; UPDATE then failed and both
            // application writes rolled back. The caller's hook remains installed.
            assert!(tx
                .execute(
                    "UPDATE sentinel_scan_contexts SET policy_json=policy_json WHERE scan_id=?1",
                    [&scan]
                )
                .is_err());
            tx.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                .unwrap();
        } else {
            assert_eq!(error, "tool_selection_schema_changed");
        }
        assert_eq!(
            outside_snapshot(&tx, &scan, &declared.request_id).unwrap(),
            outside
        );
        let after: String = tx
            .query_row(
                "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
                [&scan],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, policy);
        let requests: i64 = tx
            .query_row(
                "SELECT count(*) FROM native_tool_selection_requests",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(requests, 0);
        drop(fresh);
        tx.rollback().unwrap();
        drop(db);
        drop(supply);
        std::fs::remove_dir_all(root).unwrap();
    }
}
