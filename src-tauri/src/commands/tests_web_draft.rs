fn web_draft_fixture() -> (PathBuf, rusqlite::Connection, String) {
    let root = std::env::temp_dir().join(format!("oviraptor-web-draft-{}", Uuid::new_v4()));
    let path = db::initialize(&root).unwrap();
    let connection = db::open(&path).unwrap();
    connection.execute("INSERT INTO projects(id,name) VALUES(1,'Draft test')", []).unwrap();
    let scope = Uuid::new_v4().to_string();
    for id in ["identity-a", "identity-b"] {
        connection.execute(
            "INSERT INTO browser_auth_sessions(id,project_id,draft_scope_id,name,entry_url,status,session_json,expires_at) \
             VALUES(?1,1,?2,?1,'https://draft.example.test','valid',?3,?4)",
            params![id, scope, serde_json::json!({
                "scopeHosts":["draft.example.test"], "cookies":[{"name":"session","value":id}]
            }).to_string(), (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339()],
        ).unwrap();
    }
    (root, connection, scope)
}

fn submit_web_draft(connection: &rusqlite::Connection, scope: &str, instruction: &str, budget: f64) -> Result<SentinelScan, String> {
    create_sentinel_url_scan_in(
        connection, 1, "Follow-up preparation".into(),
        vec!["https://draft.example.test/ ; https://draft.example.test/second".into()],
        Some("deep".into()), Some(budget), Some("identity-a".into()),
        Some(vec!["identity-b".into(), "identity-a".into()]), Some(scope.into()),
        Some(vec![3, 3, 0]), Some(instruction.into()), Some("proof".into()),
    )
}

fn web_draft_snapshot(connection: &rusqlite::Connection) -> JsonValue {
    let mut snapshot = serde_json::Map::new();
    for table in ["sentinel_scans", "sentinel_scan_contexts", "sentinel_targets", "sentinel_scan_attempts", "agent_runs", "agent_authorization_controls"] {
        let count: i64 = connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0)).unwrap();
        snapshot.insert(table.into(), serde_json::json!(count));
    }
    let mut statement = connection.prepare("SELECT id,owner_scan_id,draft_scope_id,updated_at FROM browser_auth_sessions ORDER BY id").unwrap();
    let identities = statement.query_map([], |row| Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?, row.get::<_,String>(3)?)))
        .unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    snapshot.insert("identities".into(), serde_json::json!(identities));
    JsonValue::Object(snapshot)
}

