struct GapReviewFixture {
    source: FollowupFixture,
    context: AgentRunContext,
    session: MultiAgentSession,
    // The born follow-up Root freezes one model endpoint for its lifetime;
    // each review phase selects its Reviewer behaviour through `mode`.
    seen: SpecialistTestSeen,
    mode: std::sync::Arc<std::sync::Mutex<&'static str>>,
    _stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    target: GapReviewTarget,
}

// Restart the exact original approved loopback port, not a new target scope.
struct GapReviewTarget {
    seen: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for GapReviewTarget {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn gap_review_original_target(target: &str) -> GapReviewTarget {
    let url = reqwest::Url::parse(target).unwrap();
    assert_eq!(url.scheme(), "http");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    let address = format!("127.0.0.1:{}", url.port().unwrap());
    let limit = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let listener = loop {
        match std::net::TcpListener::bind(&address) {
            Ok(listener) => break listener,
            Err(error) if std::time::Instant::now() < limit && error.kind()==std::io::ErrorKind::AddrInUse =>
                std::thread::sleep(std::time::Duration::from_millis(10)),
            Err(error) => panic!("cannot restart original fixture target: {error}"),
        }
    };
    listener.set_nonblocking(true).unwrap();
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let counted = seen.clone(); let stopping = stop.clone();
    let thread = std::thread::spawn(move || {
        use std::io::{Read, Write};
        while !stopping.load(std::sync::atomic::Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream,_)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
                    let mut buffer = [0u8;4096];
                    let read = stream.read(&mut buffer).unwrap();
                    assert!(String::from_utf8_lossy(&buffer[..read]).starts_with("GET "));
                    counted.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
                    let body = "control denied access";
                    write!(stream,"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",body.len()).unwrap();
                }
                Err(error) if error.kind()==std::io::ErrorKind::WouldBlock =>
                    std::thread::sleep(std::time::Duration::from_millis(10)),
                Err(error) => panic!("original fixture target failed: {error}"),
            }
        }
    });
    GapReviewTarget {seen,stop,thread:Some(thread)}
}

#[test]
fn gap_review_history_does_not_recursively_embed_execution_guidance() {
    let original = json!({"target":"https://fixture.invalid", "evidence":{"observations":["sealed"]},
        "findingCandidates":[{"id":"original-hypothesis"}]});
    let mut previous = original.clone();
    for index in 0..32 {
        let next = json!({"target":original["target"],"findingCandidates":[],
            "evidence":{"followupObjective":{"historicalCandidate":previous},"observations":["fresh"]},
            "gapFollowup":{"historicalCandidate":previous}});
        let sealed = next.clone();
        previous = gap_review_historical_hypothesis(&next);
        assert_eq!(previous,original,"coverage-only chain {index}");
        assert_eq!(next,sealed,"projection must not mutate sealed source");
    }
    let mut fresh = json!({"target":original["target"],"findingCandidates":[{"id":"fresh-hypothesis"}],
        "evidence":{"observations":["fresh"],"followupObjective":{"historicalCandidate":original}},
        "gapFollowup":{"historicalCandidate":original}});
    let projected = gap_review_historical_hypothesis(&fresh);
    assert_eq!(projected["findingCandidates"],fresh["findingCandidates"]);
    assert_eq!(projected["evidence"],json!({"observations":["fresh"]}));
    // Also sanitize guidance inherited from a previously frozen bundle.
    fresh["findingCandidates"] = json!([]);
    fresh["gapFollowup"]["historicalCandidate"]["evidence"]["followupObjective"] = json!({"nested":"old guidance"});
    assert_eq!(gap_review_historical_hypothesis(&fresh),original);
}

