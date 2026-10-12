#[test]
fn native_budget_diagnostics_are_scoped_and_read_only() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    connection.execute_batch(
        "INSERT INTO sentinel_scans(id,project_name,status,scan_type,attempt_count)
            VALUES('other-budget-scan','fixture','scanning','web',1);
         INSERT INTO agent_runs(id,scan_id,attempt_number,target_url) VALUES
            ('budget-current','source-regression',1,'https://one.example.test'),
            ('budget-old','source-regression',0,'https://old.example.test'),
            ('budget-other','other-budget-scan',1,'https://other.example.test');
         INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,root_run_id,role) VALUES
            ('budget-child','source-regression',1,'https://one.example.test','budget-current','spa_api_mapper');
         INSERT INTO agent_budget_ledger(root_run_id,total_tokens,total_requests,reserved_tokens,reserved_requests,lease_epoch,fencing_token)
            VALUES('budget-current',100,10,8,2,1,'fence');
         INSERT INTO agent_assignments(id,coordinator_run_id,dedup_key,reserved_tokens,reserved_requests)
            VALUES('budget-assignment','budget-current','test',8,2);
         INSERT INTO agent_coordinator_leases(scan_id,attempt_number,target_key,root_run_id,fencing_token,lease_expires_at)
            VALUES('source-regression',1,'https://one.example.test','budget-current','fence','future');
         INSERT INTO tool_invocations(id,run_id,tool_name,contract_key,finished_at)
            VALUES(1,'budget-child','test','','');",
    ).unwrap();
    let before = connection.total_changes();
    let report =
        native_budget_diagnostics_for_attempt(&connection, "source-regression", 1).unwrap();
    assert_eq!(connection.total_changes(), before);
    assert_eq!(report["authoritative"], false);
    assert_eq!(report["schema"], "summary_gap_v1");
    assert_eq!(report["totalRoots"], 1);
    assert_eq!(report["roots"].as_array().unwrap().len(), 1);
    assert_eq!(report["roots"][0]["rootRunId"], "budget-current");
    assert!(report["roots"][0]["singleOriginalHistory"].is_null(), "Source history is not a Single Web invoice");
    assert_eq!(report["roots"][0]["gaps"]["reservationTokenDelta"], 0);
    assert_eq!(report["roots"][0]["gaps"]["indeterminateInvocations"], 1);
    assert_eq!(
        report["roots"][0]["gaps"]["fencingMatchesCoordinator"],
        true
    );
    assert!(!report.to_string().contains("budget-other"));
    assert!(!report.to_string().contains("budget-old"));
    assert!(!report.to_string().contains("budget-child"));
    assert!(native_budget_diagnostics_for_attempt(&connection, "source-regression", 0).is_err());
    assert!(native_budget_diagnostics_for_attempt(&connection, "source-regression", 2).is_err());
    assert!(native_budget_diagnostics_for_attempt(&connection, "missing", 1).is_err());
    connection
        .execute(
            "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('source-regression')",
            [],
        )
        .unwrap();
    assert!(native_budget_diagnostics_for_attempt(&connection, "source-regression", 1).is_err());
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_budget_diagnostics_count_unclosed_web_calls_without_cross_root_receipts() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    connection
        .execute_batch(
            "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url) VALUES
        ('web-budget-current','source-regression',1,'https://one.example.test'),
        ('web-budget-old','source-regression',0,'https://one.example.test');",
        )
        .unwrap();
    for (call, owner, phase, round) in [
        ("pending-private-call", "web-budget-current", "dispatch", 1),
        (
            "uncertain-private-call",
            "web-budget-current",
            "dispatch",
            2,
        ),
        (
            "uncertain-private-call",
            "web-budget-current",
            "uncertain",
            2,
        ),
        ("known-private-call", "web-budget-current", "dispatch", 3),
        ("known-private-call", "web-budget-current", "received", 3),
        ("unsent-private-call", "web-budget-current", "dispatch", 4),
        ("unsent-private-call", "web-budget-current", "unsent", 4),
        ("old-private-call", "web-budget-old", "dispatch", 5),
        ("mismatch-private-call", "web-budget-current", "dispatch", 6),
        ("mismatch-private-call", "web-budget-old", "received", 6),
    ] {
        connection.execute("INSERT INTO agent_web_model_journal
            (call_id,root_run_id,assignment_id,child_run_id,lease_epoch,fencing_token,round,request_hash,phase,receipt_json)
            VALUES(?1,?2,'private-assignment','private-child',1,'private-fence',?3,'private-hash',?4,'{}')",
            rusqlite::params![call,owner,round,phase]).unwrap();
    }
    let before = connection.total_changes();
    let report =
        native_budget_diagnostics_for_attempt(&connection, "source-regression", 1).unwrap();
    assert_eq!(report["roots"][0]["gaps"]["unclosedWebModelCalls"], 3);
    assert_eq!(connection.total_changes(), before);
    assert_eq!(report["totalRoots"], 1);
    assert!(!report.to_string().contains("private-"));
    assert!(!report.to_string().contains("web-budget-old"));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
