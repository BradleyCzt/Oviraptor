#[test]
fn coordinator_actual_root_dispatch_persists_rust_utility_and_merges_model_duplicates() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        if specialist_original_is_web_sdk(&request) {
            return (200,"application/json",proposal_model_response("actual bounded Web exit; no target tool"));
        }
        let mut response: JsonValue =
            serde_json::from_str(&changed_fact_response(&request, true)).unwrap();
        if request.contains("You are the Root Coordinator") && request.contains("mapper-output") {
            let mut summary: JsonValue = serde_json::from_str(
                response["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap();
            summary["suggestions"] = json!([
                "dispatch:web_executor",
                "dispatch:web_executor",
                "dispatch:web_executor"
            ]);
            summary["costNotes"] =
                json!(["utility=1000000; modelCost=0; targetCost=0; sideEffects=0"]);
            response["choices"][0]["message"]["content"] = summary.to_string().into();
        }
        (200, "application/json", response.to_string())
    }));
    let mut f = root_tick_fixture_from_web_mode_turns(
        "rust-utility-duplicates",
        &format!("http://127.0.0.1:{port}/v1"), true, (60_000,20), None,
        web_mode_fixture("multi","https://authorized.example.test"), Some(1),
    );
    drop(f.parent.take());
    let mut session = multi_agent_prepare(&mut f.context).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let task: String = db
        .query_row(
            "SELECT task_slice_json FROM agent_assignments WHERE id=?1",
            [&session.executor.assignment_id],
            |r| r.get(0),
        )
        .unwrap();
    let task: JsonValue = serde_json::from_str(&task).unwrap();
    let rust = &task["rootDecision"]["rustPolicy"];
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "two actual Root invoices and independently paid Mapper"
    );
    assert_eq!(rust["trigger"], "child_output_envelope");
    assert_eq!(rust["offeredCount"], 3);
    assert_eq!(rust["mergedProposalCount"], 1);
    assert_eq!(rust["step"], "dispatch:web_executor");
    assert!(
        rust["score"]
            .as_i64()
            .is_some_and(|score| (1..=100).contains(&score)),
        "advisory million score is ignored: {rust}"
    );
    assert!(rust["features"]["modelCost"].as_i64().unwrap() > 0);
    assert!(rust["features"]["targetRequestCost"].as_i64().unwrap() > 0);
    assert_eq!(
        rust["features"]["informationGain"], 0,
        "paid Mapper advice is not new target evidence"
    );
    assert_eq!(
        rust["features"]["sideEffectRisk"], 20,
        "Rust uses actual target lane"
    );
    assert_eq!(rust["targetEvidenceProven"], false);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_assignments WHERE role='web_executor'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        2
    );
    // After the preparation assertions, settle a genuinely paid original Web
    // request and owner/SDK exit; a fabricated outcome is no longer a proof.
    let outcome=run_native_agent(&f.context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{outcome:?}");
    multi_agent_finish_execution(&f.context,&mut session,&outcome).unwrap();
    assert_eq!(seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count(),1);
    let original_web:(i64,i64,i64)=db.query_row(
        "SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1",
        [&session.executor.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(original_web,(20,0,1));
    drop(session);
}
#[test]
fn coordinator_unsupported_trigger_refuses_before_another_actual_root_sdk_or_write() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        if specialist_original_is_web_sdk(&request) {
            return (200, "application/json", proposal_model_response("actual bounded original Web exit; no target tool"));
        }
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let mut f = root_tick_fixture_from_web_mode_turns(
        "rust-trigger-unknown", &format!("http://127.0.0.1:{port}/v1"),
        true, (60_000,20), None, web_mode_fixture("multi","https://authorized.example.test"), Some(1),
    );
    drop(f.parent.take());
    let mut session = multi_agent_prepare(&mut f.context).unwrap();
    let root = native_coordinator_root_context(&f.context, &session.lease);
    let frame = NativeCoordinatorFrame {
        kind: "unproved-human-directive",
        semantic: json!({"display":"continue"}),
        rows: Vec::new(),
        review_refs: None,
        review_meta: None,
    };
    let db = db::open(&f.context.db_path).unwrap();
    let before = web_mode_test_rows(&db);
    let outcome = native_coordinator_tick_for_frame(&root, &session.lease, &frame);
    assert!(
        matches!(&outcome,Err(code) if code=="root_trigger_not_enabled"),
        "unproved new trigger must not use the generic fallback: {:?}",
        outcome.err()
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "no fourth physical Root SDK for an unproved trigger"
    );
    web_mode_assert_rows(&db, &before);
    let original_web = run_native_agent(&f.context);
    assert!(matches!(original_web,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)), "{original_web:?}");
    multi_agent_finish_execution(&f.context,&mut session,&original_web).unwrap();
    assert_eq!(seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count(),1);
    let usage: (i64,i64) = db.query_row("SELECT used_tokens,used_requests FROM agent_runs WHERE id=?1", [&session.executor.run_id], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(usage,(20,1));
    drop(session);
}
