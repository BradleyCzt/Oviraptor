// Actual Source tools gateway, independent of preliminary specialist SDKs.
#[test]
fn source_round_inflight_original_sdk_blocks_root_terminal_until_actual_exit() {source_round_inflight_boundary(1);}
#[test]
fn source_round_inflight_second_actual_sdk_keeps_first_paid_round_and_blocks_root_terminal() {source_round_inflight_boundary(2);}
fn source_round_inflight_boundary(blocked:usize) {
    use crate::agent_runtime::{contract::{AgentLane,AgentRole},multi_agent::{scheduler,source,supervisor::WorkerSupervisor}};
    use std::sync::{Arc,Mutex,mpsc};
    let (arrive_tx,arrive)=mpsc::channel();let (release_tx,release)=mpsc::channel();let release=Mutex::new(release);
    let calls=std::sync::atomic::AtomicUsize::new(0);
    let (port,seen,_stop)=crate::scanning::agent_tests::spawn_endpoint(Arc::new(move |_| {
        if calls.fetch_add(1,std::sync::atomic::Ordering::SeqCst)+1<blocked {
            return (200,"application/json",source_round_inflight_reply("repo.inventory",json!({})));
        }
        let _=arrive_tx.send(());let _=release.lock().unwrap().recv_timeout(Duration::from_secs(8));
        (200,"application/json",json!({"choices":[{"message":{"role":"assistant","content":"delayed actual Source response"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
    }));
    let model=source_specialist_test_environment(port);
    let (directory,db,_record,actor)=source_tool_true_born_fixture_model(Some(&model));
    db.execute("UPDATE agent_runs SET status='running',started_at=datetime('now','localtime') WHERE id=?1",[&actor.root_run_id]).unwrap();
    let role=AgentRole::RepoMapper;
    crate::agent_runtime::multi_agent::directive::source_guidance::freeze(&db,&actor,role,true).unwrap();
    let slice=source::tool_task_slice(&db,&actor,role,1).unwrap();
    let child=scheduler::schedule_child(&db,&actor,role,AgentLane::ReadOnlyAnalysis,"source_tools_ready",
        &slice,1,&source::tool_capabilities(role).unwrap(),24_000,3).unwrap();
    scheduler::mark_child_running(&db,&actor,&child).unwrap();
    let database=directory.join("oviraptor.sqlite3");let work=directory.join("attempt-0001");
    let parent=WorkerSupervisor::start(&database,&actor).unwrap();let ticket=parent.ticket();
    let outcome=AgentTargetOutcome::incomplete("original Source tools SDK is still in flight");
    let (closing,unchanged,early)=std::thread::scope(|scope| {
        let (done_tx,done)=mpsc::channel();let actor=&actor;let database=&database;let work=&work;let model=&model;
        scope.spawn(move || {
            let connection=db::open(database).unwrap();
            let context=SpecialistTransportContext {supervision:Some(ticket),db_path:database,scan_id:&actor.scan_id,
                attempt_number:actor.attempt_number,target_key:&actor.target_key,run_id:&actor.root_run_id,
                environment:model,proxy:None,usage_dir:work,deadline:Some(std::time::Instant::now()+Duration::from_secs(60))};
            let _=done_tx.send(execute_source_tool_assignment(&connection,&context,actor,&child,&slice));
        });
        arrive.recv_timeout(Duration::from_secs(5)).expect("actual original Source tools SDK must arrive");
        assert_eq!(db.query_row("SELECT count(*) FROM agent_source_model_rounds WHERE root_run_id=?1 AND state='executing'",
            [&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_source_model_rounds WHERE state='received'",[],|r|r.get::<_,i64>(0)).unwrap(),blocked as i64-1);
        let before=application_table_snapshot(&db);
        let closing=finish_coordinator_run(&db,actor,&outcome);
        let unchanged=application_table_snapshot(&db)==before;
        db.execute("UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1",[&actor.root_run_id]).unwrap();
        let early=done.recv_timeout(Duration::from_secs(3));let _=release_tx.send(());
        let returned=match early {Ok(result)=>Some(result),Err(_)=>{let _=done.recv_timeout(Duration::from_secs(5));None}};
        (closing,unchanged,returned)
    });
    let error=closing.expect_err("Root published terminal while actual original Source tools SDK remained in flight");
    assert!(error.contains("source_round_transport_not_idle"),"{error}");
    assert!(unchanged,"busy Source tools SDK must preserve every original application row");
    assert!(early.is_some_and(|r|r.is_err()),"original cancellation must return from SDK before provider release");
    let deadline=std::time::Instant::now()+Duration::from_secs(2);
    while seen.lock().unwrap().len()<blocked && std::time::Instant::now()<deadline {std::thread::yield_now();}
    assert_eq!(seen.lock().unwrap().len(),blocked);
    use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
    let fees=Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap();
    let rounds=Rows::read(&db,"SELECT rowid,* FROM agent_source_model_rounds ORDER BY rowid",[]).unwrap();
    finish_coordinator_run(&db,&actor,&outcome).unwrap();
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap()==fees);
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_source_model_rounds ORDER BY rowid",[]).unwrap()==rounds);
    let before=application_table_snapshot(&db);finish_coordinator_run(&db,&actor,&outcome).unwrap();assert!(application_table_snapshot(&db)==before);
    drop(db);drop(parent);fs::remove_dir_all(directory).unwrap();
}

fn source_round_inflight_reply(name:&str,args:JsonValue)->String {
    json!({"choices":[{"message":{"role":"assistant","tool_calls":[{"id":name,"type":"function",
        "function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}],
        "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string()
}
struct SourceRoundClosureFixture {
    directory:PathBuf,db:rusqlite::Connection,
    actor:crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child:crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    parent:crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor,
    seen:crate::scanning::agent_tests::Seen,
}
fn source_round_inflight_finished(paid:bool)->SourceRoundClosureFixture {
    use crate::agent_runtime::{contract::{AgentLane,AgentRole},multi_agent::{scheduler,source,supervisor::WorkerSupervisor}};
    let (port,seen,_stop)=crate::scanning::agent_tests::spawn_endpoint(std::sync::Arc::new(move |_| {
        if paid {(200,"application/json",source_round_inflight_reply("assignment.finish",json!({"summary":"actual scoped finish","gaps":[]})))}
        else {(503,"application/json",json!({"error":{"message":"source fixture unavailable"}}).to_string())}
    }));
    let model=source_specialist_test_environment(port);
    let (directory,db,_record,actor)=source_tool_true_born_fixture_model(Some(&model));
    db.execute("UPDATE agent_runs SET status='running',started_at=datetime('now','localtime') WHERE id=?1",[&actor.root_run_id]).unwrap();
    let role=AgentRole::SourceAnalyst;
    crate::agent_runtime::multi_agent::directive::source_guidance::freeze(&db,&actor,role,true).unwrap();
    let slice=source::tool_task_slice(&db,&actor,role,1).unwrap();
    let child=scheduler::schedule_child(&db,&actor,role,AgentLane::ReadOnlyAnalysis,"source_tools_ready",
        &slice,1,&source::tool_capabilities(role).unwrap(),24_000,3).unwrap();
    scheduler::mark_child_running(&db,&actor,&child).unwrap();
    let database=directory.join("oviraptor.sqlite3");let work=directory.join("attempt-0001");
    let parent=WorkerSupervisor::start(&database,&actor).unwrap();
    let context=SpecialistTransportContext {supervision:Some(parent.ticket()),db_path:&database,scan_id:&actor.scan_id,
        attempt_number:1,target_key:&actor.target_key,run_id:&actor.root_run_id,environment:&model,proxy:None,
        usage_dir:&work,deadline:Some(std::time::Instant::now()+Duration::from_secs(60))};
    let result=execute_source_tool_assignment(&db,&context,&actor,&child,&slice);
    assert_eq!(result.is_ok(),paid,"{result:?}");assert_eq!(seen.lock().unwrap().len(),1);
    SourceRoundClosureFixture {directory,db,actor,child,parent,seen}
}
fn source_round_inflight_original_path(f:&SourceRoundClosureFixture)->PathBuf {
    use sha2::Digest;
    let path=fs::canonicalize(f.db.path().unwrap()).unwrap();let mut name=path.file_name().unwrap().to_os_string();name.push(".invocations");
    let key=serde_json::to_vec(&(&f.actor.scan_id,f.actor.attempt_number,"source-round-sdk",&f.child.run_id)).unwrap();
    path.with_file_name(name).join(format!("{:x}.lock",sha2::Sha256::digest(key)))
}
fn source_round_inflight_cleanup(f:SourceRoundClosureFixture) {
    let SourceRoundClosureFixture {directory,db,parent,..}=f;drop(db);drop(parent);fs::remove_dir_all(directory).unwrap();
}
#[test]
fn source_round_inflight_missing_or_foreign_original_inode_refuses_closure_and_reentry_without_creating_proof() {
    for foreign in [false,true] {
        let f=source_round_inflight_finished(false);let path=source_round_inflight_original_path(&f);assert!(path.is_file());fs::remove_file(&path).unwrap();
        if foreign {drop(crate::agent_runtime::execution_owner::claim_native_invocation(Path::new(f.db.path().unwrap()),
            &f.actor.scan_id,1,"source-round-sdk","foreign-source-worker").unwrap());}
        let before=application_table_snapshot(&f.db);
        let error=finish_coordinator_run(&f.db,&f.actor,&AgentTargetOutcome::incomplete("original Source exit proof missing")).unwrap_err();
        assert!(error.contains("source_round_transport_original_exit_proof_missing"),"{error}");
        assert!(application_table_snapshot(&f.db)==before);assert!(!path.exists());
        let (raw,reserved):(String,i64)=f.db.query_row("SELECT request_json,reserved_tokens FROM agent_source_model_rounds WHERE child_run_id=?1",[&f.child.run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();let request:JsonValue=serde_json::from_str(&raw).unwrap();
        assert!(crate::agent_runtime::multi_agent::source_rounds::start_for_transport(&f.db,&f.actor,&f.child,1,&request,reserved,|_|Ok(())).is_err());
        assert!(application_table_snapshot(&f.db)==before);assert!(!path.exists());assert_eq!(f.seen.lock().unwrap().len(),1);
        source_round_inflight_cleanup(f);
    }
}
#[test]
fn source_round_inflight_corrupt_original_hash_task_or_worker_refuses_closure_with_all_rows_preserved() {
    for damage in [
        "UPDATE agent_source_model_rounds SET request_hash='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'",
        "UPDATE agent_source_model_rounds SET request_json='{}'",
        "UPDATE agent_runs SET role='repo_mapper' WHERE role='source_analyst'",
    ] {
        let f=source_round_inflight_finished(false);f.db.execute_batch("DROP TRIGGER agent_source_round_immutable").unwrap();f.db.execute_batch(damage).unwrap();
        let before=application_table_snapshot(&f.db);
        assert!(finish_coordinator_run(&f.db,&f.actor,&AgentTargetOutcome::incomplete("original Source round damaged")).is_err(),"{damage}");
        assert!(application_table_snapshot(&f.db)==before,"{damage}");assert_eq!(f.seen.lock().unwrap().len(),1);
        source_round_inflight_cleanup(f);
    }
}
#[test]
fn source_round_inflight_actual_paid_finish_closes_root_preserves_original_fee_and_cannot_dispatch_again() {
    let f=source_round_inflight_finished(true);
    use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
    let fees=Rows::read(&f.db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap();
    let calls=Rows::read(&f.db,"SELECT rowid,* FROM agent_source_model_rounds ORDER BY rowid",[]).unwrap();
    let outcome=AgentTargetOutcome::incomplete("actual scoped Source tool worker returned");finish_coordinator_run(&f.db,&f.actor,&outcome).unwrap();
    assert!(Rows::read(&f.db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap()==fees);
    assert!(Rows::read(&f.db,"SELECT rowid,* FROM agent_source_model_rounds ORDER BY rowid",[]).unwrap()==calls);
    assert_eq!(crate::agent_runtime::multi_agent::budget::balance(&f.db,&f.actor.root_run_id,None,"model_requests").unwrap().consumed,1);
    let before=application_table_snapshot(&f.db);let path=source_round_inflight_original_path(&f);let request:JsonValue=serde_json::from_str(&f.db.query_row("SELECT request_json FROM agent_source_model_rounds WHERE child_run_id=?1",[&f.child.run_id],|r|r.get::<_,String>(0)).unwrap()).unwrap();
    assert!(crate::agent_runtime::multi_agent::source_rounds::start_for_transport(&f.db,&f.actor,&f.child,2,&request,1,|_|Ok(())).is_err());
    finish_coordinator_run(&f.db,&f.actor,&outcome).unwrap();assert!(application_table_snapshot(&f.db)==before);
    assert!(path.is_file());assert_eq!(f.seen.lock().unwrap().len(),1);source_round_inflight_cleanup(f);
}
