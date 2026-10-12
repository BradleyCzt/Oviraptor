#[test]
fn identity_root_feedback_actual_closed_worker_returns_metadata_once_before_web_grant() {
    let _real=RealSpecialistTransport::enter();
    let boundary=std::sync::Arc::new(std::sync::Mutex::new(None));
    let (mut f,seen,stop)=identity_root_fixture("identity-root-feedback",false,false,boundary);
    let mut session=multi_agent_prepare(&mut f.context).unwrap();
    let db=db::open(&f.context.db_path).unwrap();
    assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),3,"closed paid identity output was not independently supervised by original Root");
    assert_eq!(seen.lock().unwrap().len(),5);
    let wires=seen.lock().unwrap();
    let root_wire=wires.iter().find(|w|w.contains("identity-session-output")).unwrap();
    let request:JsonValue=serde_json::from_str(root_wire.split_once("\r\n\r\n").unwrap().1).unwrap();
    let input:JsonValue=serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
    let fact=&input["basis"]["changedFact"]["semantic"];
    assert_eq!(fact["role"],"identity_session");assert_eq!(fact["identityCount"],1);
    assert_eq!(fact["output"]["authorizationProven"],false);
    assert_eq!(fact["targetRequestsGranted"],0);assert_eq!(fact["authorizationProven"],false);
    assert!(!wires.join("\n").contains("fixture-identity-secret"));drop(wires);
    let rows=web_mode_test_rows(&db);
    let returned=native_coordinator_identity_feedback(&f.context,&session.lease,&session.mapper,&identity_root_child(&db,&session.lease.root_run_id)).unwrap();
    assert!(returned.replayed);assert_eq!(seen.lock().unwrap().len(),5);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_http_request_claims",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    let root=native_coordinator_root_context(&f.context,&session.lease);
    let tx=rusqlite::Transaction::new_unchecked(&db,rusqlite::TransactionBehavior::Immediate).unwrap();
    let mut frame=NativeCoordinatorFrame::identity_session(&tx,&session.lease,&identity_root_child(&db,&session.lease.root_run_id)).unwrap();
    frame.rows.extend(NativeCoordinatorFrame::mapper(&tx,&session.lease,&session.mapper).unwrap().rows);tx.commit().unwrap();
    let decision=native_coordinator_tick_for_frame(&root,&session.lease,&frame).unwrap();
    assert_eq!(native_coordinator_rust_dispatch_policy(&root,&frame,&decision,crate::agent_runtime::contract::AgentRole::WebExecutor,
        512,0,[Some(60000),Some(20),Some(32)]).unwrap_err(),"root_trigger_role_conflict");
    web_mode_assert_rows(&db,&rows);
    let outcome=run_native_agent(&f.context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{}",outcome.detail());
    multi_agent_finish_execution(&f.context,&mut session,&outcome).unwrap();
    drop(session);drop(f);stop.store(true,std::sync::atomic::Ordering::SeqCst);
}
#[test]
fn identity_root_feedback_paid_advisory_cannot_authorize_web_execution() {
    let _real=RealSpecialistTransport::enter();
    let boundary=std::sync::Arc::new(std::sync::Mutex::new(None));
    let (mut f,seen,stop)=identity_root_fixture("identity-root-forbidden",true,false,boundary);
    let error=multi_agent_prepare(&mut f.context).err().unwrap();
    assert_eq!(error,"root_identity_step_not_bounded");
    let db=db::open(&f.context.db_path).unwrap();
    assert_eq!(seen.lock().unwrap().len(),4);
    assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),2);
    assert_eq!(crate::agent_runtime::multi_agent::budget::balance(&db,&f.actor.root_run_id,Some(""),"model_requests").unwrap().consumed,2);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE role='web_executor'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_http_request_claims",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(f);stop.store(true,std::sync::atomic::Ordering::SeqCst);
}
#[test]
fn identity_root_feedback_invalid_paid_identity_claim_is_not_a_root_or_target_fact() {
    let _real=RealSpecialistTransport::enter();
    let boundary=std::sync::Arc::new(std::sync::Mutex::new(None));
    let (mut f,seen,stop)=identity_root_fixture("identity-root-false-proof",false,true,boundary);
    assert_eq!(multi_agent_prepare(&mut f.context).err().unwrap(),"root_identity_schema_invalid");
    let db=db::open(&f.context.db_path).unwrap();
    assert_eq!(seen.lock().unwrap().len(),3);
    assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),1);
    assert_eq!(db.query_row("SELECT used_tokens FROM agent_runs WHERE role='identity_session'",[],|r|r.get::<_,i64>(0)).unwrap(),20);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE role='web_executor'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(f);stop.store(true,std::sync::atomic::Ordering::SeqCst);
}
#[test]
fn identity_root_feedback_original_consumed_ack_and_payload_cannot_be_reinterpreted_as_new_fact() {
    for (mutation,expected) in [
        ("acknowledged_at='2000-01-01 00:00:00'","root_identity_output_events_invalid"),
        ("payload_json=json_set(payload_json,'$.newCapability','replay_http')","root_identity_output_receipt_mismatch"),
        ("correlation_id='identity:altered'","root_identity_output_missing"),
    ] {
        let _real=RealSpecialistTransport::enter();
        let boundary=std::sync::Arc::new(std::sync::Mutex::new(None));
        let (mut f,seen,stop)=identity_root_fixture("identity-root-original-message",false,false,boundary);
        let mut session=multi_agent_prepare(&mut f.context).unwrap();
        let db=db::open(&f.context.db_path).unwrap();
        db.execute(&format!("UPDATE agent_messages SET {mutation} WHERE from_agent='identity_session' AND kind='identity_assessment'"),[]).unwrap();
        let before=web_mode_test_rows(&db);
        let error=native_coordinator_identity_feedback(&f.context,&session.lease,&session.mapper,&identity_root_child(&db,&session.lease.root_run_id)).err().unwrap();
        web_mode_assert_rows(&db,&before);assert_eq!(seen.lock().unwrap().len(),5);
        assert_eq!(error,expected);
        let outcome=run_native_agent(&f.context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{}",outcome.detail());
    multi_agent_finish_execution(&f.context,&mut session,&outcome).unwrap();
        drop(session);drop(f);stop.store(true,std::sync::atomic::Ordering::SeqCst);
    }
}
#[test]
fn identity_root_feedback_paid_worker_or_metadata_change_withholds_publication_and_recovers_locally() {
    for metadata in [false,true] {
        let _real=RealSpecialistTransport::enter();
        let saved=std::sync::Arc::new(std::sync::Mutex::new(None::<(String,String)>));let original=saved.clone();
        let boundary:IdentityRootBoundary=std::sync::Arc::new(std::sync::Mutex::new(Some(Box::new(move |db:&rusqlite::Connection| {
            if metadata {
                let (id,value)=db.query_row("SELECT id,status FROM browser_auth_sessions WHERE id='identity-one'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
                db.execute("UPDATE browser_auth_sessions SET status='invalid' WHERE id='identity-one'",[]).unwrap();
                *original.lock().unwrap()=Some((id,value));
            } else {
                let (id,value)=db.query_row("SELECT w.id,w.finished_at FROM agent_assignment_attempts w JOIN agent_runs r ON r.id=w.child_run_id WHERE r.role='identity_session'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
                db.execute("UPDATE agent_assignment_attempts SET finished_at='' WHERE id=?1",[&id]).unwrap();
                *original.lock().unwrap()=Some((id,value));
            }
        }))));
        let (mut f,seen,stop)=identity_root_fixture("identity-root-original-changed",false,false,boundary);
        assert_eq!(multi_agent_prepare(&mut f.context).err().unwrap(),"root_frame_original_fact_changed");
        let db=db::open(&f.context.db_path).unwrap();
        assert_eq!(seen.lock().unwrap().len(),4);assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),1);
        assert_eq!(crate::agent_runtime::multi_agent::budget::balance(&db,&f.actor.root_run_id,Some(""),"model_requests").unwrap().consumed,2);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE role='web_executor'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        let parent=start_original_fixture_supervisor(&f.context.db_path,&f.actor).unwrap();
        f.context.supervision=Some(parent.ticket());f.parent=Some(parent);
        let mapper=identity_root_mapper(&db,&f.actor.root_run_id);let child=identity_root_child(&db,&f.actor.root_run_id);
        let before=web_mode_test_rows(&db);
        assert!(native_coordinator_identity_feedback(&f.context,&f.actor,&mapper,&child).is_err());
        web_mode_assert_rows(&db,&before);assert_eq!(seen.lock().unwrap().len(),4);
        let (id,value)=saved.lock().unwrap().take().unwrap();
        if metadata {db.execute("UPDATE browser_auth_sessions SET status=?2 WHERE id=?1",params![id,value]).unwrap();}
        else {db.execute("UPDATE agent_assignment_attempts SET finished_at=?2 WHERE id=?1",params![id,value]).unwrap();}
        let costs=crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries ORDER BY rowid",[]).unwrap();
        let returned=native_coordinator_identity_feedback(&f.context,&f.actor,&mapper,&child).unwrap();
        assert!(returned.replayed);assert_eq!(seen.lock().unwrap().len(),4);assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),2);
        assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries ORDER BY rowid",[]).unwrap()==costs);
        drop(f);stop.store(true,std::sync::atomic::Ordering::SeqCst);
    }
}
