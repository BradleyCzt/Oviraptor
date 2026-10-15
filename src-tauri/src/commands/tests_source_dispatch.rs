fn source_dispatch_fixture(model: &ModelRuntimeEnv, dollar_limit: Option<f64>) -> (PathBuf,rusqlite::Connection,WorkbenchStartRecord) {
    source_dispatch_fixture_configure(model,dollar_limit,|_,_|{})
}

fn source_dispatch_fixture_configure(model: &ModelRuntimeEnv, dollar_limit: Option<f64>, configure:impl FnOnce(&rusqlite::Connection,&mut WorkbenchStartRecord)) -> (PathBuf,rusqlite::Connection,WorkbenchStartRecord) {
    analysis_view_fixture_configure("full",true,|connection,record| {
        connection.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles',json(?1),'$.activeModelProfileId','source-fixture') WHERE id=(SELECT id FROM config_profiles ORDER BY is_default DESC,id LIMIT 1)",
            [json!([{"id":"source-fixture","llm":model.llm,"apiKey":model.api_key,"apiBase":model.api_base,"deployment":model.deployment}]).to_string()]).unwrap();
        record.llm_policy=source_model_policy(model);
        record.policy["maxBudgetUsd"]=json!(dollar_limit);
        configure(connection,record);
    })
}

