fn source_round_fixture() -> (PathBuf,rusqlite::Connection,AgentRunContext,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,JsonValue) {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{source,scheduler},secrets::redact_json};
    let (root,connection,record,lease)=source_tool_true_born_fixture_model(None);
    connection.execute("UPDATE agent_runs SET status='running',started_at=datetime('now','localtime') WHERE id=?1",[&lease.root_run_id]).unwrap();
    let role=AgentRole::SourceAnalyst;
    let slice=source::tool_task_slice(&connection,&lease,role,1).unwrap();
    let child=scheduler::schedule_child(&connection,&lease,role,AgentLane::ReadOnlyAnalysis,"source_tools_ready",
        &slice,1,&source::tool_capabilities(role).unwrap(),24_000,3).unwrap();
    scheduler::mark_child_running(&connection,&lease,&child).unwrap();
    let mut context=crate::commands::agent_tests::test_context(&root.join("oviraptor.sqlite3"),&lease.target_key,Vec::new());
    context.scan_id=record.scan_id;context.attempt_number=1;
    context.run=Some(AgentRunLedger {db_path:context.db_path.clone(),run_id:child.run_id.clone()});
    context.execution_plan.schema_version=255;
    let tools=agent_tool_specs_for(&context).iter().map(|s|s.as_function_spec()).collect::<Vec<_>>();
    let request=redact_json(&json!({"messages":[{"role":"user","content":redact_json(&json!({"sourceTask":slice})).to_string()}],"tools":tools}));
    (root,connection,context,lease,child,request)
}

fn source_round_result(id:&str,name:&str,args:JsonValue)->crate::agent_runtime::model::gateway::ModelResponse {
    use crate::agent_runtime::model::gateway::{ModelResponse,ToolCall};
    ModelResponse {text:"Inspect the frozen source material.".into(),
        tool_calls:vec![ToolCall {id:id.into(),name:name.into(),arguments:args}],
        usage:AgentTokenUsage {input_tokens:12,cached_input_tokens:2,output_tokens:8,total_tokens:20,model_requests:1},usage_reported:true,cached_usage_reported:false,finish_reason:"tool_calls".into()}
}

