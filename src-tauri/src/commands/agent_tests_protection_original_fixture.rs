// Actual signed creator, independent Root/Mapper SDKs and one protected GET.
type ProtectionFixtureBoundary = std::sync::Arc<dyn Fn(&Path) + Send + Sync>;
struct ProtectionOriginalFixture {
    h: AgentHarness,
    session: Option<MultiAgentSession>,
    actor: crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    outcome: AgentTargetOutcome,
    receipt: String,
    model_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _transport: RealSpecialistTransport,
    original_target_owner: Option<crate::agent_runtime::execution_owner::NativeInvocationOwner>,
}
impl Drop for ProtectionOriginalFixture {
    fn drop(&mut self) {
        drop(self.session.take());
        self.model_stop
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = fs::remove_dir_all(&self.h.root);
    }
}
impl ProtectionOriginalFixture {
    fn new(tag: &str) -> Self { Self::with_protection_reply(tag, None) }
    fn with_original_target_owner(tag: &str) -> Self {
        Self::with_protection_boundary_and_target_owner(tag, None, None, true)
    }
    fn with_protection_reply(tag: &str, protection_reply: Option<String>) -> Self {
        Self::with_protection_boundary(tag, protection_reply, None)
    }
    fn with_protection_boundary(tag: &str, protection_reply: Option<String>, boundary: Option<ProtectionFixtureBoundary>) -> Self {
        Self::with_protection_boundary_and_target_owner(tag, protection_reply, boundary, false)
    }
    fn with_protection_boundary_and_target_owner(tag: &str, protection_reply: Option<String>, boundary: Option<ProtectionFixtureBoundary>, own_target: bool) -> Self {
        let transport = RealSpecialistTransport::enter();
        let mut h = fresh_multi_production_harness(tag);
        let target = h.context.target_url.clone();
        let protection_directory = h.context.target_dir.clone();
        let (port, seen, model_stop) = spawn_endpoint(std::sync::Arc::new(move |wire| {
            let response = if let Some(root) = fresh_multi_root_wire_response(&wire) {
                let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
                let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
                if input["basis"]["phase"] == "protection-changed" {
                    if let Some(boundary) = &boundary { boundary(&protection_directory); }
                    protection_reply.clone().unwrap_or(root)
                } else { root }
            } else if wire.contains("SPA/API Mapper") {
                proposal_model_response(
                    r#"{"summary":"frozen frontend map only","priorityContracts":[],"risks":[]}"#,
                )
            } else {
                model_round(
                    &[(
                        "replay_http",
                        json!({"identity":"anonymous","method":"GET",
                        "url":format!("{target}/api/gated")}),
                    )],
                    10,
                )
            };
            (200, "application/json", response)
        }));
        h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        h.model_seen = seen;
        h.context.execution_plan.hard_total_tokens = 60000;
        h.context.execution_plan.soft_uncached_tokens = 30000;
        h.context.execution_plan.hard_model_requests = 20;
        h.context.execution_plan.soft_model_requests = 10;
        h.context.execution_plan.max_turns = 2;
        // The optional outer pipeline owns this inode before ANY Root SDK.
        // Historical pause later only probes it; no paid exit is backfilled.
        let original_target_owner = own_target.then(|| claim_native_invocation(
            &h.db_path, &h.context.scan_id, h.context.attempt_number,
            "target", &h.context.target_url,
        ).unwrap());
        freeze_fresh_multi_production_harness(&mut h);
        let session = multi_agent_prepare(&mut h.context).unwrap();
        let outcome = run_native_agent(&h.context);
        assert!(
            matches!(outcome,AgentTargetOutcome::Limited(ref stop) if stop.code==AGENT_STOP_WAF),
            "{}",
            outcome.detail()
        );
        assert_eq!(h.site_seen.lock().unwrap().len(), 1);
        let db = db::open(&h.db_path).unwrap();
        let receipts: Vec<String> = db
            .prepare("SELECT receipt_id FROM agent_protection_response_receipts")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(receipts.len(), 1);
        let actor = session.lease.clone();
        Self {
            h,
            session: Some(session),
            actor,
            outcome,
            receipt: receipts[0].clone(),
            model_stop,
            _transport: transport,
            original_target_owner,
        }
    }

    fn fact(&self, db: &rusqlite::Connection) -> JsonValue {
        let raw: String = db
            .query_row(
                "SELECT fact_json FROM agent_protection_response_receipts WHERE receipt_id=?1",
                [&self.receipt],
                |r| r.get(0),
            )
            .unwrap();
        serde_json::from_str(&raw).unwrap()
    }

    fn assert_original_read_only(&self, db: &rusqlite::Connection, want_ok: bool) {
        let check = |snapshot: &rusqlite::Connection| {
            let before = super::tests::application_table_snapshot(snapshot);
            let changes = snapshot.total_changes();
            let sdk = self.h.model_seen.lock().unwrap().len();
            let get = self.h.site_seen.lock().unwrap().len();
            match native_original_protection_response_on(snapshot, &self.actor, &self.receipt) {
                Ok(_) => assert!(want_ok, "corrupted original response must be rejected"),
                Err(error) => assert!(!want_ok, "original response rejected: {error}"),
            }
            assert_eq!(super::tests::application_table_snapshot(snapshot), before);
            assert_eq!(snapshot.total_changes(), changes);
            assert_eq!(self.h.model_seen.lock().unwrap().len(), sdk);
            assert_eq!(self.h.site_seen.lock().unwrap().len(), get);
        };
        if db.is_autocommit() {
            let tx = db.unchecked_transaction().unwrap();
            check(&tx);
            tx.commit().unwrap();
        } else {
            check(db);
        }
    }
}

#[test]
fn protection_original_actual_first_receipt_is_pure_before_and_after_web_closure() {
    let mut f = ProtectionOriginalFixture::new("protection-original-closure");
    let db = db::open(&f.h.db_path).unwrap();
    f.assert_original_read_only(&db, true);
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    f.assert_original_read_only(&db, true);
    let child = &f.session.as_ref().unwrap().executor;
    let closure:(String,String,i64,i64,bool) = db.query_row(
        "SELECT a.state,w.state,a.reserved_tokens,a.reserved_requests,a.budget_settled_at<>'' FROM agent_assignments a JOIN agent_assignment_attempts w ON w.assignment_id=a.id AND w.child_run_id=?2 WHERE a.id=?1",
        params![child.assignment_id,child.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(
        closure,
        ("completed".into(), "completed".into(), 0, 0, true)
    );
    let ack:(i64,String) = db.query_row(
        "SELECT count(*),json_extract(payload_json,'$.terminalCode') FROM agent_messages WHERE root_run_id=?1 AND assignment_id=?2 AND from_run_id=?3 AND kind='execution_result' AND acknowledged_at<>''",
        params![f.actor.root_run_id,child.assignment_id,child.run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(ack, (1, AGENT_STOP_WAF.into()));
    let live_caps:i64 = db.query_row("SELECT count(*) FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 AND revoked_at=''",
        params![child.assignment_id,child.run_id],|r|r.get(0)).unwrap();
    assert_eq!(live_caps, 0);
    let costs = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "target_requests",
    )
    .unwrap();
    assert_eq!(
        (costs.consumed, costs.reserved, costs.indeterminate),
        (1, 0, 0)
    );
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    drop(f.session.take());
    f.assert_original_read_only(&db, true);
    f.assert_original_read_only(&db, true);
}
