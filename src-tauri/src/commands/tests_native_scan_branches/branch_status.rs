#[test]
fn native_web_cancellation_and_publication_are_fenced_by_attempt() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    let scan = "source-regression";
    let token = agent_scan_cancel_token(&path, scan, 1);
    assert!(!token.is_cancelled());
    native_web_attempt_progress(&path, scan, 1, "current progress");
    let checkpoint = || connection.query_row("SELECT current_checkpoint FROM sentinel_scans WHERE id=?1", [scan], |r| r.get::<_, String>(0)).unwrap();
    assert_eq!(checkpoint(), "current progress");
    let recon = json!({"targets":[{"url":"http://127.0.0.1:1","analysisSummary":{"reconCacheVersion":4}}]});
    assert!(insert_native_frontend_recon(&path, scan, 1, &recon).is_ok());
    let old = NativeAgentState::fresh(1, AgentBackendKind::Native, "evidence", "plan", vec![]);
    old.persist(&path, scan, "http://127.0.0.1:1").unwrap();
    connection.execute("UPDATE sentinel_scans SET attempt_count=2,current_checkpoint='new attempt' WHERE id=?1", [scan]).unwrap();
    assert!(token.is_cancelled(), "old transport token must stop even while the new attempt is scanning");
    assert!(!native_web_attempt_current(&path, scan, 1));
    assert!(native_web_attempt_active(&path, scan, 2));
    native_web_attempt_progress(&path, scan, 1, "stale progress");
    assert_eq!(checkpoint(), "new attempt");
    assert!(insert_native_frontend_recon(&path, scan, 1, &recon).unwrap_err().contains("replaced"));
    let current = NativeAgentState::fresh(2, AgentBackendKind::Native, "new evidence", "new plan", vec![]);
    current.persist(&path, scan, "http://127.0.0.1:1").unwrap();
    assert!(old.persist(&path, scan, "http://127.0.0.1:1").unwrap_err().contains("replaced"));
    assert_eq!(NativeAgentState::read(&path, scan, "http://127.0.0.1:1").unwrap().attempt_number, 2);
    for status in ["pausing", "paused", "cancelled", "completed"] {
        connection.execute("UPDATE sentinel_scans SET status=?1 WHERE id=?2", params![status,scan]).unwrap();
        assert!(agent_scan_cancel_token(&path, scan, 2).is_cancelled());
        assert!(insert_native_frontend_recon(&path, scan, 2, &recon).is_err());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_branch_barrier_waits_for_both_orders_and_retains_source_gaps() {
    for order in [["web", "source"], ["source", "web"]] {
        let (root, _, path) = source_regression_fixture();
        let connection = db::open(&path).unwrap();
        register_native_branches(&connection, "source-regression", 1, &["source", "web"]).unwrap();
        for (index, branch) in order.iter().enumerate() {
            let status = if *branch == "source" { "completed_with_gaps" } else { "completed" };
            assert!(finish_native_branch(&path, "source-regression", 1, branch, status, "branch checkpoint", &json!({"branch":branch})).unwrap());
            let current: String = connection.query_row("SELECT status FROM sentinel_scans WHERE id='source-regression'", [], |r| r.get(0)).unwrap();
            assert_eq!(current, if index == 0 { "scanning" } else { "completed_with_gaps" });
        }
        let attempts: String = connection.query_row("SELECT report_json FROM native_scan_branches WHERE branch='source'", [], |r| r.get(0)).unwrap();
        assert!(attempts.contains("source"));
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn native_branch_failure_is_never_hidden_by_web_success() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    register_native_branches(&connection, "source-regression", 1, &["source", "web"]).unwrap();
    finish_native_branch(&path, "source-regression", 1, "source", "failed", "analyzer unavailable", &json!({})).unwrap();
    finish_native_branch(&path, "source-regression", 1, "web", "completed", "web finished", &json!({})).unwrap();
    let state: (String,String) = connection.query_row("SELECT status,current_checkpoint FROM sentinel_scans WHERE id='source-regression'", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(state.0, "partial");
    assert!(state.1.contains("analyzer unavailable"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_branch_receipts_are_immutable_and_fenced_by_attempt_and_pause() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    register_native_branches(&connection, "source-regression", 1, &["source", "web"]).unwrap();
    assert!(register_native_branches(&connection, "source-regression", 1, &["web"]).is_err());
    assert!(!finish_native_branch(&path, "source-regression", 1, "unknown", "completed", "invalid", &json!({})).unwrap());
    assert!(finish_native_branch(&path, "source-regression", 1, "source", "failed", "original", &json!({})).unwrap());
    assert!(!finish_native_branch(&path, "source-regression", 1, "source", "completed", "rewrite", &json!({})).unwrap());
    for (status, attempt) in [("pausing", 1),("paused", 1),("scanning", 2),("cancelled", 1),("completed", 1)] {
        connection.execute("UPDATE sentinel_scans SET status=?1,attempt_count=?2,current_checkpoint='unchanged'", params![status,attempt]).unwrap();
        assert!(!finish_native_branch(&path, "source-regression", 1, "web", "completed", "late", &json!({})).unwrap());
        let row: String = connection.query_row("SELECT current_checkpoint FROM sentinel_scans", [], |r| r.get(0)).unwrap();
        assert_eq!(row, "unchanged");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_branch_reducer_serializes_concurrent_connections() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    register_native_branches(&connection, "source-regression", 1, &["source", "web"]).unwrap();
    let threads: Vec<_> = ["source", "web"].into_iter().map(|branch| {
        let path = path.clone();
        std::thread::spawn(move || finish_native_branch(&path, "source-regression", 1, branch, "completed", "finished", &json!({})).unwrap())
    }).collect();
    for thread in threads { assert!(thread.join().unwrap()); }
    let status: String = connection.query_row("SELECT status FROM sentinel_scans", [], |r| r.get(0)).unwrap();
    assert_eq!(status,"completed");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_branch_worker_exit_is_a_failure_but_cannot_overwrite_completed_receipts() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    // Workbench startup registers the repository itself as this attempt's
    // scanning target; the Source finalizer only publishes when that row
    // carries the branch result for the same attempt.
    connection.execute_batch(
        "INSERT INTO projects(id,name) VALUES(1,'branch exit fixture');
         UPDATE sentinel_scans SET project_id=1 WHERE id='source-regression';
         INSERT INTO sentinel_targets(project_id,scan_id,company,url,status,last_attempt_number)
         SELECT project_id,id,project_name,source_path,'scanning',attempt_count FROM sentinel_scans WHERE id='source-regression';",
    ).unwrap();
    register_native_branches(&connection, "source-regression", 1, &["source", "web"]).unwrap();
    let guard = NativeBranchGuard::claim(&path, "source-regression", 1, "source").unwrap();
    drop(guard);
    let guard = NativeBranchGuard::claim(&path, "source-regression", 1, "web").unwrap();
    finish_native_branch(&path,"source-regression",1,"web","completed","done",&json!({})).unwrap();
    drop(guard);
    let status: String = connection.query_row("SELECT status FROM sentinel_scans", [], |r| r.get(0)).unwrap();
    assert_eq!(status,"partial");
    let web: String = connection.query_row("SELECT status FROM native_scan_branches WHERE branch='web'", [], |r| r.get(0)).unwrap();
    assert_eq!(web,"completed");
    let source: (String, String) = connection.query_row(
        "SELECT b.status,t.status FROM native_scan_branches b JOIN sentinel_scans s ON s.id=b.scan_id
         JOIN sentinel_targets t ON t.scan_id=s.id AND t.url=s.source_path AND t.last_attempt_number=b.attempt_number
         WHERE b.branch='source'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(source, ("failed".to_string(), "failed".to_string()), "the unpaid worker exit is a recorded failure");
    fs::remove_dir_all(root).unwrap();
}

/// Guard still bites: without the attempt's own Source target row the Source
/// finalizer cannot prove its projection, so a worker exit publishes nothing and
/// the branch stays pending for recovery instead of inventing a failure.
#[test]
fn native_source_worker_exit_without_its_attempt_target_is_not_published() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    register_native_branches(&connection, "source-regression", 1, &["source", "web"]).unwrap();
    assert_eq!(
        finish_native_source_branch(&path, "source-regression", 1, &json!({"error":"branch_worker_exited"})),
        Err("source_branch_projection_unconfirmed".to_string())
    );
    let guard = NativeBranchGuard::claim(&path, "source-regression", 1, "source").unwrap();
    drop(guard);
    let branches: Vec<(String, String)> = connection
        .prepare("SELECT branch,status FROM native_scan_branches ORDER BY branch").unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(branches, vec![("source".to_string(), "pending".to_string()), ("web".to_string(), "pending".to_string())]);
    let status: String = connection.query_row("SELECT status FROM sentinel_scans", [], |r| r.get(0)).unwrap();
    assert_eq!(status, "scanning");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_status_read_is_attempt_scoped_read_only_and_does_not_expose_container_owner_tokens() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    register_native_branches(&connection, "source-regression", 1, &["source", "web"]).unwrap();
    connection.execute("INSERT INTO analyzer_container_receipts(receipt_id,invocation_key,scan_id,attempt_number,purpose,container_name,owner_token,create_args_json) VALUES('r','i','source-regression',1,'semgrep','container-r','private-owner','[]')", []).unwrap();
    let before = connection.total_changes();
    let status = native_scan_status(&connection, "source-regression").unwrap();
    assert_eq!(before,connection.total_changes());
    assert_eq!(status["branches"].as_array().unwrap().len(),2);
    assert_eq!(status["unresolvedContainers"].as_array().unwrap().len(),1);
    assert_eq!(status["multiAgentReady"],false);
    assert_eq!(status["stopDiagnostic"]["code"], "cleanup_unconfirmed");
    assert_eq!(status["stopDiagnostic"]["category"], "hard");
    assert_eq!(status["stopDiagnostic"]["automaticResumeAllowed"], false);
    assert!(status["stopDiagnostic"]["obligations"].as_array().unwrap().iter().any(|item| item["kind"] == "cleanup"));
    assert!(!status.to_string().contains("private-owner"));
    connection.execute("UPDATE sentinel_scans SET attempt_count=2", []).unwrap();
    let new_status = native_scan_status(&connection,"source-regression").unwrap();
    assert!(new_status["branches"].as_array().unwrap().is_empty());
    assert_eq!(new_status["unresolvedContainers"].as_array().unwrap().len(),1,"previous attempts still block container creation");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_status_refuses_a_corrupt_usage_ledger_instead_of_reporting_zero() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    connection.execute(
        "UPDATE sentinel_scans SET llm_requests='broken' WHERE id='source-regression'",
        [],
    ).unwrap();
    let error = native_scan_status(&connection, "source-regression").unwrap_err();
    assert!(error.contains("用量"), "damaged ledger must fail the status read: {error}");
    connection.execute(
        "UPDATE sentinel_scans SET llm_requests=0,total_tokens='broken' WHERE id='source-regression'",
        [],
    ).unwrap();
    let error = native_scan_status(&connection, "source-regression").unwrap_err();
    assert!(error.contains("用量"), "damaged tokens must fail the status read: {error}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_status_distinguishes_missing_scan_from_corrupt_attempt() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    assert_eq!(native_scan_status(&connection, "missing-scan").unwrap_err(), "任务不存在");
    connection.execute(
        "UPDATE sentinel_scans SET attempt_count='broken' WHERE id='source-regression'",
        [],
    ).unwrap();
    let error = native_scan_status(&connection, "source-regression").unwrap_err();
    assert!(error.contains("读取任务状态"), "corrupt attempt is not a missing scan: {error}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_status_reports_lease_projection_failure_at_its_boundary() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    connection.execute_batch("CREATE TEMP TABLE agent_coordinator_leases (invalid_column TEXT);").unwrap();
    let error = native_scan_status(&connection, "source-regression").unwrap_err();
    assert!(error.contains("Coordinator 租约状态"), "broken team projection must be visible: {error}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_status_refuses_a_corrupt_branch_report_instead_of_hiding_it_as_null() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    register_native_branches(&connection, "source-regression", 1, &["source"]).unwrap();
    connection.execute(
        "UPDATE native_scan_branches SET report_json='{broken' WHERE scan_id='source-regression' AND branch='source'",
        [],
    ).unwrap();
    let error = native_scan_status(&connection, "source-regression").unwrap_err();
    assert!(error.contains("分支报告") && error.contains("source"), "corrupt report must be named: {error}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_stop_diagnostic_distinguishes_completion_gaps_pause_and_failure() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    register_native_branches(&connection, "source-regression", 1, &["source", "web"]).unwrap();
    let running = native_scan_status(&connection, "source-regression").unwrap();
    assert_eq!(running["stopDiagnostic"]["category"], "active");
    assert_eq!(running["stopDiagnostic"]["stage"], "branch");
    assert_eq!(running["stopDiagnostic"]["obligations"].as_array().unwrap().len(), 2);
    connection.execute("UPDATE sentinel_scans SET status='pausing' WHERE id='source-regression'", []).unwrap();
    let pausing = native_scan_status(&connection, "source-regression").unwrap();
    assert_eq!(pausing["stopDiagnostic"]["code"], "pause_waiting_for_quiescence");
    assert_eq!(pausing["stopDiagnostic"]["category"], "active");
    assert_eq!(pausing["stopDiagnostic"]["nextAction"], "wait_for_worker_exit");
    assert_eq!(pausing["stopDiagnostic"]["continuationConstraint"], "blocked_until_quiescent");
    assert_eq!(pausing["stopDiagnostic"]["automaticResumeAllowed"], false);
    connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id='source-regression'", []).unwrap();
    let paused = native_scan_status(&connection, "source-regression").unwrap();
    assert_eq!(paused["stopDiagnostic"]["code"], "paused_requires_review");
    assert_eq!(paused["stopDiagnostic"]["category"], "soft");
    assert_eq!(paused["stopDiagnostic"]["automaticResumeAllowed"], false);
    connection.execute("UPDATE sentinel_scans SET status='scanning' WHERE id='source-regression'", []).unwrap();
    finish_native_branch(&path, "source-regression", 1, "source", "completed_with_gaps", "source gap", &json!({})).unwrap();
    finish_native_branch(&path, "source-regression", 1, "web", "completed", "done", &json!({})).unwrap();
    let gaps = native_scan_status(&connection, "source-regression").unwrap();
    assert_eq!(gaps["stopDiagnostic"]["code"], "completed_with_gaps");
    assert_eq!(gaps["stopDiagnostic"]["category"], "natural");
    assert!(gaps["stopDiagnostic"]["obligations"].as_array().unwrap().iter().any(|item| item["kind"] == "branch_gap"));
    connection.execute("UPDATE sentinel_scans SET status='completed' WHERE id='source-regression'", []).unwrap();
    let inconsistent = native_scan_status(&connection, "source-regression").unwrap();
    assert_eq!(inconsistent["stopDiagnostic"]["code"], "incomplete_obligations");
    assert_eq!(inconsistent["stopDiagnostic"]["category"], "hard");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_stop_diagnostic_does_not_infer_waf_or_safe_retry_from_text() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    register_native_branches(&connection, "source-regression", 1, &["web"]).unwrap();
    finish_native_branch(&path, "source-regression", 1, "web", "failed", "maybe WAF / 429; try again", &json!({})).unwrap();
    connection.execute("INSERT INTO projects(id,name) VALUES(1,'stop diagnostic fixture')", []).unwrap();
    connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,url,status) VALUES(1,'source-regression','https://example.invalid/queued','queued')", []).unwrap();
    let status = native_scan_status(&connection, "source-regression").unwrap();
    assert_eq!(status["stopDiagnostic"]["code"], "branch_failed");
    assert_eq!(status["stopDiagnostic"]["category"], "hard");
    assert_eq!(status["stopDiagnostic"]["automaticResumeAllowed"], false);
    assert!(status["stopDiagnostic"]["obligations"].as_array().unwrap().iter().any(|item| item["kind"] == "target"));
    assert!(!status["stopDiagnostic"].to_string().contains("waf"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_stop_diagnostic_only_classifies_explicit_retired_backend_as_capability() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    register_native_branches(&connection, "source-regression", 1, &["web"]).unwrap();
    finish_native_branch(&path, "source-regression", 1, "web", "failed", "legacy backend unavailable", &json!({"error":"backend_retired"})).unwrap();
    let status = native_scan_status(&connection, "source-regression").unwrap();
    assert_eq!(status["stopDiagnostic"]["code"], "backend_retired");
    assert_eq!(status["stopDiagnostic"]["category"], "capability");
    assert_eq!(status["stopDiagnostic"]["automaticResumeAllowed"], false);
    fs::remove_dir_all(root).unwrap();
}
