// Original v5 retained-floor contract, genuine creator and paid independent
// facts. Exact seven/eight-request limits remain; the held worker has no target I/O.
struct RootSupervisionHeadroomFixture {
    f: RootTickFixture,
    mapper: crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    frame: NativeCoordinatorFrame,
    decision: NativeCoordinatorTickReceipt,
    seen: Seen,
}
fn root_supervision_headroom_fixture(
    limits: (i64, i64),
    reply: u8,
) -> RootSupervisionHeadroomFixture {
    let rounds = std::sync::atomic::AtomicUsize::new(0);
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |request| {
        if request.contains("You are the Root Coordinator")
            && request.contains("identity-session-output")
        {
            if reply == 1 {
                return (
                    503,
                    "application/json",
                    json!({"error":{"message":"temporary provider failure"}}).to_string(),
                );
            }
            if reply == 2 && rounds.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                return (200,"application/json",json!({"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"held-budget-observation","type":"function","function":{"name":"capability_budget.read","arguments":"{}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string());
            }
        }
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let f = root_tick_fixture_original_v5_limits(
        "root-supervision-headroom",
        &format!("http://127.0.0.1:{port}/v1"),
        limits,
    );
    let db = db::open(&f.context.db_path).unwrap();
    let document = json!({"scopeHosts":["authorized.example.test"],"cookies":[{"name":"session","value":"temporary-identity-fixture"}]});
    db.execute("INSERT INTO browser_auth_sessions(id,project_id,owner_scan_id,name,entry_url,status,session_json,expires_at) SELECT 'headroom-identity',project_id,id,'Temporary identity',?2,'valid',?3,?4 FROM sentinel_scans WHERE id=?1",params![f.context.scan_id,f.context.target_url,document.to_string(),(chrono::Utc::now()+chrono::Duration::hours(8)).to_rfc3339()]).unwrap();
    db.execute("UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,'$.authSessionIds',json(?2)) WHERE scan_id=?1",params![f.context.scan_id,json!(["headroom-identity"]).to_string()]).unwrap();
    let bootstrap = native_coordinator_tick(&f.context, &f.actor).unwrap();
    let mapper = native_coordinator_prepare_mapper(
        &f.context,
        &f.actor,
        &bootstrap,
        &bootstrap_mapper_task(&f),
    )
    .unwrap();
    let (text, usage) = multi_agent_child_round(
        &f.context,
        &f.actor,
        &mapper,
        "Independent SPA/API Mapper",
        json!({"frozenEvidence":f.context.evidence}),
    )
    .unwrap();
    deliver_readonly_assessment(
        &db,
        &f.actor,
        &mapper,
        &usage,
        &json!({"summary":text}),
        None,
    )
    .unwrap();
    let (frame, decision) =
        native_coordinator_mapper_decision(&f.context, &f.actor, &mapper).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 3);
    RootSupervisionHeadroomFixture {
        f,
        mapper,
        frame,
        decision,
        seen,
    }
}
impl RootSupervisionHeadroomFixture {
    fn admit(
        &self,
    ) -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
        executor_allocation_admit(&self.f, &self.mapper, &self.frame, &self.decision)
    }
    fn close_identity(&self) -> crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler};
        let f = &self.f;
        let db = db::open(&f.context.db_path).unwrap();
        let (mode, handles) = crate::auth_session::validated_scan_identities(
            &db,
            &f.actor.scan_id,
            &f.actor.target_key,
        )
        .unwrap();
        let child=scheduler::prepare_supervised_readonly_child(&db,&f.actor,AgentRole::IdentitySession,"validated_identity_metadata_ready",&json!({"identityMode":format!("{mode:?}"),"identityCount":handles.len(),"mapperAssignmentId":self.mapper.assignment_id,"objective":"readonly metadata, no target access"}),4_000).unwrap();
        let (text, usage) = multi_agent_child_round(
            &f.context,
            &f.actor,
            &child,
            "Independent IdentitySession metadata only",
            json!({"frozenEvidence":f.context.evidence}),
        )
        .unwrap();
        deliver_readonly_assessment(
            &db,
            &f.actor,
            &child,
            &usage,
            &json!({"summary":text,"identityMode":format!("{mode:?}")}),
            None,
        )
        .unwrap();
        assert_eq!(self.seen.lock().unwrap().len(), 4);
        child
    }
}