fn gap_review_fixture(with_fact: bool) -> GapReviewFixture {
    // Migrated positive fixture: the follow-up draft is started through the
    // actual web-mode creator/startup and gets a genuinely born frozen Web
    // Root/C (persist_frozen_web_execution_plan + runtime_open_run) instead of
    // a hand-rolled partial root. Original 60,000 tokens / 20 requests
    // (soft 30,000 / 10) unchanged.
    let source = followup_fixture();
    let mut connection = db::open(&source.context.db_path).unwrap();
    let target = source.context.target_url.clone();
    let target_endpoint = gap_review_original_target(&target);
    // Production follow-up startup requires fresh identities and an explicit
    // operator control group (same setup as the passing
    // followup_fresh_identities_and_explicit_controls_are_required_before_start).
    let mut input = followup_input(&source);
    add_followup_identities(&connection,&mut input);
    let draft = create_agent_gap_followup_in(&connection,&input).unwrap();
    let setup = authorization_control_setup_in(&connection,&source.root,&draft.id,&target).unwrap();
    save_authorization_control_in(&mut connection,&source.root,&SaveAuthorizationControlInput {
        scan_id:draft.id.clone(),attempt_number:setup.attempt_number,target_url:target.clone(),
        contract_key:"idor|/api/orders".into(),owner_object_url:format!("{target}/api/orders?id=owner"),
        tester_control_url:format!("{target}/api/orders?id=tester"),object_query_key:"id".into(),
        owner_object_value:"owner".into(),tester_object_value:"tester".into(),response_object_pointer:"/order/id".into(),
        owner_identity:"fresh-owner".into(),tester_identity:"fresh-tester".into(),
    }).unwrap();
    web_mode_test_start(&source.root,&mut connection,&draft.id,WebStartMode::Confirm).unwrap();
    let mut plan = test_plan_for("standard",&target).with_attempt(1);
    plan.hard_total_tokens = 60_000;
    plan.soft_uncached_tokens = 30_000;
    plan.hard_model_requests = 20;
    plan.soft_model_requests = 10;
    plan.max_turns = if with_fact {2} else {1}; // Original birth covers real GET plus final SDK exit.
    persist_frozen_web_execution_plan(&source.context.db_path,&draft.id,1,&target,&plan).unwrap();
    let mode = std::sync::Arc::new(std::sync::Mutex::new("valid"));
    let handler_mode = mode.clone();
    let web_target = target.clone();
    let web_rounds = std::sync::atomic::AtomicUsize::new(0);
    let (port,seen,stop)=spawn_endpoint(std::sync::Arc::new(move |request| {
        if request.contains("You are the Root Coordinator") {
            let body:JsonValue=serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input:JsonValue=serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let phase = input["basis"]["phase"].as_str().unwrap_or("");
            let step = match phase {"new-evidence"=>Some("assess:new_evidence"),
                "client-side-output"=>Some("assess:client_side_configuration"),
                "input-parser-output"=>Some(""),_=>None};
            let reply = if let Some(step)=step {
                proposal_model_response(&json!({"schemaVersion":1,"observed":["actual original control HTTP receipt"],
                    "missing":[],"suggestions":if step.is_empty(){json!([])}else{json!([step])},"costNotes":[],"risks":[]}).to_string())
            } else {changed_fact_reviewer_response(&request,true)};
            return (200,"application/json",reply);
        }
        if request.contains("SPA/API Mapper") {
            return (200,"application/json",proposal_model_response(r#"{"summary":"actual frozen Mapper","priorityContracts":[],"risks":[]}"#));
        }
        if specialist_original_is_web_sdk(&request) {
            let round = web_rounds.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            let reply = if with_fact && round==0 {
                model_round(&[("replay_http",json!({"identity":"anonymous","method":"GET","url":web_target}))],10)
            } else {proposal_model_response("actual bounded Web exit; no new target tool")};
            return (200,"application/json",reply);
        }
        if request.contains("你是独立ClientSide配置事实专家") {
            let body:JsonValue=serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input:JsonValue=serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let refs:Vec<_>=input["observations"].as_array().unwrap().iter().map(|row|row["id"].clone()).collect();
            return (200,"application/json",proposal_model_response(&json!({"summary":"actual local control metadata only",
                "observationRefs":refs,"gaps":["missing_browser_validation"],"candidates":[]}).to_string()));
        }
        let mutation: &'static str = *handler_mode.lock().unwrap();
        if request.contains("你是独立的 IdentitySession 分析专家") {
            // The required fresh identities make the born Root run its real
            // read-only IdentitySession metadata assessment during prepare.
            return (200,"application/json",changed_fact_reviewer_response(&request,true));
        }
        let body: JsonValue=serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let candidate: JsonValue=serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        if candidate.get("gapFollowup").is_none() {
            return (200,"application/json",proposal_model_response(&gap_receipt_response()));
        }
        let mut response=gap_review_response(&candidate,if mutation=="insufficient" {"insufficient_evidence"} else {"rejected"});
        match mutation {
            "old_fact"=>response["gapAssessment"]["items"][0]["factRefs"]=json!(["ev-from-old-root"]),
            "no_assessment"=>{response.as_object_mut().unwrap().remove("gapAssessment");},
            "wrong_source"=>response["gapAssessment"]["sourceHash"]=json!("wrong-source"),
            _=>{},
        }
        (200,"application/json",proposal_model_response(&response.to_string()))
    }));
    let mut context = test_context(&source.context.db_path,&target,vec![AgentIdentity::anonymous()]);
    context.scan_id = draft.id.clone();
    context.execution_plan = plan;
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let run = runtime_open_run(&context.db_path,&draft.id,&context.route).unwrap();
    let work: String = connection.query_row(
        "SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=1",[&draft.id],|r|r.get(0)).unwrap();
    context.target_dir = PathBuf::from(work).join("url-pipeline/target-00000");
    fs::create_dir_all(&context.target_dir).unwrap();
    fs::write(context.target_dir.join(".oviraptor-scan-id"),&context.scan_id).unwrap();
    fs::write(context.target_dir.join("frontend-evidence.json"),context.evidence.to_string()).unwrap();
    context.run = Some(run);
    bind_agent_evidence_location(&context).unwrap();
    native_frozen_web_root_mode(&context).unwrap();
    let _transport = RealSpecialistTransport::enter();
    let mut session = {
        let _transport = RealSpecialistTransport::enter();
        multi_agent_prepare(&mut context).unwrap()
    };
    let outcome = {
        let _transport = RealSpecialistTransport::enter();
        run_native_agent(&context)
    };
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)), "{}",outcome.detail());
    assert_eq!(target_endpoint.seen.load(std::sync::atomic::Ordering::SeqCst),usize::from(with_fact));
    multi_agent_finish_execution(&context,&mut session,&outcome).unwrap();
    let web_calls=seen.lock().unwrap().iter().filter(|wire|specialist_original_is_web_sdk(wire)).count();
    assert_eq!(web_calls,if with_fact {2}else{1});
    let usage:(i64,i64,i64)=connection.query_row("SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1",
        [&session.executor.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(usage,if with_fact {(70,0,2)}else{(20,0,1)},"actual Web SDK invoices retained");
    let claims:i64=connection.query_row("SELECT count(*) FROM agent_http_request_claims WHERE run_id=?1 AND response_status=200 AND received_at<>''",
        [&session.executor.run_id],|r|r.get(0)).unwrap();
    assert_eq!(claims,i64::from(with_fact));
    let observed:i64=connection.query_row("SELECT count(*) FROM agent_evidence_nodes WHERE root_run_id=?1 AND created_by_run_id=?2 AND kind='request_record' AND provenance='observed'",
        params![session.lease.root_run_id,session.executor.run_id],|r|r.get(0)).unwrap();
    assert_eq!(observed,i64::from(with_fact),"observed control facts come from the actual original Web Broker GET");
    if with_fact {
        let record:JsonValue=serde_json::from_slice(&fs::read(context.target_dir.join("agent-http/0001.json")).unwrap()).unwrap();
        assert!(record["response"]["httpClaim"]["requestHash"].as_str().is_some_and(|v|v.len()==64),"actual transport ties artifact to original HTTP claim: {record}");
        assert_eq!(fs::read(context.target_dir.join("agent-http/0001.body")).unwrap(),b"control denied access");
    }
    let target_cost=crate::agent_runtime::multi_agent::budget::balance(&connection,&session.lease.root_run_id,None,"target_requests").unwrap();
    assert_eq!((target_cost.consumed,target_cost.reserved,target_cost.indeterminate),(i64::from(with_fact),0,0));
    let before=super::tests::application_table_snapshot(&connection);
    let sdk=seen.lock().unwrap().len();
    multi_agent_finish_execution(&context,&mut session,&outcome).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&connection),before);
    assert_eq!(seen.lock().unwrap().len(),sdk);
    assert_eq!(target_endpoint.seen.load(std::sync::atomic::Ordering::SeqCst),usize::from(with_fact));
    connection.execute("UPDATE agent_runs SET status='terminal',terminal_state='incomplete' WHERE id=?1",[&source.root_run_id]).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='completed_with_gaps' WHERE id=?1",[&source.context.scan_id]).unwrap();
    connection.execute("UPDATE sentinel_scan_attempts SET status='completed_with_gaps' WHERE scan_id=?1",[&source.context.scan_id]).unwrap();
    GapReviewFixture {source,context,session,seen,mode,_stop:stop,target:target_endpoint}
}

