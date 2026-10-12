#[test]
fn lease_input_parser_actual_readonly_expiry_delivery_paid_assessment_and_zero_replay() {
    lease_input_parser_lifecycle(false);
}

#[test]
fn lease_input_parser_delivery_before_failed_publication_reentry_uses_original_paid_result() {
    lease_input_parser_lifecycle(true);
}

fn lease_input_parser_lifecycle(publication_fault: bool) {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_root_fixture("lease-ip-real", &["dispatch:input_parser"]);
    let child = input_parser_sdk_mint(&f);
    let db = db::open(&f.h.db_path).unwrap();
    let calls = f.calls();
    let publications = root_tick_count(&db,&f.root,"publication");
    if publication_fault {
        db.execute_batch("CREATE TRIGGER lease_ip_publication_fault BEFORE INSERT ON agent_root_tick_receipts WHEN NEW.phase='publication' BEGIN SELECT RAISE(ABORT,'lease_ip_publication_fault'); END;").unwrap();
    }
    INPUT_PARSER_EXPIRE_RECEIVED_ONCE.with(|flag|flag.set(true));
    let result = multi_agent_input_parser_readonly(&f.h.context,f.lease(),&child);
    INPUT_PARSER_EXPIRE_RECEIVED_ONCE.with(|flag|flag.set(false));
    assert_eq!(f.calls(),calls+2,"actual readonly InputParser and original Root lease assessment");
    if publication_fault {assert!(result.is_err());} else {result.unwrap();}
    assert_eq!(crate::agent_runtime::multi_agent::attempts::current(&db,f.lease(),&child.assignment_id).unwrap().state,"expired");
    let before = super::tests::application_table_snapshot(&db);
    assert!(input_parser_completed_original_replay(&db,f.lease(),&child,&f.h.context.target_dir).unwrap());
    assert_eq!(super::tests::application_table_snapshot(&db),before);
    if publication_fault {
        assert!(multi_agent_input_parser_readonly(&f.h.context,f.lease(),&child).is_err());
        assert_eq!(f.calls(),calls+2);
        assert_eq!(super::tests::application_table_snapshot(&db),before);
        db.execute_batch("DROP TRIGGER lease_ip_publication_fault").unwrap();
        let fees = client_delivery_rows(&db,"agent_budget_entries");
        multi_agent_input_parser_readonly(&f.h.context,f.lease(),&child).unwrap();
        assert_eq!(client_delivery_rows(&db,"agent_budget_entries"),fees);
    }
    assert_eq!(root_tick_count(&db,&f.root,"publication"),publications+1);
    let paid = super::tests::application_table_snapshot(&db);
    multi_agent_input_parser_readonly(&f.h.context,f.lease(),&child).unwrap();
    assert_eq!(f.calls(),calls+2);
    assert_eq!(super::tests::application_table_snapshot(&db),paid);
    assert_eq!(f.h.site_seen.lock().unwrap().len(),1,"no new target transport");
    for (damage,restore) in [
        ("UPDATE agent_runs SET used_cached_tokens=used_cached_tokens+1 WHERE role='input_parser'",
         "UPDATE agent_runs SET used_cached_tokens=used_cached_tokens-1 WHERE role='input_parser'"),
        ("UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.grant.reservedModelTokens',4001) WHERE role='input_parser'",
         "UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.grant.reservedModelTokens',4000) WHERE role='input_parser'"),
        ("UPDATE agent_messages SET acknowledged_at='' WHERE kind='input_assessment'", ""),
    ] {
        db.execute_batch(damage).unwrap();
        let damaged = super::tests::application_table_snapshot(&db);
        assert!(multi_agent_input_parser_readonly(&f.h.context,f.lease(),&child).is_err());
        assert_eq!(f.calls(),calls+2);
        assert_eq!(super::tests::application_table_snapshot(&db),damaged);
        if !restore.is_empty() {
            db.execute_batch(restore).unwrap();
            multi_agent_input_parser_readonly(&f.h.context,f.lease(),&child).unwrap();
            assert_eq!(f.calls(),calls+2,"each next negative starts from valid original source");
        }
    }
}

