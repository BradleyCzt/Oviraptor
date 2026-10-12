#[test]
fn lease_paid_actual_bootstrap_mapper_signed_expiry_delivery_assessment_and_zero_replay() {
    lease_paid_actual_lifecycle("normal");
}

#[test]
fn lease_paid_wrong_step_keeps_original_bill_without_publication_or_resend() {
    lease_paid_actual_lifecycle("wrong_step");
}

fn lease_paid_actual_lifecycle(mode: &'static str) {
    let _real = RealSpecialistTransport::enter();
    let drift = std::sync::Arc::new(std::sync::Mutex::new(None::<(std::path::PathBuf,String)>));
    let on_response = drift.clone();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |request| {
        let response = if request.contains("You are the Root Coordinator") && request.contains("lease-expired") {
            if mode == "source_drift" {
                let guard = on_response.lock().unwrap();
                let (path,child) = guard.as_ref().unwrap();
                db::open(path).unwrap().execute("UPDATE agent_messages SET acknowledged_at='' WHERE from_run_id=?1 AND kind='evidence_summary'",[child]).unwrap();
            }
            let reply = proposal_model_response(&json!({"schemaVersion":1,"observed":["original signed expiry and saved invoice"],"missing":[],"suggestions":[if mode == "wrong_step" { "dispatch:mapper" } else { "assess:lease_expired" }],"costNotes":[],"risks":[]}).to_string());
            if mode == "unknown_usage" {
                let mut response: JsonValue = serde_json::from_str(&reply).unwrap();
                response.as_object_mut().unwrap().remove("usage");
                response.to_string()
            } else { reply }
        } else { changed_fact_response(&request, true) };
        (200, "application/json", response)
    }));
    let mut f = root_tick_fixture("lease-paid-real", &format!("http://127.0.0.1:{port}/v1"));
    let bootstrap = native_coordinator_bootstrap(&f.context, &f.actor).unwrap();
    let child = native_coordinator_prepare_mapper(&f.context, &f.actor, &bootstrap,
        &json!({"target":f.actor.target_key,"objective":"original readonly mapping"})).unwrap();
    let (text, usage) = multi_agent_child_round_transport(&f.context, &f.actor, &child,
        "independent SPA/API Mapper", json!({"frozen":true})).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
    let db = db::open(&f.context.db_path).unwrap();
    db.execute("UPDATE agent_assignment_attempts SET expires_at=datetime('now','localtime','+1 second') WHERE child_run_id=?1", [&child.run_id]).unwrap();
    let deadline = std::time::Instant::now()+Duration::from_secs(5);
    while crate::agent_runtime::multi_agent::attempts::current(&db,&f.actor,&child.assignment_id).unwrap().state != "expired" {
        f.parent.as_ref().unwrap().check().unwrap();
        assert!(std::time::Instant::now()<deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    deliver_readonly_assessment(&db,&f.actor,&child,&usage,&json!({"summary":text}),None).unwrap();
    #[cfg(unix)]
    if mode == "process_kill" {
        lease_paid_kill_and_recover(&mut f,&child,&seen);
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
        return;
    }
    let original_ack: String = db.query_row("SELECT acknowledged_at FROM agent_messages WHERE from_run_id=?1 AND kind='evidence_summary'",[&child.run_id],|r|r.get(0)).unwrap();
    if mode.starts_with("authority_before_") || mode.starts_with("authority_paid_") {
        let sql = if mode.ends_with("cancel") {
            "UPDATE agent_runs SET cancel_requested_at='2000-01-01 00:00:00' WHERE id=?1"
        } else if mode.ends_with("successor") {
            "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='fixture-successor' WHERE root_run_id=?1"
        } else {
            "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1"
        };
        let publications = root_tick_count(&db,&f.actor.root_run_id,"publication");
        if mode.starts_with("authority_before_") {
            db.execute(sql,[&f.actor.root_run_id]).unwrap();
        } else {
            let path = f.context.db_path.clone();
            let root = f.actor.root_run_id.clone();
            ROOT_TICK_AFTER_PAID_TEST_BOUNDARY.with(|slot|*slot.borrow_mut()=Some(Box::new(move || {
                db::open(&path).unwrap().execute(sql,[&root]).unwrap();
            })));
        }
        let before = super::tests::application_table_snapshot(&db);
        let result = native_coordinator_lease_expiry_feedback(&f.context,&f.actor,&child);
        ROOT_TICK_AFTER_PAID_TEST_BOUNDARY.with(|slot|slot.borrow_mut().take());
        assert!(result.is_err(),"{mode}");
        let calls = if mode.starts_with("authority_before_") {2} else {3};
        assert_eq!(seen.lock().unwrap().len(),calls);
        assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),publications);
        assert_eq!(crate::agent_runtime::multi_agent::budget::balance(&db,&f.actor.root_run_id,None,"model_requests").unwrap().consumed,calls as i64);
        if calls==2 {assert_eq!(super::tests::application_table_snapshot(&db),before);}
        let stopped = super::tests::application_table_snapshot(&db);
        assert!(native_coordinator_lease_expiry_feedback(&f.context,&f.actor,&child).is_err());
        assert_eq!(seen.lock().unwrap().len(),calls);
        assert_eq!(super::tests::application_table_snapshot(&db),stopped);
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
        return;
    }
    if mode == "missing_ack" {
        db.execute("UPDATE agent_messages SET acknowledged_at='' WHERE from_run_id=?1 AND kind='evidence_summary'",[&child.run_id]).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(native_coordinator_lease_expiry_feedback(&f.context,&f.actor,&child).is_err());
        assert_eq!(seen.lock().unwrap().len(),2);
        assert_eq!(super::tests::application_table_snapshot(&db),before);
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
        return;
    }
    if mode == "source_drift" || mode == "unknown_usage" {
        *drift.lock().unwrap() = Some((f.context.db_path.clone(),child.run_id.clone()));
        let publications = root_tick_count(&db,&f.actor.root_run_id,"publication");
        let result = native_coordinator_lease_expiry_feedback(&f.context,&f.actor,&child);
        assert!(result.is_err());
        if mode == "unknown_usage" {assert_eq!(result.err().as_deref(),Some("budget_indeterminate_requires_reconciliation"));}
        assert_eq!(seen.lock().unwrap().len(),3);
        assert_eq!(crate::agent_runtime::multi_agent::budget::balance(&db,&f.actor.root_run_id,None,"model_requests").unwrap().consumed,3);
        assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),publications);
        let paid = super::tests::application_table_snapshot(&db);
        assert!(native_coordinator_lease_expiry_feedback(&f.context,&f.actor,&child).is_err());
        assert_eq!(seen.lock().unwrap().len(),3);
        assert_eq!(super::tests::application_table_snapshot(&db),paid);
        if mode == "source_drift" {
            db.execute("UPDATE agent_messages SET acknowledged_at=?2 WHERE from_run_id=?1 AND kind='evidence_summary'",params![child.run_id,original_ack]).unwrap();
            let fees = client_delivery_rows(&db,"agent_budget_entries");
            // Updating ACK again emits a new physical mailbox event. Restoring
            // one column does not restore the original closed source identity.
            let restored = super::tests::application_table_snapshot(&db);
            assert_eq!(native_coordinator_lease_expiry_feedback(&f.context,&f.actor,&child).err().as_deref(),Some("root_lease_closed_original_delivery_events_invalid"));
            assert_eq!(seen.lock().unwrap().len(),3);
            assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),publications);
            assert_eq!(client_delivery_rows(&db,"agent_budget_entries"),fees);
            assert_eq!(super::tests::application_table_snapshot(&db),restored);
        } else {
            assert!(crate::agent_runtime::multi_agent::budget::balance(&db,&f.actor.root_run_id,Some(""),"model_input_tokens").unwrap().indeterminate>0);
        }
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
        return;
    }
    if mode == "publication_abort" || mode == "publication_ignore" {
        let publications = root_tick_count(&db,&f.actor.root_run_id,"publication");
        let sql = if mode == "publication_abort" {
            "CREATE TRIGGER lease_test_publication_fault BEFORE INSERT ON agent_root_tick_receipts WHEN NEW.phase='publication' BEGIN SELECT RAISE(ABORT,'lease_paid_publication_fault'); END;"
        } else {
            "CREATE TRIGGER lease_test_publication_fault BEFORE INSERT ON agent_root_tick_receipts WHEN NEW.phase='publication' BEGIN SELECT RAISE(IGNORE); END;"
        };
        db.execute_batch(sql).unwrap();
        assert!(native_coordinator_lease_expiry_feedback(&f.context,&f.actor,&child).is_err());
        assert_eq!(seen.lock().unwrap().len(),3);
        assert_eq!(crate::agent_runtime::multi_agent::budget::balance(&db,&f.actor.root_run_id,None,"model_requests").unwrap().consumed,3);
        assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),publications);
        let fees = client_delivery_rows(&db,"agent_budget_entries");
        let paid = super::tests::application_table_snapshot(&db);
        assert!(native_coordinator_lease_expiry_feedback(&f.context,&f.actor,&child).is_err());
        assert_eq!(seen.lock().unwrap().len(),3);
        assert_eq!(super::tests::application_table_snapshot(&db),paid);
        db.execute_batch("DROP TRIGGER lease_test_publication_fault").unwrap();
        let recovered = native_coordinator_lease_expiry_feedback(&f.context,&f.actor,&child).unwrap();
        assert!(recovered.replayed);
        assert_eq!(seen.lock().unwrap().len(),3);
        assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),publications+1);
        assert_eq!(client_delivery_rows(&db,"agent_budget_entries"),fees);
        let published = super::tests::application_table_snapshot(&db);
        let replay = native_coordinator_lease_expiry_feedback(&f.context,&f.actor,&child).unwrap();
        assert_eq!(replay.event_sequence,recovered.event_sequence);
        assert_eq!(super::tests::application_table_snapshot(&db),published);
        assert_eq!(seen.lock().unwrap().len(),3);
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
        return;
    }
    if mode == "wrong_step" {
        let publications: i64 = db.query_row("SELECT count(*) FROM agent_root_tick_receipts WHERE phase='publication'", [], |r|r.get(0)).unwrap();
        assert_eq!(native_coordinator_lease_expiry_feedback(&f.context,&f.actor,&child).err().as_deref(),Some("root_lease_expired_step_not_bounded"));
        assert_eq!(seen.lock().unwrap().len(),3);
        assert_eq!(crate::agent_runtime::multi_agent::budget::balance(&db,&f.actor.root_run_id,None,"model_requests").unwrap().consumed,3);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts WHERE phase='publication'", [], |r|r.get::<_,i64>(0)).unwrap(),publications);
        let before = super::tests::application_table_snapshot(&db);
        assert_eq!(native_coordinator_lease_expiry_feedback(&f.context,&f.actor,&child).err().as_deref(),Some("root_lease_expired_step_not_bounded"));
        assert_eq!(seen.lock().unwrap().len(),3);
        assert_eq!(super::tests::application_table_snapshot(&db),before);
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
        return;
    }
    let paid = native_coordinator_lease_expiry_feedback(&f.context,&f.actor,&child).unwrap();
    assert!(!paid.replayed);
    assert_eq!(seen.lock().unwrap().len(),3);
    let before = super::tests::application_table_snapshot(&db);
    let replay = native_coordinator_lease_expiry_feedback(&f.context,&f.actor,&child).unwrap();
    assert!(replay.replayed);
    assert_eq!(paid.event_sequence,replay.event_sequence);
    assert_eq!(seen.lock().unwrap().len(),3);
    assert_eq!(super::tests::application_table_snapshot(&db),before);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn lease_paid_full_prepare_original_signed_expiry_runs_foreground_hook() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|request| {
        let response = if request.contains("You are the Root Coordinator") && request.contains("lease-expired") {
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original signed expiry"],"missing":[],"suggestions":["assess:lease_expired"],"costNotes":[],"risks":[]}).to_string())
        } else if request.contains("You are the Root Coordinator") || request.contains("SPA/API Mapper") {changed_fact_response(&request,true)}
        else {model_round(&[],10)};
        (200,"application/json",response)
    }));
    let fixture = web_mode_fixture("multi","https://authorized.example.test");
    let mut f = root_tick_fixture_from_web_mode_turns("lease-full-prepare",&format!("http://127.0.0.1:{port}/v1"),true,(60_000,20),None,fixture,Some(1));
    drop(f.parent.take());
    EXPIRE_RECEIVED_MAPPER_ONCE.with(|flag|flag.set(true));
    let result = multi_agent_prepare(&mut f.context);
    EXPIRE_RECEIVED_MAPPER_ONCE.with(|flag|flag.set(false));
    let mut session = result.unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(seen.lock().unwrap().len(),4);
    assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),3);
    let before = super::tests::application_table_snapshot(&db);
    let replay = native_coordinator_lease_expiry_feedback(&f.context,&session.lease,&session.mapper).unwrap();
    assert!(replay.replayed);
    assert_eq!(seen.lock().unwrap().len(),4);
    assert_eq!(super::tests::application_table_snapshot(&db),before);
    let outcome = run_native_agent(&f.context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{}",outcome.detail());
    multi_agent_finish_execution(&f.context,&mut session,&outcome).unwrap();
    let observed = seen.lock().unwrap();
    assert_eq!(observed.len(),7);
    let mut q=db.prepare("SELECT call_id,COALESCE(json_extract(fact_json,'$.request.basis.phase'),'bootstrap'),(SELECT count(*) FROM agent_root_model_journal j WHERE j.call_id=t.call_id AND j.phase='dispatch'),(SELECT count(*) FROM agent_root_model_journal j WHERE j.call_id=t.call_id AND j.phase='received'),(SELECT count(*) FROM agent_root_tick_receipts p WHERE p.call_id=t.call_id AND p.phase='publication'),(SELECT COALESCE(sum(amount),0) FROM agent_budget_entries e WHERE e.root_run_id=t.root_run_id AND e.assignment_id='' AND e.source_id=t.call_id AND e.dimension='model_requests' AND e.kind='consume') FROM agent_root_tick_receipts t WHERE t.root_run_id=?1 AND t.phase='request' ORDER BY round").unwrap();
    let root_calls=q.query_map([&f.actor.root_run_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,i64>(5)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let mut q=db.prepare("SELECT a.role,a.id,(SELECT COALESCE(sum(amount),0) FROM agent_budget_entries e WHERE e.root_run_id=a.coordinator_run_id AND e.assignment_id=a.id AND e.dimension='model_requests' AND e.kind='consume'),(SELECT count(*) FROM agent_specialist_calls s WHERE s.assignment_id=a.id AND s.state='received'),(SELECT count(*) FROM agent_web_model_journal w WHERE w.assignment_id=a.id AND w.phase='received') FROM agent_assignments a WHERE a.coordinator_run_id=?1 ORDER BY a.role,a.id").unwrap();
    let child_calls=q.query_map([&f.actor.root_run_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert_eq!(root_calls.iter().map(|row|row.1.as_str()).collect::<Vec<_>>(), vec!["bootstrap","lease-expired","mapper-output","budget-allocation","budget-consumption"]);
    let mut unique_calls=std::collections::BTreeSet::new();
    for (call, _, dispatch, received, publication, requests) in &root_calls {
        assert!(unique_calls.insert(call));
        assert_eq!((*dispatch,*received,*publication,*requests),(1,1,1,1));
        let receipts:(i64,i64,i64)=db.query_row("SELECT sum(phase='request'),sum(phase='decision'),sum(phase='publication') FROM agent_root_tick_receipts WHERE root_run_id=?1 AND call_id=?2",params![f.actor.root_run_id,call],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(receipts,(1,1,1));
    }
    assert_eq!(child_calls.iter().map(|row|(row.0.as_str(),row.2,row.3,row.4)).collect::<Vec<_>>(),vec![("spa_api_mapper",1,1,0),("web_executor",1,0,1)]);
    assert_eq!(root_calls.iter().map(|row|row.5).sum::<i64>()+child_calls.iter().map(|row|row.2).sum::<i64>(),7);
    assert_eq!(crate::agent_runtime::multi_agent::budget::balance(&db,&f.actor.root_run_id,None,"model_requests").unwrap().consumed,7);
    let phases=observed.iter().map(|wire| {
        let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
        let system = body["messages"][0]["content"].as_str().unwrap_or("");
        if system.contains("You are the Root Coordinator") {
            let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            format!("Root:{}",input["basis"]["phase"].as_str().unwrap_or("bootstrap"))
        } else if system.contains("Mapper") { "Mapper".into() }
        else { "non-Root specialist/executor".into() }
    }).collect::<Vec<String>>();
    assert_eq!(phases,vec!["Root:bootstrap","Mapper","Root:lease-expired","Root:mapper-output","Root:budget-allocation","non-Root specialist/executor","Root:budget-consumption"]);
    drop(observed);
    drop(session);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn lease_paid_missing_original_ack_refuses_before_sdk_without_writes() {lease_paid_actual_lifecycle("missing_ack");}
#[test]
fn lease_paid_source_ack_drift_keeps_bill_and_duplicate_ack_cannot_publish_or_resend() {lease_paid_actual_lifecycle("source_drift");}
#[test]
fn lease_paid_unknown_usage_retains_original_debt_without_publication_or_resend() {lease_paid_actual_lifecycle("unknown_usage");}

#[test]
fn lease_paid_publication_abort_recovers_original_paid_result_without_resend() {lease_paid_actual_lifecycle("publication_abort");}
#[test]
fn lease_paid_publication_ignore_recovers_original_paid_result_without_resend() {lease_paid_actual_lifecycle("publication_ignore");}

#[test]
fn lease_paid_cancel_before_sdk_retains_original_fees_without_publication_or_resend() {lease_paid_actual_lifecycle("authority_before_cancel");}

#[test]
fn lease_paid_successor_before_sdk_retains_original_fees_without_publication_or_resend() {lease_paid_actual_lifecycle("authority_before_successor");}

#[test]
fn lease_paid_expired_c_before_sdk_retains_original_fees_without_publication_or_resend() {lease_paid_actual_lifecycle("authority_before_expired");}

#[test]
fn lease_paid_cancel_after_received_retains_original_fees_without_publication_or_resend() {lease_paid_actual_lifecycle("authority_paid_cancel");}

#[test]
fn lease_paid_successor_after_received_retains_original_fees_without_publication_or_resend() {lease_paid_actual_lifecycle("authority_paid_successor");}
