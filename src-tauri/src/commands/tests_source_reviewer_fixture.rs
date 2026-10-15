// V3 uses the production entry and an actual localhost model transport. V1
// material tests remain pinned to their historical four-assignment contract.
fn source_reviewer_execution_fixture(response_mode: &'static str, fault: Option<&str>) -> (
    PathBuf, rusqlite::Connection, crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    Result<JsonValue,String>,
) {
    source_reviewer_execution_fixture_schema(response_mode,fault,3)
}

fn source_reviewer_execution_fixture_schema(response_mode: &'static str, fault: Option<&str>, schema:i64) -> (
    PathBuf, rusqlite::Connection, crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    Result<JsonValue,String>,
) {
    source_reviewer_execution_fixture_policy(response_mode,fault,schema,None)
}

fn source_reviewer_execution_fixture_policy(response_mode: &'static str, fault: Option<&str>, schema:i64, policy:Option<JsonValue>) -> (
    PathBuf, rusqlite::Connection, crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    Result<JsonValue,String>,
) {
    source_reviewer_execution_fixture_using(response_mode,fault,schema,policy,|root,_,record,_| {
        run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
    })
}

fn source_reviewer_execution_fixture_using(response_mode: &'static str, fault: Option<&str>, schema:i64, policy:Option<JsonValue>,
    execute:impl FnOnce(&Path,&rusqlite::Connection,&WorkbenchStartRecord,&crate::agent_runtime::multi_agent::lease::CoordinatorLease)->Result<JsonValue,String>,
) -> (PathBuf,rusqlite::Connection,crate::agent_runtime::multi_agent::lease::CoordinatorLease,Result<JsonValue,String>) {
    source_reviewer_execution_fixture_using_calls(response_mode,fault,schema,policy,7,execute)
}

fn source_reviewer_execution_fixture_using_calls(response_mode: &'static str, fault: Option<&str>, schema:i64, policy:Option<JsonValue>,expected_calls:usize,
    execute:impl FnOnce(&Path,&rusqlite::Connection,&WorkbenchStartRecord,&crate::agent_runtime::multi_agent::lease::CoordinatorLease)->Result<JsonValue,String>,
) -> (PathBuf,rusqlite::Connection,crate::agent_runtime::multi_agent::lease::CoordinatorLease,Result<JsonValue,String>) {
    source_reviewer_execution_fixture_configured(response_mode,fault,schema,policy,expected_calls,None,execute)
}

fn source_reviewer_execution_fixture_configured(response_mode: &'static str, fault: Option<&str>, schema:i64, policy:Option<JsonValue>,expected_calls:usize,scope:Option<&str>,
    execute:impl FnOnce(&Path,&rusqlite::Connection,&WorkbenchStartRecord,&crate::agent_runtime::multi_agent::lease::CoordinatorLease)->Result<JsonValue,String>,
) -> (PathBuf,rusqlite::Connection,crate::agent_runtime::multi_agent::lease::CoordinatorLease,Result<JsonValue,String>) {
    source_reviewer_execution_fixture_configured_before_birth(response_mode,fault,schema,policy,expected_calls,(scope,|_,_|{}),execute)
}