#[test]
fn source_round_estimated_usage_never_grants_tools_or_closes_cost() {
    use crate::agent_runtime::multi_agent::{source_rounds as rounds,budget};
    let (root,db,context,lease,child,request)=source_round_fixture();
    let check=|db:&rusqlite::Connection|agent_native_source_tool_authority(db,&context,"assignment.finish").map(|_|()).map_err(str::to_string);
    let rounds::Start::Dispatch(call)=rounds::start_authorized(&db,&lease,&child,1,&request,8_000,check).unwrap() else {panic!()};
    let mut response=source_round_result("read-1","repo.read_slice",json!({"path":"app.py"}));
    response.usage_reported=false;
    let receipt=rounds::record_received(&db,&call,&response).unwrap();
    assert_eq!(receipt.response["rejection"],"model_usage_requires_reconciliation");
    assert!(receipt.response["toolCalls"].as_array().unwrap().is_empty());
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_tool_receipts WHERE assignment_id=?1",[&child.assignment_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    let b=budget::balance(&db,&lease.root_run_id,Some(&child.assignment_id),"model_input_tokens").unwrap();
    assert_eq!((b.reserved,b.consumed,b.indeterminate),(16_000,0,8_000));
    assert_eq!(budget::balance(&db,&lease.root_run_id,Some(&child.assignment_id),"model_requests").unwrap().consumed,1);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_rounds_two_rounds_durable_tools_exact_history_and_restart() {
    use crate::agent_runtime::multi_agent::source_rounds::{self as rounds,Start};
    let (root,connection,context,lease,child,request)=source_round_fixture();
    let check=|db:&rusqlite::Connection|agent_native_source_tool_authority(db,&context,"assignment.finish").map(|_|()).map_err(str::to_string);
    let Start::Dispatch(call)=rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).unwrap() else {panic!()};
    assert_eq!(call.request(),&request);
    assert!(rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).unwrap_err().contains("unknown"));
    let result=source_round_result("read-1","repo.read_slice",json!({"path":"app.py"}));
    rounds::record_received(&connection,&call,&result).unwrap();
    rounds::record_received(&connection,&call,&result).unwrap();
    assert!(rounds::continuation(&connection,&call).unwrap_err().contains("pending"));
    let (_,mut broker,_)=agent_source_broker(&context).unwrap();
    let output=rounds::execute_tool(&connection,&call,0,check,|db,name,args|broker.call(db,name,args).map_err(|e|e.code.to_string())).unwrap();
    assert!(output.to_string().contains("print('changed')"));
    assert_eq!(rounds::execute_tool(&connection,&call,0,check,|_,_,_|panic!("replayed tool" )).unwrap(),output);
    let next=rounds::continuation(&connection,&call).unwrap();
    assert_eq!(next["messages"].as_array().unwrap().len(),3);
    let mut forged=next.clone();forged["messages"][2]["content"]=json!("forged result");
    assert!(rounds::start_authorized(&connection,&lease,&child,2,&forged,8_000,check).unwrap_err().contains("transcript"));
    let Start::Dispatch(second)=rounds::start_authorized(&connection,&lease,&child,2,&next,8_000,check).unwrap() else {panic!()};
    rounds::record_received(&connection,&second,&source_round_result("inventory-2","repo.inventory",json!({}))).unwrap();
    let state=crate::agent_runtime::store::read_snapshot(&connection,&child.run_id).unwrap().unwrap();
    assert_eq!(state.snapshot["turns"],2);assert_eq!(state.snapshot["usedTokens"],40);
    let count:i64=connection.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",[&child.run_id],|r|r.get(0)).unwrap();
    assert_eq!(count,2);
    drop(connection);
    let connection=rusqlite::Connection::open(root.join("oviraptor.sqlite3")).unwrap();
    let Start::Received(recovered,receipt)=rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).unwrap() else {panic!()};
    assert_eq!(receipt.usage.total_tokens,20);
    assert_eq!(rounds::continuation(&connection,&recovered).unwrap(),next);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_rounds_unknown_and_budget_cannot_be_retried_or_bypassed() {
    use crate::agent_runtime::multi_agent::source_rounds::{self as rounds,Start};
    let (root,connection,context,lease,child,request)=source_round_fixture();
    let check=|db:&rusqlite::Connection|agent_native_source_tool_authority(db,&context,"assignment.finish").map(|_|()).map_err(str::to_string);
    assert!(rounds::start_authorized(&connection,&lease,&child,1,&request,24_001,check).unwrap_err().contains("budget"));
    assert!(rounds::start_authorized(&connection,&lease,&child,2,&request,8_000,check).is_err());
    let Start::Dispatch(call)=rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).unwrap() else {panic!()};
    rounds::record_uncertain(&connection,&call,"transport disconnected").unwrap();
    let before_unknown=crate::commands::web_mode_test_rows(&connection);
    let unknown=rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).unwrap_err();
    assert_eq!(unknown,"source_tool_original_finance_unavailable","original unknown fee is rejected before a round can be replayed");
    crate::commands::web_mode_assert_rows(&connection,&before_unknown);
    assert!(rounds::record_received(&connection,&call,&source_round_result("r1","repo.inventory",json!({}))).is_err());
    let reserved:(i64,i64)=connection.query_row("SELECT reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",[&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(reserved,(24_000,3));
    assert!(connection.execute("UPDATE agent_source_model_rounds SET state='executing'",[]).is_err());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_rounds_reject_tool_forgery_but_preserve_provider_usage() {
    use crate::agent_runtime::multi_agent::source_rounds::{self as rounds,Start};
    for case in ["host","duplicate","duplicate_action","secret","overspend","finish_mixed"] {
        let (root,connection,context,lease,child,request)=source_round_fixture();
        let check=|db:&rusqlite::Connection|agent_native_source_tool_authority(db,&context,"assignment.finish").map(|_|()).map_err(str::to_string);
        let Start::Dispatch(call)=rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).unwrap() else {panic!()};
        let mut response=source_round_result("r1","repo.inventory",json!({}));
        match case {
            "host"=>response.tool_calls[0].name="shell.execute".into(),
            "duplicate"=>response.tool_calls.push(response.tool_calls[0].clone()),
            "duplicate_action"=>{let mut duplicate=response.tool_calls[0].clone();duplicate.id="different-id".into();response.tool_calls.push(duplicate);}
            "secret"=>response.tool_calls[0].arguments=json!({"password":"never-store-this"}),
            "overspend"=>response.usage.total_tokens=8_001,
            _=>{let mut finish=response.tool_calls[0].clone();finish.id="finish".into();finish.name="assignment.finish".into();response.tool_calls.push(finish);}
        }
        let receipt=rounds::record_received(&connection,&call,&response).unwrap();
        assert_ne!(receipt.response["rejection"],"","{case}");
        assert_eq!(receipt.usage.total_tokens,response.usage.total_tokens);
        assert_eq!(receipt.response["toolCalls"],json!([]));
        assert!(!receipt.response.to_string().contains("never-store-this"));
        assert!(rounds::continuation(&connection,&call).is_err());
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_rounds_claim_and_tool_callbacks_rollback_on_revocation() {
    use crate::agent_runtime::multi_agent::source_rounds::{self as rounds,Start};
    let (root,connection,context,lease,child,request)=source_round_fixture();
    let check=|db:&rusqlite::Connection|agent_native_source_tool_authority(db,&context,"assignment.finish").map(|_|()).map_err(str::to_string);
    connection.execute_batch("CREATE TRIGGER source_round_test_revoke AFTER INSERT ON agent_source_model_rounds BEGIN UPDATE agent_capability_leases SET revoked_at='revoked'; END;").unwrap();
    assert!(rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).is_err());
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_model_rounds",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    connection.execute_batch("DROP TRIGGER source_round_test_revoke").unwrap();
    let Start::Dispatch(call)=rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).unwrap() else {panic!()};
    rounds::record_received(&connection,&call,&source_round_result("candidate","evidence.submit_candidate",json!({"path":"app.py","line":1,"title":"check","rationale":"independent review needed"}))).unwrap();
    connection.execute_batch("CREATE TRIGGER source_tool_test_revoke AFTER UPDATE ON agent_source_tool_receipts BEGIN UPDATE agent_capability_leases SET revoked_at='revoked'; END;").unwrap();
    let (_,mut broker,_)=agent_source_broker(&context).unwrap();
    assert!(rounds::execute_tool(&connection,&call,0,check,|db,name,args|broker.call(db,name,args).map_err(|e|e.code.to_string())).is_err());
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_evidence_nodes",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    let state:String=connection.query_row("SELECT state FROM agent_source_tool_receipts",[],|r|r.get(0)).unwrap();assert_eq!(state,"planned");
    connection.execute_batch("DROP TRIGGER source_tool_test_revoke").unwrap();
    let output=rounds::execute_tool(&connection,&call,0,check,|db,name,args|broker.call(db,name,args).map_err(|e|e.code.to_string())).unwrap();
    assert!(output["id"].is_string());
    // Production migration installs the revision trigger. A successfully
    // submitted real candidate must be visible to the effective graph reader
    // used by review; direct node-table existence alone is not enough.
    let revision:i64=connection.query_row("SELECT MAX(revision) FROM agent_evidence_revisions WHERE root_run_id=?1",
        [&lease.root_run_id],|r|r.get(0)).unwrap();
    assert_eq!(revision,1);
    let visible=crate::agent_runtime::evidence_graph::store::list_effective_evidence_nodes_at_revision(
        &connection,&lease.root_run_id,revision).unwrap();
    assert_eq!(visible.len(),1);
    assert_eq!(visible[0].id,output["id"].as_str().unwrap());
    assert_eq!(visible[0].created_by_run_id,child.run_id);
    assert_eq!(visible[0].payload["reviewState"],"candidate");
    assert!(crate::agent_runtime::evidence_graph::store::list_effective_evidence_nodes_at_revision(
        &connection,&lease.root_run_id,revision+1).unwrap().is_empty());
    assert!(connection.execute("UPDATE agent_source_tool_receipts SET output_json='{}'",[]).is_err());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_rounds_broker_denial_rolls_back_partial_candidate_but_keeps_receipt() {
    use crate::agent_runtime::multi_agent::source_rounds::{self as rounds,Start};
    let (root,connection,context,lease,child,request)=source_round_fixture();
    let check=|db:&rusqlite::Connection|agent_native_source_tool_authority(db,&context,"assignment.finish").map(|_|()).map_err(str::to_string);
    let Start::Dispatch(call)=rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).unwrap() else {panic!()};
    rounds::record_received(&connection,&call,&source_round_result("candidate","evidence.submit_candidate",json!({"title":"suspect","rationale":"needs review","path":"app.py","line":1}))).unwrap();
    // Deliberately bypass immutability only in this fault-injection fixture so
    // the handler writes successfully and fails its subsequent scope check.
    connection.execute_batch("DROP TRIGGER analysis_view_no_update; CREATE TRIGGER source_broker_partial_fault AFTER INSERT ON agent_evidence_nodes BEGIN UPDATE source_analysis_views SET view_root='/corrupted-view'; END;").unwrap();
    let (_,mut broker,_)=agent_source_broker(&context).unwrap();
    let output=rounds::execute_tool(&connection,&call,0,check,|db,name,args|source_local_broker_result(db,||broker.call(db,name,args))).unwrap();
    assert_eq!(output["code"],"source_analysis_integrity");
    let history=native_attempt_execution_history(&connection,&context.scan_id,1,None,50).unwrap();
    assert_eq!(history["invocations"][0]["status"],"refused");
    assert_eq!(history["invocations"][0]["receiptIntegrity"],"matched");
    assert_eq!(history["invocations"][0]["policyDecision"],"deny");
    assert_eq!(history["invocations"][0]["errorClass"],"source_tool_refused");
    assert!(!history.to_string().contains("/corrupted-view"));
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_evidence_nodes",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_evidence_revisions",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(connection.query_row("SELECT count(*) FROM source_analysis_views",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(connection.query_row("SELECT state FROM agent_source_tool_receipts",[],|r|r.get::<_,String>(0)).unwrap(),"completed");
    let replay=rounds::execute_tool(&connection,&call,0,check,|_,_,_|panic!("denial receipt must replay without executing")).unwrap();
    assert_eq!(replay,output);
    check(&connection).unwrap();
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_rounds_late_result_records_usage_without_granting_execution() {
    use crate::agent_runtime::multi_agent::source_rounds::{self as rounds,Start};
    let (root,connection,context,lease,child,request)=source_round_fixture();
    let check=|db:&rusqlite::Connection|agent_native_source_tool_authority(db,&context,"assignment.finish").map(|_|()).map_err(str::to_string);
    let Start::Dispatch(call)=rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).unwrap() else {panic!()};
    connection.execute("UPDATE agent_capability_leases SET revoked_at='revoked'",[]).unwrap();
    let receipt=rounds::record_received(&connection,&call,&source_round_result("read","repo.inventory",json!({}))).unwrap();
    assert_eq!(receipt.usage.total_tokens,20);
    assert!(rounds::execute_tool(&connection,&call,0,check,|_,_,_|panic!("revoked tool must not run")).is_err());
    assert!(rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).is_err());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_rounds_budget_and_checkpoint_trigger_changes_are_atomic() {
    use crate::agent_runtime::multi_agent::source_rounds::{self as rounds,Start};
    let (root,connection,context,lease,child,request)=source_round_fixture();
    let check=|db:&rusqlite::Connection|agent_native_source_tool_authority(db,&context,"assignment.finish").map(|_|()).map_err(str::to_string);
    connection.execute_batch("CREATE TRIGGER source_round_budget_change AFTER INSERT ON agent_source_model_rounds BEGIN UPDATE agent_assignments SET reserved_tokens=1; END;").unwrap();
    assert!(rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).is_err());
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_model_rounds",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    connection.execute_batch("DROP TRIGGER source_round_budget_change").unwrap();
    let Start::Dispatch(call)=rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).unwrap() else {panic!()};
    connection.execute_batch("CREATE TRIGGER source_round_checkpoint_change AFTER INSERT ON agent_snapshots BEGIN UPDATE agent_snapshots SET snapshot_json='{}'; END;").unwrap();
    let result=source_round_result("r1","repo.inventory",json!({}));
    assert!(rounds::record_received(&connection,&call,&result).unwrap_err().contains("checkpoint"));
    assert_eq!(connection.query_row("SELECT state FROM agent_source_model_rounds",[],|r|r.get::<_,String>(0)).unwrap(),"executing");
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_tool_receipts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_events WHERE event_type='model_round_completed'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    connection.execute_batch("DROP TRIGGER source_round_checkpoint_change").unwrap();
    rounds::record_received(&connection,&call,&result).unwrap();
    connection.execute("UPDATE agent_snapshots SET snapshot_json='{}'",[]).unwrap();
    assert!(rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).unwrap_err().contains("checkpoint"));
    assert!(rounds::execute_tool(&connection,&call,0,check,|_,_,_|panic!("corrupt accounting")).is_err());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_rounds_tool_order_deleted_receipts_and_unknown_usage_block_progress() {
    use crate::agent_runtime::multi_agent::source_rounds::{self as rounds,Start};
    let (root,connection,context,lease,child,request)=source_round_fixture();
    let check=|db:&rusqlite::Connection|agent_native_source_tool_authority(db,&context,"assignment.finish").map(|_|()).map_err(str::to_string);
    let Start::Dispatch(call)=rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).unwrap() else {panic!()};
    let mut result=source_round_result("r1","repo.inventory",json!({}));
    result.usage=AgentTokenUsage::default();
    assert!(rounds::record_received(&connection,&call,&result).unwrap_err().contains("usage"));
    assert!(rounds::start_authorized(&connection,&lease,&child,1,&request,8_000,check).unwrap_err().contains("unknown"));
    result=source_round_result("r1","repo.inventory",json!({}));
    result.tool_calls.push(source_round_result("r2","repo.read_slice",json!({"path":"app.py"})).tool_calls.remove(0));
    rounds::record_received(&connection,&call,&result).unwrap();
    assert!(rounds::execute_tool(&connection,&call,1,check,|_,_,_|panic!("out of order")).unwrap_err().contains("order"));
    connection.execute("DELETE FROM agent_source_tool_receipts WHERE call_index=0",[]).unwrap();
    assert!(rounds::execute_tool(&connection,&call,1,check,|_,_,_|panic!("missing receipt")).is_err());
    assert!(rounds::continuation(&connection,&call).is_err());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_rounds_cumulative_budget_limits_and_event_replay_match_receipts() {
    use crate::agent_runtime::multi_agent::source_rounds::{self as rounds,Start};
    let (root,connection,context,lease,child,mut request)=source_round_fixture();
    let check=|db:&rusqlite::Connection|agent_native_source_tool_authority(db,&context,"assignment.finish").map(|_|()).map_err(str::to_string);
    let (_,mut broker,_)=agent_source_broker(&context).unwrap();
    for number in 1..=3 {
        let Start::Dispatch(call)=rounds::start_authorized(&connection,&lease,&child,number,&request,8_000,check).unwrap() else {panic!()};
        rounds::record_received(&connection,&call,&source_round_result(&format!("r{number}"),"repo.inventory",json!({}))).unwrap();
        rounds::execute_tool(&connection,&call,0,check,|db,name,args|broker.call(db,name,args).map_err(|e|e.code.to_string())).unwrap();
        request=rounds::continuation(&connection,&call).unwrap();
        if number==1 {
            assert!(rounds::start_authorized(&connection,&lease,&child,2,&request,23_981,check).unwrap_err().contains("budget"));
        }
    }
    assert!(rounds::start_authorized(&connection,&lease,&child,4,&request,8_000,check).unwrap_err().contains("budget"));
    let state=crate::agent_runtime::store::read_snapshot(&connection,&child.run_id).unwrap().unwrap();
    assert_eq!(state.snapshot["modelRequests"],3);assert_eq!(state.snapshot["usedTokens"],60);
    let events=crate::agent_runtime::store::read_events_after(&connection,&child.run_id,0).unwrap();
    let replay=crate::agent_runtime::checkpoint::replay(&crate::agent_runtime::checkpoint::RunState::new(&child.run_id,Vec::new()),&events);
    assert_eq!(replay.model_requests,3);assert_eq!(replay.used_tokens,60);assert_eq!(replay.target_requests,0);
    assert_eq!(replay.input_tokens,36);assert_eq!(replay.cached_input_tokens,6);assert_eq!(replay.output_tokens,24);
    let reserved:(i64,i64)=connection.query_row("SELECT reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",[&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(reserved,(24_000,3),"journal does not double-charge or prematurely settle assignment");
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_rounds_received_usage_is_charged_before_tools_and_not_again_at_settlement() {
    use crate::agent_runtime::multi_agent::{source_rounds as rounds,budget};
    let (root,db,context,lease,child,request)=source_round_fixture();
    let check=|db:&rusqlite::Connection|agent_native_source_tool_authority(db,&context,"assignment.finish").map(|_|()).map_err(str::to_string);
    let rounds::Start::Dispatch(call)=rounds::start_authorized(&db,&lease,&child,1,&request,8_000,check).unwrap() else {panic!()};
    let result=source_round_result("charged-before-tool","repo.inventory",json!({}));
    rounds::record_received(&db,&call,&result).unwrap();
    for (dimension,spent) in budget::DIMENSIONS[..4].iter().zip([12,2,8,1]) {
        let b=budget::balance(&db,&lease.root_run_id,Some(&child.assignment_id),dimension).unwrap();assert_eq!(b.consumed,spent,"{dimension}");
    }
    let changes=db.total_changes();rounds::record_received(&db,&call,&result).unwrap();assert_eq!(db.total_changes(),changes);
    crate::commands::settle_child_usage(&db,&lease,&child,&result.usage).unwrap();
    for (dimension,spent) in budget::DIMENSIONS[..4].iter().zip([12,2,8,1]) {
        let b=budget::balance(&db,&lease.root_run_id,Some(&child.assignment_id),dimension).unwrap();assert_eq!((b.reserved,b.consumed,b.indeterminate),(0,spent,0));
    }
    drop(db);fs::remove_dir_all(root).unwrap();
}
