#[test]
fn assignment_attempt_identity_is_independent_and_replay_does_not_reclaim() {
    use crate::agent_runtime::{contract::{AgentLane,AgentRole},multi_agent::scheduler};
    let (root,path,_,lease)=multi_agent_test_root("independent-attempt",1000,10);
    let db=db::open(&path).unwrap();
    let mapper=schedule_authority_fixture(&db,&lease).unwrap();
    let reviewer=scheduler::schedule_child(&db,&lease,AgentRole::EvidenceReviewer,AgentLane::Review,
        "attempt-review",&serde_json::json!({}),1,&["evidence.read".into(),"review.write".into()],100,1).unwrap();
    let count:i64=db.query_row("SELECT count(DISTINCT lease_attempt_id) FROM agent_budget_entries
        WHERE root_run_id=?1 AND assignment_id IN (?2,?3)",
        rusqlite::params![lease.root_run_id,mapper.assignment_id,reviewer.assignment_id],|r|r.get(0)).unwrap();
    assert_eq!(count,2,"different workers must not charge under the same Coordinator epoch/fence identity");
    let rows=db.prepare("SELECT id,worker_id,fencing_token,lease_epoch,coordinator_epoch,coordinator_fencing_token
        FROM agent_assignment_attempts WHERE root_run_id=?1 ORDER BY id").unwrap()
        .query_map([&lease.root_run_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,
            r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,String>(5)?)))
        .unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    assert_eq!(rows.len(),2);
    for row in &rows {
        assert!(!row.0.is_empty() && !row.1.is_empty() && !row.2.is_empty());
        assert_ne!(row.2,lease.fencing_token);
        assert_eq!((row.3,row.4,row.5.as_str()),(1,lease.lease_epoch,lease.fencing_token.as_str()));
    }
    assert_ne!(rows[0].0,rows[1].0);assert_ne!(rows[0].1,rows[1].1);assert_ne!(rows[0].2,rows[1].2);
    scheduler::mark_child_running(&db,&lease,&mapper).unwrap();
    let before=receipt_database_snapshot(&db);
    assert_eq!(schedule_authority_fixture(&db,&lease).unwrap(),mapper);
    assert_eq!(receipt_database_snapshot(&db),before);
    drop(db);std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_expiry_blocks_web_tool_and_model_admission() {
    let (root,context,_,_)=http_journal_fixture("http://127.0.0.1:9/",0);
    let db=db::open(&context.db_path).unwrap();
    db.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01'",[]).unwrap();
    let before=receipt_database_snapshot(&db);
    assert!(agent_authorize_tool_on(&db,&context,"replay_http").is_err(),
        "Web executor must not borrow live Coordinator authority");
    assert!(native_model_budget_admission(&context,1,"expired-worker-request".into()).is_err());
    assert_eq!(receipt_database_snapshot(&db),before);
    drop(db);std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_late_http_receipt_keeps_original_worker_budget_identity() {
    use crate::agent_runtime::multi_agent::{attempts,budget};
    let (root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    let db=db::open(&context.db_path).unwrap();
    let tx=db.unchecked_transaction().unwrap();
    let (lease,assignment)=budget::target::child_owner(&tx,&context.run.as_ref().unwrap().run_id).unwrap().unwrap();
    tx.rollback().unwrap();
    let original=attempts::current(&db,&lease,&assignment).unwrap();
    let claim=claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap().unwrap();
    db.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01'",[]).unwrap();
    receive_agent_http_headers(&context,&claim,200).unwrap();
    let b=budget::balance(&db,&lease.root_run_id,Some(&assignment),"target_requests").unwrap();
    assert_eq!((b.reserved,b.consumed,b.indeterminate),(0,1,0));
    assert!(db.query_row("SELECT count(*)=2 AND count(DISTINCT lease_attempt_id)=1 AND min(lease_attempt_id)=?1 FROM agent_budget_entries WHERE assignment_id=?2 AND dimension='target_requests' AND kind IN ('forfeit','reconcile')",rusqlite::params![original.id,assignment],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(attempts::current(&db,&lease,&assignment).unwrap().expires_at,"2000-01-01");
    assert!(agent_authorize_tool_on(&db,&context,"replay_http").is_err());
    drop(db);std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_expiry_cannot_reserve_fresh_budget() {
    use crate::agent_runtime::multi_agent::{budget,scheduler};
    let (root,path,_,lease)=multi_agent_test_root("expired-worker-budget",1000,10);
    let db=db::open(&path).unwrap();let child=schedule_authority_fixture(&db,&lease).unwrap();
    scheduler::mark_child_running(&db,&lease,&child).unwrap();
    db.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01'",[]).unwrap();
    let before=receipt_database_snapshot(&db);let tx=db.unchecked_transaction().unwrap();
    assert!(budget::append(&tx,&lease,&child.assignment_id,"model_requests",budget::Kind::Reserve,
        1,"fresh-reserve-after-worker-expiry","expired-worker").is_err());
    tx.rollback().unwrap();assert_eq!(receipt_database_snapshot(&db),before);
    drop(db);std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_initialization_never_backfills_existing_native_workers() {
    let (root,path,_,lease)=multi_agent_test_root("worker-no-backfill",1000,10);
    let db=db::open(&path).unwrap();schedule_authority_fixture(&db,&lease).unwrap();
    // Isolated upgrade fixture: retain exact Native runs/assignments/ledger,
    // removing only the new schema that the preceding application lacked.
    let snapshot=|db:&rusqlite::Connection|->Vec<Vec<Vec<rusqlite::types::Value>>> {
        ["agent_runs","agent_assignments","agent_budget_ledger","agent_budget_entries",
            "agent_budget_limits","agent_capability_leases","agent_lane_leases"].iter().map(|table| {
            let mut q=db.prepare(&format!("SELECT * FROM {table} ORDER BY rowid")).unwrap();let count=q.column_count();
            q.query_map([],|r|(0..count).map(|i|r.get(i)).collect()).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
        }).collect()
    };
    let before=snapshot(&db);
    db.execute_batch("DROP TABLE agent_assignment_attempts").unwrap();
    drop(db);db::initialize(&root).unwrap();let db=db::open(&path).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM agent_assignment_attempts",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(snapshot(&db),before);
    let before=receipt_database_snapshot(&db);
    assert!(schedule_authority_fixture(&db,&lease).is_err());
    assert_eq!(receipt_database_snapshot(&db),before);
    drop(db);std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_expiry_blocks_dedicated_target_authorities() {
    let (root,mut context,lease)=public_surface_fixture("http://127.0.0.1:9/");
    let child=public_surface_child(&mut context,&lease);let db=db::open(&context.db_path).unwrap();
    db.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01' WHERE child_run_id=?1",[&child.run_id]).unwrap();
    let before=receipt_database_snapshot(&db);
    assert!(public_surface_authority(&db,&context,&lease,&child).is_err());
    assert!(claim_public_surface_capture(&context,&lease,&child).is_err());
    assert_eq!(receipt_database_snapshot(&db),before);drop(db);std::fs::remove_dir_all(root).unwrap();
    let (root,context,child,control)=budget_authorization_fixture();let db=db::open(&context.db_path).unwrap();
    db.execute("UPDATE agent_assignment_attempts SET expires_at='2000-01-01' WHERE child_run_id=?1",[&child.run_id]).unwrap();
    let before=receipt_database_snapshot(&db);
    assert!(authorization_probe_authority(&db,&context,&child,&control,"owner",&control.owner_identity,&control.owner_object_url,false).is_err());
    assert!(claim_authorization_probe(&context,&child,&control,"owner",&control.owner_identity,&control.owner_object_url).is_err());
    assert_eq!(receipt_database_snapshot(&db),before);drop(db);std::fs::remove_dir_all(root).unwrap();
}
