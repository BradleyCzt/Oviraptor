// Original signed Source publication and actual SDK/tools/coverage/exit.
#[test]
fn source_paid_deletion_original_closed_execution_keeps_cold_audit_and_finance() {
    use crate::agent_runtime::{deleted_scan_audit, multi_agent::budget};
    let (root, db, record, report, guard, calls) = source_branch_production_report(false);
    let path = root.join("oviraptor.sqlite3");
    let actor = report["sourceMultiAgent"]["rootRunId"].as_str().unwrap();
    assert_eq!(calls, 7);
    assert!(finish_native_source_branch(&path, &record.scan_id, 1, &report).unwrap());
    drop(guard);
    let original = source_exit_snapshot(&db);
    for name in [
        "source_scope_contracts",
        "source_runtime_contracts",
        "source_analysis_views",
        "source_analysis_results",
        "source_snapshots",
        "analyzer_runs",
        "analyzer_container_receipts",
        "agent_source_model_rounds",
        "agent_source_tool_receipts",
        "agent_multi_exit_receipts",
    ] {
        let n: i64 = db
            .query_row(&format!("SELECT count(*) FROM {name}"), [], |r| r.get(0))
            .unwrap();
        eprintln!("Fresh Source disposable database inventory: {name} rows={n}");
    }
    assert!(crate::agent_runtime::web_mode::root::read(&db, actor)
        .unwrap()
        .is_none());
    let tx = db.unchecked_transaction().unwrap();
    budget::clock::FinalClock::verify_original_exit(&tx, actor).unwrap();
    let proof = deleted_scan_audit::prepare(&tx, &record.scan_id).expect(
        "actual closed paid Source needs its own original audit without fabricated Web mode",
    );
    proof.verify_scope(&tx, &record.scan_id).unwrap();
    drop(proof);
    tx.rollback().unwrap();
    assert_eq!(source_exit_snapshot(&db), original);
    delete_sentinel_scan_inner(&path, &record.scan_id).unwrap();
    assert!(deleted_scan_audit::verify_deleted(&db, &record.scan_id).unwrap());
    let cold = source_exit_snapshot(&db);
    for name in [
        "agent_budget_entries",
        "agent_root_budget_attempts",
        "agent_multi_exit_receipts",
        "agent_model_cost_facts",
        "agent_assignment_attempts",
    ] {
        assert_eq!(
            cold.iter().find(|(n, _)| n == name),
            original.iter().find(|(n, _)| n == name),
            "original finance {name}"
        );
    }
    assert!(budget::root::RootOwner::load_original(&db, actor).is_err());
    delete_sentinel_scan_inner(&path, &record.scan_id).unwrap();
    assert_eq!(source_exit_snapshot(&db), cold);
    drop(db);
    let reopened = db::open(&path).unwrap();
    assert!(deleted_scan_audit::verify_deleted(&reopened, &record.scan_id).unwrap());
    assert_eq!(source_exit_snapshot(&reopened), cold);
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_paid_deletion_original_unknown_usage_preserves_every_row() {
    use crate::agent_runtime::multi_agent::budget;
    let (root, db, record, report, guard, calls) = source_branch_production_report(true);
    let path = root.join("oviraptor.sqlite3");
    assert_eq!(calls, 1);
    assert!(finish_native_source_branch(&path, &record.scan_id, 1, &report).unwrap());
    drop(guard);
    let actor: String = db
        .query_row(
            "SELECT id FROM agent_runs WHERE scan_id=?1 AND role='coordinator'",
            [&record.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    let original = source_exit_snapshot(&db);
    let error = delete_sentinel_scan_inner(&path, &record.scan_id).unwrap_err();
    assert!(
        error.contains("native_paid_audit_retention_required"),
        "{error}"
    );
    assert_eq!(source_exit_snapshot(&db), original);
    for (d, value) in [
        ("model_requests", 1),
        ("model_input_tokens", 4406),
        ("model_output_tokens", 4406),
    ] {
        let cost = budget::balance(&db, &actor, None, d).unwrap();
        assert_eq!((cost.consumed, cost.indeterminate), (0, value));
    }
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_paid_deletion_original_accepted_sarif_and_independent_review_survive_cold_audit() {
    use std::sync::atomic::Ordering;
    let (port, seen, stop) = crate::scanning::agent_tests::spawn_endpoint(Arc::new(|request| {
        let body: JsonValue =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let messages = body["messages"].as_array().unwrap();
        let input: JsonValue = messages.last().unwrap()["content"]
            .as_str()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(JsonValue::Null);
        let message = if input["phase"] == "source_coverage_review" {
            json!({"role":"assistant","content":source_coverage_fixture_response(&input).to_string()})
        } else if input["phase"] == "source_review" {
            json!({"role":"assistant","content":source_review_contract_response(&input["reviewMaterial"]["decisionContract"]).to_string()})
        } else if let Some(tools) = body["tools"].as_array() {
            let analyst = tools
                .iter()
                .any(|t| t["function"]["name"] == "evidence.submit_candidate");
            let (name, args) = if messages.iter().any(|m| m["role"] == "tool") {
                (
                    "assignment.finish",
                    json!({"summary":"Original frozen material inspected","gaps":[]}),
                )
            } else if analyst {
                ("analyzer.list_results", json!({}))
            } else {
                ("repo.inventory", json!({}))
            };
            json!({"role":"assistant","tool_calls":[{"id":name,"type":"function","function":{"name":name,"arguments":args.to_string()}}]})
        } else {
            json!({"role":"assistant","content":"Initial assessment; independent candidate and coverage review are separate."})
        };
        (
            200,
            "application/json",
            json!({"choices":[{"message":message,"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}})
            .to_string(),
        )
    }));
    let (root, db, record) =
        source_dispatch_fixture(&source_specialist_test_environment(port), None);
    let path = root.join("oviraptor.sqlite3");
    let guard = claim_workbench_pipeline_branch(&path, &record.scan_id, 1, "source").unwrap();
    verify_source_runtime_contract(&db, &record.scan_id, 1, &root.join("attempt-0001")).unwrap();
    let mut report = analysis_view_run(&root, &record, |_, engine, _, _, scratch, _| {
        Ok(source_result_outcome(
            engine,
            scratch,
            "app.py",
            "original-candidate",
        ))
    })
    .unwrap();
    let assessment =
        run_native_source_assessments(&path, &record.scan_id, 1, &root.join("attempt-0001"))
            .unwrap();
    merge_native_source_assessments(&mut report, assessment);
    assert_eq!(
        report["sourceMultiAgent"]["independentCandidateReviewCompleted"],
        true
    );
    assert_eq!(seen.lock().unwrap().len(), 8);
    assert!(finish_native_source_branch(&path, &record.scan_id, 1, &report).unwrap());
    drop(guard);
    let before = source_exit_snapshot(&db);
    let originals = before
        .iter()
        .find(|(n, _)| n == "import_record_revisions")
        .unwrap();
    assert!(!originals.1.is_empty());
    let artifact_dir = root.join("artifact-import-cas");
    fn file_bytes(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut pending = vec![root.to_owned()];
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    out.push((path.clone(), fs::read(path).unwrap()));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
    let files = file_bytes(&artifact_dir);
    assert!(!files.is_empty());
    delete_sentinel_scan_inner(&path, &record.scan_id).unwrap();
    assert!(
        crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &record.scan_id).unwrap()
    );
    let cold = source_exit_snapshot(&db);
    for name in [
        "import_record_revisions",
        "agent_budget_entries",
        "agent_root_budget_attempts",
        "agent_multi_exit_receipts",
    ] {
        assert_eq!(
            before.iter().find(|(n, _)| n == name),
            cold.iter().find(|(n, _)| n == name),
            "{name}"
        );
    }
    assert_eq!(file_bytes(&artifact_dir), files);
    let raw: String = db
        .query_row(
            "SELECT audit_json FROM native_deleted_scan_audits_v2 WHERE scan_id=?1",
            [&record.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT version FROM native_deleted_scan_audits_v2 WHERE scan_id=?1",
            [&record.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_deleted_scan_audits WHERE scan_id=?1",
            [&record.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let audit: JsonValue = serde_json::from_str(&raw).unwrap();
    assert_eq!(audit["version"], 2);
    let retained = audit["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "import_record_revisions")
        .unwrap();
    assert_eq!(
        retained["rows"].as_array().unwrap().len(),
        originals.1.len()
    );
    drop(db);
    let reopened = db::open(&path).unwrap();
    assert!(
        crate::agent_runtime::deleted_scan_audit::verify_deleted(&reopened, &record.scan_id)
            .unwrap()
    );
    assert_eq!(source_exit_snapshot(&reopened), cold);
    assert_eq!(seen.lock().unwrap().len(), 8);
    stop.store(true, Ordering::SeqCst);
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}