#[test]
fn source_dispatch_production_entry_runs_real_coverage_review_without_hiding_gaps() {
    let (port,seen,stop)=crate::scanning::agent_tests::spawn_endpoint(std::sync::Arc::new(|request| {
        let body:JsonValue=serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["max_tokens"],2048);
        let input:JsonValue=body["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_str().and_then(|text|serde_json::from_str(text).ok()).unwrap_or(JsonValue::Null);
        let message=if input["phase"]=="source_coverage_review" {
            assert!(body.get("tools").is_none());
            json!({"role":"assistant","content":source_coverage_fixture_response(&input).to_string()})
        } else if body.get("tools").is_none() {
            json!({"role":"assistant","content":"{\"summary\":\"initial assessment\"}"})
        } else {
            let finished=body["messages"].as_array().unwrap().last().unwrap()["role"]=="tool";
            let (name,args)=if finished {("assignment.finish",json!({"summary":"Inventory inspected; independent review still required","gaps":[]}))}
                else {("repo.inventory",json!({}))};
            json!({"role":"assistant","content":"Inspect scoped repository","tool_calls":[{"id":name,"type":"function","function":{"name":name,"arguments":args.to_string()}}]})
        };
        (200,"application/json",json!({"choices":[{"message":message,"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
    }));
    let environment=source_specialist_test_environment(port);
    let (root,connection,record)=source_dispatch_fixture(&environment,None);
    // Real production entry; missing pinned analyzers remain truthful gaps.
    let report=run_native_source_scan(&root.join("oviraptor.sqlite3"),&root,&record.scan_id,1,&root.join("attempt-0001"),
        &record.source_path,&record.scan_type,&record.diff_base).unwrap();
    assert_eq!(report["sourceMultiAgent"]["status"],"source_coverage_reviewed","{report}");
    assert_eq!(report["sourceMultiAgent"]["independentReviewCompleted"],true);
    assert_eq!(report["sourceMultiAgent"]["independentCandidateReviewCompleted"],false,"zero candidates must not invent a candidate Reviewer");
    assert_eq!(report["sourceMultiAgent"]["sourceCoverageDecision"]["decision"]["coverageSufficient"],false);
    assert!(!report["gaps"].as_array().unwrap().is_empty(),"missing pinned analyzers cannot become sufficient coverage");
    assert!(!report["gaps"].as_array().unwrap().contains(&json!("source_review_not_completed")));
    assert_ne!(report["gate"]["status"],"passed");
    let root_id=report["sourceMultiAgent"]["rootRunId"].as_str().unwrap();
    let state:(String,String,i64,i64)=connection.query_row("SELECT status,terminal_state,(SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND state='completed' AND budget_settled_at<>''),(SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND acknowledged_at<>'') FROM agent_runs WHERE id=?1",
        [root_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(state,("terminal".into(),"completed_with_gaps".into(),5,5));
    let plan:String=connection.query_row("SELECT plan_json FROM agent_runs WHERE id=?1",[root_id],|r|r.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<JsonValue>(&plan).unwrap()["schemaVersion"],4);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_coverage_decisions WHERE root_run_id=?1",[root_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND trigger_code='source_candidates_ready'",[root_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_production_source_execution_history(&connection,&record.scan_id);
    let usage:(i64,i64,i64,i64)=connection.query_row("SELECT spent_tokens,spent_requests,reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",[root_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(usage,(140,7,0,0));
    assert_eq!(report["sourceMultiAgent"]["verifiedToolResults"],2,"finish markers are not data tools");
    assert_eq!(report["sourceMultiAgent"]["modelRequests"],7);
    assert_eq!(report["sourceMultiAgent"]["totalTokens"],140);
    let root_usage:(i64,i64)=connection.query_row("SELECT used_tokens,used_requests FROM agent_runs WHERE id=?1",[root_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(root_usage,(0,0),"root did not make its own model calls");
    let (trace,trace_events)=collect_native_agent_trace(&connection,&record.scan_id,true,true).unwrap();
    assert_eq!((trace.total_tokens,trace.llm_requests),(140,7),"child costs must not be double-counted at the root");
    assert_eq!((trace.tool_call_count,trace.tool_result_count),(4,4),"atomic source completions are actual calls; finish remains a trace tool, not a finding");
    assert_eq!(trace.tools.len(),2);
    for name in ["repo.inventory","assignment.finish"] {
        let tool=trace.tools.iter().find(|tool|tool.name==name).unwrap();
        assert_eq!((tool.calls,tool.results),(2,2));
    }
    let tool_events=trace_events.iter().filter(|event|event.event_type=="tool_invocation_completed").collect::<Vec<_>>();
    assert_eq!(tool_events.len(),4);
    assert_eq!(tool_events.iter().map(|event|&event.call_id).collect::<HashSet<_>>().len(),4,"model call IDs may repeat but trace invocation identities must not");
    assert!(tool_events.iter().all(|event| !event.name.is_empty() && event.call_id.starts_with("source:")));
    let messages=trace_events.iter().filter(|event|event.name=="mailbox").collect::<Vec<_>>();
    assert_eq!(messages.len(),5);
    assert_eq!(messages.iter().filter(|message|message.role=="evidence_reviewer").count(),1);
    for message in messages {
        assert!(matches!(message.role.as_str(),"repo_mapper"|"source_analyst"|"evidence_reviewer"),"result sender is not the coordinator: {}",message.role);
        assert_ne!(message.session_id,root_id);
        assert_eq!(message.status,"acknowledged");
        let sender_role:String=connection.query_row("SELECT role FROM agent_runs WHERE id=?1",[&message.session_id],|row|row.get(0)).unwrap();
        assert_eq!(sender_role,message.role);
    }
    let (summary_only,no_events)=collect_native_agent_trace(&connection,&record.scan_id,false,true).unwrap();
    assert!(no_events.is_empty());
    assert_eq!((summary_only.tool_call_count,summary_only.tool_result_count),(4,4));
    connection.execute_batch("SAVEPOINT trace_web_compatibility").unwrap();
    for kind in [crate::agent_runtime::contract::AgentEventKind::ToolInvocationStarted,crate::agent_runtime::contract::AgentEventKind::ToolInvocationCompleted] {
        crate::agent_runtime::store::append_event(&connection,root_id,kind,
            &json!({"tool":"http_request","invocationId":"existing-web-id"}),&[]).unwrap();
    }
    let (with_web,events)=collect_native_agent_trace(&connection,&record.scan_id,true,true).unwrap();
    assert_eq!((with_web.tool_call_count,with_web.tool_result_count),(5,5),"paired Web events must count once, not once per event");
    let web=with_web.tools.iter().find(|tool|tool.name=="http_request").unwrap();
    assert_eq!((web.calls,web.results),(1,1));
    assert_eq!(events.iter().filter(|event|event.call_id=="existing-web-id" && event.name=="http_request").count(),2);
    connection.execute_batch("ROLLBACK TO trace_web_compatibility; RELEASE trace_web_compatibility").unwrap();
    for mutation in [
        "UPDATE agent_messages SET from_run_id='missing-sender'",
        "UPDATE agent_messages SET from_agent='coordinator'",
        "UPDATE agent_messages SET root_run_id='unrelated-root'",
        "UPDATE agent_runs SET attempt_number=2 WHERE role IN ('repo_mapper','source_analyst','evidence_reviewer')",
        "UPDATE agent_runs SET target_url='unrelated-target' WHERE role IN ('repo_mapper','source_analyst','evidence_reviewer')",
    ] {
        connection.execute_batch("SAVEPOINT trace_sender").unwrap();
        connection.execute_batch(mutation).unwrap();
        let (_,events)=collect_native_agent_trace(&connection,&record.scan_id,true,false).unwrap();
        let messages=events.iter().filter(|event|event.name=="mailbox").collect::<Vec<_>>();
        assert_eq!(messages.len(),5);
        assert!(messages.iter().all(|event|event.role=="unknown" && event.session_id==root_id),"{mutation}");
        connection.execute_batch("ROLLBACK TO trace_sender; RELEASE trace_sender").unwrap();
    }
    connection.execute_batch("SAVEPOINT trace_legacy_sender; UPDATE agent_messages SET from_run_id=''").unwrap();
    let (_,legacy_events)=collect_native_agent_trace(&connection,&record.scan_id,true,true).unwrap();
    assert!(legacy_events.iter().filter(|event|event.name=="mailbox")
        .all(|event|matches!(event.role.as_str(),"repo_mapper"|"source_analyst"|"evidence_reviewer") && event.session_id==root_id),"legacy messages keep their recorded sender label without inventing a child identity");
    connection.execute_batch("ROLLBACK TO trace_legacy_sender; RELEASE trace_legacy_sender").unwrap();
    let closure:String=connection.query_row("SELECT payload_json FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",[root_id],|r|r.get(0)).unwrap();
    let closure:JsonValue=serde_json::from_str(&closure).unwrap();
    assert_eq!(closure["sourceClosureVersion"],4);
    assert_eq!(closure["assignments"].as_array().unwrap().len(),5);
    assert_eq!(closure["independentReviewCompleted"],true);
    assert_eq!(closure["independentCandidateReviewCompleted"],false);
    assert_eq!(seen.lock().unwrap().len(),7);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_model_rounds WHERE state='received'",[],|r|r.get::<_,i64>(0)).unwrap(),4);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_tool_receipts WHERE state='completed'",[],|r|r.get::<_,i64>(0)).unwrap(),4);
    assert!(run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001")).is_err());
    assert_eq!(seen.lock().unwrap().len(),7,"terminal roots never mint another budget");
    // Audit the same committed production receipts without reactivating work.
    // Every mutation is rolled back and must be detected, even when all four
    // assignments are still labelled completed and settled.
    let lease=crate::agent_runtime::multi_agent::lease::CoordinatorLease {
        scan_id:record.scan_id.clone(),attempt_number:1,target_key:connection.query_row("SELECT target_url FROM agent_runs WHERE id=?1",[root_id],|r|r.get(0)).unwrap(),
        root_run_id:root_id.into(),lease_epoch:connection.query_row("SELECT lease_epoch FROM agent_coordinator_leases",[],|r|r.get(0)).unwrap(),
        fencing_token:connection.query_row("SELECT fencing_token FROM agent_coordinator_leases",[],|r|r.get(0)).unwrap(),lease_expires_at:String::new()};
    let bases=[crate::agent_runtime::contract::AgentRole::RepoMapper,crate::agent_runtime::contract::AgentRole::SourceAnalyst].into_iter().map(|role| {
        let text:String=connection.query_row("SELECT task_slice_json FROM agent_assignments WHERE role=?1 AND json_extract(task_slice_json,'$.phase') IS NULL",[role.as_str()],|r|r.get(0)).unwrap();
        (role,serde_json::from_str(&text).unwrap())
    }).collect::<Vec<_>>();
    for (label,mutation) in [
        ("unchanged","SELECT 1;"),
        ("duplicate_role","UPDATE agent_assignments SET role='repo_mapper' WHERE role='source_analyst';"),
        ("child_parent","UPDATE agent_runs SET parent_run_id=NULL WHERE role='repo_mapper';"),
        ("phase_changed","UPDATE agent_assignments SET task_slice_json=json_remove(task_slice_json,'$.phase') WHERE json_extract(task_slice_json,'$.phase')='source_tools';"),
        ("wrong_revision","UPDATE agent_assignments SET evidence_revision=99 WHERE json_extract(task_slice_json,'$.phase')='source_tools';"),
        ("unacknowledged","UPDATE agent_messages SET acknowledged_at='' WHERE kind='evidence_summary';"),
        ("mailbox_route","UPDATE agent_messages SET to_run_id=from_run_id WHERE kind='source_tool_result';"),
        ("mailbox_payload","UPDATE agent_messages SET payload_json='{}' WHERE kind='source_tool_result';"),
        ("mailbox_redelivery","UPDATE agent_messages SET delivery_attempts=2;"),
        ("ledger_drift","UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1;"),
        ("child_usage","UPDATE agent_runs SET used_tokens=used_tokens+1 WHERE role='repo_mapper';"),
        ("missing_assessment_event","DELETE FROM agent_events WHERE event_type='model_round_completed' AND run_id IN (SELECT child_run_id FROM agent_assignments WHERE json_extract(task_slice_json,'$.phase') IS NULL);"),
        ("missing_early_tool_receipt","DELETE FROM agent_source_tool_receipts WHERE round_number=1;"),
        ("missing_early_tool_event","DELETE FROM agent_events WHERE event_type='tool_invocation_completed' AND json_extract(payload_json,'$.call.name')='repo.inventory';"),
        ("missing_model_round","DELETE FROM agent_source_model_rounds WHERE round_number=1;"),
        ("live_capability","UPDATE agent_capability_leases SET revoked_at='';"),
    ] {
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
        tx.execute_batch(mutation).unwrap();
        let proof=source_assessment_completion(&tx,&lease,&bases);
        assert_eq!(proof.is_ok(),label=="unchanged","{label}: {proof:?}");
        drop(tx);
    }
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_source_trace_identity_requires_scoped_round_coordinates() {
    let value=json!({"assignmentId":"assignment","sourceRound":1,"callIndex":0,"call":{"id":"reused-model-id","name":"repo.inventory"}});
    assert_eq!(native_source_trace_tool(&value),Some(("repo.inventory".into(),"source:assignment:1:0".into())));
    for (field,invalid) in [("assignmentId",json!("")),("sourceRound",json!(0)),("callIndex",json!(-1)),("sourceRound",json!("1")),("call",json!({"name":""}))] {
        let mut damaged=value.clone();damaged[field]=invalid;
        assert!(native_source_trace_tool(&damaged).is_none(),"{damaged}");
    }
    let mut later=value.clone();later["sourceRound"]=json!(2);
    assert_ne!(native_source_trace_tool(&value),native_source_trace_tool(&later));
    assert!(native_source_trace_tool(&json!({"tool":"http_request","invocationId":"web-call"})).is_none());
}

#[test]
fn source_dispatch_closure_rolls_back_post_write_damage_after_real_coverage_review() {
    for (label,body) in [
        ("ignored_terminal","SELECT RAISE(IGNORE);"),
        ("mailbox_ack","UPDATE agent_messages SET acknowledged_at='';"),
        ("mailbox_payload","UPDATE agent_messages SET payload_json='{}';"),
        ("ledger","UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1;"),
        ("child_usage","UPDATE agent_runs SET used_tokens=used_tokens+1 WHERE role='repo_mapper';"),
        ("root_cancel","UPDATE agent_runs SET cancel_requested_at='cancelled' WHERE id=NEW.id;"),
        ("attempt_stop","UPDATE sentinel_scans SET status='paused' WHERE id=NEW.scan_id;"),
        ("missing_model_event","DELETE FROM agent_events WHERE event_type='model_round_completed';"),
        ("runtime_change","UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles[0].apiKey','changed');"),
        ("root_budget","UPDATE agent_runs SET hard_token_budget=hard_token_budget+1 WHERE id=NEW.id; UPDATE agent_budget_ledger SET total_tokens=total_tokens+1;"),
        ("root_binding","UPDATE agent_runs SET evidence_hash='changed' WHERE id=NEW.id;"),
    ] {
        let (port,seen,stop)=crate::scanning::agent_tests::spawn_endpoint(std::sync::Arc::new(|request| {
            let body:JsonValue=serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            let input:JsonValue=body["messages"].as_array().unwrap().last().unwrap()["content"]
                .as_str().and_then(|text|serde_json::from_str(text).ok()).unwrap_or(JsonValue::Null);
            let message=if input["phase"]=="source_coverage_review" {json!({"role":"assistant","content":source_coverage_fixture_response(&input).to_string()})}
                else if body.get("tools").is_none() {json!({"role":"assistant","content":"initial assessment"})}
                else {
                    let finished=body["messages"].as_array().unwrap().last().unwrap()["role"]=="tool";
                    let (name,args)=if finished {("assignment.finish",json!({"summary":"Actual inventory inspected; independent coverage remains gapped","gaps":[]}))} else {("repo.inventory",json!({}))};
                    json!({"role":"assistant","tool_calls":[{"id":name,"type":"function","function":{"name":name,"arguments":args.to_string()}}]})
                };
            (200,"application/json",json!({"choices":[{"message":message,"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
        }));
        let (root,connection,record)=source_dispatch_fixture(&source_specialist_test_environment(port),None);
        connection.execute_batch(&format!("CREATE TRIGGER source_closure_damage {} UPDATE ON agent_runs WHEN NEW.role='coordinator' AND NEW.terminal_state='completed_with_gaps' AND OLD.status<>'terminal' BEGIN {body} END;",if label=="ignored_terminal" {"BEFORE"} else {"AFTER"})).unwrap();
        let report=run_native_source_scan(&root.join("oviraptor.sqlite3"),&root,&record.scan_id,1,&root.join("attempt-0001"),&record.source_path,&record.scan_type,&record.diff_base).unwrap();
        assert_eq!(report["sourceMultiAgent"]["status"],"incomplete","{label}: {report}");
        assert_ne!(report["gate"]["status"],"passed","{label}");
        assert_eq!(seen.lock().unwrap().len(),7,"{label}: no repeated model calls");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs WHERE role='coordinator' AND terminal_state='completed_with_gaps'",[],|r|r.get::<_,i64>(0)).unwrap(),0,"{label}");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_events WHERE event_type='terminal_reduced' AND json_extract(payload_json,'$.sourceClosureVersion')=4",[],|r|r.get::<_,i64>(0)).unwrap(),0,"{label}");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE acknowledged_at<>''",[],|r|r.get::<_,i64>(0)).unwrap(),5,"{label}: damage rolled back");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_coverage_decisions",[],|r|r.get::<_,i64>(0)).unwrap(),1,"{label}: real coverage delivery survives failed root closure");
        assert_eq!(connection.query_row("SELECT spent_tokens FROM agent_budget_ledger",[],|r|r.get::<_,i64>(0)).unwrap(),140,"{label}");
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_tool_dispatch_cancel_checks_tool_contract_not_assessment_grants() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{source,scheduler}};
    for mutation in ["live","cancel_child","cancel_root","pause_assignment","revoke_capability","expire_assignment","lost_lane","runtime_changed","deadline"] {
        let environment=source_specialist_test_environment(9);
        let (root,connection,_record,lease)=source_tool_true_born_fixture_model(Some(&environment));
        connection.execute("UPDATE agent_runs SET status='running',started_at=datetime('now','localtime') WHERE id=?1",[&lease.root_run_id]).unwrap();
        let slice=source::tool_task_slice(&connection,&lease,AgentRole::RepoMapper,1).unwrap();
        let child=scheduler::schedule_child(&connection,&lease,AgentRole::RepoMapper,AgentLane::ReadOnlyAnalysis,"source_tools_ready",
            &slice,1,&source::tool_capabilities(AgentRole::RepoMapper).unwrap(),24_000,3).unwrap();
        scheduler::mark_child_running(&connection,&lease,&child).unwrap();
        let database=root.join("oviraptor.sqlite3");let work=root.join("attempt-0001");
        let context=SpecialistTransportContext { supervision: None,db_path:&database,scan_id:&lease.scan_id,attempt_number:1,target_key:&lease.target_key,
            run_id:&lease.root_run_id,environment:&environment,proxy:None,usage_dir:&work,
            deadline:Some(std::time::Instant::now()+Duration::from_secs(if mutation=="deadline" {0} else {60}))};
        match mutation {
            "cancel_child"=>{connection.execute("UPDATE agent_runs SET cancel_requested_at='requested' WHERE id=?1",[&child.run_id]).unwrap();},
            "cancel_root"=>{connection.execute("UPDATE agent_runs SET cancel_requested_at='requested' WHERE id=?1",[&lease.root_run_id]).unwrap();},
            "pause_assignment"=>{connection.execute("UPDATE agent_assignments SET state='paused'",[]).unwrap();},
            "revoke_capability"=>{connection.execute("UPDATE agent_capability_leases SET revoked_at='revoked' WHERE capability='repo.inventory'",[]).unwrap();},
            "expire_assignment"=>{connection.execute("UPDATE agent_assignments SET lease_expires_at='2000-01-01'",[]).unwrap();},
            "lost_lane"=>{connection.execute("DELETE FROM agent_lane_leases",[]).unwrap();},
            "runtime_changed"=>{connection.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles[0].apiKey','changed')",[]).unwrap();},
            _=>{},
        }
        assert_eq!(source_tool_model_cancel_token(&context,&lease,&child).is_cancelled(),mutation!="live","{mutation}");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_model_rounds",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_tool_dispatch_failures_never_fake_finish_refund_or_retry() {
    for failure in ["model_http","text_only","finish_denied","mailbox_fault","settlement_fault"] {
        let (port,seen,stop)=crate::scanning::agent_tests::spawn_endpoint(std::sync::Arc::new(move |request| {
            let body:JsonValue=serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            let tools=body.get("tools").is_some();
            if tools && failure=="model_http" {return (500,"application/json","{}".into());}
            let message=if !tools || failure=="text_only" {json!({"role":"assistant","content":"summary is not a finish receipt"})}
                else {json!({"role":"assistant","tool_calls":[{"id":"finish","type":"function","function":{"name":"assignment.finish",
                    "arguments":json!({"summary":if failure=="finish_denied" {""} else {"scoped work finished"},"gaps":[]}).to_string()}}]})};
            (200,"application/json",json!({"choices":[{"message":message,"finish_reason":"stop"}],
                "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
        }));
        let environment=source_specialist_test_environment(port);
        let (root,connection,record)=source_dispatch_fixture(&environment,None);
        if failure=="mailbox_fault" {
            connection.execute_batch("CREATE TRIGGER source_mailbox_fault BEFORE INSERT ON agent_messages WHEN NEW.kind='source_tool_result' BEGIN SELECT RAISE(ABORT,'mailbox fault'); END;").unwrap();
        }
        if failure=="settlement_fault" {
            connection.execute_batch("CREATE TRIGGER source_settlement_fault BEFORE UPDATE ON agent_assignments WHEN json_extract(NEW.task_slice_json,'$.phase')='source_tools' AND NEW.budget_settled_at<>'' BEGIN SELECT RAISE(ABORT,'settlement fault'); END;").unwrap();
        }
        let report=run_native_source_scan(&root.join("oviraptor.sqlite3"),&root,&record.scan_id,1,&root.join("attempt-0001"),
            &record.source_path,&record.scan_type,&record.diff_base).unwrap();
        assert_eq!(report["sourceMultiAgent"]["status"],"incomplete","{failure}: {report}");
        assert!(report["gaps"].as_array().unwrap().contains(&json!("source_multi_agent_execution_incomplete")));
        assert_eq!(seen.lock().unwrap().len(),3,"{failure}: never retry uncertain model IO");
        let state:(String,String,i64,i64)=connection.query_row("SELECT state,budget_settled_at,reserved_tokens,reserved_requests FROM agent_assignments WHERE json_extract(task_slice_json,'$.phase')='source_tools'",[],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(state.0,"paused","{failure}");assert!(state.1.is_empty());assert!(state.2>0);assert_eq!(state.3,3);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE kind='source_tool_result'",[],|r|r.get::<_,i64>(0)).unwrap(),0,"{failure}");
        assert_eq!(connection.query_row("SELECT state FROM agent_source_model_rounds",[],|r|r.get::<_,String>(0)).unwrap(),if failure=="model_http" {"uncertain"} else {"received"});
        let ledger:(i64,i64,i64)=connection.query_row("SELECT spent_tokens,spent_requests,reserved_tokens FROM agent_budget_ledger",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!((ledger.0,ledger.1),(40,2));assert_eq!(ledger.2,state.2);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_tool_receipts WHERE state='completed'",[],|r|r.get::<_,i64>(0)).unwrap(),if ["model_http","text_only"].contains(&failure) {0} else {1});
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_dispatch_explicit_dollar_limit_does_not_create_unfunded_runs() {
    assert_eq!(source_model_cost_admission(true).unwrap_err(),"source_model_cost_accounting_unavailable");
    assert!(source_model_cost_admission(false).is_ok());
    let environment=source_specialist_test_environment(9);
    let (root,connection,record)=source_dispatch_fixture(&environment,Some(1.0));
    analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_result_outcome(engine,scratch,"app.py","cost admission"))).unwrap();
    assert_eq!(run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001")).unwrap_err(),"source_model_cost_accounting_unavailable");
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_dispatch_runtime_reservation_deadline_and_trigger_changes_never_reach_model() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler}};
    let (port,seen,stop)=crate::scanning::agent_tests::spawn_endpoint(std::sync::Arc::new(|_|(500,"application/json","{}".into())));
    for mutation in ["credential","endpoint","proxy","reservation","deadline","insert_revocation","root_budget"] {
        let mut environment=source_specialist_test_environment(port);
        let (root,connection,record,lease)=source_specialist_fixture_model(Some(&environment));
        let role=AgentRole::RepoMapper;
        let input=source::assessment_input(&connection,&lease,role).unwrap();
        let (tokens,_)=source_assessment_budget(&source_assessment_messages(SOURCE_ASSESSMENT_SYSTEM,&input),&agent_model_profile(&environment,None).unwrap()).unwrap();
        let slice=source::task_slice(&connection,&lease,role).unwrap();
        let child=scheduler::prepare_readonly_child(&connection,&lease,role,"source_results_ready",&slice,if mutation=="reservation" {1} else {tokens}).unwrap();
        match mutation {
            "credential"=>environment.api_key="unpublished-key".into(),
            "endpoint"=>environment.api_base="http://127.0.0.1:9/v1".into(),
            "insert_revocation"=>connection.execute_batch("CREATE TRIGGER source_dispatch_revocation AFTER INSERT ON agent_specialist_calls BEGIN UPDATE agent_runs SET cancel_requested_at='revoked' WHERE id=NEW.root_run_id; END;").unwrap(),
            "root_budget"=>{connection.execute("UPDATE agent_runs SET hard_token_budget=999999 WHERE id=?1",[&lease.root_run_id]).unwrap();},
            _=>{},
        }
        let db_path=root.join("oviraptor.sqlite3");
        let context=SpecialistTransportContext { supervision: None,db_path:&db_path,scan_id:&record.scan_id,attempt_number:1,target_key:&lease.target_key,run_id:&lease.root_run_id,
            environment:&environment,proxy:if mutation=="proxy" {Some("http://127.0.0.1:9")} else {None},usage_dir:&root,
            deadline:if mutation=="deadline" {Some(std::time::Instant::now())} else {None}};
        assert!(specialist_round_transport(&context,&lease,&child,SOURCE_ASSESSMENT_SYSTEM,input).is_err(),"{mutation}");
        assert_eq!(seen.lock().unwrap().len(),0,"{mutation}");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_specialist_calls",[],|r|r.get::<_,i64>(0)).unwrap(),0,"{mutation}");
        if mutation=="insert_revocation" {
            assert_eq!(connection.query_row("SELECT cancel_requested_at FROM agent_runs WHERE id=?1",[&lease.root_run_id],|r|r.get::<_,String>(0)).unwrap(),"","trigger side effects must roll back with claim");
        }
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn source_dispatch_provider_failure_preserves_unknown_usage_and_does_not_retry() {
    let (port,seen,stop)=crate::scanning::agent_tests::spawn_endpoint(std::sync::Arc::new(|_|(500,"application/json","{\"error\":\"unavailable\"}".into())));
    let environment=source_specialist_test_environment(port);
    let (root,connection,record,_)=source_specialist_true_born_model_fixture(&environment);
    let invoke=||run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
    assert!(invoke().is_err());
    assert_eq!(seen.lock().unwrap().len(),1);
    let row:(String,String,String,i64,i64)=connection.query_row("SELECT root.terminal_state,a.state,c.state,a.reserved_tokens,a.reserved_requests FROM agent_runs root JOIN agent_assignments a ON a.coordinator_run_id=root.id JOIN agent_specialist_calls c ON c.assignment_id=a.id",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!((&*row.0,&*row.1,&*row.2),("paused","paused","uncertain"));
    assert!(row.3>0);assert_eq!(row.4,1);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert!(invoke().is_err());assert_eq!(seen.lock().unwrap().len(),1);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_dispatch_invocation_owner_blocks_duplicate_without_mutating_root() {
    let (root,connection,record,lease)=source_specialist_fixture();
    let path=root.join("oviraptor.sqlite3");
    let owner=claim_native_invocation(&path,&record.scan_id,1,"source-model","source").unwrap();
    let before=source_coordinator_row(&connection,&lease.root_run_id);
    assert!(run_native_source_assessments(&path,&record.scan_id,1,&root.join("attempt-0001")).unwrap_err().starts_with("native_invocation_not_owned:"));
    assert_eq!(source_coordinator_row(&connection,&lease.root_run_id),before);
    drop(owner);drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_dispatch_output_admission_is_bounded_for_cloud_and_local() {
    let mut profile=agent_model_profile(&source_specialist_test_environment(9),None).unwrap();
    let messages=source_assessment_messages("read only",&json!({"source":"汉字🦖"}));
    let (tokens,output)=source_assessment_budget(&messages,&profile).unwrap();
    assert_eq!(output,2048);assert!(tokens>2048+512);
    profile.max_output_tokens=Some(100);
    let (local_tokens,output)=source_assessment_budget(&messages,&profile).unwrap();
    assert_eq!(output,100);assert_eq!(tokens-local_tokens,1948);
    profile.max_context_tokens=local_tokens as u64-1;
    assert_eq!(source_assessment_budget(&messages,&profile).unwrap_err(),"source_model_context_budget_exceeded");
    profile.max_output_tokens=Some(0);
    assert!(source_assessment_budget(&messages,&profile).is_err());
}

#[test]
fn source_dispatch_historical_recovery_cannot_mint_original_finance_from_mutable_started_at() {
    let (root,connection,record,lease)=source_specialist_fixture();
    assert_eq!(source_model_remaining_seconds(&connection,&lease.root_run_id,600).unwrap(),600);
    connection.execute("UPDATE agent_runs SET started_at=datetime('now','-120 seconds','localtime') WHERE id=?1",[&lease.root_run_id]).unwrap();
    let remaining=source_model_remaining_seconds(&connection,&lease.root_run_id,600).unwrap();
    assert!((479..=481).contains(&remaining));
    connection.execute("UPDATE agent_runs SET started_at='2000-01-01 00:00:00' WHERE id=?1",[&lease.root_run_id]).unwrap();
    assert_eq!(source_model_remaining_seconds(&connection,&lease.root_run_id,600).unwrap(),0);
    let before=crate::scanning::web_mode_test_rows(&connection);
    let error=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001")).unwrap_err();
    assert_eq!(error,"source_root_original_finance_missing");
    crate::scanning::web_mode_assert_rows(&connection,&before);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(connection.query_row("SELECT started_at FROM agent_runs WHERE id=?1",[&lease.root_run_id],|r|r.get::<_,String>(0)).unwrap(),"2000-01-01 00:00:00");
    drop(connection);fs::remove_dir_all(root).unwrap();
}
