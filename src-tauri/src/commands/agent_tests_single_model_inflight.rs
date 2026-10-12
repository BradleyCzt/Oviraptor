// Genuine Single creator/startup/frozen owner and actual blocked provider.
// Two independent scans share the application database, not a recovery grant.
#[test]
fn single_model_inflight_cache_provenance_is_atomic_original_and_immutable() {
    for explicit in [false, true] {
        let (f, mut context) = single_exit_private_fixture();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
            let mut body: JsonValue = serde_json::from_str(&model_round(&[], 20)).unwrap();
            if !explicit { body["usage"].as_object_mut().unwrap().remove("prompt_tokens_details"); }
            (200, "application/json", body.to_string())
        }));
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let client = AgentModelClient::new(agent_model_profile(&context.environment, None).unwrap(), &[]);
        let reply = native_model_transport(&context, &client, vec![json!({"role":"user","content":"original cache provenance"})], &[], 1)
            .unwrap_or_else(|_| panic!("actual original cache provenance SDK must return"));
        assert_eq!(reply.cached_usage_reported, explicit);
        let db = db::open(&f.path).unwrap();
        let matched: i64 = db.query_row("SELECT count(*) FROM agent_root_model_cache_provenance p
            JOIN agent_root_model_journal j ON j.call_id=p.call_id AND j.root_run_id=p.root_run_id
              AND j.lease_attempt_id=p.lease_attempt_id AND j.round=p.round AND j.request_hash=p.request_hash
              AND j.phase='received' AND json_extract(j.receipt_json,'$.responseHash')=p.response_hash
            WHERE p.cached_usage_reported=?1", [explicit], |r| r.get(0)).unwrap();
        assert_eq!(matched, 1);
        assert!(db.execute("UPDATE agent_root_model_cache_provenance SET cached_usage_reported=1", []).is_err());
        assert!(db.execute("DELETE FROM agent_root_model_cache_provenance", []).is_err());
        assert!(db.execute("INSERT OR REPLACE INTO agent_root_model_cache_provenance SELECT * FROM agent_root_model_cache_provenance", []).is_err());
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_cache_provenance", [], |r|r.get::<_,i64>(0)).unwrap(), 1);
    }
}

