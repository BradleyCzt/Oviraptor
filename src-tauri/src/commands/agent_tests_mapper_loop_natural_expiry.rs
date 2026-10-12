// Real original 600-second worker deadline; no clock/state/fee mutation.
#[test]
fn mapper_loop_natural_expiry_actual_paid_delivery_and_root_feedback_free_replay() {
    use crate::agent_runtime::multi_agent::{attempts, specialist::mapper_loop};
    use std::time::{Duration, Instant};
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        let wire: JsonValue =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let reply = if request.contains("You are the Root Coordinator") {
            let input: JsonValue =
                serde_json::from_str(wire["messages"][1]["content"].as_str().unwrap()).unwrap();
            let step = match input["basis"]["phase"].as_str().unwrap() {
                "bootstrap" => "dispatch:spa_api_mapper",
                "lease-expired" => "assess:lease_expired",
                other => panic!("unexpected natural-expiry Root phase {other}"),
            };
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original signed expired Mapper output"],"missing":[],"suggestions":[step],"costNotes":[],"risks":[]}).to_string())
        } else {
            changed_fact_response(&request, true)
        };
        (200, "application/json", reply)
    }));
    let f = root_tick_fixture_from_web_mode_evidence_protocol(
        "mapper-loop-natural-expiry",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (60_000, 20),
        Some(1200),
        web_mode_fixture("multi", "https://authorized.example.test"),
        RootFixtureEvidence {
            mapper_v15: true,
            ..Default::default()
        },
    );
    let decision = native_coordinator_tick(&f.context, &f.actor).unwrap();
    let bounds = mapper_loop_allocation_bounds(&f);
    let db = db::open(&f.context.db_path).unwrap();
    let child = native_coordinator_prepare_mapper_loop_allocated(
        &db,
        &f.context,
        &f.actor,
        &decision,
        &bootstrap_mapper_task(&f),
        &mapper_loop_original_basis(&decision),
        &bounds,
    )
    .unwrap();
    let output = super::mapper_executor_adapter::native_run_mapper_controlled_executor(
        &f.context, &f.actor, &child, &bounds,
    )
    .unwrap();
    assert_eq!(output.usage().model_requests, 1);
    assert_eq!(seen.lock().unwrap().len(), 2);
    let worker = attempts::current(&db, &f.actor, &child.assignment_id).unwrap();
    let original_deadline = worker.expires_at;
    let stop = Instant::now() + Duration::from_secs(630);
    loop {
        let current = attempts::current(&db, &f.actor, &child.assignment_id).unwrap();
        assert_eq!(
            current.expires_at, original_deadline,
            "only the parent C may renew"
        );
        if current.state == "expired" {
            break;
        }
        assert!(
            Instant::now() < stop,
            "original signed expiry sweep did not arrive"
        );
        f.parent.as_ref().unwrap().check().unwrap();
        assert_eq!(
            seen.lock().unwrap().len(),
            2,
            "waiting cannot issue SDK work"
        );
        std::thread::sleep(Duration::from_secs(1));
    }
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        mapper_loop::expired_seal::read_original_on(&tx, &f.actor, &child)
            .unwrap()
            .is_none()
    );
    let state: String = tx
        .query_row(
            "SELECT state FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "paused");
    drop(tx);
    let payload = json!({"summary":output.text()});
    assert_eq!(
        complete_expired_mapper_assessment(&db, &f.actor, &child, output.usage(), &payload)
            .unwrap(),
        payload
    );
    let tx = db.unchecked_transaction().unwrap();
    let receipt = mapper_loop::expired_seal::read_original_on(&tx, &f.actor, &child)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.worker_attempt_id(), worker.id);
    assert_eq!(
        mapper_loop::expired_saved::read_closed_original_on(&tx, &f.actor, &child)
            .unwrap()
            .output()
            .usage(),
        output.usage()
    );
    assert_eq!(
        attempts::current(&tx, &f.actor, &child.assignment_id)
            .unwrap()
            .state,
        "expired"
    );
    drop(tx);
    let paid = native_coordinator_lease_expiry_feedback(&f.context, &f.actor, &child).unwrap();
    assert_eq!(
        paid.summary.as_json()["suggestions"],
        json!(["assess:lease_expired"])
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "bootstrap, original Mapper, expired-output Root"
    );
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        complete_expired_mapper_assessment(&db, &f.actor, &child, output.usage(), &payload)
            .unwrap(),
        payload
    );
    let replay = native_coordinator_lease_expiry_feedback(&f.context, &f.actor, &child).unwrap();
    assert_eq!(replay.event_sequence, paid.event_sequence);
    assert_eq!(seen.lock().unwrap().len(), 3);
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}
