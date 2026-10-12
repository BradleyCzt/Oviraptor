// A03 source acceptance uses the real Broker artifact/observation writers.
#[test]
fn new_evidence_frame_broker_source_freezes_and_rejects_artifact_or_binding_drift() {
    let _real = RealSpecialistTransport::enter();
    // Original fresh creator, signed pre-birth v8, paid Root/Mapper/Web SDK,
    // then production replay_http with its original claim, fee and HTTP seal.
    let fixture = http_budget_receipt_actual_fixture();
    let context = &fixture.h.context;
    assert_eq!(
        fixture.h.site_seen.lock().unwrap().len(),
        1,
        "actual Broker target GET"
    );
    let db = db::open(&context.db_path).unwrap();
    let actor = &fixture.session.as_ref().unwrap().lease;
    let (id, refs): (String, String) = db
        .query_row(
            "SELECT id,artifact_refs_json FROM agent_evidence_nodes WHERE created_by_run_id=?1",
            [&context.run.as_ref().unwrap().run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let refs: Vec<String> = serde_json::from_str(&refs).unwrap();
    let artifact = refs[0].strip_prefix("agent-http/").unwrap().to_string();
    let frame = input_parser_frame_zero_write(&db, || {
        NativeCoordinatorFrame::new_http_evidence(&db, actor, &context.target_dir, &id).unwrap()
    });
    assert_eq!(frame.kind, "new-evidence");
    assert_eq!(frame.semantic["dispatchGranted"], false);
    assert_eq!(frame.semantic["targetRequestsGranted"], 0);
    assert_eq!(native_supervision_frame_admission(frame.kind), Ok(()));
    input_parser_frame_zero_write(&db, || frame.verify(&db, context, actor).unwrap());
    let repeated = input_parser_frame_zero_write(&db, || {
        NativeCoordinatorFrame::new_http_evidence(&db, actor, &context.target_dir, &id).unwrap()
    });
    assert_eq!(frame.fact(), repeated.fact());
    let mut forged = frame.clone();
    forged.semantic["dispatchGranted"] = true.into();
    assert_eq!(
        input_parser_frame_zero_write(&db, || forged.verify(&db, context, actor)),
        Err("root_new_evidence_binding_changed".into())
    );
    let body = context
        .target_dir
        .join(AGENT_HTTP_DIRECTORY)
        .join(artifact.replace(".json", ".body"));
    fs::write(body, b"different actual bytes").unwrap();
    assert_eq!(
        input_parser_frame_zero_write(&db, || frame.verify(&db, context, actor)),
        Err("root_new_evidence_source_invalid".into())
    );
    drop(db);
}

#[test]
fn new_evidence_frame_wrong_root_author_or_revision_never_writes() {
    let (root, context, actor) = input_parser_frame_fixture();
    let artifact = input_parser_trigger_observe(
        &context,
        1,
        json!({"method":"GET","url":context.target_url,"identity":"anonymous"}),
        "new-evidence-scope-request",
    );
    let db = db::open(&context.db_path).unwrap();
    let id = input_parser_trigger_node(&db, &artifact);
    let mut foreign = actor.clone();
    foreign.attempt_number += 1;
    assert_eq!(
        input_parser_frame_zero_write(&db, || NativeCoordinatorFrame::new_http_evidence(
            &db,
            &foreign,
            &context.target_dir,
            &id
        )
        .err()),
        Some("root_new_evidence_author_invalid".into())
    );
    foreign = actor.clone();
    foreign.root_run_id = "unrelated-root".into();
    assert_eq!(
        input_parser_frame_zero_write(&db, || NativeCoordinatorFrame::new_http_evidence(
            &db,
            &foreign,
            &context.target_dir,
            &id
        )
        .err()),
        Some("root_new_evidence_source_invalid".into())
    );
    db.execute(
        "UPDATE agent_evidence_nodes SET revision=revision+1 WHERE id=?1",
        [&id],
    )
    .unwrap();
    assert_eq!(
        input_parser_frame_zero_write(&db, || NativeCoordinatorFrame::new_http_evidence(
            &db,
            &actor,
            &context.target_dir,
            &id
        )
        .err()),
        Some("root_new_evidence_author_invalid".into())
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn new_evidence_frame_graph_and_file_without_transport_claim_are_refused() {
    let (root, context, actor) = input_parser_frame_fixture();
    let artifact = input_parser_trigger_observe(
        &context,
        1,
        json!({"method":"GET","url":context.target_url,"identity":"anonymous"}),
        "source-writer-without-transport",
    );
    let db = db::open(&context.db_path).unwrap();
    let id = input_parser_trigger_node(&db, &artifact);
    assert_eq!(
        input_parser_frame_zero_write(&db, || NativeCoordinatorFrame::new_http_evidence(
            &db,
            &actor,
            &context.target_dir,
            &id
        )
        .err()),
        Some("root_new_evidence_http_claim_missing".into())
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn new_evidence_frame_fabricated_claim_metadata_cannot_replace_durable_receipt() {
    let (root, context, actor) = input_parser_frame_fixture();
    let bytes = b"local file without any target send";
    let view = json!({"requestId":"invented-display-id","status":200,
        "bodySha256":format!("{:x}",Sha256::digest(bytes)),
        "httpClaim":{"schemaVersion":1,"runId":context.run.as_ref().unwrap().run_id,
            "invocationId":"invented-invocation","requestIndex":1,"budgetAttempt":1,
            "ordinal":1,"requestHash":"a".repeat(64)}});
    let artifact = agent_write_http_record(
        &context,
        1,
        &json!({"method":"GET","url":context.target_url,"identity":"anonymous"}),
        &view,
        bytes,
    )
    .unwrap();
    agent_record_http_observation(
        &context,
        "replay_http",
        &artifact,
        &view,
        "invented-display-id",
    )
    .unwrap();
    let db = db::open(&context.db_path).unwrap();
    let id = input_parser_trigger_node(&db, &artifact);
    assert_eq!(
        input_parser_frame_zero_write(&db, || NativeCoordinatorFrame::new_http_evidence(
            &db,
            &actor,
            &context.target_dir,
            &id
        )
        .err()),
        Some("root_new_evidence_http_receipt_invalid".into())
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_http_request_claims", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
