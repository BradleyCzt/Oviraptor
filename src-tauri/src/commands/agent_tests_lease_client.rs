#[test]
fn lease_client_full_finish_signed_expiry_and_original_task_reentry_zero_sdk() {
    let _real = RealSpecialistTransport::enter();
    CLIENT_EXPIRE_RECEIVED_ONCE.with(|flag|flag.set(true));
    let mut f = input_parser_production_fixture("lease-client-full");
    CLIENT_EXPIRE_RECEIVED_ONCE.with(|flag|assert!(!flag.replace(false)));
    let db = db::open(&f.h.db_path).unwrap();
    let (assignment,run):(String,String) = db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='client_side' AND state='completed'",[&f.root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(db.query_row("SELECT state FROM agent_assignment_attempts WHERE child_run_id=?1",[&run],|r|r.get::<_,String>(0)).unwrap(),"expired");
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {assignment_id:assignment,run_id:run,role:crate::agent_runtime::contract::AgentRole::ClientSide};
    let calls = f.calls();
    let before = super::tests::application_table_snapshot(&db);
    let replay = native_coordinator_lease_expiry_feedback(&f.h.context,f.lease(),&child).unwrap();
    assert!(replay.replayed);
    multi_agent_finish_execution(&f.h.context,f.session.as_mut().unwrap(),&f.outcome).unwrap();
    assert_eq!(f.calls(),calls);
    assert_eq!(super::tests::application_table_snapshot(&db),before);
    assert_eq!(f.h.site_seen.lock().unwrap().len(),1);
    db.execute("DELETE FROM agent_evidence_nodes WHERE root_run_id=?1",[&f.root]).unwrap();
    let damaged = super::tests::application_table_snapshot(&db);
    assert!(multi_agent_client_side_readonly(&f.h.context,f.session.as_ref().unwrap()).is_err(),"existing original task cannot vanish into no-task branch");
    assert_eq!(f.calls(),calls);
    assert_eq!(super::tests::application_table_snapshot(&db),damaged);
    db.execute("UPDATE agent_assignments SET trigger_code='cold-damaged-trigger' WHERE id=?1",[&child.assignment_id]).unwrap();
    let damaged = super::tests::application_table_snapshot(&db);
    assert_eq!(multi_agent_client_side_readonly(&f.h.context,f.session.as_ref().unwrap()).err().as_deref(),Some("client_side_original_trigger_conflict"));
    assert_eq!(f.calls(),calls);
    assert_eq!(super::tests::application_table_snapshot(&db),damaged);
}