#[test]
fn single_model_inflight_cache_writer_fault_keeps_original_hold_and_never_retries() {
    for table in ["agent_root_model_cache_provenance", "agent_root_model_cache_capture"] {
    let (f, mut context) = single_exit_private_fixture();
    let db = db::open(&f.path).unwrap();
    db.execute_batch(&format!("CREATE TRIGGER cache_provenance_writer_fault BEFORE INSERT ON {table}
        BEGIN SELECT RAISE(ABORT,'cache_provenance_fixture_fault'); END;")).unwrap();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (200, "application/json", model_round(&[], 20))));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(agent_model_profile(&context.environment, None).unwrap(), &[]);
    assert!(native_model_transport(&context, &client, vec![json!({"role":"user","content":"actual cache writer failure"})], &[], 1).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE phase='dispatch'", [], |r|r.get::<_,i64>(0)).unwrap(), 1);
    for query in ["SELECT count(*) FROM agent_root_model_journal WHERE phase<>'dispatch'",
        "SELECT count(*) FROM agent_root_model_cache_provenance",
        "SELECT count(*) FROM agent_root_model_cache_capture",
        "SELECT count(*) FROM agent_budget_entries WHERE dimension<>'wall_time_ms' AND kind<>'reserve'"] {
        assert_eq!(db.query_row(query, [], |r|r.get::<_,i64>(0)).unwrap(), 0);
    }
    let before = super::tests::application_table_snapshot(&db);
    assert!(native_model_transport(&context, &client, vec![json!({"role":"user","content":"must not retry the paid call"})], &[], 2).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
    }
}

#[test]
fn single_model_inflight_two_actual_scans_overlap_and_pause_is_scope_local() {
    use crate::agent_runtime::multi_agent::budget;
    use std::sync::{mpsc, Arc, Mutex};
    let mut h = fresh_single_production_harness("parallel-original-scans", mock_site);
    let mut connection = db::open(&h.db_path).unwrap();
    let second = create_sentinel_url_scan_with_mode_in(
        &connection, 9001, "Second independent scan".into(),
        vec![h.context.target_url.clone()], Some("standard".into()),
        None, None, None, None, None, None, None, Some("single".into()),
    ).unwrap();
    let work = web_mode_test_start(&h.root, &mut connection, &second.id, WebStartMode::Confirm).unwrap();
    let mut other = h.context.clone();
    other.scan_id = second.id;
    other.target_dir = work.join("url-pipeline/target-00001");
    other.log_path = work.join("runner.log");
    fs::create_dir_all(&other.target_dir).unwrap();
    fs::write(other.target_dir.join(".oviraptor-scan-id"), &other.scan_id).unwrap();
    fs::write(other.target_dir.join("frontend-evidence.json"), other.evidence.to_string()).unwrap();
    let (arrived_tx, arrived) = mpsc::channel();
    let mut endpoints = Vec::new();
    let mut releases = Vec::new();
    for (index, context) in [&mut h.context, &mut other].into_iter().enumerate() {
        let (release_tx, release) = mpsc::channel();
        let release = Mutex::new(release);
        let arrived_tx = arrived_tx.clone();
        let (port, seen, stop) = spawn_endpoint(Arc::new(move |_| {
            arrived_tx.send(index).unwrap();
            release.lock().unwrap().recv_timeout(Duration::from_secs(15)).unwrap();
            (200, "application/json", model_round(&[], 20))
        }));
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        context.execution_plan = build_agent_execution_plan(
            &AgentBudgetSettings::from_json(&json!({})), &context.route,
            &context.environment, AgentBackendKind::Native, &h.db_path, &context.scan_id,
        ).with_attempt(1);
        releases.push(release_tx);
        endpoints.push((seen, stop));
    }
    freeze_fresh_single_production_harness(&mut h);
    persist_frozen_web_execution_plan(&h.db_path, &other.scan_id, 1, &other.target_url, &other.execution_plan).unwrap();
    other.run = Some(runtime_open_run(&h.db_path, &other.scan_id, &other.route).unwrap());
    bind_agent_evidence_location(&other).unwrap();
    let roots = [&h.context, &other].map(|c| c.run.as_ref().unwrap().run_id.clone());
    assert_ne!(roots[0], roots[1]);
    let results = std::thread::scope(|scope| {
        let mut completed = Vec::new();
        for context in [&h.context, &other] {
            let (done_tx, done) = mpsc::channel();
            completed.push(done);
            scope.spawn(move || {
                let client = AgentModelClient::new(agent_model_profile(&context.environment, None).unwrap(), &[]);
                let result = native_model_transport(context, &client,
                    vec![json!({"role":"user","content":"parallel independently owned original SDK"})], &[], 1);
                if let Ok(reply) = &result {
                    context.run.as_ref().unwrap().model_round(1, &reply.usage, &[]).unwrap();
                }
                done_tx.send(result.is_ok()).unwrap();
            });
        }
        let mut arrivals = (0..2).map(|_| arrived.recv_timeout(Duration::from_secs(8)).unwrap()).collect::<Vec<_>>();
        arrivals.sort();
        assert_eq!(arrivals, vec![0, 1], "both physical SDK requests arrive before either response is released");
        for root in &roots {
            assert_eq!(connection.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='dispatch'", [root], |r| r.get::<_, i64>(0)).unwrap(), 1);
            assert_eq!(connection.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'", [root], |r| r.get::<_, i64>(0)).unwrap(), 0);
        }
        let changes = connection.total_changes();
        let snapshot = connection.unchecked_transaction().unwrap();
        for root in &roots {
            let history = native_single_original_history(&snapshot, root).unwrap();
            assert_eq!(history["state"], "not_verified", "in-flight requests cannot be displayed as closed invoices");
            assert_eq!(history["executionAllowed"], false);
        }
        snapshot.rollback().unwrap();
        assert_eq!(connection.total_changes(), changes);
        request_sentinel_pause(&h.db_path, &h.context.scan_id).unwrap();
        let paused = completed[0].recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(completed[1].try_recv(), Err(mpsc::TryRecvError::Empty)), "pausing one scan must not terminate the other in-flight SDK");
        releases[1].send(()).unwrap();
        let independent = completed[1].recv_timeout(Duration::from_secs(8)).unwrap();
        releases[0].send(()).unwrap();
        (paused, independent)
    });
    assert_eq!(results, (false, true));
    let until = std::time::Instant::now() + Duration::from_secs(2);
    while endpoints.iter().any(|(seen, _)| seen.lock().unwrap().is_empty())
        && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    for (seen, _) in &endpoints {
        assert_eq!(seen.lock().unwrap().len(), 1, "one physical provider request per independent Root; no automatic retry");
    }
    let received = budget::balance(&connection, &roots[1], None, "model_requests").unwrap();
    assert_eq!((received.consumed, received.reserved, received.indeterminate), (1, 0, 0));
    let cancelled = budget::balance(&connection, &roots[0], None, "model_input_tokens").unwrap();
    assert!(cancelled.indeterminate > 0, "cancelled physical request retains its unknown original token obligation");
    let before_inventory = super::tests::application_table_snapshot(&connection);
    let inventory = native_root_cost_reconciliations_for_attempt(&connection, &h.context.scan_id, 1).unwrap();
    assert_eq!(inventory["blocked"].as_array().unwrap().len(), 1,
        "the actual uncertain original SDK must remain visible as an unconfirmable bill");
    assert!(inventory["items"].as_array().unwrap().is_empty());
    assert!(inventory["receipts"].as_array().unwrap().is_empty());
    assert_eq!(inventory["executionAllowed"], false);
    assert_eq!(inventory["automaticResumeAllowed"], false);
    assert_eq!(inventory["blocked"][0]["rootRunId"], roots[0]);
    let blocked_call = inventory["blocked"][0]["callId"].as_str().unwrap();
    assert!(budget::root::model::tick::Tick::confirm_cost_reconciliation(
        &connection, &h.context.scan_id, 1, &roots[0], blocked_call, &"0".repeat(64)
    ).is_err(), "a visible unknown bill is not a confirmable invoice");
    assert_eq!(super::tests::application_table_snapshot(&connection), before_inventory);
    let changes = connection.total_changes();
    for dimension in ["", "input_tokens", "usd", "model_requests "] {
        assert_eq!(budget::balance(&connection, &roots[1], None, dimension).unwrap_err(),
            "budget_dimension_unknown", "an unknown dimension must not look like a zero paid balance");
    }
    assert_eq!(connection.total_changes(), changes);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_http_request_claims", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert!(h.site_seen.lock().unwrap().is_empty());
    drop(connection);
    fs::remove_dir_all(h.root).unwrap();
}
#[test]
fn single_model_inflight_original_sdk_refuses_financial_exit_until_actual_return() {single_model_inflight_boundary(1);}
#[test]
fn single_model_inflight_second_actual_sdk_retains_first_paid_round_and_refuses_exit() {single_model_inflight_boundary(2);}
#[test]
fn single_model_inflight_unknown_bill_remains_visible_after_hundred_known_bills() {single_model_inflight_boundary(102);}
fn single_model_inflight_boundary(blocked:usize) {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    use std::sync::{Arc, Mutex, mpsc};
    let (f, mut context)=if blocked > 100 {
        let f = web_mode_fixture("single", "https://single-bill-window.example.test/app");
        let mut plan = web_mode_test_plan(&f);
        // Freeze this fixture's capacity before original Root birth. Never
        // enlarge an existing owner's budget to manufacture history.
        plan.hard_model_requests = 110;
        plan.soft_model_requests = 110;
        plan.hard_total_tokens = 1_000_000;
        plan.soft_uncached_tokens = 1_000_000;
        plan.timeout_seconds = 300;
        persist_frozen_web_execution_plan(&f.path, &f.scan, 1, &f.target, &plan).unwrap();
        let mut context = test_context(&f.path, &f.target, vec![AgentIdentity::anonymous()]);
        context.scan_id = f.scan.clone();
        context.execution_plan = plan;
        context.run = runtime_open_run(&f.path, &f.scan, &context.route);
        (f, context)
    } else { single_exit_private_fixture() };
    let (arrived_tx,arrived)=mpsc::channel();
    let (release_tx,release)=mpsc::channel();let release=Mutex::new(release);
    let count=std::sync::atomic::AtomicUsize::new(0);
    let (port,seen,_stop)=spawn_endpoint(Arc::new(move |_| {
        let call = count.fetch_add(1,std::sync::atomic::Ordering::SeqCst)+1;
        let _=arrived_tx.send(call);
        if call<blocked {return (200,"application/json",model_round(&[],20));}
        let _=release.lock().unwrap().recv_timeout(Duration::from_secs(8));
        (200,"application/json",model_round(&[],20))
    }));
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let db=db::open(&f.path).unwrap();let root=&context.run.as_ref().unwrap().run_id;
    let (closed,unchanged,early)=std::thread::scope(|scope| {
        let (done_tx,done)=mpsc::channel();let context=&context;
        scope.spawn(move || {
            let client=AgentModelClient::new(agent_model_profile(&context.environment,None).unwrap(),&[]);
            for round in 1..=blocked {
                let result=native_model_transport(context,&client,vec![json!({"role":"user","content":"actual in-flight Single original request"})],&[],round as i64);
                if round<blocked {
                    let reply=result.unwrap_or_else(|_|panic!("actual first paid SDK must return"));
                    context.run.as_ref().unwrap().model_round(round as i64,&reply.usage,&[]).unwrap();
                } else {let _=done_tx.send(result.is_err());}
            }
        });
        // Observe progress per actual SDK instead of imposing a total 30s
        // benchmark on 102 durable transactions during concurrent compilation.
        for expected in 1..=blocked {
            assert_eq!(arrived.recv_timeout(Duration::from_secs(5)).expect("each actual original Single SDK must arrive"), expected);
        }
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='dispatch'",[root],|r|r.get::<_,i64>(0)).unwrap(),blocked as i64);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE phase='received'",[],|r|r.get::<_,i64>(0)).unwrap(),blocked as i64-1);
        let original=RootOwner::load_single(&db,root).unwrap();let before=single_finally_physical(&db);
        let closed=original.close_single_finance(&db);let unchanged=single_finally_physical(&db)==before;
        db.execute("UPDATE sentinel_scans SET status='pausing' WHERE id=?1",[&f.scan]).unwrap();
        let early=done.recv_timeout(Duration::from_secs(3));let _=release_tx.send(());
        let early=match early {Ok(v)=>Some(v),Err(_)=>{let _=done.recv_timeout(Duration::from_secs(5));None}};
        (closed,unchanged,early)
    });
    let error=closed.expect_err("Single finance exited while original actual SDK was still in flight");
    assert!(error.contains("single_model_transport_not_idle"),"{error}");
    assert!(unchanged,"busy original Single SDK must preserve every original row");
    assert_eq!(early,Some(true),"original pause must return from SDK before provider release");
    let until=std::time::Instant::now()+Duration::from_secs(2);
    while seen.lock().unwrap().len()<blocked && std::time::Instant::now()<until {std::thread::yield_now();}
    assert_eq!(seen.lock().unwrap().len(),blocked);
    use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
    let fees=Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap();
    let calls=Rows::read(&db,"SELECT rowid,* FROM agent_root_model_journal ORDER BY rowid",[]).unwrap();
    let owner=RootOwner::load_single(&db,root).unwrap();let fact=owner.close_single_finance(&db).unwrap();
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap()==fees);
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_root_model_journal ORDER BY rowid",[]).unwrap()==calls);
    let before=single_finally_physical(&db);assert_eq!(owner.close_single_finance(&db).unwrap(),fact);
    assert_eq!(single_finally_physical(&db),before);assert!(owner.require_live(&db).is_err());
    let original_tables = super::tests::application_table_snapshot(&db);
    let inventory = native_root_cost_reconciliations_for_attempt(&db, &f.scan, 1).unwrap();
    assert_eq!(inventory["blocked"].as_array().unwrap().len(), 1,
        "the prior known bill cannot be displayed as another unresolved bill");
    assert!(inventory["items"].as_array().unwrap().is_empty());
    assert!(inventory["receipts"].as_array().unwrap().is_empty());
    assert_eq!(inventory["truncated"], false, "known bills do not consume the pending display window");
    let unknown: String = db.query_row("SELECT call_id FROM agent_root_model_journal WHERE phase='uncertain'",
        [], |r|r.get(0)).unwrap();
    assert_eq!(inventory["blocked"][0]["callId"], unknown);
    assert_eq!(super::tests::application_table_snapshot(&db), original_tables);
}

fn single_model_inflight_paid() -> (WebModeFixture,AgentRunContext,Seen) {
    let (f,mut context)=single_exit_private_fixture();
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",model_round(&[],20))));
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let client=AgentModelClient::new(agent_model_profile(&context.environment,None).unwrap(),&[]);
    let reply=native_model_transport(&context,&client,vec![json!({"role":"user","content":"actual paid Single exit fixture"})],&[],1)
        .unwrap_or_else(|_|panic!("actual paid original Single SDK must return"));
    context.run.as_ref().unwrap().model_round(1,&reply.usage,&[]).unwrap();
    assert_eq!(reply.usage.total_tokens,60);assert_eq!(seen.lock().unwrap().len(),1);
    (f,context,seen)
}
fn single_model_inflight_path(f:&WebModeFixture,context:&AgentRunContext)->PathBuf {
    use sha2::Digest;
    let canonical=fs::canonicalize(&f.path).unwrap();let mut name=canonical.file_name().unwrap().to_os_string();name.push(".invocations");
    let key=serde_json::to_vec(&(&f.scan,1,"single-root-sdk",&context.run.as_ref().unwrap().run_id)).unwrap();
    canonical.with_file_name(name).join(format!("{:x}.lock",sha2::Sha256::digest(key)))
}
#[test]
fn single_model_inflight_missing_or_foreign_inode_refuses_exit_and_next_sdk_without_creating_proof() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    for foreign in [false,true] {
        let (f,context,seen)=single_model_inflight_paid();let db=db::open(&f.path).unwrap();let root=&context.run.as_ref().unwrap().run_id;
        let owner=RootOwner::load_single(&db,root).unwrap();let path=single_model_inflight_path(&f,&context);assert!(path.is_file());fs::remove_file(&path).unwrap();
        if foreign {drop(crate::agent_runtime::execution_owner::claim_native_invocation(&f.path,&f.scan,1,"single-root-sdk","foreign-single-root").unwrap());}
        let before=single_finally_physical(&db);let error=owner.close_single_finance(&db).unwrap_err();
        assert!(error.contains("single_model_transport_original_exit_proof_missing"),"{error}");
        assert_eq!(single_finally_physical(&db),before);assert!(!path.exists());
        let client=AgentModelClient::new(agent_model_profile(&context.environment,None).unwrap(),&[]);
        assert!(native_model_transport(&context,&client,vec![json!({"role":"user","content":"missing original Single proof cannot authorize second SDK"})],&[],2).is_err());
        assert_eq!(single_finally_physical(&db),before);assert!(!path.exists());assert_eq!(seen.lock().unwrap().len(),1);
    }
}
#[test]
fn single_model_inflight_original_hash_metadata_or_role_damage_preserves_every_row() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    for damage in [
        "DROP TRIGGER root_model_no_update; UPDATE agent_root_model_journal SET request_hash='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' WHERE phase='dispatch'",
        "DROP TRIGGER root_model_no_update; UPDATE agent_root_model_journal SET receipt_json='{}' WHERE phase='dispatch'",
        "UPDATE agent_runs SET role='web_executor' WHERE role='coordinator'",
    ] {
        let (f,context,seen)=single_model_inflight_paid();let db=db::open(&f.path).unwrap();let owner=RootOwner::load_single(&db,&context.run.as_ref().unwrap().run_id).unwrap();
        db.execute_batch(damage).unwrap();let before=single_finally_physical(&db);
        assert!(owner.close_single_finance(&db).is_err(),"{damage}");assert_eq!(single_finally_physical(&db),before);assert_eq!(seen.lock().unwrap().len(),1);
    }
}
#[test]
fn single_model_inflight_paid_exit_preserves_original_fees_and_replay_cannot_authorize_sdk() {
    use crate::agent_runtime::{multi_agent::budget::root::RootOwner,multi_agent::attempts::audit_rows::Rows};
    let (f,context,seen)=single_model_inflight_paid();let db=db::open(&f.path).unwrap();let root=&context.run.as_ref().unwrap().run_id;
    let fees=Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap();
    let calls=Rows::read(&db,"SELECT rowid,* FROM agent_root_model_journal ORDER BY rowid",[]).unwrap();
    let before_exit = single_finally_physical(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(crate::agent_runtime::multi_agent::budget::continuation::verify_settled_history_on(&snapshot, root).unwrap_err(),
        "budget_single_exit_receipt_missing", "known token totals alone cannot prove the original final financial interval");
    snapshot.rollback().unwrap();
    assert_eq!(single_finally_physical(&db), before_exit);
    let owner=RootOwner::load_single(&db,root).unwrap();let fact=owner.close_single_finance(&db).unwrap();
    let snapshot = db.unchecked_transaction().unwrap();
    let history = crate::agent_runtime::multi_agent::budget::continuation::verify_settled_history_on(&snapshot, root)
        .expect("the original paid Single exit must be audited through Single receipts, not Multi Tick receipts");
    assert_eq!(history["proofKind"], "settled_original_history");
    assert_eq!(history["executionAllowed"], false);
    snapshot.rollback().unwrap();
    let before_inventory = super::tests::application_table_snapshot(&db);
    let inventory = native_root_cost_reconciliations_for_attempt(&db, &f.scan, 1).unwrap();
    assert!(inventory["items"].as_array().unwrap().is_empty());
    assert!(inventory["blocked"].as_array().unwrap().is_empty(),
        "verified closed Single fees must not be displayed as unresolved Multi bills");
    assert!(inventory["receipts"].as_array().unwrap().is_empty());
    assert_eq!(super::tests::application_table_snapshot(&db), before_inventory);
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid",[]).unwrap()==fees);
    assert!(Rows::read(&db,"SELECT rowid,* FROM agent_root_model_journal ORDER BY rowid",[]).unwrap()==calls);
    let before=single_finally_physical(&db);assert_eq!(owner.close_single_finance(&db).unwrap(),fact);assert_eq!(single_finally_physical(&db),before);
    let client=AgentModelClient::new(agent_model_profile(&context.environment,None).unwrap(),&[]);
    assert!(native_model_transport(&context,&client,vec![json!({"role":"user","content":"exit is not a continuation grant"})],&[],2).is_err());
    assert_eq!(single_finally_physical(&db),before);assert_eq!(seen.lock().unwrap().len(),1);
}
#[test]
fn single_model_inflight_empty_journal_closes_without_creating_invocation_inode() {
    let (f,context)=single_exit_private_fixture();let db=db::open(&f.path).unwrap();let root=&context.run.as_ref().unwrap().run_id;
    let path=single_model_inflight_path(&f,&context);assert!(!path.exists());
    crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db,root).unwrap().close_single_finance(&db).unwrap();
    assert!(!path.exists());assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn single_history_received_invoice_and_original_paid_entries_must_agree() {
    use crate::agent_runtime::multi_agent::budget::{continuation, root::RootOwner};
    for (case, expected) in [
        ("redistributed", "continuation_single_original_cost_conflict"),
        ("unreported", "continuation_single_known_usage_required"),
        ("extra_usage", "continuation_single_usage_invalid"),
    ] {
        let (f, context, seen) = single_model_inflight_paid();
        let db = db::open(&f.path).unwrap();
        let root = &context.run.as_ref().unwrap().run_id;
        let raw: String = db.query_row("SELECT receipt_json FROM agent_root_model_journal
            WHERE root_run_id=?1 AND phase='received'", [root], |r| r.get(0)).unwrap();
        let mut bad: JsonValue = serde_json::from_str(&raw).unwrap();
        match case {
            "redistributed" => {
                bad["usage"]["inputTokens"] = json!(bad["usage"]["inputTokens"].as_i64().unwrap() - 1);
                bad["usage"]["outputTokens"] = json!(bad["usage"]["outputTokens"].as_i64().unwrap() + 1);
            }
            "unreported" => bad["usageReported"] = json!(false),
            "extra_usage" => bad["usage"]["extraCounter"] = json!(0),
            _ => unreachable!(),
        }
        // Model a damaged original invoice before exit, not an after-exit
        // hash failure. The coarse total and original paid ledger stay intact.
        let update_trigger: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='root_model_no_update'",
            [], |r| r.get(0)).unwrap();
        db.execute_batch("DROP TRIGGER root_model_no_update").unwrap();
        db.execute("UPDATE agent_root_model_journal SET receipt_json=?1 WHERE root_run_id=?2 AND phase='received'",
            params![bad.to_string(), root]).unwrap();
        db.execute_batch(&update_trigger).unwrap();
        RootOwner::load_single(&db, root).unwrap().close_single_finance(&db).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let snapshot = db.unchecked_transaction().unwrap();
        assert_eq!(continuation::verify_settled_history_on(&snapshot, root).unwrap_err(), expected, "{case}");
        let diagnostic = native_single_original_history(&snapshot, root).unwrap();
        assert_eq!(diagnostic["state"], "not_verified");
        assert_eq!(diagnostic["executionAllowed"], false);
        assert!(!diagnostic.to_string().contains(expected), "raw audit errors are not projected");
        snapshot.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(seen.lock().unwrap().len(), 1, "audit never reissues an SDK request");
    }
}

#[test]
fn single_history_actual_terminal_models_and_http_are_read_only_paid_history() {
    use crate::agent_runtime::multi_agent::budget::continuation;
    let (h, _owned) = fresh_single_paid_callback_fixture("single-history-real-terminal");
    let db = db::open(&h.db_path).unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    let models = h.model_seen.lock().unwrap().len();
    assert!(models > 1);
    assert_eq!(h.site_seen.lock().unwrap().len(), 1);
    let before = super::tests::application_table_snapshot(&db);
    assert!(matches!(continuation::single::original_source_on(&db, root), Err(error)
        if error == "continuation_financial_snapshot_required"));
    let snapshot = db.unchecked_transaction().unwrap();
    let original = continuation::verify_settled_history_on(&snapshot, root).unwrap();
    assert_eq!(continuation::single::original_source_on(&snapshot, root).unwrap().history(), original);
    let remaining = continuation::single_remaining::remaining_on(&snapshot, root).unwrap();
    assert_eq!(remaining["executionAllowed"], false);
    assert_eq!(remaining["remaining"].as_array().unwrap().len(), 10);
    assert_eq!(continuation::single_remaining::remaining_for_new_work_on(&snapshot, root).unwrap(), remaining);
    assert_eq!(original["rootRunId"], *root);
    assert_eq!(original["executionAllowed"], false);
    assert_eq!(continuation::verify_settled_history_on(&snapshot, root).unwrap(), original);
    snapshot.rollback().unwrap();
    let attempt: i64 = db.query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1", [&h.context.scan_id], |r| r.get(0)).unwrap();
    let visible = native_budget_diagnostics_for_attempt(&db, &h.context.scan_id, attempt).unwrap();
    let projected = visible["roots"].as_array().unwrap().iter().find(|row| row["rootRunId"] == *root).unwrap();
    assert_eq!(projected["singleOriginalHistory"]["state"], "verified_closed_history");
    assert_eq!(projected["singleOriginalHistory"]["executionAllowed"], false);
    assert_eq!(projected["singleOriginalHistory"]["targetRequestsGranted"], 0);
    assert_eq!(projected["singleOriginalHistory"]["basis"], "original_runtime_records_not_provider_dollar_invoice");
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(h.model_seen.lock().unwrap().len(), models);
    assert_eq!(h.site_seen.lock().unwrap().len(), 1);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn single_history_closed_original_invoice_stays_readable_after_actual_deadline() {
    use crate::agent_runtime::multi_agent::budget::{clock, continuation, root::RootOwner};
    let f = web_mode_fixture("single", "https://single-history-expiry.example.test/app");
    let mut plan = web_mode_test_plan(&f);
    plan.timeout_seconds = 3; // Frozen before birth; never rewrite a live clock.
    persist_frozen_web_execution_plan(&f.path, &f.scan, 1, &f.target, &plan).unwrap();
    let mut context = test_context(&f.path, &f.target, vec![AgentIdentity::anonymous()]);
    context.scan_id = f.scan.clone();
    context.execution_plan = plan;
    context.run = runtime_open_run(&f.path, &f.scan, &context.route);
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (200, "application/json", model_round(&[], 20))));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(agent_model_profile(&context.environment, None).unwrap(), &[]);
    let reply = native_model_transport(&context, &client, vec![json!({"role":"user","content":"original paid history with a frozen short deadline"})], &[], 1)
        .unwrap_or_else(|_| panic!("original request must complete inside its own frozen deadline"));
    context.run.as_ref().unwrap().model_round(1, &reply.usage, &[]).unwrap();
    let db = db::open(&f.path).unwrap();
    let root = &context.run.as_ref().unwrap().run_id;
    let exit = RootOwner::load_single(&db, root).unwrap().close_single_finance(&db).unwrap();
    assert!(!exit.exhausted);
    let before = super::tests::application_table_snapshot(&db);
    std::thread::sleep(Duration::from_millis(3100));
    assert_eq!(clock::remaining(&db, root).unwrap_err(), "budget_wall_time_exhausted");
    let snapshot = db.unchecked_transaction().unwrap();
    let history = continuation::verify_settled_history_on(&snapshot, root).unwrap();
    assert_eq!(history["executionAllowed"], false);
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn single_history_actual_queue_cancel_releases_only_unsent_call_and_keeps_prior_paid_cost() {
    use crate::agent_runtime::{model::{admission, CancelToken}, multi_agent::budget::{self, continuation, root::RootOwner}};
    for (paid, wrong_phase_code) in [(false, false), (true, false), (true, true)] {
        let (f, mut context) = single_exit_private_fixture();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (200, "application/json", model_round(&[], 20))));
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let client = AgentModelClient::new(agent_model_profile(&context.environment, None).unwrap(), &[]);
        if paid {
            let reply = native_model_transport(&context, &client, vec![json!({"role":"user","content":"original paid first round"})], &[], 1)
                .unwrap_or_else(|_| panic!("original first SDK must succeed"));
            assert_eq!((reply.usage.input_tokens, reply.usage.output_tokens), (20, 40));
            context.run.as_ref().unwrap().model_round(1, &reply.usage, &[]).unwrap();
        }
        let gate = admission::gate_for(client.profile().local, &client.profile().endpoint);
        let capacity = if client.profile().local { 1 } else { admission::CLOUD_MAX_IN_FLIGHT };
        // Occupy admission, not the provider: the tested call reaches the
        // real gateway queue and obtains its own BeforeTransport proof.
        let occupied = (0..capacity).map(|_| gate.acquire(&CancelToken::new()).unwrap()).collect::<Vec<_>>();
        std::thread::scope(|scope| {
            let (done_tx, done) = std::sync::mpsc::channel();
            let context = &context;
            let client = &client;
            scope.spawn(move || {
                let result = native_model_transport(context, client,
                    vec![json!({"role":"user","content":"cancel this original queued call before transport"})], &[], if paid { 2 } else { 1 });
                done_tx.send(result.is_err()).unwrap();
            });
            let until = std::time::Instant::now() + Duration::from_secs(5);
            while gate.queued() == 0 && std::time::Instant::now() < until { std::thread::sleep(Duration::from_millis(5)); }
            assert_eq!(gate.queued(), 1);
            request_sentinel_pause(&f.path, &f.scan).unwrap();
            assert!(done.recv_timeout(Duration::from_secs(3)).unwrap());
        });
        drop(occupied);
        let db = db::open(&f.path).unwrap();
        let root = &context.run.as_ref().unwrap().run_id;
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='unsent'", [root], |r| r.get::<_, i64>(0)).unwrap(), 1);
        let requests = budget::balance(&db, root, None, "model_requests").unwrap();
        assert_eq!((requests.consumed, requests.reserved, requests.indeterminate), (i64::from(paid), 0, 0));
        let inputs = budget::balance(&db, root, None, "model_input_tokens").unwrap();
        assert_eq!((inputs.consumed, inputs.reserved, inputs.indeterminate), (if paid { 20 } else { 0 }, 0, 0));
        if wrong_phase_code {
            let trigger: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='root_model_no_update'", [], |r| r.get(0)).unwrap();
            db.execute_batch("DROP TRIGGER root_model_no_update").unwrap();
            db.execute("UPDATE agent_root_model_journal SET receipt_json=?1 WHERE root_run_id=?2 AND phase='unsent'",
                params![json!({"code":"model_network"}).to_string(), root]).unwrap();
            db.execute_batch(&trigger).unwrap();
        }
        RootOwner::load_single(&db, root).unwrap().close_single_finance(&db).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let snapshot = db.unchecked_transaction().unwrap();
        if wrong_phase_code {
            assert_eq!(continuation::verify_settled_history_on(&snapshot, root).unwrap_err(), "continuation_single_unsent_proof_invalid");
            assert_eq!(native_single_original_history(&snapshot, root).unwrap()["state"], "not_verified");
        } else {
            assert_eq!(continuation::verify_settled_history_on(&snapshot, root).unwrap()["executionAllowed"], false);
            assert_eq!(native_single_original_history(&snapshot, root).unwrap()["state"], "verified_closed_history");
        }
        snapshot.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(seen.lock().unwrap().len(), usize::from(paid));
    }
}

