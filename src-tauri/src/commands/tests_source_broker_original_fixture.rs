type SourceBrokerOriginalCalls = Vec<(&'static str, JsonValue)>;
type SourceBrokerOriginalSeen = Arc<std::sync::Mutex<Vec<String>>>;
type SourceBrokerOriginalContinuation =
    Box<dyn FnOnce(&SourceBrokerOriginalProbe, &SourceBrokerOriginalSeen)>;
#[derive(Default)]
struct SourceBrokerOriginalScript {
    rounds: Vec<SourceBrokerOriginalCalls>,
    // Test provider may import late data only after an earlier original tool
    // receipt exists. This never creates an execution grant or a result receipt.
    before_analyst_response:
        Option<Box<dyn Fn(usize) -> Option<SourceBrokerOriginalCalls> + Send + Sync>>,
}

// Actual signed Source publication -> fresh original Root -> production
// supervisor/roles/SDK/SourceBroker/receipts/exit. No synthetic Root/grant issuer.
struct SourceBrokerOriginalProbe {
    root: PathBuf,
    db: rusqlite::Connection,
    record: WorkbenchStartRecord,
    actor: crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    result: Result<JsonValue, String>,
    calls: usize,
}
impl SourceBrokerOriginalProbe {
    fn output(&self, round: i64, index: i64) -> JsonValue {
        let raw:String=self.db.query_row("SELECT t.output_json FROM agent_source_tool_receipts t JOIN agent_runs r ON r.id=t.child_run_id
            WHERE r.root_run_id=?1 AND r.role='source_analyst' AND t.round_number=?2 AND t.call_index=?3 AND t.state='completed'",
            params![self.actor.root_run_id,round,index],|r|r.get(0)).unwrap();
        serde_json::from_str(&raw).unwrap()
    }
    fn cleanup(self) {
        drop(self.db);
        fs::remove_dir_all(self.root).unwrap();
    }
}
fn source_broker_original_probe(
    mode: &str,
    valid_base: bool,
    rounds: Vec<Vec<(&'static str, JsonValue)>>,
    fault: Option<&str>,
    configure: impl FnOnce(&rusqlite::Connection, &mut WorkbenchStartRecord),
) -> SourceBrokerOriginalProbe {
    source_broker_original_probe_using(
        mode,
        valid_base,
        |_, _, _, _| rounds,
        fault,
        configure,
        source_regression_outcome,
    )
}

fn source_broker_original_probe_using(
    mode: &str,
    valid_base: bool,
    rounds_for_results: impl FnOnce(
        &Path,
        &rusqlite::Connection,
        &WorkbenchStartRecord,
        &JsonValue,
    ) -> Vec<Vec<(&'static str, JsonValue)>>,
    fault: Option<&str>,
    configure: impl FnOnce(&rusqlite::Connection, &mut WorkbenchStartRecord),
    analyze: impl FnMut(&str, &Path) -> analyzer::AnalyzerOutcome,
) -> SourceBrokerOriginalProbe {
    source_broker_original_probe_scripted(
        mode,
        valid_base,
        |root, db, record, report| SourceBrokerOriginalScript {
            rounds: rounds_for_results(root, db, record, report),
            before_analyst_response: None,
        },
        fault,
        configure,
        analyze,
    )
}

fn source_broker_original_probe_scripted(
    mode: &str,
    valid_base: bool,
    prepare: impl FnOnce(
        &Path,
        &rusqlite::Connection,
        &WorkbenchStartRecord,
        &JsonValue,
    ) -> SourceBrokerOriginalScript,
    fault: Option<&str>,
    configure: impl FnOnce(&rusqlite::Connection, &mut WorkbenchStartRecord),
    analyze: impl FnMut(&str, &Path) -> analyzer::AnalyzerOutcome,
) -> SourceBrokerOriginalProbe {
    source_broker_original_probe_continued(
        mode, valid_base, prepare, fault, configure, analyze, None,
    )
}

fn source_broker_original_probe_continued(
    mode: &str,
    valid_base: bool,
    prepare: impl FnOnce(
        &Path,
        &rusqlite::Connection,
        &WorkbenchStartRecord,
        &JsonValue,
    ) -> SourceBrokerOriginalScript,
    fault: Option<&str>,
    configure: impl FnOnce(&rusqlite::Connection, &mut WorkbenchStartRecord),
    mut analyze: impl FnMut(&str, &Path) -> analyzer::AnalyzerOutcome,
    continuation: Option<SourceBrokerOriginalContinuation>,
) -> SourceBrokerOriginalProbe {
    use crate::agent_runtime::multi_agent::budget;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let configured_rounds = Arc::new(std::sync::Mutex::new(SourceBrokerOriginalScript::default()));
    let rounds = configured_rounds.clone();
    let analyst_round = AtomicUsize::new(0);
    let mapper_round = AtomicUsize::new(0);
    let (port, seen, stop) = crate::scanning::agent_tests::spawn_endpoint(Arc::new(
        move |request| {
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
            } else if input["phase"] == "source_review" {
                assert!(body.get("tools").is_none());
                json!({"role":"assistant","content":source_review_contract_response(&input["reviewMaterial"]["decisionContract"]).to_string()})
            } else if let Some(tools) = body["tools"].as_array() {
                let analyst = tools
                    .iter()
                    .any(|t| t["function"]["name"] == "evidence.submit_candidate");
                let role = if analyst {
                    crate::agent_runtime::contract::AgentRole::SourceAnalyst
                } else {
                    crate::agent_runtime::contract::AgentRole::RepoMapper
                };
                let names: std::collections::BTreeSet<_> = tools
                    .iter()
                    .map(|t| t["function"]["name"].as_str().unwrap().to_owned())
                    .collect();
                assert_eq!(
                    names,
                    crate::agent_runtime::multi_agent::source::tool_capabilities(role)
                        .unwrap()
                        .into_iter()
                        .collect()
                );
                assert!(!names.contains("replay_http") && !names.contains("callgraph.get_slice"));
                let number = if analyst {
                    analyst_round.fetch_add(1, Ordering::SeqCst)
                } else {
                    mapper_round.fetch_add(1, Ordering::SeqCst)
                };
                let calls = if analyst {
                    let script = rounds.lock().unwrap();
                    script.before_analyst_response.as_ref().and_then(|hook|hook(number))
                        .or_else(||script.rounds.get(number).cloned())
                        .unwrap_or_else(||vec![("assignment.finish",json!({"summary":"Scoped local source work requires independent review","gaps":[]}))])
                } else if !messages.iter().any(|message| message["role"] == "tool") {
                    vec![("repo.inventory", json!({}))]
                } else {
                    vec![(
                        "assignment.finish",
                        json!({"summary":"Scoped source inventory","gaps":[]}),
                    )]
                };
                json!({"role":"assistant","tool_calls":calls.into_iter().enumerate().map(|(i,(name,args))|json!({"id":format!("{}-{number}-{i}",role.as_str()),"type":"function","function":{"name":name,"arguments":args.to_string()}})).collect::<Vec<_>>()})
            } else {
                json!({"role":"assistant","content":"Initial source assessment only; tools and independent review are separate."})
            };
            (200,"application/json",json!({"choices":[{"message":message,"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
        },
    ));
    let model = source_specialist_test_environment(port);
    let (root, db, record) = analysis_view_fixture_configure(mode, valid_base, |db, record| {
        db.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles',json(?1),'$.activeModelProfileId','source-fixture') WHERE id=(SELECT id FROM config_profiles ORDER BY is_default DESC,id LIMIT 1)",
            [json!([{"id":"source-fixture","llm":model.llm,"apiKey":model.api_key,"apiBase":model.api_base,"deployment":model.deployment}]).to_string()]).unwrap();
        record.llm_policy = source_model_policy(&model);
        configure(db, record);
    });
    // Retry tests own a real branch before analyzer/SDK effects. Existing
    // caller seams retain their prior lifecycle when no continuation is given.
    let branch = continuation.as_ref().map(|_| {
        claim_workbench_pipeline_branch(
            &root.join("oviraptor.sqlite3"),
            &record.scan_id,
            1,
            "source",
        )
        .unwrap()
    });
    let analysis = analysis_view_run(&root, &record, |_, engine, _, _, scratch, _| {
        Ok(analyze(engine, scratch))
    })
    .unwrap();
    *configured_rounds.lock().unwrap() = prepare(&root, &db, &record, &analysis);
    let work = root.join("attempt-0001");
    let born = prepare_native_source_coordinator(&db, &record.scan_id, 1, &work).unwrap();
    let actor = native_source_fresh_finance::original_for_execution(&db, &born.run_id).unwrap();
    assert_eq!(actor.lease_epoch, 1);
    let original = source_exit_snapshot(&db)
        .into_iter()
        .filter(|(n, _)| {
            matches!(
                n.as_str(),
                "agent_root_budget_attempts" | "agent_budget_limits" | "agent_budget_clock_origins"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(original.len(), 3);
    assert!(original.iter().all(|(_, r)| !r.is_empty()));
    if let Some(sql) = fault {
        db.execute_batch(sql).unwrap();
    }
    let result =
        run_native_source_assessments(&root.join("oviraptor.sqlite3"), &record.scan_id, 1, &work);
    let calls = seen.lock().unwrap().len();
    eprintln!(
        "Original Source scope {mode}: calls={calls}, error={:?}",
        result.as_ref().err()
    );
    assert_eq!(
        source_exit_snapshot(&db)
            .into_iter()
            .filter(|(n, _)| matches!(
                n.as_str(),
                "agent_root_budget_attempts" | "agent_budget_limits" | "agent_budget_clock_origins"
            ))
            .collect::<Vec<_>>(),
        original
    );
    let owner = budget::root::RootOwner::load_original(&db, &actor.root_run_id).unwrap();
    owner.require_original_coordinator(&db, &actor).unwrap();
    for (dimension, expected) in [
        ("model_requests", calls as i64),
        ("model_input_tokens", 10 * calls as i64),
        ("model_output_tokens", 10 * calls as i64),
        ("target_requests", 0),
        ("browser_actions", 0),
        ("controlled_writes", 0),
        ("upload_bytes", 0),
    ] {
        let cost = budget::balance(&db, &actor.root_run_id, None, dimension).unwrap();
        assert_eq!(cost.consumed, expected, "{dimension}: {result:?}");
        assert_eq!(
            cost.indeterminate, 0,
            "known actual provider receipts cannot become unknown"
        );
        if result.is_ok() {
            assert_eq!(cost.reserved, 0);
        }
    }
    let tx = db.unchecked_transaction().unwrap();
    budget::clock::FinalClock::verify_original_exit(&tx, &actor.root_run_id).unwrap();
    tx.rollback().unwrap();
    let proof = SourceBrokerOriginalProbe {
        root,
        db,
        record,
        actor,
        result,
        calls,
    };
    if proof.result.is_ok() {
        let before = source_exit_snapshot(&proof.db);
        assert!(run_native_source_assessments(
            &proof.root.join("oviraptor.sqlite3"),
            &proof.record.scan_id,
            1,
            &proof.root.join("attempt-0001")
        )
        .is_err());
        assert_eq!(
            source_exit_snapshot(&proof.db),
            before,
            "a closed original Source cannot execute or rewrite paid rows on retry"
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            calls,
            "no extra SDK after original closure"
        );
    }
    if let Some(continuation) = continuation {
        let mut report = analysis;
        merge_native_source_assessments(&mut report, proof.result.as_ref().unwrap().clone());
        assert!(!report["gaps"].as_array().unwrap().is_empty());
        assert!(finish_native_branch(
            &proof.root.join("oviraptor.sqlite3"),
            &proof.record.scan_id,
            1,
            "source",
            "completed_with_gaps",
            "Original Source completed with reviewed gaps",
            &report
        )
        .unwrap());
        let mut branch = branch.unwrap();
        branch.disarm();
        drop(branch);
        continuation(&proof, &seen);
    }
    stop.store(true, Ordering::SeqCst);
    proof
}
