// Every case drives the actual prepared pipeline; provider answers are scripted.
#[test]
fn client_root_feedback_actual_production_chain_returns_closed_readonly_output_to_root_once() {
    let _real = RealSpecialistTransport::enter();
    let mut f = client_root_fixture("client-root-feedback", false);
    f.finish().unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='client_side' AND state='completed'", [&f.root], |r|r.get::<_,i64>(0)).unwrap(), 1);
    assert_eq!(root_tick_count(&db, &f.root, "publication"), 5);
    assert_eq!(f.root_cost(&db), 5);
    let wires = f.h.model_seen.lock().unwrap();
    let last = wires.last().unwrap();
    let body: JsonValue = serde_json::from_str(last.split_once("\r\n\r\n").unwrap().1).unwrap();
    let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
    let semantic = &input["basis"]["changedFact"]["semantic"];
    assert_eq!(semantic["role"], "client_side");
    assert_eq!(semantic["targetRequestsGranted"], 0);
    assert_eq!(semantic["browserActionsGranted"], 0);
    assert_eq!(semantic["newTargetEvidenceProven"], false);
    assert_eq!(semantic["browserImpactProven"], false);
    assert_eq!(semantic["output"]["candidates"], json!([]));
    assert_eq!(semantic["output"]["gaps"], json!(["missing_browser_validation"]));
    assert!(!semantic["observations"].as_array().unwrap().is_empty());
    assert_eq!(wires.iter().filter(|w|w.contains("You are the Root Coordinator")).count(), 5);
    drop(wires);
    let paid_calls = f.h.model_seen.lock().unwrap().len();
    let costs = client_hook_target_costs(&db);
    let budget = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db, "SELECT rowid,* FROM agent_budget_entries ORDER BY rowid", []).unwrap();
    f.finish().unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), paid_calls);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(client_hook_target_costs(&db), costs);
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db, "SELECT rowid,* FROM agent_budget_entries ORDER BY rowid", []).unwrap() == budget);
    let physical = web_mode_test_rows(&db);
    f.feedback(&db).unwrap();
    web_mode_assert_rows(&db, &physical);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), paid_calls);
}
#[test]
fn client_root_feedback_paid_advisory_cannot_issue_another_target_grant() {
    let _real = RealSpecialistTransport::enter();
    let mut f = client_root_fixture("client-root-forbidden", true);
    assert_eq!(f.finish().unwrap_err(), "executor_followup_failed:client_side:root_client_step_not_bounded");
    let db = db::open(&f.h.db_path).unwrap();
    assert_eq!(f.root_cost(&db), 5);
    assert_eq!(root_tick_count(&db,&f.root,"publication"),5);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE role='web_executor'", [], |r|r.get::<_,i64>(0)).unwrap(), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM sentinel_findings WHERE kind='vulnerability'", [], |r|r.get::<_,i64>(0)).unwrap(), 0);
    let calls = f.h.model_seen.lock().unwrap().len();
    let physical = web_mode_test_rows(&db);
    assert_eq!(f.feedback(&db).unwrap_err(), "root_client_step_not_bounded");
    web_mode_assert_rows(&db,&physical);
    // Even a caller presenting a validated paid Client frame to Rust dispatch
    // policy cannot treat it as Mapper's target-authorizing envelope.
    let session = f.session.as_ref().unwrap();
    let context = native_coordinator_root_context(&f.h.context,&session.lease);
    let tx = rusqlite::Transaction::new_unchecked(&db,rusqlite::TransactionBehavior::Immediate).unwrap();
    let mut frame = NativeCoordinatorFrame::client_side(&tx,&context,&session.lease,&f.child(&db)).unwrap();
    frame.rows.extend(NativeCoordinatorFrame::mapper(&tx,&session.lease,&session.mapper).unwrap().rows);
    tx.commit().unwrap();
    let decision = native_coordinator_tick_for_frame(&context,&session.lease,&frame).unwrap();
    assert_eq!(native_coordinator_rust_dispatch_policy(&context,&frame,&decision,
        crate::agent_runtime::contract::AgentRole::WebExecutor,512,0,[Some(60000),Some(20),Some(32)]).unwrap_err(),
        "root_trigger_role_conflict");
    web_mode_assert_rows(&db,&physical);
    assert_eq!(f.h.model_seen.lock().unwrap().len(),calls);
    assert_eq!(f.h.site_seen.lock().unwrap().len(),1);
}
#[test]
fn client_root_feedback_paid_worker_changed_before_response_retains_fee_and_restores_locally() {
    let _real = RealSpecialistTransport::enter();
    let mut f = client_root_fixture("client-root-worker-change", false);
    let saved = std::sync::Arc::new(std::sync::Mutex::new(None::<(String,String)>));
    let restore = saved.clone();
    *f.boundary.lock().unwrap() = Some(Box::new(move |db| {
        let original:(String,String) = db.query_row("SELECT w.id,w.finished_at FROM agent_assignment_attempts w JOIN agent_runs r ON r.id=w.child_run_id WHERE r.role='client_side'", [], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert!(!original.1.is_empty());
        db.execute("UPDATE agent_assignment_attempts SET finished_at='' WHERE id=?1", [&original.0]).unwrap();
        *restore.lock().unwrap() = Some(original);
    }));
    let code = f.finish().unwrap_err();
    assert_eq!(code,"executor_followup_failed:client_side:root_frame_original_fact_changed");
    let db = db::open(&f.h.db_path).unwrap();
    assert_eq!(f.root_cost(&db),5);
    assert_eq!(root_tick_count(&db,&f.root,"publication"),4);
    let calls = f.h.model_seen.lock().unwrap().len();
    let costs = client_hook_target_costs(&db);
    let (id,finished) = saved.lock().unwrap().take().unwrap();
    db.execute("UPDATE agent_assignment_attempts SET finished_at=?2 WHERE id=?1", params![id,finished]).unwrap();
    f.finish().unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(),calls);
    assert_eq!(f.h.site_seen.lock().unwrap().len(),1);
    assert_eq!(f.root_cost(&db),5);
    assert_eq!(root_tick_count(&db,&f.root,"publication"),5);
    assert_eq!(client_hook_target_costs(&db),costs);
}
#[test]
fn client_root_feedback_paid_original_source_changed_blocks_publication_and_zero_sdk_replay() {
    let _real = RealSpecialistTransport::enter();
    let mut f = client_root_fixture("client-root-source-change", false);
    let saved = std::sync::Arc::new(std::sync::Mutex::new(None::<(PathBuf,Vec<u8>)>));
    let restore = saved.clone();
    let directory = f.h.context.target_dir.clone();
    *f.boundary.lock().unwrap() = Some(Box::new(move |db| {
        let raw:String = db.query_row("SELECT task_slice_json FROM agent_assignments WHERE role='client_side'", [], |r|r.get(0)).unwrap();
        let task:JsonValue = serde_json::from_str(&raw).unwrap();
        let path = directory.join(task["observations"][0]["artifact"]["path"].as_str().unwrap());
        let original = fs::read(&path).unwrap();
        fs::write(&path,b"changed original source after actual paid dispatch").unwrap();
        *restore.lock().unwrap() = Some((path,original));
    }));
    let code = f.finish().unwrap_err();
    assert_eq!(code,"executor_followup_failed:client_side:client_side_original_http_fact_invalid");
    let db = db::open(&f.h.db_path).unwrap();
    assert_eq!(f.root_cost(&db),5);
    assert_eq!(root_tick_count(&db,&f.root,"publication"),4);
    let calls = f.h.model_seen.lock().unwrap().len();
    let physical = web_mode_test_rows(&db);
    assert!(f.feedback(&db).is_err());
    web_mode_assert_rows(&db,&physical);
    assert_eq!(f.h.model_seen.lock().unwrap().len(),calls);
    let (path,original) = saved.lock().unwrap().take().unwrap();
    fs::write(path,original).unwrap();
    f.finish().unwrap();
    assert_eq!(root_tick_count(&db,&f.root,"publication"),5);
    assert_eq!(f.root_cost(&db),5);
    assert_eq!(f.h.model_seen.lock().unwrap().len(),calls);
    assert_eq!(f.h.site_seen.lock().unwrap().len(),1);
}