fn gap_review_response(candidate: &JsonValue, verdict: &str) -> JsonValue {
    let addressed=verdict!="insufficient_evidence";
    json!({"verdict":verdict,"reasonCodes":["fresh_control"],"missingEvidence":if addressed {json!([])} else {json!(["missing control"])},
        "confidence":0.9,"summary":"Fresh control reviewed; no new finding is fabricated",
        "gapAssessment":{"sourceHash":candidate["gapFollowup"]["source"]["sourceHash"],"hypothesisVerdict":verdict,"items":[{
            "index":0,"missingEvidence":"missing control","status":if addressed {"addressed"} else {"insufficient"},
            "factRefs":if addressed {candidate["persistedFactRefs"].clone()} else {json!([])},
            "reason":"The fresh control supplies the previously missing observation"}]}})
}

fn run_gap_review(f: &mut GapReviewFixture, mutation: &'static str) -> (AgentTargetOutcome,usize) {
    *f.mode.lock().unwrap() = mutation;
    // Per-phase count of Reviewer/Investigator SDK requests, as before; the
    // born Root's own ticks and Mapper are routed separately, not counted.
    let before = specialist_handler_call_count(&f.seen);
    let target_before=f.target.seen.load(std::sync::atomic::Ordering::SeqCst);
    let web_before=f.seen.lock().unwrap().iter().filter(|wire|specialist_original_is_web_sdk(wire)).count();
    let _transport=RealSpecialistTransport::enter();
    let outcome=multi_agent_review(&f.context,&mut f.session,AgentTargetOutcome::incomplete("fixture coverage"));
    let count=specialist_handler_call_count(&f.seen)-before;
    assert_eq!(f.target.seen.load(std::sync::atomic::Ordering::SeqCst),target_before);
    assert_eq!(f.seen.lock().unwrap().iter().filter(|wire|specialist_original_is_web_sdk(wire)).count(),web_before);
    (outcome,count)
}

