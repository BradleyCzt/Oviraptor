#[test]
fn source_branch_actual_unfinished_tools_cannot_publish_completed_with_gaps() {
    use crate::agent_runtime::multi_agent::budget;
    use std::sync::atomic::Ordering;
    let round = AtomicUsize::new(0);
    let (port, seen, stop) = crate::scanning::agent_tests::spawn_endpoint(Arc::new(
        move |request| {
            let body: JsonValue =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            let message = if let Some(tools) = body["tools"].as_array() {
                let analyst = tools
                    .iter()
                    .any(|t| t["function"]["name"] == "evidence.submit_candidate");
                let (name, args) = if analyst {
                    // Three valid, actually paid reads never call assignment.finish.
                    ("analyzer.list_results", json!({}))
                } else if body["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|m| m["role"] == "tool")
                {
                    (
                        "assignment.finish",
                        json!({"summary":"Actual scoped inventory inspected","gaps":[]}),
                    )
                } else {
                    ("repo.inventory", json!({}))
                };
                assert!(tools.iter().any(|t| t["function"]["name"] == name));
                json!({"role":"assistant","tool_calls":[{"id":format!("actual-{}",round.fetch_add(1,Ordering::SeqCst)),"type":"function","function":{"name":name,"arguments":args.to_string()}}]})
            } else {
                json!({"role":"assistant","content":"Initial source assessment only; actual tool and review phases are separate."})
            };
            (200,"application/json",json!({"choices":[{"message":message,"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
        },
    ));
    let (root, db, record) =
        source_dispatch_fixture(&source_specialist_test_environment(port), None);
    let path = root.join("oviraptor.sqlite3");
    let work = root.join("attempt-0001");
    launch_native_source_pipeline(
        path.clone(),
        root.clone(),
        record.scan_id.clone(),
        1,
        work,
        record.source_path.clone(),
        record.scan_type.clone(),
        record.diff_base.clone(),
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let status = loop {
        let status: String = db
            .query_row(
                "SELECT status FROM sentinel_scans WHERE id=?1",
                [&record.scan_id],
                |r| r.get(0),
            )
            .unwrap();
        let exited = crate::agent_runtime::execution_owner::probe_native_invocation(
            &path,
            &record.scan_id,
            1,
            "branch",
            "source",
        )
        .is_ok_and(|guard| guard.is_some());
        if status != "scanning" && exited {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "actual Source launcher did not exit: {status}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    stop.store(true, Ordering::SeqCst);
    let (actor,state,terminal):(String,String,String)=db.query_row("SELECT id,status,terminal_state FROM agent_runs WHERE scan_id=?1 AND role='coordinator'",[&record.scan_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let report:String=db.query_row("SELECT report_json FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=1 AND branch='source'",[&record.scan_id],|r|r.get(0)).unwrap();
    let report: JsonValue = serde_json::from_str(&report).unwrap();
    eprintln!(
        "Actual Source launcher: scan={status}, Root={state}/{terminal}, SDK={}, report={}",
        seen.lock().unwrap().len(),
        report["sourceMultiAgent"]
    );
    assert_eq!((state.as_str(), terminal.as_str()), ("terminal", "paused"));
    assert_eq!(seen.lock().unwrap().len(), 7);
    for (dimension, expected) in [
        ("model_requests", 7),
        ("model_input_tokens", 70),
        ("model_output_tokens", 70),
        ("target_requests", 0),
        ("browser_actions", 0),
        ("controlled_writes", 0),
        ("upload_bytes", 0),
    ] {
        let cost = budget::balance(&db, &actor, None, dimension).unwrap();
        assert_eq!((cost.consumed, cost.indeterminate), (expected, 0));
    }
    let tx = db.unchecked_transaction().unwrap();
    budget::clock::FinalClock::verify_original_exit(&tx, &actor).unwrap();
    tx.rollback().unwrap();
    assert_eq!(report["sourceMultiAgent"]["status"], "incomplete");
    assert_eq!(
        report["sourceMultiAgent"]["reason"],
        "source_tool_phase_round_budget_exhausted_without_finish"
    );
    assert_eq!(
        status, "partial",
        "actual uncompleted Root cannot be presented as completed source coverage"
    );
    let (branch,target):(String,String)=db.query_row("SELECT b.status,t.status FROM native_scan_branches b JOIN sentinel_targets t ON t.scan_id=b.scan_id WHERE b.scan_id=?1 AND b.branch='source'",[&record.scan_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((branch.as_str(), target.as_str()), ("partial", "partial"));
    assert_source_exhausted_launcher_accounting(&db, &actor, &path, &record.scan_id);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

// Uses the signed production publication, actual branch claim and Source entry.
// No synthetic Root, permission, result material, usage or exit receipt.
fn source_branch_production_report(
    unknown: bool,
) -> (
    PathBuf,
    rusqlite::Connection,
    WorkbenchStartRecord,
    JsonValue,
    NativeBranchGuard,
    usize,
) {
    use std::sync::atomic::Ordering;
    let number = AtomicUsize::new(0);
    let (port, seen, stop) = crate::scanning::agent_tests::spawn_endpoint(Arc::new(
        move |request| {
            if unknown {
                return (
                    503,
                    "application/json",
                    json!({"error":{"message":"scripted unavailable with no usage"}}).to_string(),
                );
            }
            let body: JsonValue =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            let messages = body["messages"].as_array().unwrap();
            let input: JsonValue = messages.last().unwrap()["content"]
                .as_str()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(JsonValue::Null);
            let message = if input["phase"] == "source_coverage_review" {
                assert!(body.get("tools").is_none());
                json!({"role":"assistant","content":source_coverage_fixture_response(&input).to_string()})
            } else if let Some(tools) = body["tools"].as_array() {
                let done = messages.iter().any(|m| m["role"] == "tool");
                let analyst = tools
                    .iter()
                    .any(|t| t["function"]["name"] == "evidence.submit_candidate");
                let (name, args) = if done {
                    (
                        "assignment.finish",
                        json!({"summary":"Actual frozen source material inspected","gaps":[]}),
                    )
                } else if analyst {
                    ("analyzer.list_results", json!({}))
                } else {
                    ("repo.inventory", json!({}))
                };
                assert!(tools.iter().any(|t| t["function"]["name"] == name));
                json!({"role":"assistant","tool_calls":[{"id":format!("actual-done-{}",number.fetch_add(1,Ordering::SeqCst)),"type":"function","function":{"name":name,"arguments":args.to_string()}}]})
            } else {
                json!({"role":"assistant","content":"Actual initial assessment; scoped tools and independent coverage follow separately."})
            };
            (200,"application/json",json!({"choices":[{"message":message,"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
        },
    ));
    let (root, db, record) =
        source_dispatch_fixture(&source_specialist_test_environment(port), None);
    let path = root.join("oviraptor.sqlite3");
    let guard = claim_workbench_pipeline_branch(&path, &record.scan_id, 1, "source").unwrap();
    let report = run_native_source_scan(
        &path,
        &root,
        &record.scan_id,
        1,
        &root.join("attempt-0001"),
        &record.source_path,
        &record.scan_type,
        &record.diff_base,
    )
    .unwrap();
    stop.store(true, Ordering::SeqCst);
    let calls = seen.lock().unwrap().len();
    (root, db, record, report, guard, calls)
}

#[test]
fn source_branch_actual_completed_report_requires_original_facts_and_projection_rolls_back() {
    use crate::agent_runtime::multi_agent::budget;
    let (root, db, record, report, guard, calls) = source_branch_production_report(false);
    let path = root.join("oviraptor.sqlite3");
    let actor = report["sourceMultiAgent"]["rootRunId"].as_str().unwrap();
    assert_eq!(calls, 7);
    assert_eq!(
        report["sourceMultiAgent"]["independentReviewCompleted"],
        true
    );
    let original = source_exit_snapshot(&db);
    for (field, value, expected) in [
        (
            "rootRunId",
            json!("unrelated-original-root"),
            "source_branch_report_root_changed",
        ),
        (
            "modelRequests",
            json!(0),
            "source_branch_report_completion_changed",
        ),
        (
            "totalTokens",
            json!(0),
            "source_branch_report_completion_changed",
        ),
        (
            "independentReviewCompleted",
            json!(false),
            "source_branch_report_completion_changed",
        ),
    ] {
        let mut forged = report.clone();
        forged["sourceMultiAgent"][field] = value;
        assert_eq!(
            finish_native_source_branch(&path, &record.scan_id, 1, &forged).unwrap_err(),
            expected
        );
        assert_eq!(
            source_exit_snapshot(&db),
            original,
            "report rejection mutated {field}"
        );
    }
    let mut forged = report.clone();
    forged["analysisResultsDigest"] = json!("changed");
    assert_eq!(
        finish_native_source_branch(&path, &record.scan_id, 1, &forged).unwrap_err(),
        "source_branch_report_results_changed"
    );
    assert_eq!(source_exit_snapshot(&db), original);
    assert_eq!(
        finish_native_source_branch(&path, &record.scan_id, 1, &json!({"error":"worker_exited"}))
            .unwrap_err(),
        "source_branch_completed_root_missing_report"
    );
    assert_eq!(source_exit_snapshot(&db), original);
    assert_eq!(
        finish_native_source_branch(
            &path,
            &record.scan_id,
            1,
            &json!({"error":"spawn_failed","executionStarted":false})
        )
        .unwrap_err(),
        "source_branch_preexecution_claim_conflicts_with_root"
    );
    assert_eq!(source_exit_snapshot(&db), original);
    assert!(!finish_native_source_branch(&path, &record.scan_id, 2, &report).unwrap());
    assert_eq!(source_exit_snapshot(&db), original);
    db.execute_batch("CREATE TRIGGER source_branch_original_corruption AFTER UPDATE ON native_scan_branches WHEN NEW.branch='source' BEGIN
        UPDATE agent_runs SET terminal_reason='changed during Source projection' WHERE scan_id=NEW.scan_id AND attempt_number=NEW.attempt_number AND role='coordinator'; END;").unwrap();
    let before = source_exit_snapshot(&db);
    let error = finish_native_source_branch(&path, &record.scan_id, 1, &report).unwrap_err();
    eprintln!("Source projection original-fact trigger rejection: {error}");
    assert_eq!(error, "budget_clock_final_persistence_conflict");
    assert_eq!(
        source_exit_snapshot(&db),
        before,
        "projection and its trigger must roll back every physical row"
    );
    db.execute_batch("DROP TRIGGER source_branch_original_corruption")
        .unwrap();
    db.execute_batch("CREATE TRIGGER source_branch_projection_corruption AFTER UPDATE ON native_scan_branches WHEN NEW.branch='source' AND NEW.status='completed_with_gaps' BEGIN
        UPDATE native_scan_branches SET status='failed' WHERE scan_id=NEW.scan_id AND attempt_number=NEW.attempt_number AND branch='source'; END;").unwrap();
    let before = source_exit_snapshot(&db);
    assert_eq!(
        finish_native_source_branch(&path, &record.scan_id, 1, &report).unwrap_err(),
        "source_branch_projection_unconfirmed"
    );
    assert_eq!(
        source_exit_snapshot(&db),
        before,
        "invalid presentation must roll back all physical rows"
    );
    db.execute_batch("DROP TRIGGER source_branch_projection_corruption")
        .unwrap();
    assert!(finish_native_source_branch(&path, &record.scan_id, 1, &report).unwrap());
    let (scan,branch,target,reason):(String,String,String,String)=db.query_row("SELECT s.status,b.status,t.status,b.checkpoint FROM sentinel_scans s JOIN native_scan_branches b ON b.scan_id=s.id JOIN sentinel_targets t ON t.scan_id=s.id WHERE s.id=?1 AND b.branch='source'",[&record.scan_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(
        (scan.as_str(), branch.as_str(), target.as_str()),
        (
            "completed_with_gaps",
            "completed_with_gaps",
            "completed_with_gaps"
        )
    );
    let saved: String = db
        .query_row(
            "SELECT terminal_reason FROM agent_runs WHERE id=?1",
            [actor],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reason, saved);
    let after = source_exit_snapshot(&db);
    for ((name, old), (actual, new)) in original.iter().zip(after.iter()) {
        assert_eq!(name, actual);
        if ![
            "sentinel_scans",
            "sentinel_scan_attempts",
            "sentinel_targets",
            "native_scan_branches",
        ]
        .contains(&name.as_str())
        {
            assert_eq!(
                old, new,
                "Source presentation changed original table {name}"
            );
        }
    }
    for (dimension, expected) in [
        ("model_requests", 7),
        ("model_input_tokens", 70),
        ("model_output_tokens", 70),
    ] {
        let balance = budget::balance(&db, actor, None, dimension).unwrap();
        assert_eq!((balance.consumed, balance.indeterminate), (expected, 0));
    }
    let tx = db.unchecked_transaction().unwrap();
    budget::clock::FinalClock::verify_original_exit(&tx, actor).unwrap();
    tx.rollback().unwrap();
    assert!(!finish_native_source_branch(&path, &record.scan_id, 1, &report).unwrap());
    assert!(
        !finish_native_source_branch(&path, &record.scan_id, 1, &json!({"error":"late"})).unwrap()
    );
    assert_eq!(source_exit_snapshot(&db), after);
    drop(guard);
    assert_eq!(source_exit_snapshot(&db), after);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

include!("tests_source_branch_unknown_original.rs");

include!("tests_source_branch_original_discovery.rs");