fn source_reviewer_execution_fixture_configured_before_birth(response_mode: &'static str, fault: Option<&str>, schema:i64, policy:Option<JsonValue>,expected_calls:usize,
    configuration:(Option<&str>,impl FnOnce(&rusqlite::Connection,&mut WorkbenchStartRecord)),
    execute:impl FnOnce(&Path,&rusqlite::Connection,&WorkbenchStartRecord,&crate::agent_runtime::multi_agent::lease::CoordinatorLease)->Result<JsonValue,String>,
) -> (PathBuf,rusqlite::Connection,crate::agent_runtime::multi_agent::lease::CoordinatorLease,Result<JsonValue,String>) {
    let (scope,configure)=configuration;
    let no_candidates=response_mode.starts_with("no_candidates");
    let (port,seen,stop)=crate::scanning::agent_tests::spawn_endpoint(std::sync::Arc::new(move |request| {
        let body:JsonValue=serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let last=body["messages"].as_array().unwrap().last().unwrap();
        let input:JsonValue=last["content"].as_str().and_then(|s|serde_json::from_str(s).ok()).unwrap_or(JsonValue::Null);
        let message=if input["phase"]=="source_coverage_review" {
            assert!(body.get("tools").is_none());
            assert_eq!(input["toolsGranted"],json!([]));
            assert_eq!(input["targetRequestsGranted"],0);
            assert_eq!(input["hostActionsGranted"],0);
            assert_eq!(body["messages"][0]["content"],crate::agent_runtime::multi_agent::source_coverage_reviewer::SYSTEM);
            assert!(input["reviewMaterial"]["evidenceMaterial"].is_object());
            assert_eq!(input["reviewMaterial"]["phaseAssessments"].as_array().unwrap().len(),4);
            let mut response=source_coverage_fixture_response(&input);
            match response_mode {
                "coverage_drop_gap"=>{response["outstandingGaps"]=json!([]);response["coverageSufficient"]=json!(true);},
                "coverage_foreign"=>response["materialDigest"]=json!("another-task"),
                "coverage_fabricated"=>response["evidenceRefs"]=json!(["phase:not-in-material"]),
                _=>{},
            }
            json!({"role":"assistant","content":response.to_string()})
        } else if input["phase"]=="source_review" {
            assert!(body.get("tools").is_none());
            assert_eq!(input["targetRequestsGranted"],0);
            assert_eq!(input["hostActionsGranted"],0);
            assert_eq!(body["messages"][0]["content"],crate::agent_runtime::multi_agent::source_reviewer::SYSTEM);
            let mut response=source_review_contract_response(&input["reviewMaterial"]["decisionContract"]);
            match response_mode {
                "missing"=>{response["decisions"].as_array_mut().unwrap().pop();},
                "foreign"=>response["materialDigest"]=json!("another-task"),
                "fabricated"=>response["decisions"][0]["evidenceRefs"]=json!(["tool:not-in-material"]),
                "confirmed"|"rejected"|"graph_confirmed"|"all_confirmed"=>{
                    for decision in response["decisions"].as_array_mut().unwrap() {
                        let kind=if response_mode=="graph_confirmed" {"graph_candidate"} else {"analyzer_revision"};
                        if response_mode!="all_confirmed" && decision["identity"]["kind"]!=kind {continue;}
                        let requirement=input["reviewMaterial"]["decisionContract"]["requirements"]["candidates"]
                            .as_array().unwrap().iter().find(|c|c["identity"]==decision["identity"]).unwrap();
                        decision["verdict"]=json!(if matches!(response_mode,"graph_confirmed"|"all_confirmed") {"confirmed"} else {response_mode});
                        decision["severity"]=json!("high");
                        decision["missingEvidence"]=json!([]);
                        decision["evidenceRefs"]=json!([requirement["evidenceRefs"][0]]);
                        decision["rationale"]=json!("Fixture independent review of the frozen analyzer evidence");
                        decision["reasonCodes"]=json!(["fixture_source_review"]);
                    }
                },
                "valid"=>{},mode if mode.starts_with("coverage_")=>{},_=>panic!("unknown fixture response"),
            }
            json!({"role":"assistant","content":response.to_string()})
        } else if body.get("tools").is_none() {
            json!({"role":"assistant","content":"Initial assessment; not independent review"})
        } else {
            let analyst=body["tools"].as_array().unwrap().iter().any(|t|t["function"]["name"]=="evidence.submit_candidate");
            let gaps=if response_mode=="coverage_gap" {vec!["dependency_graph_incomplete"]} else {vec![]};
            let (name,args)=if response_mode=="tool_exhausted" {("repo.inventory",json!({}))}
                else if response_mode=="analyst_exhausted" && analyst {("analyzer.list_results",json!({}))}
                else if last["role"]=="tool" {("assignment.finish",json!({"summary":"Source analysis complete; needs review","gaps":gaps}))}
                else if analyst && !no_candidates {("evidence.submit_candidate",json!({"title":"Independent review test candidate","rationale":"Needs corroboration","path":"app.py","line":1}))}
                else if matches!(response_mode,"graph_confirmed"|"all_confirmed") {("repo.read_slice",json!({"path":"app.py"}))}
                else {("repo.inventory",json!({}))};
            let id=if matches!(response_mode,"tool_exhausted"|"analyst_exhausted") {format!("{name}-{}",body["messages"].as_array().unwrap().len())} else {name.to_string()};
            json!({"role":"assistant","tool_calls":[{"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}}]})
        };
        (200,"application/json",json!({"choices":[{"message":message,"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
    }));
    let (root,connection,record)=source_dispatch_fixture_configure(&source_specialist_test_environment(port),None,|db,record| {
        if response_mode=="no_candidates_separate_git" {
            // An actual repository with external Git metadata has no excluded
            // metadata directory in its source tree. Do not mutate gap receipts
            // or erase exclusions after freezing to manufacture a clean review.
            let repository=Path::new(&record.source_path);
            let metadata=repository.parent().unwrap().join("source-fixture-git-metadata");
            analysis_view_git(repository,&["init","-q","--separate-git-dir",metadata.to_str().unwrap()]);
        }
        if let Some(scope)=scope {record.scope_mode=scope.into();}
        if let Some(policy)=policy {for key in ["maxCritical","maxHigh","blockRelease"] {record.policy[key]=policy[key].clone();}}
        configure(db,record);
    });
    let analysis=analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(if no_candidates {
        source_regression_outcome(engine,scratch)
    } else {source_result_outcome(engine,scratch,"app.py","independent-review")})).unwrap();
    let initial_plan=NativeSourcePlan::load(&connection,&record.scan_id,1).unwrap().unwrap();
    let work=root.join("attempt-0001");
    let coordinator=prepare_native_source_coordinator_fresh_schema_for_test(&connection,&record.scan_id,1,&work,schema).unwrap();
    // Read its born C; the fixture must never extend expiry with acquire.
    let mut lease=native_source_fresh_finance::original_for_execution(&connection,&coordinator.run_id).unwrap();
    if let Some(sql)=fault {connection.execute_batch(sql).unwrap();}
    let result=execute(&root,&connection,&record,&lease);
    if let Ok(report)=&result {
        if !report["gate"].is_null() {assert_eq!(report["gate"]["freeze"],analysis["gate"]["freeze"],"post-review freeze must match the actual analyzer input");}
        assert_eq!(NativeSourcePlan::load(&connection,&record.scan_id,1).unwrap().unwrap(),initial_plan,"review must not rewrite the frozen analyzer-stage plan");
        let mut merged=analysis.clone();
        merge_native_source_assessments(&mut merged,report.clone());
        assert_eq!(merged["sourceClaims"],analysis["sourceClaims"],"greybox suspicions are not replaced by confirmed findings");
        if !report["sourceDecisionProjection"].is_null() {
            assert_eq!(merged["sourceFindings"],report["sourceDecisionProjection"]["findings"]);
            assert_eq!(merged["analyzerGaps"],analysis["gaps"]);
            assert!(!merged["gaps"].as_array().unwrap().contains(&json!("source_review_not_completed")));
            assert_eq!(merged["gaps"].as_array().unwrap().contains(&json!("source_coverage_review")),schema<4);
        }
        if !report["gate"].is_null() {
            assert_eq!(merged["analyzerGate"],analysis["gate"]);
            assert_eq!(merged["gate"],report["gate"]);
        }
        if report["sourceCoverageDecision"].is_object() {
            assert_eq!(merged["analyzerGaps"],analysis["gaps"],"coverage review must preserve the analyzer-stage history even without candidates");
            assert_eq!(merged["sourceCoverageDecision"],report["sourceCoverageDecision"]);
            assert_eq!(merged["independentReviewCompleted"],true);
            // The same production merge is used outside CI. A missing CI gate
            // must not reintroduce preparation placeholders or hide real gaps.
            let mut without_gate=report.clone();
            without_gate["gate"]=JsonValue::Null;
            let mut non_ci=analysis.clone();
            non_ci.as_object_mut().unwrap().remove("gate");
            merge_native_source_assessments(&mut non_ci,without_gate);
            assert_eq!(non_ci["analyzerGaps"],analysis["gaps"]);
            assert_eq!(non_ci["sourceCoverageDecision"],report["sourceCoverageDecision"]);
            assert_eq!(non_ci["independentReviewCompleted"],true);
            assert_eq!(non_ci["gaps"],report["sourceCoverageDecision"]["decision"]["outstandingGaps"]);
            assert!(non_ci["gate"].is_null());
        }
    }
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
    assert_eq!(seen.lock().unwrap().len(),expected_calls,"actual HTTP calls must match the checkpoint contract; result={result:?}");
    let (epoch,fence)=connection.query_row("SELECT lease_epoch,fencing_token FROM agent_coordinator_leases WHERE root_run_id=?1",
        [&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    lease.lease_epoch=epoch;lease.fencing_token=fence;
    (root,connection,lease,result)
}

// Stop at real committed stage boundaries, without resetting terminal rows or
// fabricating receipts. The production top-level reentry opens a new connection.
fn source_reviewer_checkpoint_fixture(
    root:&Path,connection:&rusqlite::Connection,record:&WorkbenchStartRecord,
    lease:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,checkpoint:&str,
) {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler,source_reviewer}};
    let database=root.join("oviraptor.sqlite3");
    let work=root.join("attempt-0001");
    let (model,runtime,_)=verify_source_runtime_contract(connection,&record.scan_id,1,&work).unwrap();
    let proxy=source_runtime_proxy(&runtime).unwrap();
    let profile=agent_model_profile(&model,proxy).unwrap();
    let context=SpecialistTransportContext { supervision: None,db_path:&database,scan_id:&record.scan_id,attempt_number:1,
        target_key:&lease.target_key,run_id:&lease.root_run_id,environment:&model,proxy,usage_dir:&work,
        deadline:Some(std::time::Instant::now()+Duration::from_secs(180))};
    connection.execute("UPDATE agent_runs SET status='running',started_at=datetime('now','localtime') WHERE id=?1",[&lease.root_run_id]).unwrap();
    for (index,role) in [AgentRole::RepoMapper,AgentRole::SourceAnalyst].into_iter().enumerate() {
        let input=source::assessment_input(connection,lease,role).unwrap();
        let slice=source::task_slice(connection,lease,role).unwrap();
        let (tokens,_)=source_assessment_budget(&source_assessment_messages(SOURCE_ASSESSMENT_SYSTEM,&input),&profile).unwrap();
        let child=scheduler::prepare_readonly_child(connection,lease,role,"source_results_ready",&slice,tokens).unwrap();
        if checkpoint=="first_assessment_unknown" && index==0 {
            connection.execute_batch("CREATE TRIGGER first_assessment_receipt_failure BEFORE UPDATE OF state ON agent_specialist_calls
                WHEN NEW.state='received' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
            assert!(specialist_round_transport(&context,lease,&child,SOURCE_ASSESSMENT_SYSTEM,input).is_err());
            connection.execute_batch("DROP TRIGGER first_assessment_receipt_failure").unwrap();
            return;
        }
        let (text,usage)=specialist_round_transport(&context,lease,&child,SOURCE_ASSESSMENT_SYSTEM,input).unwrap();
        if (checkpoint=="first_assessment_received" && index==0)
            || (checkpoint=="second_assessment_paused_received" && index==1) {
            if index==1 {stop_failed_child_preserving_usage(connection,lease,&child,"saved assessment before local delivery").unwrap();}
            return;
        }
        deliver_readonly_assessment(connection,lease,&child,&usage,&json!({"sourceTask":slice,"summary":text}),None).unwrap();
        if checkpoint=="after_first_assessment" && index==0 {return;}
    }
    if checkpoint=="after_initial_assessments" {return;}
    if matches!(checkpoint,"after_first_tool_phase"|"first_tool_unknown"|"first_tool_received"|"first_tool_finish_received"|"first_tool_finish_settlement_failed"|"second_tool_received") {
        use crate::agent_runtime::contract::AgentLane;
        crate::agent_runtime::multi_agent::directive::source_guidance::freeze(connection,lease,AgentRole::RepoMapper,true).unwrap();
        let revision:i64=connection.query_row("SELECT COALESCE(MAX(revision),1) FROM agent_evidence_revisions WHERE root_run_id=?1",
            [&lease.root_run_id],|r|r.get(0)).unwrap();
        let slice=source::tool_task_slice(connection,lease,AgentRole::RepoMapper,revision).unwrap();
        let remaining:i64=connection.query_row("SELECT total_tokens-spent_tokens-reserved_tokens FROM agent_budget_ledger WHERE root_run_id=?1",
            [&lease.root_run_id],|r|r.get(0)).unwrap();
        let review_share=i64::from(source_reviewer::enabled(connection,lease).unwrap())
            +i64::from(crate::agent_runtime::multi_agent::source_coverage_reviewer::enabled(connection,lease).unwrap());
        let child=scheduler::schedule_child(connection,lease,AgentRole::RepoMapper,AgentLane::ReadOnlyAnalysis,
            "source_tools_ready",&slice,revision,&source::tool_capabilities(AgentRole::RepoMapper).unwrap(),
            remaining/(2+review_share),3).unwrap();
        scheduler::mark_child_running(connection,lease,&child).unwrap();
        if checkpoint=="first_tool_unknown" {
            connection.execute_batch("CREATE TRIGGER first_tool_receipt_failure BEFORE UPDATE OF state ON agent_source_model_rounds
                WHEN NEW.state='received' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
            assert!(execute_source_tool_assignment(connection,&context,lease,&child,&slice).is_err());
            connection.execute_batch("DROP TRIGGER first_tool_receipt_failure").unwrap();
            assert_eq!(connection.query_row("SELECT state FROM agent_source_model_rounds WHERE assignment_id=?1",
                [&child.assignment_id],|r|r.get::<_,String>(0)).unwrap(),"executing");
        } else if matches!(checkpoint,"first_tool_received"|"first_tool_finish_received") {
            let trigger=if checkpoint=="first_tool_finish_received" {
                "CREATE TRIGGER first_tool_local_delivery_failure BEFORE UPDATE OF state ON agent_source_tool_receipts
                    WHEN NEW.state='completed' AND NEW.round_number=2 BEGIN SELECT RAISE(IGNORE); END;"
            } else {
                "CREATE TRIGGER first_tool_local_delivery_failure BEFORE UPDATE OF state ON agent_source_tool_receipts
                    WHEN NEW.state='completed' BEGIN SELECT RAISE(IGNORE); END;"
            };
            connection.execute_batch(trigger).unwrap();
            assert!(execute_source_tool_assignment(connection,&context,lease,&child,&slice).is_err());
            connection.execute_batch("DROP TRIGGER first_tool_local_delivery_failure").unwrap();
            assert_eq!(connection.query_row("SELECT state FROM agent_source_model_rounds WHERE assignment_id=?1",
                [&child.assignment_id],|r|r.get::<_,String>(0)).unwrap(),"received");
            assert_eq!(connection.query_row("SELECT state FROM agent_source_tool_receipts WHERE assignment_id=?1 ORDER BY round_number DESC LIMIT 1",
                [&child.assignment_id],|r|r.get::<_,String>(0)).unwrap(),"planned");
        } else if checkpoint=="first_tool_finish_settlement_failed" {
            connection.execute_batch("CREATE TRIGGER source_tool_mailbox_failure BEFORE INSERT ON agent_messages
                WHEN NEW.kind='source_tool_result' BEGIN SELECT RAISE(ABORT,'mailbox failure'); END;").unwrap();
            assert!(execute_source_tool_assignment(connection,&context,lease,&child,&slice).is_err());
            connection.execute_batch("DROP TRIGGER source_tool_mailbox_failure").unwrap();
            assert_eq!(connection.query_row("SELECT state FROM agent_source_tool_receipts WHERE assignment_id=?1 ORDER BY round_number DESC LIMIT 1",
                [&child.assignment_id],|r|r.get::<_,String>(0)).unwrap(),"completed");
            assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1",
                [&child.assignment_id],|r|r.get::<_,i64>(0)).unwrap(),0);
        } else {
            execute_source_tool_assignment(connection,&context,lease,&child,&slice).unwrap();
        }
        if checkpoint=="second_tool_received" {
            crate::agent_runtime::multi_agent::directive::source_guidance::freeze(connection,lease,AgentRole::SourceAnalyst,true).unwrap();
            let revision:i64=connection.query_row("SELECT COALESCE(MAX(revision),1) FROM agent_evidence_revisions WHERE root_run_id=?1",
                [&lease.root_run_id],|r|r.get(0)).unwrap();
            let slice=source::tool_task_slice(connection,lease,AgentRole::SourceAnalyst,revision).unwrap();
            let remaining:i64=connection.query_row("SELECT total_tokens-spent_tokens-reserved_tokens FROM agent_budget_ledger WHERE root_run_id=?1",
                [&lease.root_run_id],|r|r.get(0)).unwrap();
            let child=scheduler::schedule_child(connection,lease,AgentRole::SourceAnalyst,AgentLane::ReadOnlyAnalysis,
                "source_tools_ready",&slice,revision,&source::tool_capabilities(AgentRole::SourceAnalyst).unwrap(),
                remaining/(2+review_share-1),3).unwrap();
            scheduler::mark_child_running(connection,lease,&child).unwrap();
            connection.execute_batch("CREATE TRIGGER second_tool_local_delivery_failure BEFORE UPDATE OF state ON agent_source_tool_receipts
                WHEN NEW.state='completed' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
            assert!(execute_source_tool_assignment(connection,&context,lease,&child,&slice).is_err());
            connection.execute_batch("DROP TRIGGER second_tool_local_delivery_failure").unwrap();
            assert_eq!(connection.query_row("SELECT state FROM agent_source_model_rounds WHERE assignment_id=?1",
                [&child.assignment_id],|r|r.get::<_,String>(0)).unwrap(),"received");
        }
        return;
    }
    run_source_tool_phases(connection,&context,lease).unwrap();
    if checkpoint=="before_review" {return;}
    let tx=connection.unchecked_transaction().unwrap();
    let slice=source_reviewer::task_slice(&tx,lease).unwrap().unwrap();
    tx.rollback().unwrap();
    let (tokens,_)=source_assessment_budget(&source_assessment_messages(source_reviewer::SYSTEM,&slice),&profile).unwrap();
    let child=source_reviewer::prepare(connection,lease,&slice,tokens).unwrap();
    if checkpoint=="undispatched" {return;}
    if checkpoint=="dispatch_revoked" {
        for mutation in [
            "UPDATE agent_capability_leases SET revoked_at='revoked' WHERE assignment_id=NEW.assignment_id AND capability='review.write';",
            "UPDATE agent_capability_leases SET fencing_token='different' WHERE assignment_id=NEW.assignment_id AND capability='review.write';",
            "INSERT INTO agent_capability_leases(id,root_run_id,assignment_id,child_run_id,capability,lease_epoch,fencing_token,lease_expires_at)
                SELECT 'unexpected-http',root_run_id,assignment_id,child_run_id,'http.request',lease_epoch,fencing_token,lease_expires_at
                FROM agent_capability_leases WHERE assignment_id=NEW.assignment_id AND capability='review.write';",
        ] {
            connection.execute_batch(&format!("CREATE TRIGGER checkpoint_dispatch_failure AFTER INSERT ON agent_specialist_calls
                WHEN NEW.role='evidence_reviewer' BEGIN {mutation} END;")).unwrap();
            let before=source_reviewer_reentry_state(connection);
            let error=specialist_round_transport(&context,lease,&child,source_reviewer::SYSTEM,slice.clone()).unwrap_err();
            assert!(error.contains("source_review_dispatch_capabilities_invalid"),"{error}");
            assert_eq!(source_reviewer_reentry_state(connection),before,"dispatch claim must roll back permission drift before HTTP");
            connection.execute_batch("DROP TRIGGER checkpoint_dispatch_failure").unwrap();
        }
        return;
    }
    if checkpoint=="unknown" {
        // A real HTTP response is received, but its atomic receipt transaction
        // fails. Do not fabricate a journal or repair it into a received result.
        connection.execute_batch("CREATE TRIGGER checkpoint_receipt_failure BEFORE UPDATE OF state ON agent_specialist_calls
            WHEN NEW.state='received' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        let error=specialist_round_transport(&context,lease,&child,source_reviewer::SYSTEM,slice).unwrap_err();
        assert!(error.contains("specialist_response_persist_missing"),"{error}");
        connection.execute_batch("DROP TRIGGER checkpoint_receipt_failure").unwrap();
        assert_eq!(connection.query_row("SELECT state FROM agent_specialist_calls WHERE assignment_id=?1",
            [&child.assignment_id],|r|r.get::<_,String>(0)).unwrap(),"executing");
        return;
    }
    specialist_round_transport(&context,lease,&child,source_reviewer::SYSTEM,slice).unwrap();
    if checkpoint=="paused_received" {
        stop_failed_child_preserving_usage(connection,lease,&child,"simulated interruption after saved response").unwrap();
    } else if checkpoint=="delivered" {
        deliver_source_candidate_review(connection,&context,lease,&child).unwrap();
    } else {assert_eq!(checkpoint,"received");}
}
