// Classify actual provider requests by the current input, never historical
// substrings, and bind every Root SDK to its durable original received bill.
fn followup_original_root_phase_counts(
    db: &rusqlite::Connection,
    root: &str,
    wires: &[String],
) -> std::collections::BTreeMap<String, usize> {
    let mut actual = std::collections::BTreeMap::new();
    for wire in wires
        .iter()
        .filter(|w| w.contains("You are the Root Coordinator"))
    {
        let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
        let input: JsonValue =
            serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        let phase = input["basis"]["phase"]
            .as_str()
            .unwrap_or("<bootstrap>")
            .to_string();
        *actual.entry(phase).or_insert(0usize) += 1;
    }
    let rows: Vec<(String, i64)> = db.prepare("SELECT COALESCE(json_extract(q.fact_json,'$.request.basis.phase'),'<bootstrap>'),count(*) FROM agent_root_tick_receipts q JOIN agent_root_model_journal j ON j.call_id=q.call_id AND j.root_run_id=q.root_run_id AND j.phase='received' WHERE q.root_run_id=?1 AND q.phase='request' GROUP BY COALESCE(json_extract(q.fact_json,'$.request.basis.phase'),'<bootstrap>') ORDER BY 1").unwrap().query_map([root], |r|Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    let original: std::collections::BTreeMap<_, _> = rows
        .into_iter()
        .map(|(phase, count)| {
            (
                phase,
                usize::try_from(count).expect("original SQL count must fit usize"),
            )
        })
        .collect();
    eprintln!("original Root phase ledger: {original:?}");
    assert_eq!(
        actual, original,
        "every actual Root SDK must have exactly one original received bill in its exact phase"
    );
    original
}

struct FollowupFixture {
    root: PathBuf,
    context: AgentRunContext,
    root_run_id: String,
    assessment_id: String,
    _fixture: WebModeFixture,
}

fn followup_fixture() -> FollowupFixture {
    followup_fixture_with_review_audit(FollowupLeaseFault::None, |_, _, _, _, _, _| {})
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FollowupLeaseFault {
    None,
    AckDrift,
    PublicationAbort,
    PublicationIgnore,
}

// Fault assertions run while the original session, C and HTTP endpoints remain
// alive. Retain the original call baseline plus explicitly proven financial obligations.
fn followup_fixture_with_review_audit(
    lease_fault: FollowupLeaseFault,
    audit: impl FnOnce(
        &AgentRunContext,
        &MultiAgentSession,
        &rusqlite::Connection,
        &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        &AgentTargetOutcome,
    ),
) -> FollowupFixture {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let lease_database = Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let source_database = lease_database.clone();
    let deep_expiry = EXPIRE_RECEIVED_DEEP_ONCE.with(std::cell::Cell::get);
    let reviewer_expiry = EXPIRE_RECEIVED_REVIEWER_ONCE.with(std::cell::Cell::get);
    let (target_port, target_seen, target_stop) =
        spawn_endpoint(Arc::new(|_| (200, "text/plain", "safe body".into())));
    let target = format!("http://127.0.0.1:{target_port}");
    let web_target = target.clone();
    let rounds = AtomicUsize::new(0);
    let (port, seen, model_stop) = spawn_endpoint(Arc::new(move |request| {
        if request.contains("You are the Root Coordinator") {
            let body: JsonValue =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue =
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            if input["basis"]["phase"] == "lease-expired" {
                if lease_fault != FollowupLeaseFault::None {
                    // Only after the actual lease assessment SDK arrives: the
                    // original grant, paid Deep result and both ACKs existed.
                    let path = source_database.lock().unwrap().clone().unwrap();
                    let db = rusqlite::Connection::open_with_flags(
                        path,
                        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
                    )
                    .unwrap();
                    match lease_fault {
                        FollowupLeaseFault::AckDrift => {
                            assert_eq!(db.execute("UPDATE agent_messages SET acknowledged_at='' WHERE kind='gap_proposed' AND from_agent='deep_investigator'", []).unwrap(), 1);
                        }
                        FollowupLeaseFault::PublicationAbort
                        | FollowupLeaseFault::PublicationIgnore => {
                            let raise = if lease_fault == FollowupLeaseFault::PublicationAbort {
                                "RAISE(ABORT,'deep_lease_publication_fault')"
                            } else {
                                "RAISE(IGNORE)"
                            };
                            db.execute_batch(&format!("CREATE TRIGGER deep_lease_publication_fault BEFORE INSERT ON agent_root_tick_receipts WHEN NEW.phase='publication' AND EXISTS(SELECT 1 FROM agent_root_tick_receipts r WHERE r.root_run_id=NEW.root_run_id AND r.call_id=NEW.call_id AND r.phase='request' AND json_extract(r.fact_json,'$.request.basis.phase')='lease-expired') BEGIN SELECT {raise}; END;")).unwrap();
                        }
                        FollowupLeaseFault::None => unreachable!(),
                    }
                }
                return (200,"application/json",proposal_model_response(&json!({"schemaVersion":1,"observed":["original signed expired Gap worker"],"missing":[],"suggestions":["assess:lease_expired"],"costNotes":[],"risks":[]}).to_string()));
            }
        }
        let response = if request.contains("你是独立ClientSide配置事实专家") {
            let body: JsonValue =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue =
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let refs: Vec<_> = input["observations"]
                .as_array()
                .map(|a| a.iter().map(|o| o["id"].clone()).collect())
                .unwrap_or_default();
            proposal_model_response(&json!({"summary":"original localhost header facts only","observationRefs":refs,"gaps":["missing_browser_validation"],"candidates":[]}).to_string())
        } else if request.contains("You are the Root Coordinator")
            && request.contains("client-side-output")
        {
            proposal_model_response(&json!({"schemaVersion":1,"observed":["paid original readonly configuration facts"],"missing":["missing_browser_validation"],"suggestions":["assess:client_side_configuration"],"costNotes":[],"risks":[]}).to_string())
        } else if request.contains("You are the Root Coordinator")
            && request.contains("input-parser-output")
        {
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original localhost request"],"missing":[],"suggestions":[],"costNotes":[],"risks":[]}).to_string())
        } else if request.contains("独立 Evidence Reviewer") {
            proposal_model_response(
                &json!({"verdict":"insufficient_evidence",
                "reasonCodes":["missing_control"],"missingEvidence":["missing control"],
                "confidence":0.2,"summary":"actual independent Reviewer needs control"})
                .to_string(),
            )
        } else if request.contains("You are the Root Coordinator")
            && request.contains("new-evidence")
        {
            proposal_model_response(
                &json!({"schemaVersion":1,"observed":["original localhost HTTP receipt"],
                "missing":[],"suggestions":["assess:new_evidence"],"costNotes":[],"risks":[]})
                .to_string(),
            )
        } else if request.contains("You are the Root Coordinator")
            || request.contains("SPA/API Mapper")
            || request.contains("独立的只读 Deep Investigator")
        {
            paid_proposal_response(&request, false)
        } else if rounds.fetch_add(1, Ordering::SeqCst) == 0 {
            model_round(
                &[(
                    "replay_http",
                    json!({"identity":"anonymous","method":"GET","url":web_target,"family":"authorization"}),
                )],
                10,
            )
        } else {
            model_round(&[], 10)
        };
        (200, "application/json", response)
    }));
    let fixture = web_mode_fixture("multi", &target);
    let mut f = root_tick_fixture_from_web_mode_turns(
        "gap-followup",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (60_000, 20),
        None,
        fixture,
        Some(2),
    );
    *lease_database.lock().unwrap() = Some(f.context.db_path.clone());
    drop(f.parent.take());
    let _transport = RealSpecialistTransport::enter();
    let mut session = multi_agent_prepare(&mut f.context).unwrap();
    let context = &f.context;
    let connection = db::open(&context.db_path).unwrap();
    let outcome = run_native_agent(context);
    assert!(
        matches!(
            outcome,
            AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)
        ),
        "{}",
        outcome.detail()
    );
    assert_eq!(target_seen.lock().unwrap().len(), 1);
    assert_eq!(
        review_fact_refs(&connection, &session.lease.root_run_id, &context.target_dir)
            .unwrap()
            .len(),
        1
    );
    stage_agent_finding(
        context,
        AGENT_VULNERABILITY_STAGE,
        "vulnerability",
        "followup-hypothesis",
        "independently authorized control absent",
        "info",
        &json!({"claim":"missing control"}),
    )
    .unwrap();
    multi_agent_finish_execution(context,&mut session,&outcome).unwrap_or_else(|code| {
        let rows: Vec<(String,i64,i64,String)> = connection.prepare("SELECT role,reserved_tokens,reserved_requests,state FROM agent_assignments ORDER BY rowid").unwrap().query_map([], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        let remaining = crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(&connection, &session.lease.root_run_id);
        panic!("{code}; actual calls={}; remaining={remaining:?}; grants={rows:?}", seen.lock().unwrap().len());
    });
    let reviewed = multi_agent_review(context, &mut session, outcome);
    assert!(
        !matches!(reviewed, AgentTargetOutcome::Completed(_)),
        "{reviewed:?}"
    );
    audit(
        context,
        &session,
        &connection,
        &seen,
        &target_seen,
        &reviewed,
    );
    let phases = followup_original_root_phase_counts(
        &connection,
        &session.lease.root_run_id,
        &seen.lock().unwrap(),
    );
    assert_eq!(
        phases.get("budget-consumption").copied(),
        Some(2),
        "each of the two actual Web calls has its own paid consumption assessment"
    );
    assert_eq!(
        phases.get("http-budget-consumption").copied(),
        Some(1),
        "the one actual sealed GET has an independent paid assessment"
    );
    for phase in ["budget-consumption", "http-budget-consumption"] {
        let (publications, requests): (i64,i64) = connection.query_row("SELECT count(*),sum(json_extract(p.fact_json,'$.event.usage.modelRequests')) FROM agent_root_tick_receipts p JOIN agent_root_tick_receipts q ON q.call_id=p.call_id AND q.root_run_id=p.root_run_id AND q.phase='request' WHERE p.root_run_id=?1 AND p.phase='publication' AND json_extract(q.fact_json,'$.request.basis.phase')=?2",params![session.lease.root_run_id,phase],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let calls = i64::try_from(phases[phase]).expect("actual phase count must fit i64");
        assert_eq!(
            (publications, requests),
            (calls, calls),
            "original financial assessment must be paid and published once"
        );
    }
    let financial_calls = phases["budget-consumption"] + phases["http-budget-consumption"];
    let expected_calls = 13
        + financial_calls
        + usize::from(reviewer_expiry)
        + usize::from(deep_expiry && lease_fault != FollowupLeaseFault::AckDrift);
    assert_eq!(seen.lock().unwrap().len(),expected_calls,"actual Web, budget/new-evidence/ClientSide feedback and independent review SDK, plus signed lease supervision when expired");
    if (deep_expiry || reviewer_expiry) && lease_fault != FollowupLeaseFault::AckDrift {
        let requests:i64=connection.query_row("SELECT count(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request' AND json_extract(fact_json,'$.request.basis.phase')='lease-expired'",[&session.lease.root_run_id],|row|row.get(0)).unwrap();
        let (publications,model_requests):(i64,i64)=connection.query_row("SELECT count(*),sum(json_extract(p.fact_json,'$.event.usage.modelRequests')) FROM agent_root_tick_receipts p JOIN agent_root_tick_receipts r ON r.call_id=p.call_id AND r.root_run_id=p.root_run_id AND r.phase='request' WHERE p.root_run_id=?1 AND p.phase='publication' AND json_extract(r.fact_json,'$.request.basis.phase')='lease-expired'",[&session.lease.root_run_id],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        let lease_calls = i64::from(deep_expiry) + i64::from(reviewer_expiry);
        assert_eq!(
            (requests, publications, model_requests),
            (lease_calls, lease_calls, lease_calls)
        );
    }
    if lease_fault != FollowupLeaseFault::AckDrift {
        let (candidate, revision, missing) = paid_proposal_candidate(&connection);
        assert_eq!(missing, json!(["missing control"]));
        let before = web_mode_test_rows(&connection);
        let before_all = deep_expiry.then(|| super::tests::application_table_snapshot(&connection));
        multi_agent_investigate_review_gap(context, &session, &candidate, revision, &missing)
            .unwrap();
        assert_eq!(
            seen.lock().unwrap().len(),
            expected_calls,
            "saved assessment replays without SDK or target permission"
        );
        web_mode_assert_rows(&connection, &before);
        if let Some(before_all) = before_all {
            assert!(super::tests::application_table_snapshot(&connection) == before_all);
        }
    }
    let assessment_id = connection
        .query_row(
            "SELECT id FROM agent_messages WHERE kind='proposal_assessed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let root_run_id = session.lease.root_run_id.clone();
    drop(session);
    target_stop.store(true, Ordering::SeqCst);
    model_stop.store(true, Ordering::SeqCst);
    FollowupFixture {
        root: f.f.root.clone(),
        context: f.context,
        root_run_id,
        assessment_id,
        _fixture: f.f,
    }
}

fn followup_preview(f: &FollowupFixture) -> Result<GapFollowupPreview, String> {
    let connection = db::open(&f.context.db_path)?;
    let transaction = connection.unchecked_transaction().unwrap();
    gap_followup_preview_in(&transaction, &f.context.scan_id, &f.assessment_id)
}

fn followup_input(f: &FollowupFixture) -> GapFollowupDraftInput {
    GapFollowupDraftInput {
        request_id: Uuid::new_v4().to_string(),
        source_scan_id: f.context.scan_id.clone(),
        assessment_message_id: f.assessment_id.clone(),
        source_hash: followup_preview(f).unwrap().source_hash,
        task_name: "Fresh evidence".into(),
        scan_mode: Some("standard".into()),
        max_budget_usd: Some(5.0),
        auth_session_ids: vec![],
        auth_session_scope_id: Uuid::new_v4().to_string(),
        skill_ids: vec![],
        instruction: String::new(),
        closure: None,
    }
}

#[test]
fn followup_preview_and_atomic_draft_are_readonly_and_durably_linked() {
    let f = followup_fixture();
    let connection = db::open(&f.context.db_path).unwrap();
    // Preparing a new task from a closed source is evidence reading, not
    // resuming its execution. No live source capability may be required.
    connection
        .execute(
            "UPDATE agent_runs SET status='terminal',terminal_state='incomplete' WHERE id=?1",
            [&f.root_run_id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE sentinel_scans SET status='completed_with_gaps' WHERE id=?1",
            [&f.context.scan_id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE sentinel_scan_attempts SET status='completed_with_gaps' WHERE scan_id=?1",
            [&f.context.scan_id],
        )
        .unwrap();
    let counts = |connection: &rusqlite::Connection| -> (i64, i64, i64, i64) {
        connection.query_row("SELECT (SELECT COUNT(*) FROM agent_runs),(SELECT COUNT(*) FROM agent_messages),\
            (SELECT COUNT(*) FROM agent_capability_leases),(SELECT COUNT(*) FROM sentinel_scan_attempts)",[],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
    };
    let before = counts(&connection);
    let input = followup_input(&f);
    let draft = create_agent_gap_followup_in(&connection, &input).unwrap();
    assert_eq!(draft.status, "draft");
    assert_eq!(counts(&connection), before);
    let targets: (i64, String) = connection
        .query_row(
            "SELECT COUNT(*),MIN(url) FROM sentinel_targets WHERE scan_id=?1",
            [&draft.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(targets, (1, f.context.target_url.clone()));
    let tx = connection.unchecked_transaction().unwrap();
    let outgoing = gap_followup_relations(&tx, &f.context.scan_id).unwrap();
    assert_eq!(outgoing["tasks"][0]["scanId"], draft.id);
    assert_eq!(outgoing["gapResolved"], false);
    let incoming = gap_followup_relations(&tx, &draft.id).unwrap();
    assert_eq!(incoming["source"]["sourceHash"], input.source_hash);
    drop(tx);
    assert!(
        gap_followup_control_preflight(&connection, &f.root, &draft.id)
            .unwrap_err()
            .contains("two_identities")
    );
    // Lost IPC response: replay returns the same real record, including its
    // current status. Replaying never grants permission or issues a start.
    connection
        .execute(
            "UPDATE sentinel_scans SET status='completed' WHERE id=?1",
            [&draft.id],
        )
        .unwrap();
    let replay = create_agent_gap_followup_in(&connection, &input).unwrap();
    assert_eq!(replay.id, draft.id);
    assert_eq!(replay.status, "completed");
    let mut conflict = input.clone();
    conflict.task_name.push('!');
    assert_eq!(
        create_agent_gap_followup_in(&connection, &conflict).unwrap_err(),
        "followup_request_id_conflict"
    );
    assert_eq!(counts(&connection), before);
}

#[test]
fn followup_source_and_creation_reject_mutated_or_unacknowledged_evidence() {
    for mutation in [
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='proposal_assessed'",
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='gap_proposed'",
        "UPDATE agent_messages SET payload_json=json_set(payload_json,'$.schemaVersion',2) WHERE kind='proposal_assessed'",
        "UPDATE agent_messages SET payload_json=json_set(payload_json,'$.targetRequestsGranted',3) WHERE kind='proposal_assessed'",
        "UPDATE agent_review_requests SET candidate_json='{}'",
        "UPDATE agent_review_requests SET status='superseded'",
        "UPDATE agent_assignments SET state='failed' WHERE role='deep_investigator'",
        "UPDATE projects SET status='archived'",
        "DELETE FROM agent_evidence_locations",
    ] {
        let f = followup_fixture();
        let input = followup_input(&f);
        let connection = db::open(&f.context.db_path).unwrap();
        connection.execute(mutation,[]).unwrap();
        assert!(followup_preview(&f).is_err(),"{mutation}");
        assert!(create_agent_gap_followup_in(&connection,&input).is_err(),"{mutation}");
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM sentinel_scans",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
}

#[test]
fn followup_creation_rolls_back_provenance_failure_and_retries_same_key() {
    for action in ["ABORT", "IGNORE"] {
        let f = followup_fixture();
        let input = followup_input(&f);
        let connection = db::open(&f.context.db_path).unwrap();
        let raise = if action == "ABORT" {
            "RAISE(ABORT,'fault')"
        } else {
            "RAISE(IGNORE)"
        };
        connection.execute_batch(&format!("CREATE TRIGGER reject_followup BEFORE INSERT ON agent_gap_followups BEGIN SELECT {raise}; END;")).unwrap();
        let before = web_mode_test_rows(&connection);
        assert!(create_agent_gap_followup_in(&connection, &input).is_err());
        let counts: (i64, i64, i64) = connection
            .query_row(
                "SELECT (SELECT COUNT(*) FROM sentinel_scans),\
            (SELECT COUNT(*) FROM sentinel_targets),(SELECT COUNT(*) FROM agent_gap_followups)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(counts, (1, 1, 0)); // The genuine source creator has one original target.
        web_mode_assert_rows(&connection, &before);
        connection
            .execute_batch("DROP TRIGGER reject_followup")
            .unwrap();
        assert_eq!(
            create_agent_gap_followup_in(&connection, &input)
                .unwrap()
                .status,
            "draft"
        );
    }
}

#[test]
fn followup_historical_binding_rejects_retargeting_missing_and_symlinked_directories() {
    let f = followup_fixture();
    let connection = db::open(&f.context.db_path).unwrap();
    assert!(connection
        .execute(
            "UPDATE agent_evidence_locations SET target_url='https://other.invalid'",
            []
        )
        .is_err());
    // Mutating the current URL list cannot move the persisted evidence binding.
    connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url) SELECT project_id,id,'fixture','https://other.invalid' FROM sentinel_scans",[]).unwrap();
    assert_eq!(
        followup_preview(&f).unwrap().target_url,
        f.context.target_url
    );
    let body = f.context.target_dir.join("agent-http/0001.body");
    fs::write(&body, b"fake body").unwrap();
    assert!(
        followup_preview(&f).is_err(),
        "same-length artifact replacement must invalidate a sealed source"
    );
    fs::write(&body, b"safe body").unwrap();
    assert!(followup_preview(&f).is_ok());
    let moved = f.context.target_dir.with_file_name("target-00002");
    assert_ne!(moved, f.context.target_dir);
    fs::rename(&f.context.target_dir, &moved).unwrap();
    assert!(followup_preview(&f).is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&moved, &f.context.target_dir).unwrap();
        assert_eq!(
            followup_preview(&f).unwrap_err(),
            "followup_evidence_directory_mismatch"
        );
        fs::remove_file(&f.context.target_dir).unwrap();
    }
    fs::rename(&moved, &f.context.target_dir).unwrap();
    fs::write(
        f.context.target_dir.join(".oviraptor-scan-id"),
        "other-scan",
    )
    .unwrap();
    assert_eq!(
        followup_preview(&f).unwrap_err(),
        "followup_evidence_marker_mismatch"
    );
    fs::write(
        f.context.target_dir.join(".oviraptor-scan-id"),
        &f.context.scan_id,
    )
    .unwrap();
    connection
        .execute(
            "UPDATE agent_runs SET target_url='https://other.invalid' WHERE id=?1",
            [&f.root_run_id],
        )
        .unwrap();
    assert!(followup_preview(&f).is_err());
}

#[test]
fn followup_concurrent_idempotency_creates_one_task() {
    let f = followup_fixture();
    let input = followup_input(&f);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let input = input.clone();
            let path = f.context.db_path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let connection = db::open(&path).unwrap();
                barrier.wait();
                create_agent_gap_followup_in(&connection, &input)
                    .unwrap()
                    .id
            })
        })
        .collect();
    let ids: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(ids[0], ids[1]);
    let connection = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM agent_gap_followups", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let mut stale = input.clone();
    stale.request_id = Uuid::new_v4().to_string();
    stale.source_hash = "stale".into();
    assert_eq!(
        create_agent_gap_followup_in(&connection, &stale).unwrap_err(),
        "followup_source_changed_preview_again"
    );
}

fn add_followup_identities(connection: &rusqlite::Connection, input: &mut GapFollowupDraftInput) {
    input.auth_session_ids = vec!["fresh-owner".into(), "fresh-tester".into()];
    let project: i64 = connection
        .query_row(
            "SELECT project_id FROM sentinel_scans WHERE id=?1",
            [&input.source_scan_id],
            |r| r.get(0),
        )
        .unwrap();
    let target: String = connection.query_row("SELECT target_url FROM agent_runs WHERE scan_id=?1 AND role='coordinator' ORDER BY rowid LIMIT 1", [&input.source_scan_id], |r|r.get(0)).unwrap();
    let host = reqwest::Url::parse(&target)
        .unwrap()
        .host_str()
        .unwrap()
        .to_string();
    for id in &input.auth_session_ids {
        connection.execute("INSERT INTO browser_auth_sessions(id,project_id,draft_scope_id,name,entry_url,status,session_json,expires_at) \
            VALUES(?1,?5,?2,?1,?6,'valid',?3,?4)",
            params![id,input.auth_session_scope_id,json!({"scopeHosts":[host],"cookies":[{"name":"session","value":id}]}).to_string(),
                (chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339(),project,target]).unwrap();
    }
}

#[test]
fn followup_fresh_identities_and_explicit_controls_are_required_before_start() {
    let f = followup_fixture();
    let mut connection = db::open(&f.context.db_path).unwrap();
    let mut input = followup_input(&f);
    add_followup_identities(&connection, &mut input);
    connection.execute("UPDATE browser_auth_sessions SET owner_scan_id=?1,draft_scope_id='' WHERE id='fresh-owner'",[&f.context.scan_id]).unwrap();
    assert!(
        create_agent_gap_followup_in(&connection, &input).is_err(),
        "old task identity cannot be inherited"
    );
    connection.execute("UPDATE browser_auth_sessions SET owner_scan_id='',draft_scope_id=?1 WHERE id='fresh-owner'",[&input.auth_session_scope_id]).unwrap();
    let draft = create_agent_gap_followup_in(&connection, &input).unwrap();
    assert_eq!(
        gap_followup_control_preflight(&connection, &f.root, &draft.id).unwrap_err(),
        "followup_requires_explicit_authorization_controls"
    );
    let setup =
        authorization_control_setup_in(&connection, &f.root, &draft.id, &f.context.target_url)
            .unwrap();
    let control = SaveAuthorizationControlInput {
        scan_id: draft.id.clone(),
        attempt_number: setup.attempt_number,
        target_url: f.context.target_url.clone(),
        contract_key: "idor|/api/orders".into(),
        owner_object_url: format!("{}/api/orders?id=owner", f.context.target_url),
        tester_control_url: format!("{}/api/orders?id=tester", f.context.target_url),
        object_query_key: "id".into(),
        owner_object_value: "owner".into(),
        tester_object_value: "tester".into(),
        response_object_pointer: "/order/id".into(),
        owner_identity: "fresh-owner".into(),
        tester_identity: "fresh-tester".into(),
    };
    save_authorization_control_in(&mut connection, &f.root, &control).unwrap();
    assert_eq!(
        gap_followup_control_preflight(&connection, &f.root, &draft.id).unwrap(),
        Some(setup.attempt_number)
    );
    connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url) SELECT project_id,id,'extra','https://extra.invalid' FROM sentinel_scans WHERE id=?1",[&draft.id]).unwrap();
    assert_eq!(
        gap_followup_control_preflight(&connection, &f.root, &draft.id).unwrap_err(),
        "followup_source_scope_changed"
    );
    connection
        .execute(
            "DELETE FROM sentinel_targets WHERE scan_id=?1 AND url='https://extra.invalid'",
            [&draft.id],
        )
        .unwrap();
    connection.execute("INSERT INTO projects(id,name) SELECT project_id+100,'other-followup-project' FROM sentinel_scans WHERE id=?1",[&draft.id]).unwrap();
    connection
        .execute(
            "UPDATE sentinel_targets SET project_id=project_id+100 WHERE scan_id=?1",
            [&draft.id],
        )
        .unwrap();
    assert_eq!(
        gap_followup_control_preflight(&connection, &f.root, &draft.id).unwrap_err(),
        "followup_source_scope_changed"
    );
    connection
        .execute(
            "UPDATE sentinel_targets SET project_id=project_id-100 WHERE scan_id=?1",
            [&draft.id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE sentinel_scans SET project_id=project_id+100 WHERE id=?1",
            [&draft.id],
        )
        .unwrap();
    assert_eq!(
        gap_followup_control_preflight(&connection, &f.root, &draft.id).unwrap_err(),
        "followup_source_scope_changed"
    );
    connection
        .execute(
            "UPDATE sentinel_scans SET project_id=project_id-100 WHERE id=?1",
            [&draft.id],
        )
        .unwrap();
    assert_eq!(
        gap_followup_control_preflight(&connection, &f.root, &draft.id).unwrap(),
        Some(setup.attempt_number)
    );
    let ids = crate::auth_session::validated_scan_identities(
        &connection,
        &draft.id,
        &f.context.target_url,
    )
    .unwrap()
    .1;
    assert_eq!(ids.len(), 2);
    assert_eq!(
        create_agent_gap_followup_in(&connection, &input)
            .unwrap()
            .id,
        draft.id,
        "receipt replay must not rebind consumed identities"
    );
    connection
        .execute(
            "UPDATE browser_auth_sessions SET status='expired' WHERE id='fresh-owner'",
            [],
        )
        .unwrap();
    assert!(gap_followup_control_preflight(&connection, &f.root, &draft.id).is_err());
    connection
        .execute(
            "UPDATE browser_auth_sessions SET status='valid' WHERE id='fresh-owner'",
            [],
        )
        .unwrap();
    fs::create_dir_all(
        neutral_scan_work_root(&f.root)
            .join(&draft.id)
            .join("attempt-00001"),
    )
    .unwrap();
    assert!(
        gap_followup_control_preflight(&connection, &f.root, &draft.id).is_err(),
        "a skipped attempt cannot inherit the old control group"
    );
}

#[test]
fn followup_provenance_failure_rolls_back_identity_ownership_and_all_draft_rows() {
    let f = followup_fixture();
    let connection = db::open(&f.context.db_path).unwrap();
    let mut input = followup_input(&f);
    add_followup_identities(&connection, &mut input);
    connection.execute_batch("CREATE TRIGGER reject_followup_identity BEFORE INSERT ON agent_gap_followups BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(create_agent_gap_followup_in(&connection, &input).is_err());
    let owners:i64=connection.query_row("SELECT COUNT(*) FROM browser_auth_sessions WHERE owner_scan_id='' AND draft_scope_id=?1",[&input.auth_session_scope_id],|r|r.get(0)).unwrap();
    assert_eq!(owners, 2);
    let drafts: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sentinel_scans WHERE status='draft'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(drafts, 0);
    connection
        .execute_batch("DROP TRIGGER reject_followup_identity")
        .unwrap();
    assert_eq!(
        create_agent_gap_followup_in(&connection, &input)
            .unwrap()
            .status,
        "draft"
    );
}

#[test]
fn followup_exact_target_does_not_strip_the_sources_resource_slash() {
    let f = followup_fixture();
    let connection = db::open(&f.context.db_path).unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    let target = "https://authorized.example.test/path/";
    let project: i64 = transaction
        .query_row(
            "SELECT project_id FROM sentinel_scans WHERE id=?1",
            [&f.context.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    let draft = create_sentinel_url_draft_rows(
        &transaction,
        project,
        "exact".into(),
        vec![target.into()],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(target),
    )
    .unwrap();
    let saved: String = transaction
        .query_row(
            "SELECT url FROM sentinel_targets WHERE scan_id=?1",
            [draft.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved, target);
}
