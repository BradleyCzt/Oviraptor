#[test]
fn lease_reviewer_actual_insufficient_signed_expiry_and_complete_review_readonly_replay() {
    EXPIRE_RECEIVED_REVIEWER_ONCE.with(|flag|flag.set(true));
    let mut f=followup_fixture();
    EXPIRE_RECEIVED_REVIEWER_ONCE.with(|flag|assert!(!flag.replace(false)));
    let db=db::open(&f.context.db_path).unwrap();
    let actor=db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",[&f.root_run_id],|row|Ok(crate::agent_runtime::multi_agent::lease::CoordinatorLease {scan_id:row.get(0)?,attempt_number:row.get(1)?,target_key:row.get(2)?,root_run_id:row.get(3)?,lease_epoch:row.get(4)?,fencing_token:row.get(5)?,lease_expires_at:row.get(6)?})).unwrap();
    let child=|role:crate::agent_runtime::contract::AgentRole|db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role=?2",params![f.root_run_id,role.as_str()],|row|Ok(crate::agent_runtime::multi_agent::scheduler::ScheduledChild {assignment_id:row.get(0)?,run_id:row.get(1)?,role})).unwrap();
    let reviewer=child(crate::agent_runtime::contract::AgentRole::EvidenceReviewer);
    let mapper=child(crate::agent_runtime::contract::AgentRole::SpaApiMapper);
    let executor=child(crate::agent_runtime::contract::AgentRole::WebExecutor);
    let supervisor=start_original_fixture_supervisor(&f.context.db_path,&actor).unwrap();
    f.context.supervision=Some(supervisor.ticket());
    let session=MultiAgentSession {lease:actor,supervisor,mapper,executor};
    let root=native_coordinator_root_context(&f.context,&session.lease);
    let (candidate,revision,text):(String,i64,String)=db.query_row("SELECT candidate_id,candidate_revision,candidate_json FROM agent_review_requests WHERE root_run_id=?1",[&f.root_run_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    let before=super::tests::application_table_snapshot(&db);
    let tx=db.unchecked_transaction().unwrap();
    let frame=NativeCoordinatorFrame::lease_closed_mapper(&tx,&root,&session.lease,&reviewer).unwrap();
    assert_eq!(frame.semantic["role"],"evidence_reviewer");
    frame.verify(&tx,&root,&session.lease).unwrap();
    tx.commit().unwrap();
    assert!(super::tests::application_table_snapshot(&db)==before);
    let reviewed=replay_frozen_review(&f.context,&session,&candidate,revision,&text,AgentTargetOutcome::incomplete("original fixture coverage")).unwrap();
    assert!(matches!(reviewed,AgentTargetOutcome::Incomplete(_)));
    assert!(super::tests::application_table_snapshot(&db)==before);
}

#[test]
fn lease_reviewer_original_source_and_captured_frame_reject_cold_drift_without_writes() {
    for mode in ["grant_amount","grant_source","dedup","ack_empty","decision","candidate","finding","event"] {
    EXPIRE_RECEIVED_REVIEWER_ONCE.with(|flag|flag.set(true));
    let mut f=followup_fixture();
    EXPIRE_RECEIVED_REVIEWER_ONCE.with(|flag|assert!(!flag.replace(false)));
    let db=db::open(&f.context.db_path).unwrap();
    let actor=db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",[&f.root_run_id],|row|Ok(crate::agent_runtime::multi_agent::lease::CoordinatorLease {scan_id:row.get(0)?,attempt_number:row.get(1)?,target_key:row.get(2)?,root_run_id:row.get(3)?,lease_epoch:row.get(4)?,fencing_token:row.get(5)?,lease_expires_at:row.get(6)?})).unwrap();
    let child=|role:crate::agent_runtime::contract::AgentRole|db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role=?2",params![f.root_run_id,role.as_str()],|row|Ok(crate::agent_runtime::multi_agent::scheduler::ScheduledChild {assignment_id:row.get(0)?,run_id:row.get(1)?,role})).unwrap();
    let reviewer=child(crate::agent_runtime::contract::AgentRole::EvidenceReviewer);
    let mapper=child(crate::agent_runtime::contract::AgentRole::SpaApiMapper);
    let executor=child(crate::agent_runtime::contract::AgentRole::WebExecutor);
    let supervisor=start_original_fixture_supervisor(&f.context.db_path,&actor).unwrap();
    f.context.supervision=Some(supervisor.ticket());
    let session=MultiAgentSession {lease:actor,supervisor,mapper,executor};
    let root=native_coordinator_root_context(&f.context,&session.lease);
    let (_candidate,_revision,_text):(String,i64,String)=db.query_row("SELECT candidate_id,candidate_revision,candidate_json FROM agent_review_requests WHERE root_run_id=?1",[&f.root_run_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    let tx=db.unchecked_transaction().unwrap();
    let frame=NativeCoordinatorFrame::lease_closed_mapper(&tx,&root,&session.lease,&reviewer).unwrap();
    tx.commit().unwrap();
    match mode {
        "grant_amount" | "grant_source" => {
            // Simulate cold storage damage in this disposable fixture only;
            // restore the exact original trigger before exercising either reader.
            let trigger:String=db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='budget_entry_no_update'",[],|row|row.get(0)).unwrap();
            db.execute_batch("DROP TRIGGER budget_entry_no_update").unwrap();
            let sql=if mode=="grant_source" {
                "UPDATE agent_budget_entries SET source_id='changed-original-grant-source' WHERE assignment_id=?1 AND kind='reserve' AND dimension='model_input_tokens'"
            } else {
                "UPDATE agent_budget_entries SET amount=amount+1 WHERE assignment_id=?1 AND kind='reserve' AND dimension='model_input_tokens'"
            };
            assert_eq!(db.execute(sql,[&reviewer.assignment_id]).unwrap(),1);
            db.execute_batch(&trigger).unwrap();
        },
        "dedup" => {assert_eq!(db.execute("UPDATE agent_messages SET dedup_key='changed-original-review-key' WHERE assignment_id=?1 AND kind='review_decision'",[&reviewer.assignment_id]).unwrap(),1);},
        "ack_empty" => {assert_eq!(db.execute("UPDATE agent_messages SET acknowledged_at='' WHERE assignment_id=?1 AND kind='review_decision'",[&reviewer.assignment_id]).unwrap(),1);},
        "decision" => {assert_eq!(db.execute("UPDATE agent_review_decisions SET confidence=0.9 WHERE reviewer_run_id=?1",[&reviewer.run_id]).unwrap(),1);},
        "candidate" => {assert_eq!(db.execute("UPDATE agent_review_requests SET candidate_json=json_set(candidate_json,'$.outcome','changed source') WHERE assignment_id=?1",[&reviewer.assignment_id]).unwrap(),1);},
        "finding" => {assert_eq!(db.execute("UPDATE agent_finding_candidates SET status='rejected' WHERE reviewer_run_id=?1",[&reviewer.run_id]).unwrap(),1);},
        "event" => {assert_eq!(db.execute("INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) SELECT scan_id,attempt_number+1,entity_type,entity_id,event_type,payload_json FROM agent_collaboration_events WHERE entity_id=(SELECT id FROM agent_messages WHERE assignment_id=?1 AND kind='review_decision') LIMIT 1",[&reviewer.assignment_id]).unwrap(),1);},
        _ => unreachable!(),
    }
    let changed=super::tests::application_table_snapshot(&db);
    let tx=db.unchecked_transaction().unwrap();
    let error=NativeCoordinatorFrame::lease_closed_mapper(&tx,&root,&session.lease,&reviewer).err().unwrap_or_else(||panic!("new source accepted {mode}"));
    if mode=="grant_source" {assert_eq!(error,"root_lease_reviewer_original_grant_conflict");}

    assert!(frame.verify(&tx,&root,&session.lease).is_err(),"captured source accepted {mode}");
    tx.commit().unwrap();
    assert!(super::tests::application_table_snapshot(&db)==changed,"reader wrote {mode}");
    }
}
