#[test]
fn review_completed_replay_rejects_changed_summary_and_extra_payload_without_writes() {
    for mode in ["summary","extra","reasons","confidence"] {
        let f = followup_fixture();
        let db = db::open(&f.context.db_path).unwrap();
        let (candidate,revision,text,message):(String,i64,String,String)=db.query_row(
            "SELECT q.candidate_id,q.candidate_revision,q.candidate_json,m.id FROM agent_review_requests q JOIN agent_messages m ON m.assignment_id=q.assignment_id AND m.correlation_id=q.id AND m.kind='review_decision' WHERE q.root_run_id=?1",
            [&f.root_run_id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).unwrap();
        let actor=db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",[&f.root_run_id],|row|Ok(crate::agent_runtime::multi_agent::lease::CoordinatorLease {scan_id:row.get(0)?,attempt_number:row.get(1)?,target_key:row.get(2)?,root_run_id:row.get(3)?,lease_epoch:row.get(4)?,fencing_token:row.get(5)?,lease_expires_at:row.get(6)?})).unwrap();
        let supervisor=start_original_fixture_supervisor(&f.context.db_path,&actor).unwrap();
        let child=|role:crate::agent_runtime::contract::AgentRole| db.query_row(
            "SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role=?2",
            params![f.root_run_id,role.as_str()],|row|Ok(crate::agent_runtime::multi_agent::scheduler::ScheduledChild {assignment_id:row.get(0)?,run_id:row.get(1)?,role}),
        ).unwrap();
        let mapper=child(crate::agent_runtime::contract::AgentRole::SpaApiMapper);
        let executor=child(crate::agent_runtime::contract::AgentRole::WebExecutor);
        let session=MultiAgentSession {lease:actor,supervisor,mapper,executor};
        match mode {
            "summary" => {db.execute("UPDATE agent_messages SET payload_json=json_set(payload_json,'$.summary','changed paid summary') WHERE id=?1",[&message]).unwrap();},
            "extra" => {db.execute("UPDATE agent_messages SET payload_json=json_set(payload_json,'$.dispatchGranted',1) WHERE id=?1",[&message]).unwrap();},
            "reasons" => {db.execute("UPDATE agent_review_decisions SET reason_codes_json='[\"changed_reason\"]' WHERE root_run_id=?1",[&f.root_run_id]).unwrap();},
            "confidence" => {db.execute("UPDATE agent_review_decisions SET confidence=0.9 WHERE root_run_id=?1",[&f.root_run_id]).unwrap();},
            _ => unreachable!(),
        }
        let before=super::tests::application_table_snapshot(&db);
        let result=replay_frozen_review(&f.context,&session,&candidate,revision,&text,AgentTargetOutcome::incomplete("original fixture coverage"));
        assert!(matches!(result,Err(ref code) if code=="review_replay_original_decision_conflict"),"{result:?}");
        assert!(super::tests::application_table_snapshot(&db)==before);
    }
}