#[test]
fn lease_input_parser_full_finish_natural_expiry_runs_original_paid_hook_once() {
    let _real = RealSpecialistTransport::enter();
    INPUT_PARSER_EXPIRE_RECEIVED_ONCE.with(|flag|flag.set(true));
    let mut f = input_parser_production_fixture("lease-ip-full-finish");
    INPUT_PARSER_EXPIRE_RECEIVED_ONCE.with(|flag|assert!(!flag.replace(false),"actual finish must consume original expiry boundary"));
    let db = db::open(&f.h.db_path).unwrap();
    let (assignment,run):(String,String) = db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='input_parser' AND state='completed'",[&f.root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {assignment_id:assignment,run_id:run,role:crate::agent_runtime::contract::AgentRole::InputParser};
    assert_eq!(crate::agent_runtime::multi_agent::attempts::current(&db,f.lease(),&child.assignment_id).unwrap().state,"expired");
    assert!(input_parser_completed_original_replay(&db,f.lease(),&child,&f.h.context.target_dir).unwrap());
    let wires = f.h.model_seen.lock().unwrap().clone();
    let lease_calls = wires.iter().filter(|wire| {
        if !wire.contains("You are the Root Coordinator") {return false;}
        let body:JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
        let input:JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        input["basis"]["phase"] == "lease-expired"
    }).count();
    assert_eq!(lease_calls,1,"one actual original Root lease SDK in full finish");
    assert_eq!(wires.iter().filter(|w|w.contains("你是独立 InputParser 只读输入面专家")).count(),1);
    assert_eq!(f.h.site_seen.lock().unwrap().len(),1);
    let before = super::tests::application_table_snapshot(&db);
    let calls = f.calls();
    multi_agent_input_parser_readonly(&f.h.context,f.lease(),&child).unwrap();
    assert_eq!(f.calls(),calls);
    assert_eq!(super::tests::application_table_snapshot(&db),before);
    multi_agent_finish_execution(&f.h.context,f.session.as_mut().unwrap(),&f.outcome).unwrap();
    assert_eq!(f.calls(),calls,"full finish reentry adds no original SDK call");
    assert_eq!(super::tests::application_table_snapshot(&db),before);
    crate::agent_runtime::multi_agent::budget::root::require_child_capacity(&db,&f.root,15_000,1).unwrap();
}

#[test]
fn lease_input_parser_original_grant_drift_during_root_sdk_keeps_fees_without_publication_or_resend() {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_root_fixture("lease-ip-grant-drift", &["dispatch:input_parser"]);
    let child = input_parser_sdk_mint(&f);
    let db = db::open(&f.h.db_path).unwrap();
    let calls = f.calls();
    let root_requests = f.requests(&db).consumed;
    let publications = root_tick_count(&db,&f.root,"publication");
    let original = child.assignment_id.clone();
    *f.boundary.lock().unwrap() = Some(Box::new(move |db| {
        db.execute("UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.grant.reservedModelTokens',4001) WHERE id=?1",[&original]).unwrap();
    }));
    INPUT_PARSER_EXPIRE_RECEIVED_ONCE.with(|flag|flag.set(true));
    let result = multi_agent_input_parser_readonly(&f.h.context,f.lease(),&child);
    INPUT_PARSER_EXPIRE_RECEIVED_ONCE.with(|flag|flag.set(false));
    assert!(result.is_err());
    assert_eq!(f.calls(),calls+2);
    assert_eq!(f.requests(&db).consumed,root_requests+1,"the original Root assessment bill remains consumed");
    assert_eq!(crate::agent_runtime::multi_agent::budget::balance(&db,&f.root,Some(&child.assignment_id),"model_requests").unwrap().consumed,1);
    assert_eq!(root_tick_count(&db,&f.root,"publication"),publications);
    let paid = super::tests::application_table_snapshot(&db);
    assert!(multi_agent_input_parser_readonly(&f.h.context,f.lease(),&child).is_err());
    assert_eq!(f.calls(),calls+2);
    assert_eq!(super::tests::application_table_snapshot(&db),paid);
    assert_eq!(f.h.site_seen.lock().unwrap().len(),1);
}
