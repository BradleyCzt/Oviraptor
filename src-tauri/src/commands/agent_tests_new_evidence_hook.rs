// Real localhost Web HTTP + SDK + financial close + delivery + Root hook.
#[test]
fn new_evidence_production_finish_pays_one_original_assessment_and_replays_zero() {
    let _real = RealSpecialistTransport::enter();
    let mut f = input_parser_production_fixture("ne-hook-paid");
    let db = db::open(&f.h.db_path).unwrap();
    let (id, _) = f.record(&db);
    let wires = f.h.model_seen.lock().unwrap().clone();
    // Decode the JSON wire instead of relying on escaped transport formatting.
    let inputs: Vec<JsonValue> = wires
        .iter()
        .filter(|wire| wire.contains("You are the Root Coordinator"))
        .map(|wire| {
            let body: JsonValue =
                serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap()
        })
        .filter(|input: &JsonValue| input["basis"]["phase"] == "new-evidence")
        .collect();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0]["permittedSuggestion"], "assess:new_evidence");
    assert_eq!(
        inputs[0]["basis"]["changedFact"]["semantic"]["binding"]["evidenceId"],
        id
    );
    assert_eq!(
        inputs[0]["basis"]["changedFact"]["semantic"]["dispatchGranted"],
        false
    );
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    let before = client_delivery_all_rows(&db);
    let calls = f.calls();
    let replay = native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &id).unwrap();
    assert!(native_root_paid_suggestion_selected(
        &replay,
        "assess:new_evidence"
    ));
    multi_agent_new_evidence_supervision(&f.h.context, f.session.as_ref().unwrap()).unwrap();
    let original_outcome = f.outcome.clone();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &original_outcome)
        .unwrap();
    assert_eq!(client_delivery_all_rows(&db), before);
    assert_eq!(f.calls(), calls);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn new_evidence_same_revision_two_real_http_nodes_each_pay_once_full_finish_replays_zero() {
    let _real = RealSpecialistTransport::enter();
    let mut f = input_parser_root_fixture_inner(
        "ne-two-nodes",
        &["dispatch:input_parser"],
        (60000, 20),
        true,
        input_parser_child_valid_reply,
        2,
    );
    let db = db::open(&f.h.db_path).unwrap();
    let (nodes, revisions): (i64,i64) = db.query_row(
        "SELECT count(*),count(DISTINCT revision) FROM agent_evidence_nodes
         WHERE root_run_id=?1 AND created_by_run_id=?2 AND kind='request_record' AND provenance='observed'",
        params![f.root, f.session.as_ref().unwrap().executor.run_id], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((nodes, revisions), (2, 1));
    let inputs: Vec<JsonValue> =
        f.h.model_seen
            .lock()
            .unwrap()
            .iter()
            .filter(|wire| wire.contains("You are the Root Coordinator"))
            .map(|wire| {
                let body: JsonValue =
                    serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap()
            })
            .filter(|input: &JsonValue| input["basis"]["phase"] == "new-evidence")
            .collect();
    assert_eq!(inputs.len(), 2);
    assert_ne!(
        inputs[0]["basis"]["changedFact"],
        inputs[1]["basis"]["changedFact"]
    );
    let calls = f.calls();
    let before = client_delivery_all_rows(&db);
    let outcome = f.outcome.clone();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    assert_eq!(client_delivery_all_rows(&db), before);
    assert_eq!(f.calls(), calls);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 2);
}

#[test]
fn new_evidence_original_paid_hook_expired_c_replay_denies_without_new_sdk_or_writes() {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_production_fixture("ne-hook-expired");
    let db = db::open(&f.h.db_path).unwrap();
    let (id, _) = f.record(&db);
    let calls = f.calls();
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1", [&f.root]).unwrap();
    let before = client_delivery_all_rows(&db);
    assert!(native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &id).is_err());
    assert_eq!(client_delivery_all_rows(&db), before);
    assert_eq!(f.calls(), calls);
}
