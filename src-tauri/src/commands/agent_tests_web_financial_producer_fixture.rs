// Real creator/paid Root+Mapper+budget frame before each Web financial fault.
type WebFinancialReply =
    std::sync::Arc<dyn Fn(String) -> (u16, &'static str, String) + Send + Sync>;
struct WebFinancialProducer {
    f: BudgetTriggerFixture,
    web_reply: std::sync::Arc<std::sync::Mutex<WebFinancialReply>>,
    web_seen: Seen,
    _real: RealSpecialistTransport,
    _endpoint_seen: Seen,
}
impl WebFinancialProducer {
    fn context(&self) -> &AgentRunContext {
        &self.f.h.context
    }
    fn set_web_reply(
        &self,
        reply: impl Fn(String) -> (u16, &'static str, String) + Send + Sync + 'static,
    ) {
        *self.web_reply.lock().unwrap() = std::sync::Arc::new(reply);
    }
    fn assert_calls(&self, web: usize) {
        assert_eq!(self.web_seen.lock().unwrap().len(), web);
        assert_eq!(self.f.h.model_seen.lock().unwrap().len(), 4 + web);
        let db = db::open(&self.f.h.db_path).unwrap();
        let s = self.f.session.as_ref().unwrap();
        assert_eq!(root_tick_count(&db, &s.lease.root_run_id, "publication"), 3);
        let root = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &s.lease.root_run_id,
            Some(""),
            "model_requests",
        )
        .unwrap();
        assert_eq!((root.consumed, root.indeterminate), (3, 0));
        assert!(self.f.h.site_seen.lock().unwrap().is_empty());
    }
    fn grant(&self) -> (i64, i64) {
        let db = db::open(&self.f.h.db_path).unwrap();
        let session = self.f.session.as_ref().unwrap();
        crate::agent_runtime::multi_agent::budget::model::original_web_grant(
            &db,
            &session.lease,
            &session.executor.assignment_id,
        )
        .unwrap()
    }
}
fn web_financial_producer_fixture() -> WebFinancialProducer {
    web_financial_producer_fixture_with_site(mock_site)
}
fn web_financial_producer_fixture_with_site(
    site: impl Fn(String) -> (u16, &'static str, String) + Send + Sync + 'static,
) -> WebFinancialProducer {
    let real = RealSpecialistTransport::enter();
    let mut h = fresh_multi_production_harness_with_site("web-financial-producer", site);
    let web_reply: std::sync::Arc<std::sync::Mutex<WebFinancialReply>> =
        std::sync::Arc::new(std::sync::Mutex::new(std::sync::Arc::new(|_| {
            (200, "application/json", model_round(&[], 10))
        })));
    let active = web_reply.clone();
    let web_seen: Seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = web_seen.clone();
    let model_seen: Seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let arrivals = model_seen.clone();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        arrivals.lock().unwrap().push(wire.clone());
        if let Some(response) = fresh_multi_root_wire_response(&wire) {
            return (200, "application/json", response);
        }
        if wire.contains("SPA/API Mapper") {
            return (
                200,
                "application/json",
                proposal_model_response(
                    r#"{"summary":"actual original readonly Mapper","priorityContracts":[],"risks":[]}"#,
                ),
            );
        }
        observed.lock().unwrap().push(wire.clone());
        let reply = active.lock().unwrap().clone();
        reply(wire)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = model_seen;
    h.context.execution_plan.hard_total_tokens = 60_000;
    h.context.execution_plan.soft_uncached_tokens = 30_000;
    h.context.execution_plan.hard_model_requests = 6;
    h.context.execution_plan.soft_model_requests = 6;
    h.context.execution_plan.max_turns = 8;
    // Preserve the original six-request financial/stop boundary under v5.
    freeze_fresh_multi_production_harness_original_v5(&mut h);
    let mut f = WebFinancialProducer {
        f: BudgetTriggerFixture {
            h,
            session: None,
            boundary: std::sync::Arc::new(std::sync::Mutex::new(None)),
            stop,
        },
        web_reply,
        web_seen,
        _real: real,
        _endpoint_seen: seen,
    };
    f.f.session = Some(multi_agent_prepare(&mut f.f.h.context).unwrap());
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 3);
    native_coordinator_budget_before_executor(f.context()).unwrap();
    assert_eq!(
        f.f.h.model_seen.lock().unwrap().len(),
        4,
        "Root x3 plus Mapper, all original paid SDKs retained"
    );
    assert!(f.web_seen.lock().unwrap().is_empty());
    assert_eq!(f.grant().1, 1);
    f
}
