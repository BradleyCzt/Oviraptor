// Actual new role failure must close its original worker without refund/retry.
#[test]
fn client_side_readonly_actual_unknown_sdk_root_close_pauses_original_worker_without_refund() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::scheduler};
    let (directory,path,root,lease,mut context,slice)=client_readonly_negative_fixture("client-original-root-close");
    let (port,seen,stop)=spawn_endpoint(std::sync::Arc::new(|_|(503,"application/json",r#"{"error":{"message":"original provider failure"}}"#.into())));
    let _cleanup=ClientSideReadonlyTestGuard {root:directory,stop};
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let parent=start_original_fixture_supervisor(&path,&lease).unwrap();context.supervision=Some(parent.ticket());
    let db=db::open(&path).unwrap();
    let child=scheduler::prepare_readonly_child(&db,&lease,AgentRole::ClientSide,"client_side_frozen_observations_ready",&slice,8000).unwrap();
    let failed=multi_agent_child_round_transport(&context,&lease,&child,"只读配置分析，无工具；保留行为缺口",slice.clone());
    assert!(failed.is_err());assert_eq!(seen.lock().unwrap().len(),1,"original ClientSide SDK must actually arrive");
    let before=specialist_readonly_closure_frozen(&db,&child);
    let outcome=AgentTargetOutcome::incomplete("original ClientSide provider bill requires reconciliation");
    finish_coordinator_run(&db,&lease,&outcome).expect("new role actual SDK exit must allow Root to revoke original worker authority");
    let state:(String,String,String,i64,i64)=db.query_row("SELECT a.state,r.status,x.state,
        (SELECT count(*) FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at=''),
        (SELECT count(*) FROM agent_lane_leases WHERE assignment_id=a.id)
        FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id JOIN agent_assignment_attempts x ON x.assignment_id=a.id
        WHERE a.id=?1",[&child.assignment_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(state,("paused".into(),"paused".into(),"paused".into(),0,0));
    assert!(specialist_readonly_closure_frozen(&db,&child)==before);
    let b=crate::agent_runtime::multi_agent::budget::balance(&db,&root,Some(&child.assignment_id),"model_requests").unwrap();
    assert_eq!((b.consumed,b.indeterminate),(0,1));
    let before=receipt_database_snapshot(&db);finish_coordinator_run(&db,&lease,&outcome).unwrap();assert_eq!(receipt_database_snapshot(&db),before);
    assert!(multi_agent_child_round_transport(&context,&lease,&child,"只读配置分析，无工具；保留行为缺口",slice).is_err());
    assert_eq!(receipt_database_snapshot(&db),before);assert_eq!(seen.lock().unwrap().len(),1);
    drop(parent);
}

#[test]
fn client_side_readonly_actual_busy_sdk_refuses_root_close_then_preserves_paid_undelivered_result() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::scheduler};
    use std::sync::{Arc,Mutex,mpsc};
    let (directory,path,_,lease,mut context,slice)=client_readonly_negative_fixture("client-original-busy-close");
    let (arrive_tx,arrive)=mpsc::channel();let (release_tx,release)=mpsc::channel();let release=Mutex::new(release);
    let (port,seen,stop)=spawn_endpoint(Arc::new(move |_| {
        let _=arrive_tx.send(());let _=release.lock().unwrap().recv_timeout(Duration::from_secs(8));
        (200,"application/json",proposal_model_response(r#"{"summary":"original configuration only","observationRefs":["csp-1"],"gaps":["missing_browser_validation"],"candidates":[]}"#))
    }));
    let _cleanup=ClientSideReadonlyTestGuard {root:directory,stop};
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let parent=start_original_fixture_supervisor(&path,&lease).unwrap();context.supervision=Some(parent.ticket());
    let db=db::open(&path).unwrap();
    let child=scheduler::prepare_readonly_child(&db,&lease,AgentRole::ClientSide,"client_side_frozen_observations_ready",&slice,8000).unwrap();
    let outcome=AgentTargetOutcome::incomplete("original ClientSide SDK must exit before Root closure");
    let (closing,unchanged,result)=std::thread::scope(|scope| {
        let (c,a,ch,input)=(&context,&lease,&child,slice.clone());
        let worker=scope.spawn(move ||multi_agent_child_round_transport(c,a,ch,"configuration only; no browser validation",input));
        arrive.recv_timeout(Duration::from_secs(5)).unwrap();
        let before=super::tests::application_table_snapshot(&db);
        let closing=finish_coordinator_run(&db,&lease,&outcome);
        let unchanged=super::tests::application_table_snapshot(&db)==before;
        release_tx.send(()).unwrap();
        (closing,unchanged,worker.join().unwrap())
    });
    let error=closing.unwrap_err();assert!(error.contains("specialist_transport_not_idle"),"{error}");
    assert!(unchanged);result.unwrap();assert_eq!(seen.lock().unwrap().len(),1);
    let paid=specialist_readonly_closure_frozen(&db,&child);
    finish_coordinator_run(&db,&lease,&outcome).unwrap();
    assert!(specialist_readonly_closure_frozen(&db,&child)==paid);
    assert_eq!(db.query_row("SELECT state FROM agent_assignments WHERE id=?1",[&child.assignment_id],|r|r.get::<_,String>(0)).unwrap(),"paused");
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(db.query_row("SELECT count(*) FROM sentinel_findings",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    let before=super::tests::application_table_snapshot(&db);
    finish_coordinator_run(&db,&lease,&outcome).unwrap();assert!(super::tests::application_table_snapshot(&db)==before);
    assert_eq!(seen.lock().unwrap().len(),1);drop(parent);
}
