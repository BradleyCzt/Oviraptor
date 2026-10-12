#[test]
fn exceptional_terminal_actual_late_original_root_bill_after_close_stays_financial_only() {
    use crate::agent_runtime::{
        multi_agent::budget::{self,clock::elapsed_fact,root::model::tick::{Tick,Begin,INVOCATION_KIND}},
        execution_owner,
    };
    for expired in [false,true] {
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|{
        let mut value:JsonValue=serde_json::from_str(&proposal_model_response(&root_tick_valid_text("original late invoice"))).unwrap();
        value["usage"]=json!({"prompt_tokens":2,"completion_tokens":1,"total_tokens":3});
        (200,"application/json",value.to_string())
    }));
    let mut f=root_tick_fixture_protocol_limits_timeout("exceptional-terminal-late-actual",&format!("http://127.0.0.1:{port}/v1"),
        false,(60000,20),expired.then_some(2));
    let path=f.context.db_path.clone();let root=f.actor.root_run_id.clone();let lease=f.actor.clone();
    let db=db::open(&path).unwrap();
    let prepared=native_coordinator_request_for_frame(&f.context,&lease,None).unwrap();
    let original=execution_owner::claim_native_invocation(&path,&lease.scan_id,lease.attempt_number,INVOCATION_KIND,&root).unwrap();
    let tick={
        let tx=db.unchecked_transaction().unwrap();native_coordinator_tick_authority(&tx,&f.context,&lease).unwrap();
        let Begin::Dispatch(tick)=Tick::begin(&tx,&root,1,&prepared.basis,&prepared.fact,prepared.estimate).unwrap() else {panic!()};
        tx.commit().unwrap();tick
    };
    let call=tick.original_model_call_for_test(&db).unwrap();
    let log=crate::agent_runtime::model::diagnostics::ModelLog::coordinator(&path,&root,1);
    // Actual original provider response is held locally before its financial
    // writer. SDK and diagnostics really exit; no refund or no-send is claimed.
    let response=prepared.client.complete_once_observed_with_lifecycle(&prepared.request,
        &CancelToken::from_checker(||false),log.observer()).unwrap();
    // Frozen v3 financial policy (production coordinator_tick_once order): the
    // physical invoice is retained in its own transaction right after the SDK
    // returns; only its settlement below is the late financial writer.
    tick.persist_received_invoice(&db,&response).unwrap();
    drop(log);drop(original);drop(f.parent.take());
    if expired {
        std::thread::sleep(std::time::Duration::from_millis(2100));
        assert_eq!(finish_coordinator_run(&db,&lease,&AgentTargetOutcome::Cancelled).unwrap_err(),
            "stale_coordinator_fencing_token","expired original C cannot publish a terminal Root");
    } else {
        finish_coordinator_run(&db,&lease,&AgentTargetOutcome::Cancelled).unwrap();
    }
    let fact = elapsed_fact::read(&db, &root).unwrap();
    assert_eq!(fact.is_some(),expired);
    let status:String=db.query_row("SELECT status FROM agent_runs WHERE id=?1",[&root],|r|r.get(0)).unwrap();
    assert_eq!(status,if expired {"running"} else {"terminal"});
    let wall = budget::balance(&db, &root, None, "wall_time_ms").unwrap();
    let root_before: Vec<String> = {
        let mut q = db
            .prepare("SELECT rowid,* FROM agent_runs WHERE id=?1")
            .unwrap();
        let count = q.column_count();
        q.query_map([&root], |r| {
            Ok(format!(
                "{:?}",
                (0..count)
                    .map(|i| r.get::<_, rusqlite::types::Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()?
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
    };
    // Settle only the captured actual response under the original financial call.
    let tx = db.unchecked_transaction().unwrap();
    assert!(!call.terminal(&tx, "received", Some(&response), "").unwrap());
    tx.commit().unwrap();
    // v3: the bill is an obligation until its original terminal proof commits
    // (production received()+persist_terminal_capture). Financial only: no
    // semantic decision, publication or Root reopening is written here.
    let capture = tick.capture_terminal_proof(&db, "received", Some(&response));
    tick.persist_terminal_capture(&db, capture).unwrap();
    assert_eq!(
        budget::balance(&db, &root, None, "model_input_tokens")
            .unwrap()
            .consumed,
        2
    );
    assert_eq!(
        budget::balance(&db, &root, None, "model_output_tokens")
            .unwrap()
            .consumed,
        1
    );
    assert_eq!(
        budget::balance(&db, &root, None, "wall_time_ms").unwrap(),
        wall
    );
    assert_eq!(elapsed_fact::read(&db, &root).unwrap(), fact);
    let root_after: Vec<String> = {
        let mut q = db
            .prepare("SELECT rowid,* FROM agent_runs WHERE id=?1")
            .unwrap();
        let count = q.column_count();
        q.query_map([&root], |r| {
            Ok(format!(
                "{:?}",
                (0..count)
                    .map(|i| r.get::<_, rusqlite::types::Value>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()?
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
    };
    assert_eq!(
        root_before, root_after,
        "late bill cannot reopen or change the closed original Root"
    );
    assert!(call.require_next_work(&db).is_err());
    assert_eq!(budget::admission::require_determinate(&db, &root).is_err(),expired,
        "known billing is determinate; an exceptional cutoff still forbids new work");
    drop(db);
    assert_eq!(seen.lock().unwrap().len(),1,"late accounting never repeats the original SDK");
    }
}
