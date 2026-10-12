#[test]
fn new_evidence_actual_sdk_source_drift_keeps_original_bill_and_restored_fact_publishes_without_resend(
) {
    let _real = RealSpecialistTransport::enter();
    let f = new_evidence_closed_original_fixture("ne-source-drift");
    let db = db::open(&f.h.db_path).unwrap();
    let (node, _) = f.record(&db);
    let original: String = db
        .query_row(
            "SELECT payload_json FROM agent_evidence_nodes WHERE id=?1",
            [&node],
            |r| r.get(0),
        )
        .unwrap();
    let changed = node.clone();
    *f.boundary.lock().unwrap() = Some(Box::new(move |db| {
        db.execute("UPDATE agent_evidence_nodes SET payload_json=json_set(payload_json,'$.status',500) WHERE id=?1",[&changed]).unwrap();
    }));
    let (calls, consumed, publications) = (
        f.calls(),
        f.requests(&db).consumed,
        root_tick_count(&db, &f.root, "publication"),
    );
    assert!(native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &node).is_err());
    assert_eq!(f.calls(), calls + 1);
    assert_eq!(f.requests(&db).consumed, consumed + 1);
    assert_eq!(root_tick_count(&db, &f.root, "publication"), publications);
    let paid = client_delivery_all_rows(&db);
    assert!(native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &node).is_err());
    assert_eq!(client_delivery_all_rows(&db), paid);
    assert_eq!(f.calls(), calls + 1);
    // Restore only this disposable fixture's original bytes, never real evidence.
    db.execute(
        "UPDATE agent_evidence_nodes SET payload_json=?2 WHERE id=?1",
        params![node, original],
    )
    .unwrap();
    let costs = client_delivery_rows(&db, "agent_budget_entries");
    let decision =
        native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &node).unwrap();
    assert!(decision.replayed);
    assert_eq!(f.calls(), calls + 1);
    assert_eq!(f.requests(&db).consumed, consumed + 1);
    assert_eq!(
        root_tick_count(&db, &f.root, "publication"),
        publications + 1
    );
    assert_eq!(client_delivery_rows(&db, "agent_budget_entries"), costs);
    let restored = client_delivery_all_rows(&db);
    native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &node).unwrap();
    assert_eq!(client_delivery_all_rows(&db), restored);
    assert_eq!(f.calls(), calls + 1);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn new_evidence_actual_unknown_sdk_retains_original_debt_without_publication_or_retry() {
    let _real = RealSpecialistTransport::enter();
    let f = new_evidence_closed_original_fixture("ne-unknown-invoice");
    let db = db::open(&f.h.db_path).unwrap();
    let (node, _) = f.record(&db);
    *f.new_evidence_reply.lock().unwrap() = Some(Box::new(|_| {
        let mut response: JsonValue = serde_json::from_str(&proposal_model_response(
            &json!({"schemaVersion":1,"observed":["closed physical observation"],
                "missing":[],"suggestions":["assess:new_evidence"],"costNotes":[],"risks":[]})
            .to_string(),
        ))
        .unwrap();
        response.as_object_mut().unwrap().remove("usage");
        response.to_string()
    }));
    let calls = f.calls();
    let consumed = f.requests(&db).consumed;
    let publications = root_tick_count(&db, &f.root, "publication");
    let error = native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &node)
        .err()
        .unwrap();
    // The provider returned a response; only its usage invoice is missing.
    // Transport uncertainty uses root_tick_uncertain, received unknown usage
    // has this distinct original financial reconciliation contract.
    assert_eq!(error, "budget_indeterminate_requires_reconciliation");
    assert_eq!(f.calls(), calls + 1);
    assert_eq!(root_tick_count(&db, &f.root, "publication"), publications);
    // Receipt proves one provider request even when token usage is unknown.
    assert_eq!(f.requests(&db).consumed, consumed + 1);
    assert_eq!(f.requests(&db).indeterminate, 0);
    assert!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.root,
            Some(""),
            "model_input_tokens",
        )
        .unwrap()
        .indeterminate
            > 0
    );
    let debt = client_delivery_all_rows(&db);
    assert!(native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &node).is_err());
    assert_eq!(client_delivery_all_rows(&db), debt);
    assert_eq!(f.calls(), calls + 1);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