#[test]
fn client_root_feedback_paid_publication_fault_keeps_client_and_root_bills_for_local_recovery() {
    for action in ["SELECT RAISE(IGNORE)","UPDATE projects SET name=name||'-forbidden'",
        "INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) VALUES('foreign',99,'other','other','other','{}')"] {
        let _real = RealSpecialistTransport::enter();
        let mut f = client_root_fixture("client-root-publication-fault",false);
        *f.boundary.lock().unwrap() = Some(Box::new(move |db| {
            db.execute_batch(&format!("CREATE TRIGGER client_root_publication_fault BEFORE INSERT ON agent_root_tick_timeline_receipts BEGIN {action}; END;")).unwrap();
        }));
        let db = db::open(&f.h.db_path).unwrap();
        let projects = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,"SELECT rowid,* FROM projects ORDER BY rowid",[]).unwrap();
        assert!(f.finish().is_err());
        assert_eq!(f.root_cost(&db),5);
        assert_eq!(root_tick_count(&db,&f.root,"publication"),4);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE role='client_side' AND state='completed'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_collaboration_events WHERE scan_id='foreign'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,"SELECT rowid,* FROM projects ORDER BY rowid",[]).unwrap()==projects);
        let calls = f.h.model_seen.lock().unwrap().len();
        let budget = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries ORDER BY rowid",[]).unwrap();
        let physical = web_mode_test_rows(&db);
        assert!(f.feedback(&db).is_err());
        web_mode_assert_rows(&db,&physical);
        assert_eq!(f.h.model_seen.lock().unwrap().len(),calls);
        db.execute_batch("DROP TRIGGER client_root_publication_fault").unwrap();
        f.finish().unwrap();
        assert_eq!(root_tick_count(&db,&f.root,"publication"),5);
        assert_eq!(f.root_cost(&db),5);
        assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries ORDER BY rowid",[]).unwrap()==budget);
        assert_eq!(f.h.model_seen.lock().unwrap().len(),calls);
        assert_eq!(f.h.site_seen.lock().unwrap().len(),1);
    }
}
#[test]
fn client_root_feedback_original_consumed_message_tampering_refuses_before_sdk_and_changes_no_row() {
    for mutation in ["payload_json='{}'","correlation_id='client-side:altered'","acknowledged_at=''"] {
        let _real = RealSpecialistTransport::enter();
        let mut f = client_root_fixture("client-root-mailbox-tamper",false);
        f.finish().unwrap();
        let db = db::open(&f.h.db_path).unwrap();
        assert_eq!(db.execute(&format!("UPDATE agent_messages SET {mutation} WHERE from_agent='client_side' AND kind='evidence_summary'"),[]).unwrap(),1);
        let physical = web_mode_test_rows(&db);
        let calls = f.h.model_seen.lock().unwrap().len();
        assert!(f.feedback(&db).is_err(),"original message mutation accepted: {mutation}");
        web_mode_assert_rows(&db,&physical);
        assert_eq!(f.h.model_seen.lock().unwrap().len(),calls);
        assert_eq!(f.h.site_seen.lock().unwrap().len(),1);
        assert_eq!(f.root_cost(&db),5);
    }
}
