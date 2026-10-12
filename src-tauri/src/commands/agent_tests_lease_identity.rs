#[test]
fn lease_identity_actual_prepare_signed_expiry_paid_metadata_and_zero_replay() {
    let _real = RealSpecialistTransport::enter();
    let boundary = std::sync::Arc::new(std::sync::Mutex::new(None));
    let (mut f,seen,stop) = identity_root_fixture("lease-identity-real",false,false,boundary);
    EXPIRE_RECEIVED_IDENTITY_ONCE.with(|flag|flag.set(true));
    let session = multi_agent_prepare(&mut f.context).unwrap();
    EXPIRE_RECEIVED_IDENTITY_ONCE.with(|flag|assert!(!flag.replace(false)));
    let db = db::open(&f.context.db_path).unwrap();
    let child = identity_root_child(&db,&f.actor.root_run_id);
    assert_eq!(crate::agent_runtime::multi_agent::attempts::current(&db,&session.lease,&child.assignment_id).unwrap().state,"expired");
    assert_eq!(seen.lock().unwrap().len(),6,"bootstrap/Mapper/handoff/Identity/lease/identity feedback");
    let before = super::tests::application_table_snapshot(&db);
    let replay = native_coordinator_lease_expiry_feedback(&f.context,&session.lease,&child).unwrap();
    assert!(replay.replayed);
    assert_eq!(seen.lock().unwrap().len(),6);
    assert_eq!(super::tests::application_table_snapshot(&db),before);
    db.execute("UPDATE browser_auth_sessions SET name='renamed-valid-identity' WHERE owner_scan_id=?1",[&f.context.scan_id]).unwrap();
    let renamed = super::tests::application_table_snapshot(&db);
    assert!(native_coordinator_lease_expiry_feedback(&f.context,&session.lease,&child).is_err(),"same physical withdrawal receipt cannot issue another paid round after metadata drift");
    assert_eq!(seen.lock().unwrap().len(),6);
    assert_eq!(super::tests::application_table_snapshot(&db),renamed);
    db.execute("UPDATE browser_auth_sessions SET name='Identity One' WHERE owner_scan_id=?1",[&f.context.scan_id]).unwrap();
    let restored = super::tests::application_table_snapshot(&db);
    let replay = native_coordinator_lease_expiry_feedback(&f.context,&session.lease,&child).unwrap();
    assert!(replay.replayed);
    assert_eq!(seen.lock().unwrap().len(),6);
    assert_eq!(super::tests::application_table_snapshot(&db),restored);
    db.execute("UPDATE browser_auth_sessions SET status='invalid' WHERE owner_scan_id=?1",[&f.context.scan_id]).unwrap();
    let changed = super::tests::application_table_snapshot(&db);
    assert!(native_coordinator_lease_expiry_feedback(&f.context,&session.lease,&child).is_err());
    assert_eq!(seen.lock().unwrap().len(),6);
    assert_eq!(super::tests::application_table_snapshot(&db),changed);
    drop(session);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn lease_review_entry_cannot_renew_original_expired_or_replaced_coordinator() {
    for (mutation, downgrade) in [("lease_expires_at='2000-01-01 00:00:00'",false), ("fencing_token='replacement-owner'",false), ("lease_expires_at='2000-01-01 00:00:00'",true)] {
        let _real = RealSpecialistTransport::enter();
        let boundary = std::sync::Arc::new(std::sync::Mutex::new(None));
        let (mut f,seen,stop) = identity_root_fixture("lease-review-original-c",false,false,boundary);
        let mut session = multi_agent_prepare(&mut f.context).unwrap();
        let outcome = run_native_agent(&f.context);
        assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{}",outcome.detail());
        multi_agent_finish_execution(&f.context,&mut session,&outcome).unwrap();
        let db = db::open(&f.context.db_path).unwrap();
        db.execute(&format!("UPDATE agent_coordinator_leases SET {mutation} WHERE root_run_id=?1"),[&session.lease.root_run_id]).unwrap();
        if downgrade {
            let text:String=db.query_row("SELECT definition_json FROM agent_root_mode_definitions WHERE root_run_id=?1",[&session.lease.root_run_id],|r|r.get(0)).unwrap();
            let mut definition:JsonValue=serde_json::from_str(&text).unwrap();
            definition.as_object_mut().unwrap().remove("leaseExpirySupervision");
            let typed:crate::agent_runtime::web_mode::root::NewRootModeDeclaration=serde_json::from_value(definition).unwrap();
            let canonical=serde_json::to_string(&typed).unwrap();
            // Cold corruption of this disposable fixture; original private
            // birth signature stays intact and cannot be replaced.
            db.execute_batch("DROP TRIGGER root_mode_immutable_update").unwrap();
            db.execute("UPDATE agent_root_mode_definitions SET definition_json=?2 WHERE root_run_id=?1",params![session.lease.root_run_id,canonical]).unwrap();
            assert!(crate::agent_runtime::web_mode::root::read(&db,&session.lease.root_run_id).unwrap().unwrap().lease_expiry_supervision().is_none());
        }
        let before = super::tests::application_table_snapshot(&db);
        let calls = seen.lock().unwrap().len();
        let reviewed = multi_agent_review(&f.context,&mut session,outcome);
        assert!(matches!(reviewed,AgentTargetOutcome::Failed(_) | AgentTargetOutcome::Incomplete(_) | AgentTargetOutcome::Cancelled),"{}",reviewed.detail());
        assert_eq!(seen.lock().unwrap().len(),calls);
        assert!(super::tests::application_table_snapshot(&db)==before,"review entry must preserve original C without renewal or adoption");
        drop(session);
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
    }
}