#[test]
fn gap_review_real_independent_receipt_closes_link_without_rewriting_old_root_or_fabricating_findings() {
    let mut f=gap_review_fixture(true);
    assert_eq!(f.context.evidence["followupObjective"]["targetRequestsGranted"],0);
    assert_eq!(f.context.evidence["followupObjective"]["source"]["missingEvidence"],json!(["missing control"]));
    let connection=db::open(&f.context.db_path).unwrap();
    let source_before:String=connection.query_row("SELECT candidate_json FROM agent_review_requests WHERE root_run_id=?1",[&f.source.root_run_id],|r|r.get(0)).unwrap();
    // The fresh application fixture already contains seeded findings. The
    // review must add none, rather than assuming a globally empty database.
    let findings_query = "SELECT coalesce(json_group_array(json_object('id',id,'scanId',scan_id,'targetUrl',target_url,'stage',stage,'kind',kind,'recordKey',record_key,'title',title,'severity',severity,'record',record_json,'updatedAt',updated_at)),'[]') FROM (SELECT * FROM sentinel_findings ORDER BY id)";
    let findings_before:String=connection.query_row(findings_query,[],|r|r.get(0)).unwrap();
    let (outcome,calls)=run_gap_review(&mut f,"valid");
    assert!(outcome.detail().contains("review_gate_rejected"),"{outcome:?}");
    assert_eq!(calls,1);
    let tx=connection.unchecked_transaction().unwrap();
    let status=gap_review_status(&tx,&f.context.scan_id).unwrap();
    assert_eq!(status["status"],"resolved","{status}");
    assert_eq!(status["verdict"],"rejected");
    assert_eq!(status["candidateRevision"],1,"new root has its own revision sequence, not the source's permissions");
    // The source link carries the source Root's own reviewed revision (read
    // from the source ledger: the born source Root reviews revision 1), never
    // the new Root's sequence or permissions.
    let source_revision:i64=tx.query_row("SELECT candidate_revision FROM agent_review_decisions WHERE root_run_id=?1 AND verdict='insufficient_evidence'",[&f.source.root_run_id],|r|r.get(0)).unwrap();
    assert_eq!(f.context.evidence["followupObjective"]["source"]["candidateRevision"],source_revision);
    let source=gap_followup_relations(&tx,&f.source.context.scan_id).unwrap();
    assert_eq!(source["tasks"][0]["review"]["gapResolved"],true);
    assert_eq!(source["gapResolved"],false,"one child is not proof for the whole source scan");
    let findings_after:String=tx.query_row(findings_query,[],|r|r.get(0)).unwrap();
    assert_eq!(findings_after,findings_before,"a rejected review must not create, remove or rewrite findings, including existing coverage records");
    let source_after:String=tx.query_row("SELECT candidate_json FROM agent_review_requests WHERE root_run_id=?1",[&f.source.root_run_id],|r|r.get(0)).unwrap();
    assert_eq!(source_before,source_after);
    drop(tx);
    let (_,calls)=run_gap_review(&mut f,"valid");
    assert_eq!(calls,0,"frozen review must replay locally");
    connection.execute("UPDATE agent_runs SET status='terminal',terminal_state='incomplete' WHERE id=?1",[&f.session.lease.root_run_id]).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='completed_with_gaps' WHERE id=?1",[&f.context.scan_id]).unwrap();
    let tx=connection.unchecked_transaction().unwrap();
    assert_eq!(gap_review_status(&tx,&f.context.scan_id).unwrap()["gapResolved"],true,"history does not require live permissions");
}