#[test]
fn single_model_inflight_shared_provider_admits_three_and_cancelled_fourth_never_sends() {
    use crate::agent_runtime::{model::admission, multi_agent::budget::{self, root::RootOwner}};
    use std::sync::{mpsc, Arc, Mutex};
    let mut h = fresh_single_production_harness("shared-provider-four-scans", mock_site);
    let (arrival_tx, arrivals) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    let release = Mutex::new(release);
    let sent = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = sent.clone();
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        let index = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        arrival_tx.send(index).unwrap();
        release.lock().unwrap().recv_timeout(Duration::from_secs(15)).unwrap();
        (200, "application/json", model_round(&[], 20))
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.context.execution_plan = build_agent_execution_plan(&AgentBudgetSettings::from_json(&json!({})),
        &h.context.route, &h.context.environment, AgentBackendKind::Native, &h.db_path, &h.context.scan_id).with_attempt(1);
    freeze_fresh_single_production_harness(&mut h);
    let mut contexts = vec![h.context.clone()];
    let mut db = db::open(&h.db_path).unwrap();
    for index in 1..4 {
        let scan = create_sentinel_url_scan_with_mode_in(&db, 9001, format!("Independent scan {index}"),
            vec![h.context.target_url.clone()], Some("standard".into()), None, None, None, None, None, None, None, Some("single".into())).unwrap();
        let work = web_mode_test_start(&h.root, &mut db, &scan.id, WebStartMode::Confirm).unwrap();
        let mut context = h.context.clone();
        context.scan_id = scan.id;
        context.target_dir = work.join("url-pipeline/target-00001");
        context.log_path = work.join("runner.log");
        fs::create_dir_all(&context.target_dir).unwrap();
        fs::write(context.target_dir.join(".oviraptor-scan-id"), &context.scan_id).unwrap();
        fs::write(context.target_dir.join("frontend-evidence.json"), context.evidence.to_string()).unwrap();
        context.execution_plan = build_agent_execution_plan(&AgentBudgetSettings::from_json(&json!({})),
            &context.route, &context.environment, AgentBackendKind::Native, &h.db_path, &context.scan_id).with_attempt(1);
        persist_frozen_web_execution_plan(&h.db_path, &context.scan_id, 1, &context.target_url, &context.execution_plan).unwrap();
        context.run = Some(runtime_open_run(&h.db_path, &context.scan_id, &context.route).unwrap());
        bind_agent_evidence_location(&context).unwrap();
        contexts.push(context);
    }
    let profile = agent_model_profile(&h.context.environment, None).unwrap();
    assert!(!profile.local);
    assert_eq!(admission::CLOUD_MAX_IN_FLIGHT, 3);
    let gate = admission::gate_for(false, &profile.endpoint);
    std::thread::scope(|scope| {
        let mut completed = Vec::new();
        for (index, context) in contexts.iter().enumerate() {
            if index == 3 {
                for _ in 0..3 { arrivals.recv_timeout(Duration::from_secs(8)).unwrap(); }
                assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), 3);
            }
            let (done_tx, done) = mpsc::channel();
            completed.push(done);
            scope.spawn(move || {
                let client = AgentModelClient::new(agent_model_profile(&context.environment, None).unwrap(), &[]);
                let result = native_model_transport(context, &client,
                    vec![json!({"role":"user","content":"independent scan using one actual shared provider"})], &[], 1);
                if let Ok(reply) = &result { context.run.as_ref().unwrap().model_round(1, &reply.usage, &[]).unwrap(); }
                done_tx.send(result.is_ok()).unwrap();
            });
        }
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while gate.queued() == 0 && std::time::Instant::now() < until { std::thread::sleep(Duration::from_millis(5)); }
        assert_eq!(gate.queued(), 1);
        assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), 3);
        request_sentinel_pause(&h.db_path, &contexts[3].scan_id).unwrap();
        assert!(!completed[3].recv_timeout(Duration::from_secs(3)).unwrap());
        for pending in &completed[..3] { assert!(matches!(pending.try_recv(), Err(mpsc::TryRecvError::Empty))); }
        for _ in 0..3 { release_tx.send(()).unwrap(); }
        for done in &completed[..3] { assert!(done.recv_timeout(Duration::from_secs(8)).unwrap()); }
    });
    assert_eq!(gate.queued(), 0);
    assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(seen.lock().unwrap().len(), 3);
    for (index, context) in contexts.iter().enumerate() {
        let root = &context.run.as_ref().unwrap().run_id;
        let requests = budget::balance(&db, root, None, "model_requests").unwrap();
        assert_eq!((requests.consumed, requests.reserved, requests.indeterminate), (if index == 3 { 0 } else { 1 }, 0, 0));
        RootOwner::load_single(&db, root).unwrap().close_single_finance(&db).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let snapshot = db.unchecked_transaction().unwrap();
        assert_eq!(native_single_original_history(&snapshot, root).unwrap()["state"], "verified_closed_history");
        snapshot.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    assert!(h.site_seen.lock().unwrap().is_empty());
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn single_model_inflight_same_root_replays_cannot_reserve_or_send_a_second_sdk() {
    use crate::agent_runtime::multi_agent::budget::{self, root::RootOwner};
    use std::sync::{mpsc, Arc, Mutex};
    let (f, mut context) = single_exit_private_fixture();
    let (arrived_tx, arrived) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    let release = Mutex::new(release);
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        arrived_tx.send(()).unwrap();
        release.lock().unwrap().recv_timeout(Duration::from_secs(15)).unwrap();
        (200, "application/json", model_round(&[], 20))
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(agent_model_profile(&context.environment, None).unwrap(), &[]);
    let messages = vec![json!({"role":"user","content":"original exclusive Single SDK"})];
    let db = db::open(&f.path).unwrap();
    std::thread::scope(|scope| {
        let context = &context;
        let client = &client;
        let messages = &messages;
        let first = scope.spawn(move || native_model_transport(context, client, messages.clone(), &[], 1));
        arrived.recv_timeout(Duration::from_secs(8)).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        for (round, changed) in [(1, false), (1, true), (2, false)] {
            let request = if changed { vec![json!({"role":"user","content":"changed request on an already held round"})] } else { messages.clone() };
            assert!(native_model_transport(context, client, request, &[], round).is_err());
            assert_eq!(super::tests::application_table_snapshot(&db), before, "re-entry may not add fees or replace the original call");
        }
        assert!(seen.lock().unwrap().is_empty(), "provider response has not been released");
        release_tx.send(()).unwrap();
        let reply = first.join().unwrap().unwrap_or_else(|_| panic!("original held request must still complete"));
        context.run.as_ref().unwrap().model_round(1, &reply.usage, &[]).unwrap();
    });
    assert_eq!(seen.lock().unwrap().len(), 1);
    let root = &context.run.as_ref().unwrap().run_id;
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='dispatch'", [root], |r| r.get::<_, i64>(0)).unwrap(), 1);
    let requests = budget::balance(&db, root, None, "model_requests").unwrap();
    assert_eq!((requests.consumed, requests.reserved, requests.indeterminate), (1, 0, 0));
    RootOwner::load_single(&db, root).unwrap().close_single_finance(&db).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(native_single_original_history(&snapshot, root).unwrap()["state"], "verified_closed_history");
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}

#[test]
fn single_history_financial_fingerprint_binds_original_single_exit_physical_row() {
    use crate::agent_runtime::multi_agent::budget::{continuation, root::RootOwner};
    let (f, context, seen) = single_model_inflight_paid();
    let db = db::open(&f.path).unwrap();
    let root = &context.run.as_ref().unwrap().run_id;
    RootOwner::load_single(&db, root).unwrap().close_single_finance(&db).unwrap();
    let snapshot = db.unchecked_transaction().unwrap();
    let before = continuation::verify_settled_history_on(&snapshot, root).unwrap();
    snapshot.rollback().unwrap();
    let fees = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries ORDER BY rowid", []).unwrap();
    // Disposable cold-history relocation, not a production write. Restoring
    // the exact immutable trigger leaves the same fact bytes and fee records.
    let original: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='single_exit_no_update'", [], |r| r.get(0)).unwrap();
    db.execute_batch("DROP TRIGGER single_exit_no_update").unwrap();
    db.execute("UPDATE agent_single_exit_receipts SET rowid=rowid+1000 WHERE root_run_id=?1", [root]).unwrap();
    db.execute_batch(&original).unwrap();
    let tables = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    let after = continuation::verify_settled_history_on(&snapshot, root).unwrap();
    snapshot.rollback().unwrap();
    assert_eq!(after["rootPhysicalHash"], before["rootPhysicalHash"]);
    assert_eq!(after["ownerPhysicalHash"], before["ownerPhysicalHash"]);
    assert_ne!(after["financialPhysicalHash"], before["financialPhysicalHash"], "a physical history fingerprint must bind the verified original Single exit row");
    assert_eq!(super::tests::application_table_snapshot(&db), tables);
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries ORDER BY rowid", []).unwrap() == fees);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn financial_history_multi_rejects_single_exit_protocol_without_new_fees() {
    use crate::agent_runtime::multi_agent::budget::continuation;
    let (f, seen, receipt) = financial_issuance_fixture("multi-single-exit-protocol");
    let db = db::open(&f.context.db_path).unwrap();
    let root = &f.actor.root_run_id;
    let models = seen.lock().unwrap().len();
    let snapshot = db.unchecked_transaction().unwrap();
    let old = continuation::settled_source_history_on(&snapshot, root, &receipt).unwrap();
    assert_eq!(old.as_json()["executionAllowed"], false);
    snapshot.rollback().unwrap();
    db.execute("INSERT INTO agent_single_exit_receipts(receipt_id,root_run_id,control_id,fact_json)
        VALUES('other-scope-exit','other-scope-root','other-scope-control','{}')", []).unwrap();
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(continuation::settled_source_history_on(&snapshot, root, &receipt).unwrap().as_json(), old.as_json(),
        "unrelated Single history cannot alter an original Multi fingerprint");
    snapshot.rollback().unwrap();
    // Cold protocol contamination in this disposable database. A Multi owner
    // cannot have an original Single exit, even if its real invoices are closed.
    db.execute("INSERT INTO agent_single_exit_receipts(receipt_id,root_run_id,control_id,fact_json)
        VALUES('foreign-single-exit',?1,'foreign-single-control','{}')", [root]).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(continuation::settled_source_history_on(&snapshot, root, &receipt).err().unwrap(),
        "continuation_multi_single_exit_scope_conflict");
    assert_eq!(continuation::verify_settled_history_on(&snapshot, root).unwrap_err(),
        "continuation_multi_single_exit_scope_conflict");
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), models);
}

