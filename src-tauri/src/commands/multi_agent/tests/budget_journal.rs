#[test]
fn multi_agent_scheduler_records_immutable_budget_sources_atomically() {
    use crate::agent_runtime::{contract::{AgentLane, AgentRole}, multi_agent::scheduler};
    let (root,path,root_id,lease)=multi_agent_test_root("budget-journal",100,3);
    let db=db::open(&path).unwrap();
    let child=scheduler::schedule_child(&db,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,
        "journal",&serde_json::json!({}),1,&["evidence.read".into()],60,2).unwrap();
    let read=|| db.prepare("SELECT dimension,kind,amount,assignment_id FROM agent_budget_entries WHERE root_run_id=?1 AND dimension<>'wall_time_ms' ORDER BY dimension,kind")
        .unwrap().query_map([&root_id],|r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?)))
        .unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    let before=read();
    assert_eq!(before.len(),5);
    assert!(before.iter().all(|row|row.1=="reserve" && row.3==child.assignment_id));
    scheduler::schedule_child(&db,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,
        "journal",&serde_json::json!({}),1,&["evidence.read".into()],60,2).unwrap();
    assert_eq!(read(),before);
    assert!(db.execute("UPDATE agent_budget_entries SET amount=0 WHERE root_run_id=?1",[&root_id]).is_err());
    assert!(db.execute("DELETE FROM agent_budget_entries WHERE root_run_id=?1",[&root_id]).is_err());
    scheduler::mark_child_running(&db,&lease,&child).unwrap();
    let usage=AgentTokenUsage { input_tokens:35,cached_input_tokens:5,output_tokens:15,total_tokens:50,model_requests:1 };
    settle_child_usage(&db,&lease,&child,&usage).unwrap();
    let settled=read();
    assert!(settled.contains(&("model_input_tokens".into(),"consume".into(),35,child.assignment_id.clone())));
    assert!(settled.contains(&("model_cached_tokens".into(),"consume".into(),5,child.assignment_id.clone())));
    assert!(settled.contains(&("model_output_tokens".into(),"consume".into(),15,child.assignment_id.clone())));
    settle_child_usage(&db,&lease,&child,&usage).unwrap();
    assert_eq!(read(),settled);
    let error=settle_child_usage(&db,&lease,&child,&AgentTokenUsage {input_tokens:34,output_tokens:16,..usage}).unwrap_err();
    assert_eq!(error,"budget_entry_replay_conflict");
    assert_eq!(read(),settled);
    drop(db);std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn received_model_call_cannot_finish_and_refund_unsettled_costs() {
    use crate::agent_runtime::multi_agent::{scheduler,specialist};
    use crate::agent_runtime::contract::{AgentLane,AgentRole};
    let (root,path,_,lease)=multi_agent_test_root("journal-no-refund",100,3);
    let db=db::open(&path).unwrap();
    let child=scheduler::schedule_child(&db,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,
        "journal",&serde_json::json!({}),1,&["evidence.read".into()],60,2).unwrap();
    scheduler::mark_child_running(&db,&lease,&child).unwrap();
    let specialist::Start::Dispatch(call)=specialist::start(&db,&lease,&child,&serde_json::json!({"messages":[]})).unwrap() else {panic!("missing claim")};
    specialist::record_received(&db,&call,"received",false,&AgentTokenUsage {input_tokens:30,output_tokens:20,total_tokens:50,model_requests:1,..Default::default()}).unwrap();
    let before=receipt_database_snapshot(&db);
    assert_eq!(scheduler::finish_child(&db,&lease,&child,false,"cancel after response").unwrap_err(),"budget_indeterminate_requires_reconciliation");
    assert_eq!(receipt_database_snapshot(&db),before);
    drop(db);std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_journal_write_failure_rolls_back_assignment_lane_and_summary() {
    use crate::agent_runtime::{contract::{AgentLane,AgentRole}, multi_agent::scheduler};
    for action in ["ABORT,'journal unavailable'","FAIL,'journal unavailable'","IGNORE"] {
        let (root,path,_,lease)=multi_agent_test_root("journal-rollback",100,3);
        let db=db::open(&path).unwrap();
        let before=receipt_database_snapshot(&db);
        db.execute_batch(&format!("CREATE TRIGGER reject_budget_entry BEFORE INSERT ON agent_budget_entries BEGIN SELECT RAISE({action}); END;")).unwrap();
        assert!(scheduler::schedule_child(&db,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,
            "journal",&serde_json::json!({}),1,&["evidence.read".into()],60,2).is_err(),"{action}");
        assert_eq!(receipt_database_snapshot(&db),before,"{action}");
        drop(db);std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn journal_rejects_cross_assignment_release_and_keeps_unknown_cost_charged() {
    use crate::agent_runtime::{contract::{AgentLane,AgentRole}, multi_agent::{scheduler,budget::{self,Kind}}};
    let (root,path,root_id,lease)=multi_agent_test_root("journal-transitions",100,3);
    let db=db::open(&path).unwrap();
    let child=scheduler::schedule_child(&db,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,
        "journal",&serde_json::json!({}),1,&["evidence.read".into()],60,2).unwrap();
    let tx=db.unchecked_transaction().unwrap();
    assert_eq!(budget::append(&tx,&lease,"foreign","model_requests",Kind::Release,1,"foreign","claim").unwrap_err(),"budget_assignment_scope_conflict");
    assert_eq!(budget::append(&tx,&lease,&child.assignment_id,"model_requests",Kind::Release,3,"overflow","claim").unwrap_err(),"budget_transition_exceeds_balance");
    budget::append(&tx,&lease,&child.assignment_id,"model_requests",Kind::Forfeit,1,"unknown","claim").unwrap();
    let b=budget::balance(&tx,&root_id,None,"model_requests").unwrap();
    assert_eq!((b.reserved,b.consumed,b.indeterminate),(1,0,1));
    assert_eq!(budget::append(&tx,&lease,&child.assignment_id,"model_requests",Kind::Reserve,1,"retry","claim").unwrap_err(),"budget_indeterminate_requires_reconciliation");
    budget::append(&tx,&lease,&child.assignment_id,"model_requests",Kind::Reconcile,1,"receipt","saved-receipt").unwrap();
    let b=budget::balance(&tx,&root_id,None,"model_requests").unwrap();
    assert_eq!((b.reserved,b.consumed,b.indeterminate),(1,1,0));
    budget::append(&tx,&lease,&child.assignment_id,"model_requests",Kind::Reconcile,1,"receipt","saved-receipt").unwrap();
    assert_eq!(budget::append(&tx,&lease,&child.assignment_id,"model_requests",Kind::Reconcile,1,"receipt","different-receipt").unwrap_err(),"budget_entry_replay_conflict");
    let mut stale=lease.clone();stale.fencing_token="stale".into();
    assert!(budget::append(&tx,&stale,&child.assignment_id,"model_requests",Kind::Reserve,1,"stale","claim").is_err());
    tx.commit().unwrap();drop(db);std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn proven_before_transport_cancellation_releases_without_inventing_spend() {
    use crate::agent_runtime::{contract::{AgentLane,AgentRole},multi_agent::{scheduler,specialist,budget}};
    let (root,path,root_id,lease)=multi_agent_test_root("journal-before-send",100,3);
    let db=db::open(&path).unwrap();
    let child=scheduler::schedule_child(&db,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,
        "cancel-before-send",&serde_json::json!({}),1,&["evidence.read".into()],60,2).unwrap();
    scheduler::mark_child_running(&db,&lease,&child).unwrap();
    let specialist::Start::Dispatch(call)=specialist::start(&db,&lease,&child,&serde_json::json!({"messages":[]})).unwrap() else {panic!("missing claim")};
    specialist::record_not_sent(&db,&call,"user_cancelled").unwrap();
    scheduler::finish_child(&db,&lease,&child,false,"cancel before transport").unwrap();
    for dimension in &budget::DIMENSIONS[..4] {
        assert_eq!(budget::balance(&db,&root_id,None,dimension).unwrap(),budget::Balance::default());
    }
    drop(db);std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_journal_freezes_all_dimensions_and_denies_ungranted_writes() {
    use crate::agent_runtime::{contract::{AgentLane,AgentRole},multi_agent::{scheduler,budget::{self,Kind}}};
    let (root,path,root_id,lease)=multi_agent_test_root("journal-vector",100,3);
    let db=db::open(&path).unwrap();
    let child=scheduler::schedule_child(&db,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,
        "limits",&serde_json::json!({}),1,&["evidence.read".into()],60,2).unwrap();
    let limits:Vec<String>=db.prepare("SELECT dimension FROM agent_budget_limits WHERE root_run_id=?1 ORDER BY dimension").unwrap()
        .query_map([&root_id],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    let mut expected=budget::DIMENSIONS.map(str::to_string);expected.sort();assert_eq!(limits,expected);
    let tx=db.unchecked_transaction().unwrap();
    for dimension in ["controlled_writes","upload_bytes","browser_actions"] {
        assert_eq!(budget::append(&tx,&lease,&child.assignment_id,dimension,Kind::Reserve,1,dimension,"ungranted").unwrap_err(),"budget_dimension_denied_by_contract");
    }
    assert_eq!(budget::balance(&tx,&root_id,None,"concurrency_batches").unwrap().reserved,1);
    assert_eq!(budget::append(&tx,&lease,&child.assignment_id,"concurrency_batches",Kind::Reserve,3,"fourth","child").unwrap_err(),"budget_hard_limit_exceeded");
    tx.rollback().unwrap();
    scheduler::finish_child(&db,&lease,&child,false,"not dispatched").unwrap();
    assert_eq!(budget::balance(&db,&root_id,None,"concurrency_batches").unwrap(),budget::Balance::default());
    drop(db);std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn terminal_saved_receipt_settles_journal_without_restoring_expired_authority() {
    use crate::agent_runtime::multi_agent::{budget,directive::reconciliation::reconcile_received};
    let (root,context,id,lease, _original_parent)=saved_receipt_fixture(true);
    let db=db::open(&context.db_path).unwrap();
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1",[&lease.root_run_id]).unwrap();
    let status:String=db.query_row("SELECT status FROM agent_runs WHERE id=?1",[&lease.root_run_id],|r|r.get(0)).unwrap();assert_eq!(status,"terminal");
    reconcile_received(&db,&lease.scan_id,lease.attempt_number,&id).unwrap();
    let b=budget::balance(&db,&lease.root_run_id,None,"model_requests").unwrap();assert_eq!((b.reserved,b.consumed,b.indeterminate),(0,1,0));
    assert_eq!(budget::balance(&db,&lease.root_run_id,None,"concurrency_batches").unwrap(),budget::Balance::default());
    let before=receipt_database_snapshot(&db);
    reconcile_received(&db,&lease.scan_id,lease.attempt_number,&id).unwrap();assert_eq!(receipt_database_snapshot(&db),before);
    let authority:(String,String)=db.query_row("SELECT r.status,c.lease_expires_at FROM agent_runs r JOIN agent_coordinator_leases c ON c.root_run_id=r.id WHERE r.id=?1",[&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(authority,("terminal".into(),"2000-01-01 00:00:00".into()));
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_journal_replacement_root_fence_cannot_release_old_child_budget() {
    use crate::agent_runtime::{contract::{AgentLane,AgentRole},multi_agent::{scheduler,budget::{self,Kind}}};
    let (root,path,_,mut lease)=multi_agent_test_root("journal-rebound",100,3);
    let db=db::open(&path).unwrap();
    let child=scheduler::schedule_child(&db,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,
        "bound",&serde_json::json!({}),1,&["evidence.read".into()],60,2).unwrap();
    lease.lease_epoch+=1;lease.fencing_token="replacement".into();
    db.execute("UPDATE agent_coordinator_leases SET lease_epoch=?1,fencing_token=?2 WHERE root_run_id=?3",params![lease.lease_epoch,lease.fencing_token,lease.root_run_id]).unwrap();
    let before=receipt_database_snapshot(&db);let tx=db.unchecked_transaction().unwrap();
    assert_eq!(budget::append(&tx,&lease,&child.assignment_id,"model_requests",Kind::Release,1,"replacement","old-child").unwrap_err(),"budget_assignment_scope_conflict");
    tx.rollback().unwrap();assert_eq!(receipt_database_snapshot(&db),before);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_journal_uncertain_model_call_keeps_unknown_cost_in_the_vector() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{scheduler,specialist,budget}};
    let (root,path,id,lease)=multi_agent_test_root("model-unknown-vector",100,3);let db=db::open(&path).unwrap();
    let child=scheduler::schedule_child(&db,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,"unknown-vector",
        &serde_json::json!({}),1,&["evidence.read".into()],60,2).unwrap();scheduler::mark_child_running(&db,&lease,&child).unwrap();
    let specialist::Start::Dispatch(call)=specialist::start(&db,&lease,&child,&serde_json::json!({"messages":[]})).unwrap() else {panic!("claim missing")};
    specialist::record_uncertain(&db,&call,"transport_response_unknown").unwrap();
    for dimension in &budget::DIMENSIONS[..3] {
        let b=budget::balance(&db,&id,Some(&child.assignment_id),dimension).unwrap();
        assert_eq!((b.reserved,b.consumed,b.indeterminate),(0,0,60));
    }
    let b=budget::balance(&db,&id,Some(&child.assignment_id),"model_requests").unwrap();
    assert_eq!((b.reserved,b.consumed,b.indeterminate),(1,0,1),"one sent call, one unused reservation");
    let before=receipt_database_snapshot(&db);
    assert_eq!(scheduler::finish_child(&db,&lease,&child,false,"unknown cannot refund").unwrap_err(),"budget_indeterminate_requires_reconciliation");
    assert_eq!(receipt_database_snapshot(&db),before);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_journal_rechecks_exact_source_and_assignment_after_insert() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{scheduler,budget::{self,Kind}}};
    for (mutation,expected) in [
        ("DROP TRIGGER budget_entry_no_update; CREATE TRIGGER tamper_budget AFTER INSERT ON agent_budget_entries BEGIN UPDATE agent_budget_entries SET source_id='different-receipt' WHERE entry_id=NEW.entry_id; END;","budget_entry_persistence_conflict"),
        ("CREATE TRIGGER tamper_budget AFTER INSERT ON agent_budget_entries BEGIN UPDATE agent_assignments SET fencing_token='replacement' WHERE id=NEW.assignment_id; END;","budget_assignment_scope_conflict"),
    ] {
        let (root,path,_,lease)=multi_agent_test_root("journal-postinsert",100,3);let db=db::open(&path).unwrap();
        let child=scheduler::schedule_child(&db,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,"postinsert",
            &serde_json::json!({}),1,&["evidence.read".into()],60,2).unwrap();
        db.execute_batch(mutation).unwrap();let before=receipt_database_snapshot(&db);
        let tx=db.unchecked_transaction().unwrap();
        let result=budget::append(&tx,&lease,&child.assignment_id,"model_requests",Kind::Consume,1,"receipt-postinsert","actual-receipt");
        tx.rollback().unwrap();assert_eq!(result.unwrap_err(),expected,"{mutation}");
        assert_eq!(receipt_database_snapshot(&db),before);
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn budget_journal_specialist_receipt_consumes_once_before_assignment_finishes() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{scheduler,specialist,budget}};
    let (root,path,id,lease)=multi_agent_test_root("model-receipt-vector",100,3);let db=db::open(&path).unwrap();
    let child=scheduler::schedule_child(&db,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,"received-vector",
        &serde_json::json!({}),1,&["evidence.read".into()],60,2).unwrap();scheduler::mark_child_running(&db,&lease,&child).unwrap();
    let specialist::Start::Dispatch(call)=specialist::start(&db,&lease,&child,&serde_json::json!({"messages":[]})).unwrap() else {panic!()};
    let usage=AgentTokenUsage {input_tokens:30,cached_input_tokens:5,output_tokens:20,total_tokens:50,model_requests:1};
    specialist::record_received(&db,&call,"received",false,&usage).unwrap();
    for (dimension,spent) in budget::DIMENSIONS[..4].iter().zip([30,5,20,1]) {
        let b=budget::balance(&db,&id,Some(&child.assignment_id),dimension).unwrap();assert_eq!(b.consumed,spent,"{dimension}");
    }
    let received=receipt_database_snapshot(&db);
    specialist::record_received(&db,&call,"received",false,&usage).unwrap();assert_eq!(receipt_database_snapshot(&db),received);
    settle_child_usage(&db,&lease,&child,&usage).unwrap();
    for (dimension,spent) in budget::DIMENSIONS[..4].iter().zip([30,5,20,1]) {
        let b=budget::balance(&db,&id,Some(&child.assignment_id),dimension).unwrap();assert_eq!((b.reserved,b.consumed,b.indeterminate),(0,spent,0));
    }
    let settled=receipt_database_snapshot(&db);settle_child_usage(&db,&lease,&child,&usage).unwrap();assert_eq!(receipt_database_snapshot(&db),settled);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_journal_root_terminal_write_rechecks_cost_postconditions() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::scheduler};
    let (root,path,id,lease)=multi_agent_test_root("budget-final-postwrite",100,3);let db=db::open(&path).unwrap();
    let child=scheduler::schedule_child(&db,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,"budget-final",
        &serde_json::json!({}),1,&["evidence.read".into()],10,1).unwrap();
    scheduler::finish_child(&db,&lease,&child,false,"never dispatched").unwrap();
    db.execute_batch(&format!("CREATE TRIGGER inject_unknown AFTER UPDATE OF status ON agent_runs WHEN NEW.id='{id}' AND NEW.status='terminal'
        BEGIN INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id)
        VALUES('injected-reserve','{id}','{}','{}:{}','target_requests','reserve',1,'injected-reserve','injected');
        INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id)
        VALUES('injected-unknown','{id}','{}','{}:{}','target_requests','forfeit',1,'injected-unknown','injected'); END;",
        child.assignment_id,lease.lease_epoch,lease.fencing_token,child.assignment_id,lease.lease_epoch,lease.fencing_token)).unwrap();
    let before=receipt_database_snapshot(&db);
    let all_before=super::tests::application_table_snapshot(&db);
    let physical_before=final_elapsed_physical_rows(&db);
    let outcome=AgentTargetOutcome::Completed(AgentCompletion::without_ledger("complete",terminal_code::LEDGER_COMPLETE));
    // The outer closure writer rejects this untrusted fee trigger before it
    // can create indeterminate cost or publish a terminal receipt.
    assert_eq!(finish_coordinator_run(&db,&lease,&outcome).unwrap_err(),"无法写入 Coordinator 终态：not authorized");
    assert_eq!(receipt_database_snapshot(&db),before);
    assert!(super::tests::application_table_snapshot(&db)==all_before);
    assert_eq!(final_elapsed_physical_rows(&db),physical_before);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_journal_receipt_charges_known_unlimited_overage_without_inventing_unknown_cost() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{scheduler,specialist,budget}};
    let (root,path,id,lease)=multi_agent_test_root("unlimited-receipt-vector",0,3);let db=db::open(&path).unwrap();
    let child=scheduler::schedule_child(&db,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,"unlimited-receipt",
        &serde_json::json!({}),1,&["evidence.read".into()],10,1).unwrap();scheduler::mark_child_running(&db,&lease,&child).unwrap();
    let specialist::Start::Dispatch(call)=specialist::start(&db,&lease,&child,&serde_json::json!({"messages":[]})).unwrap() else {panic!()};
    let usage=AgentTokenUsage {input_tokens:30,cached_input_tokens:5,output_tokens:20,total_tokens:50,model_requests:1};
    specialist::record_received(&db,&call,"known unlimited receipt",false,&usage).unwrap();
    for (dimension,spent) in budget::DIMENSIONS[..4].iter().zip([30,5,20,1]) {
        let b=budget::balance(&db,&id,Some(&child.assignment_id),dimension).unwrap();assert_eq!((b.consumed,b.indeterminate),(spent,0),"{dimension}");
    }
    settle_child_usage(&db,&lease,&child,&usage).unwrap();
    let before=receipt_database_snapshot(&db);specialist::record_received(&db,&call,"known unlimited receipt",false,&usage).unwrap();
    settle_child_usage(&db,&lease,&child,&usage).unwrap();assert_eq!(receipt_database_snapshot(&db),before);
    drop(db);fs::remove_dir_all(root).unwrap();
}