#[test]
fn web_draft_creation_commits_identities_policy_and_controls_without_starting() {
    let (root, mut connection, scope) = web_draft_fixture();
    let scan = submit_web_draft(&connection, &scope, "Collect missing controls", 5.0).unwrap();
    assert_eq!(scan.status, "draft");
    assert_eq!(scan.scan_type, "web");
    let policy: String = connection.query_row("SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1", [&scan.id], |row| row.get(0)).unwrap();
    let policy: JsonValue = serde_json::from_str(&policy).unwrap();
    assert_eq!(policy["authSessionIds"], serde_json::json!(["identity-a", "identity-b"]));
    assert_eq!(policy["selectedSkillIds"], serde_json::json!([3]));
    assert_eq!(policy["closure"], "proof");
    let setup = authorization_control_setup_in(&connection, &root, &scan.id, "https://draft.example.test").unwrap();
    assert_eq!(setup.identity_ids, ["identity-a", "identity-b"]);
    assert_eq!(setup.attempt_number, 1);
    let input = SaveAuthorizationControlInput {
        scan_id: scan.id.clone(), attempt_number: setup.attempt_number,
        target_url: "https://draft.example.test".into(), contract_key: "idor|/api/orders".into(),
        owner_object_url: "https://draft.example.test/api/orders?id=owner".into(),
        tester_control_url: "https://draft.example.test/api/orders?id=tester".into(),
        object_query_key: "id".into(), owner_object_value: "owner".into(), tester_object_value: "tester".into(),
        response_object_pointer: "/order/id".into(), owner_identity: "identity-a".into(), tester_identity: "identity-b".into(),
    };
    save_authorization_control_in(&mut connection, &root, &input).unwrap();
    let snapshot = web_draft_snapshot(&connection);
    assert_eq!(snapshot["sentinel_scans"], 1);
    assert_eq!(snapshot["sentinel_targets"], 2);
    assert_eq!(snapshot["agent_authorization_controls"], 1);
    assert_eq!(snapshot["sentinel_scan_attempts"], 0);
    assert_eq!(snapshot["agent_runs"], 0);
    assert!(submit_web_draft(&connection, &scope, "", 5.0).is_err());
    assert_eq!(snapshot, web_draft_snapshot(&connection));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_draft_creation_rolls_back_every_write_failure_and_releases_identities() {
    for (table, operation, condition) in [
        ("sentinel_scans", "INSERT", "1"),
        ("sentinel_scan_contexts", "INSERT", "1"),
        ("browser_auth_sessions", "UPDATE", "NEW.id='identity-b'"),
        ("sentinel_targets", "INSERT", "NEW.url LIKE '%/second'"),
    ] {
        for fault in ["ABORT, 'injected draft fault'", "IGNORE"] {
            let (root, connection, scope) = web_draft_fixture();
            let before = web_draft_snapshot(&connection);
            connection.execute_batch(&format!("CREATE TRIGGER web_draft_fault BEFORE {operation} ON {table} WHEN {condition} BEGIN SELECT RAISE({fault}); END;")).unwrap();
            assert!(submit_web_draft(&connection, &scope, "", 5.0).is_err(), "{table}/{fault}");
            assert_eq!(before, web_draft_snapshot(&connection), "{table}/{fault}");
            connection.execute_batch("DROP TRIGGER web_draft_fault;").unwrap();
            let scan = submit_web_draft(&connection, &scope, "", 5.0).unwrap();
            assert_eq!(scan.status, "draft");
            assert_eq!(web_draft_snapshot(&connection)["sentinel_scans"], 1);
            drop(connection);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn web_draft_creation_rejects_invalid_policy_budget_and_unreadable_fuse_without_writes() {
    let (root, connection, scope) = web_draft_fixture();
    let before = web_draft_snapshot(&connection);
    assert!(submit_web_draft(&connection, &scope, &"x".repeat(12_001), 5.0).is_err());
    assert_eq!(before, web_draft_snapshot(&connection));
    for budget in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0, 10_001.0] {
        assert!(submit_web_draft(&connection, &scope, "", budget).is_err());
        assert_eq!(before, web_draft_snapshot(&connection));
    }
    connection.execute_batch("ALTER TABLE sentinel_fuse_zone RENAME TO hidden_fuse_zone;").unwrap();
    assert!(submit_web_draft(&connection, &scope, "", 5.0).unwrap_err().contains("熔断状态"));
    assert_eq!(before, web_draft_snapshot(&connection));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_draft_creation_serializes_competing_identity_claims() {
    let (root, connection, scope) = web_draft_fixture();
    let db_path = PathBuf::from(connection.path().unwrap());
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let threads = (0..2).map(|_| {
        let path = db_path.clone();
        let scope = scope.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let connection = db::open(&path).unwrap();
            barrier.wait();
            submit_web_draft(&connection, &scope, "", 5.0).map(|scan| scan.id)
        })
    }).collect::<Vec<_>>();
    let results = threads.into_iter().map(|thread| thread.join().unwrap()).collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let winner = results.iter().find_map(|result| result.as_ref().ok()).unwrap();
    let snapshot = web_draft_snapshot(&connection);
    assert_eq!(snapshot["sentinel_scans"], 1);
    assert_eq!(snapshot["sentinel_scan_contexts"], 1);
    assert_eq!(snapshot["sentinel_targets"], 2);
    assert_eq!(snapshot["agent_runs"], 0);
    for identity in snapshot["identities"].as_array().unwrap() {
        assert_eq!(identity[1].as_str(), Some(winner.as_str()));
        assert_eq!(identity[2], "");
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_draft_creation_rejects_wrong_scope_or_archived_project_without_consuming_logins() {
    let (root, connection, scope) = web_draft_fixture();
    let before = web_draft_snapshot(&connection);
    assert!(submit_web_draft(&connection, &Uuid::new_v4().to_string(), "", 5.0).is_err());
    assert_eq!(before, web_draft_snapshot(&connection));
    connection.execute("UPDATE projects SET status='archived' WHERE id=1", []).unwrap();
    assert!(submit_web_draft(&connection, &scope, "", 5.0).is_err());
    assert_eq!(before, web_draft_snapshot(&connection));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn homogeneous_draft_freezes_opt_in_goal_before_signature_and_legacy_stays_default() {
    let (root, connection, _) = web_draft_fixture();
    let scan=create_sentinel_url_scan_scheduling_in(&connection,1,"Homogeneous".into(),vec!["https://draft.example.test/".into()],Some("standard".into()),None,None,None,None,None,Some("核对现有接口观测依据".into()),Some("breadth".into()),Some("multi".into()),true).unwrap();
    assert_eq!(scan.status,"draft");
    assert!(crate::agent_runtime::multi_agent::homogeneous::model_worker::enabled(&connection,&scan.id,1).unwrap());
    let plan:String=connection.query_row("SELECT plan_json FROM native_scan_plans WHERE scan_id=?1",[&scan.id],|r|r.get(0)).unwrap();
    let plan:JsonValue=serde_json::from_str(&plan).unwrap();assert_eq!(plan["goal"],"核对现有接口观测依据");assert_eq!(plan["lane"],"read_only_analysis");assert_eq!(plan["allowedParallel"],3);
    assert!(native_scan_status_after(&connection,&scan.id,None).is_ok(),"Web scheduling metadata must not be decoded as a source plan");
    let legacy=create_sentinel_url_scan_with_mode_in(&connection,1,"Legacy".into(),vec!["https://draft.example.test/".into()],None,None,None,None,None,None,None,None,None).unwrap();
    assert!(!crate::agent_runtime::multi_agent::homogeneous::model_worker::enabled(&connection,&legacy.id,1).unwrap());
    connection.execute("UPDATE native_scan_plans SET plan_hash='tampered' WHERE scan_id=?1",[&scan.id]).unwrap();
    assert!(crate::agent_runtime::multi_agent::homogeneous::model_worker::enabled(&connection,&scan.id,1).is_err());
    drop(connection);let _=fs::remove_dir_all(root);
}
