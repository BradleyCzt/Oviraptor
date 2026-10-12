// Existing production APIs, not deterministic multi_agent_child_round.
// Stage R1 admission/SDK/delivery contract only. It does not prove browser,
// candidate/reviewer, Root tick or fresh UI producer acceptance.

struct ClientSideReadonlyTestGuard {
    root: PathBuf,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for ClientSideReadonlyTestGuard {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn client_side_readonly_actual_specialist_sdk_worker_cost_and_delivery() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (directory, path, root, lease) = multi_agent_new_task_root("client-readonly-real", 60_000, 20);
    let db = db::open(&path).unwrap();
    // Actual fresh Web creator/HMAC freezes the original owner before history.
    let original_plan: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&root],
            |r| r.get(0),
        )
        .unwrap();
    let mut context = test_context(&path, &lease.target_key, vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.run = Some(AgentRunLedger {
        db_path: path.clone(),
        run_id: root.clone(),
    });
    context.target_dir = fs::canonicalize(&directory).unwrap().join("private-client-input");
    fs::create_dir_all(&context.target_dir).unwrap();
    let content=b"<html><head><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self'\"></head><body>readonly evidence</body></html>";
    let artifact = context.target_dir.join("client-side-observed.html");
    fs::write(&artifact, content).unwrap();
    let content_hash = crate::agent_runtime::store::artifact_id(content);
    let slice = json!({
        "schemaVersion":1,"phase":"client_side_readonly","rootRunId":root,
        "scanId":lease.scan_id,"attemptNumber":lease.attempt_number,"target":lease.target_key,
        "coordinatorEpoch":lease.lease_epoch,"coordinatorFence":lease.fencing_token,
        "evidenceRevision":1,"targetRequestsGranted":0,"browserActionsGranted":0,
        "observations":[{"id":"meta-csp-1","kind":"meta_csp","classification":"source-derived",
            "artifact":{"path":"client-side-observed.html","contentHash":content_hash},
            "value":"default-src 'self'"}],"toolsGranted":[]
    });
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(
                r#"{"summary":"本地meta CSP已记录；未验证浏览器行为","observationRefs":["meta-csp-1"],"gaps":["missing_browser_validation"],"candidates":[]}"#,
            ),
        )
    }));
    let _fixture_cleanup = ClientSideReadonlyTestGuard {
        root: directory.clone(),
        stop: stop.clone(),
    };
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let parent=start_original_fixture_supervisor(&path,&lease).unwrap();
    context.supervision=Some(parent.ticket());
    let child = scheduler::prepare_readonly_child(
        &db,
        &lease,
        AgentRole::ClientSide,
        "client_side_frozen_observations_ready",
        &slice,
        8_000,
    )
    .expect("current production rejects ClientSide before dispatch; genuine role required");
    assert_eq!(child.role, AgentRole::ClientSide);
    let role_lane: (String, String) = db
        .query_row(
            "SELECT role,lane FROM agent_runs WHERE id=?1",
            [&child.run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        role_lane,
        (
            "client_side".into(),
            AgentLane::ReadOnlyAnalysis.as_str().into()
        )
    );
    let (text,usage)=multi_agent_child_round_transport(&context,&lease,&child,
        "只分析冻结本地客户端配置观察；严格JSON、无工具；没有浏览器行为或影响证据就保留缺口，不能confirmed。",slice.clone()).unwrap();
    let request = {
        let calls = seen.lock().unwrap();
        assert_eq!(calls.len(), 1);
        calls[0].clone()
    };
    let body = request.split_once("\r\n\r\n").unwrap().1;
    let wire: JsonValue = serde_json::from_str(body).unwrap();
    assert_eq!(
        wire.get("max_tokens")
            .or_else(|| wire.get("max_completion_tokens"))
            .and_then(JsonValue::as_u64),
        Some(256),
        "actual new-role SDK request must be bounded, not just its stored estimate"
    );
    assert!(wire.get("tools").is_none_or(|v| v == &json!([])));
    assert!(request.contains(&content_hash));
    assert_eq!(
        crate::agent_runtime::store::artifact_id(&fs::read(&artifact).unwrap()),
        content_hash
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_specialist_calls WHERE assignment_id=?1 AND role='client_side' AND state='received'",[&child.assignment_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    let worker: String = db
        .query_row(
            "SELECT id FROM agent_assignment_attempts WHERE assignment_id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE assignment_id=?1 AND lease_attempt_id=?2 AND dimension='model_requests' AND kind='consume' AND amount=1)",params![child.assignment_id,worker],|r|r.get::<_,bool>(0)).unwrap());
    let parsed: JsonValue = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["candidates"], json!([]));
    let replay_before = super::tests::application_table_snapshot(&db);
    let replay = multi_agent_child_round_transport(&context,&lease,&child,
        "只分析冻结本地客户端配置观察；严格JSON、无工具；没有浏览器行为或影响证据就保留缺口，不能confirmed。",slice.clone()).unwrap();
    assert_eq!(replay,(text.clone(),usage));
    assert!(super::tests::application_table_snapshot(&db)==replay_before);
    assert_eq!(seen.lock().unwrap().len(),1);
    let payload = json!({"summary":text,"clientSideTask":slice});
    complete_readonly_assessment(&db, &lease, &child, &usage, &payload).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND acknowledged_at<>''",
            [&child.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "completed"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_http_request_claims", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM sentinel_findings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0,
        "no Reviewer, no finding publication"
    );
    assert_eq!(
        db.query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&root],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        original_plan
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        10
    );
    let before = receipt_database_snapshot(&db);
    let replay = scheduler::prepare_readonly_child(
        &db,
        &lease,
        AgentRole::ClientSide,
        "client_side_frozen_observations_ready",
        &slice,
        8_000,
    )
    .unwrap();
    assert_eq!(replay, child);
    assert_eq!(
        receipt_database_snapshot(&db),
        before,
        "read-only completed preparation cannot reissue a worker/budget/lane"
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    let paid=specialist_readonly_closure_frozen(&db,&child);
    finish_coordinator_run(&db,&lease,&AgentTargetOutcome::incomplete("ClientSide configuration delivered; browser behavior remains unverified")).unwrap();
    assert!(specialist_readonly_closure_frozen(&db,&child)==paid);
    assert_eq!(db.query_row("SELECT state FROM agent_assignments WHERE id=?1",[&child.assignment_id],|r|r.get::<_,String>(0)).unwrap(),"completed");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(parent);
    drop(db);
    fs::remove_dir_all(directory).unwrap();
}