#[test]
fn single_history_balanced_unimplemented_actions_are_not_original_physical_proof() {
    use crate::agent_runtime::multi_agent::budget::{continuation, root::RootOwner};
    for dimension in ["browser_actions", "controlled_writes", "upload_bytes", "concurrency_batches"] {
        let (f, context, seen) = single_model_inflight_paid();
        let db = db::open(&f.path).unwrap();
        let root = &context.run.as_ref().unwrap().run_id;
        let owner = RootOwner::load_single(&db, root).unwrap();
        let control: String = db.query_row("SELECT id FROM agent_root_budget_attempts WHERE root_run_id=?1", [root], |r| r.get(0)).unwrap();
        // Cold balanced entries have no browser/write/upload/batch producer.
        // Zero net balance cannot turn them into original physical action proof.
        for kind in ["reserve", "release"] {
            db.execute("INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,
                dimension,kind,amount,idempotency_key,source_id) VALUES(?1,?2,'',?3,?4,?5,1,?6,'unsupported-original-action')",
                params![Uuid::new_v4().to_string(), root, control, dimension, kind, Uuid::new_v4().to_string()]).unwrap();
        }
        owner.close_single_finance(&db).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let snapshot = db.unchecked_transaction().unwrap();
        assert_eq!(continuation::verify_settled_history_on(&snapshot, root).unwrap_err(),
            "continuation_unsupported_original_action_proof", "{dimension}");
        assert_eq!(native_single_original_history(&snapshot, root).unwrap()["state"], "not_verified");
        snapshot.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