#[test]
fn gap_review_rejects_unbacked_model_claims_and_keeps_original_gap_open() {
    for (with_fact,mutation) in [(true,"old_fact"),(true,"no_assessment"),(true,"wrong_source"),(false,"valid")] {
        let mut f=gap_review_fixture(with_fact);
        let (outcome,calls)=run_gap_review(&mut f,mutation);
        assert_eq!(calls,1,"{mutation}");
        assert!(matches!(outcome,AgentTargetOutcome::Failed(_) | AgentTargetOutcome::Incomplete(_)),"{outcome:?}");
        let connection=db::open(&f.context.db_path).unwrap();
        let count:i64=connection.query_row("SELECT COUNT(*) FROM agent_gap_review_receipts",[],|r|r.get(0)).unwrap();
        assert_eq!(count,0,"{mutation}: {outcome:?}");
        let tx=connection.unchecked_transaction().unwrap();
        assert_eq!(gap_review_status(&tx,&f.context.scan_id).unwrap()["gapResolved"],false);
    }
}

#[test]
fn gap_review_receipt_insert_failure_rolls_back_entire_delivery_and_recovers_without_model_retry() {
    for (timing,failure) in [("BEFORE","SELECT RAISE(ABORT,'receipt fault');"),("BEFORE","SELECT RAISE(IGNORE);"),
        ("AFTER","DELETE FROM agent_gap_review_receipts WHERE request_id=NEW.request_id;")] {
        let mut f=gap_review_fixture(true);
        let connection=db::open(&f.context.db_path).unwrap();
        connection.execute_batch(&format!("CREATE TRIGGER gap_receipt_fault {timing} INSERT ON agent_gap_review_receipts BEGIN {failure} END;")).unwrap();
        let (outcome,calls)=run_gap_review(&mut f,"valid");
        assert_eq!(calls,1); assert!(matches!(outcome,AgentTargetOutcome::Failed(_)),"{outcome:?}");
        let count:i64=connection.query_row("SELECT COUNT(*) FROM agent_review_decisions WHERE root_run_id=?1",[&f.session.lease.root_run_id],|r|r.get(0)).unwrap();
        assert_eq!(count,0);
        connection.execute_batch("DROP TRIGGER gap_receipt_fault").unwrap();
        let (outcome,calls)=run_gap_review(&mut f,"valid");
        assert_eq!(calls,0,"{outcome:?}");
        let tx=connection.unchecked_transaction().unwrap();
        let status=gap_review_status(&tx,&f.context.scan_id).unwrap();
        assert_eq!(status["gapResolved"],true,"{outcome:?}; {status}");
    }
}

