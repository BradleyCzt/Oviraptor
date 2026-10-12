// Actual temporary SDK endpoint; original model binding is frozen before creation.
struct HumanRootProducer {
    f: BudgetTriggerFixture,
    human: Seen,
    web: Seen,
    _real: RealSpecialistTransport,
}
fn human_root_wire_input(wire: &str) -> Option<JsonValue> {
    let request: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n")?.1).ok()?;
    if !request["messages"][0]["content"]
        .as_str()?
        .contains("You are the Root Coordinator")
    {
        return None;
    }
    let input: JsonValue =
        serde_json::from_str(request["messages"][1]["content"].as_str()?).ok()?;
    (input["basis"]["phase"] == "human-directive").then_some(input)
}
fn human_root_producer(reply: u8) -> HumanRootProducer {
    human_root_producer_with_limits(reply, 20, 1)
}
fn human_root_producer_with_limits(reply: u8, requests: u8, turns: u8) -> HumanRootProducer {
    let real = RealSpecialistTransport::enter();
    let mut h = fresh_multi_production_harness("root-human-directive");
    let path = h.db_path.clone();
    let boundary: ClientRootBoundary = std::sync::Arc::new(std::sync::Mutex::new(None));
    let fault = boundary.clone();
    let human: Seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let humans = human.clone();
    let web: Seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let webs = web.clone();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        if human_root_wire_input(&wire).is_some() {
            humans.lock().unwrap().push(wire.clone());
            if let Some(apply) = fault.lock().unwrap().take() {
                let db = rusqlite::Connection::open_with_flags(
                    &path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
                )
                .unwrap();
                apply(&db);
            }
            if reply == 4 {
                std::thread::sleep(Duration::from_secs(2));
            }
            if reply == 3 {
                return (
                    503,
                    "application/json",
                    json!({"error":{"message":"temporary Human Root failure"}}).to_string(),
                );
            }
            if reply == 6 {
                return (200, "application/json", proposal_model_response("PRIVATE_INVALID_HUMAN_RESPONSE"));
            }
            let suggestions = match reply {
                1 => vec![],
                2 => vec!["dispatch:web_executor"],
                _ => vec!["assess:human_directive"],
            };
            if reply == 5 || reply == 7 {
                let mut response: JsonValue = serde_json::from_str(&proposal_model_response(&json!({"schemaVersion":1,"observed":["original human confirmation"],"missing":[],"suggestions":["assess:human_directive"],"costNotes":[],"risks":[]}).to_string())).unwrap();
                response["usage"] =
                    if reply == 7 { json!({"prompt_tokens":50_000,"completion_tokens":10,"total_tokens":50_010,"prompt_tokens_details":{"cached_tokens":10_000}}) } else { json!({"prompt_tokens":100_000,"completion_tokens":0,"total_tokens":100_000}) };
                return (200, "application/json", response.to_string());
            }
            return (200,"application/json",proposal_model_response(&json!({"schemaVersion":1,"observed":["original confirmed human request"],"missing":[],"suggestions":suggestions,"costNotes":[],"risks":[]}).to_string()));
        }
        if let Some(response) = fresh_multi_root_wire_response(&wire) {
            return (200, "application/json", response);
        }
        if wire.contains("SPA/API Mapper") {
            return (
                200,
                "application/json",
                proposal_model_response(
                    r#"{"summary":"actual original Mapper","priorityContracts":[],"risks":[]}"#,
                ),
            );
        }
        webs.lock().unwrap().push(wire);
        (200, "application/json", model_round(&[], 10))
    }));
    h.model_seen = seen;
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.context.execution_plan.hard_total_tokens = 60_000;
    h.context.execution_plan.soft_uncached_tokens = 30_000;
    h.context.execution_plan.hard_model_requests = requests.into();
    h.context.execution_plan.soft_model_requests = requests.into();
    h.context.execution_plan.max_turns = turns.into();
    if requests == 6 {
        // Historical six-request deferral cases keep their original v5 birth
        // and exact limits; normal fresh Human Root cases still exercise v6.
        freeze_fresh_multi_production_harness_original_v5(&mut h);
    } else {
        freeze_fresh_multi_production_harness(&mut h);
    }
    let mut f = HumanRootProducer {
        f: BudgetTriggerFixture {
            h,
            session: None,
            boundary,
            stop,
        },
        human,
        web,
        _real: real,
    };
    f.f.session = Some(multi_agent_prepare(&mut f.f.h.context).unwrap());
    native_coordinator_budget_before_executor(&f.f.h.context).unwrap();
    assert_eq!(f.f.h.model_seen.lock().unwrap().len(), 4);
    f
}