#[test]
fn gap_review_status_revalidates_bytes_mailbox_and_receipts_instead_of_trusting_completed_task() {
    for mutation in ["fresh_bytes","source_bytes","mailbox","response","new_attempt","receipt"] {
        let mut f=gap_review_fixture(true);
        run_gap_review(&mut f,"valid");
        let connection=db::open(&f.context.db_path).unwrap();
        match mutation {
            "fresh_bytes"=>fs::write(f.context.target_dir.join("agent-http/0001.body"),"changed").unwrap(),
            "source_bytes"=>fs::write(f.source.context.target_dir.join("agent-http/0001.body"),"changed").unwrap(),
            "mailbox"=>{connection.execute("UPDATE agent_messages SET acknowledged_at='' WHERE root_run_id=?1 AND kind='review_decision'",[&f.session.lease.root_run_id]).unwrap();},
            "response"=>{
                assert!(connection.execute("UPDATE agent_specialist_calls SET response_json='{}' WHERE root_run_id=?1 AND role='evidence_reviewer'",[&f.session.lease.root_run_id]).is_err());
                // Simulate on-disk corruption after proving the normal write is blocked.
                connection.execute_batch("DROP TRIGGER agent_specialist_call_immutable").unwrap();
                connection.execute("UPDATE agent_specialist_calls SET response_json='{}' WHERE root_run_id=?1 AND role='evidence_reviewer'",[&f.session.lease.root_run_id]).unwrap();
            },
            "new_attempt"=>{connection.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",[&f.context.scan_id]).unwrap();},
            _=>{connection.execute("DELETE FROM agent_gap_review_receipts",[]).unwrap();},
        }
        let tx=connection.unchecked_transaction().unwrap();
        assert_eq!(gap_review_status(&tx,&f.context.scan_id).unwrap()["gapResolved"],false,"{mutation}");
    }
}

#[test]
fn gap_review_assessment_requires_exact_items_and_fresh_visible_refs() {
    let f=gap_review_fixture(true);
    let connection=db::open(&f.context.db_path).unwrap();
    let tx=connection.unchecked_transaction().unwrap();
    let candidate=json!({"gapFollowup":gap_followup_review_context(&tx,&f.session.lease.root_run_id,&f.context.target_dir).unwrap(),
        "persistedFactRefs":review_fact_refs(&tx,&f.session.lease.root_run_id,&f.context.target_dir).unwrap()});
    let valid=gap_review_response(&candidate,"rejected");
    assert!(validate_gap_assessment(&candidate,&validated_review_decision(&valid.to_string()).unwrap()).unwrap());
    for mutation in ["index","text","extra","empty","duplicate","reason","verdict"] {
        let mut response=valid.clone();
        match mutation {
            "index"=>response["gapAssessment"]["items"][0]["index"]=json!(1),
            "text"=>response["gapAssessment"]["items"][0]["missingEvidence"]=json!("different gap"),
            "extra"=>response["gapAssessment"]["items"][0]["allow"]=json!(true),
            "empty"=>response["gapAssessment"]["items"][0]["factRefs"]=json!([]),
            "duplicate"=>{let id=response["gapAssessment"]["items"][0]["factRefs"][0].clone();response["gapAssessment"]["items"][0]["factRefs"]=json!([id,id]);},
            "reason"=>response["gapAssessment"]["items"][0]["reason"]=json!(""),
            _=>response["gapAssessment"]["hypothesisVerdict"]=json!("insufficient_evidence"),
        }
        assert!(validate_gap_assessment(&candidate,&validated_review_decision(&response.to_string()).unwrap()).is_err(),"{mutation}");
    }
    assert!(!validate_gap_assessment(&candidate,&validated_review_decision(&gap_review_response(&candidate,"insufficient_evidence").to_string()).unwrap()).unwrap());
    let mut distinct=valid.clone();
    distinct["verdict"]=json!("insufficient_evidence");
    distinct["missingEvidence"]=json!(["new unrelated finding lacks evidence"]);
    assert!(validate_gap_assessment(&candidate,&validated_review_decision(&distinct.to_string()).unwrap()).unwrap(),
        "new finding verdict must not overwrite the original hypothesis verdict");
}

#[test]
fn gap_review_concurrent_receipt_recovery_commits_exactly_once() {
    let mut f=gap_review_fixture(true);
    let connection=db::open(&f.context.db_path).unwrap();
    connection.execute_batch("CREATE TRIGGER gap_receipt_fault BEFORE INSERT ON agent_gap_review_receipts BEGIN SELECT RAISE(ABORT,'fault'); END;").unwrap();
    run_gap_review(&mut f,"valid");
    connection.execute_batch("DROP TRIGGER gap_receipt_fault").unwrap();
    let (candidate,revision,text):(String,i64,String)=connection.query_row(
        "SELECT candidate_id,candidate_revision,candidate_json FROM agent_review_requests WHERE root_run_id=?1",
        [&f.session.lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));
    let results=std::thread::scope(|scope| {
        let handles:Vec<_>=(0..2).map(|_|{
            let gate=barrier.clone();let path=&f.context.db_path;let lease=&f.session.lease;
            let dir=&f.context.target_dir;let candidate=&candidate;let text=&text;
            scope.spawn(move || {let db=db::open(path).unwrap();gate.wait();recover_received_review(&db,lease,candidate,revision,text,dir).is_ok()})
        }).collect();
        handles.into_iter().map(|h|h.join().unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|v|**v).count(),1);
    let count:i64=connection.query_row("SELECT COUNT(*) FROM agent_gap_review_receipts",[],|r|r.get(0)).unwrap();
    assert_eq!(count,1);
    let tx=connection.unchecked_transaction().unwrap();
    assert_eq!(gap_review_status(&tx,&f.context.scan_id).unwrap()["gapResolved"],true);
}

#[test]
fn gap_review_source_is_revalidated_before_any_new_specialist_or_target_capability() {
    let mut f=gap_review_fixture(false);
    let connection=db::open(&f.context.db_path).unwrap();
    connection.execute("UPDATE agent_messages SET acknowledged_at='' WHERE root_run_id=?1 AND kind='proposal_assessed'",[&f.source.root_run_id]).unwrap();
    f.context.run.as_mut().unwrap().run_id=f.session.lease.root_run_id.clone();
    // The born Root's first invocation still owns the native invocation lock;
    // the re-entry under test is a new invocation, so end the first one.
    let GapReviewFixture {mut context,session,..}=f;
    drop(session);
    let before=review_delivery_snapshot(&connection);
    let _transport=RealSpecialistTransport::enter();
    assert!(multi_agent_prepare(&mut context).err().unwrap().contains("followup_acknowledged_assessment_unavailable"));
    assert_eq!(review_delivery_snapshot(&connection),before);
}

#[test]
fn gap_review_no_new_facts_preserves_insufficient_receipt_and_does_not_repeat_model_rounds() {
    let mut f=gap_review_fixture(false);
    let (outcome,calls)=run_gap_review(&mut f,"insufficient");
    assert!(matches!(outcome,AgentTargetOutcome::Incomplete(_)),"{outcome:?}");
    assert_eq!(calls,2,"one Reviewer and one bounded Investigator");
    let connection=db::open(&f.context.db_path).unwrap();
    let tx=connection.unchecked_transaction().unwrap();
    let status=gap_review_status(&tx,&f.context.scan_id).unwrap();
    assert_eq!(status["status"],"insufficient_evidence","{status}");
    assert_eq!(status["gapResolved"],false);
    drop(tx);
    let (_,calls)=run_gap_review(&mut f,"insufficient");
    assert_eq!(calls,0);
}
