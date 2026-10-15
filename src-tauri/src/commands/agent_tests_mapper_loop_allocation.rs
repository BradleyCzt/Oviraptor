// Actual new signed birth + localhost paid Root. No Mapper loop completion.
fn mapper_loop_allocation_fixture(
    paid: bool,
    requests: i64,
) -> (
    RootTickFixture,
    Option<NativeCoordinatorTickReceipt>,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let f = root_tick_fixture_from_web_mode_evidence_protocol(
        "mapper-loop-allocation",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (60_000, requests),
        None,
        web_mode_fixture("multi", "https://authorized.example.test"),
        RootFixtureEvidence {
            mapper_v15: true,
            ..Default::default()
        },
    );
    let decision = paid.then(|| native_coordinator_tick(&f.context, &f.actor).unwrap());
    assert_eq!(seen.lock().unwrap().len(), usize::from(paid));
    (f, decision, seen)
}
fn mapper_loop_allocation_bounds(
    f: &RootTickFixture,
) -> super::mapper_loop_request::VerifiedMapperLoopRequestBounds {
    let db = db::open(&f.context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    let bounds = super::mapper_loop_request::VerifiedMapperLoopRequestBounds::capture_on(
        &tx,
        &f.actor,
        &f.context.target_dir,
        mapper_request_inventory(&tx, f),
        &f.context.environment,
        f.context.proxy.as_deref(),
    )
    .unwrap();
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
    bounds
}
fn mapper_loop_original_basis(decision: &NativeCoordinatorTickReceipt) -> JsonValue {
    let mut original = decision.tick.basis().clone();
    let object = original.as_object_mut().unwrap();
    for key in [
        "localParents",
        "localHistory",
        "localRound",
        "localCallsUsed",
        "budgetSnapshot",
    ] {
        object.remove(key);
    }
    original
}

#[test]
fn mapper_loop_allocation_actual_paid_birth_two_requests_original_invoices_and_replay() {
    use crate::agent_runtime::multi_agent::{attempts, budget};
    let (f, decision, seen) = mapper_loop_allocation_fixture(true, 20);
    let decision = decision.unwrap();
    let bounds = mapper_loop_allocation_bounds(&f);
    let original = mapper_loop_original_basis(&decision);
    assert_eq!(original["bootstrapDispatch"]["schemaVersion"], 15);
    let db = db::open(&f.context.db_path).unwrap();
    let child = native_coordinator_prepare_mapper_loop_allocated(
        &db,
        &f.context,
        &f.actor,
        &decision,
        &bootstrap_mapper_task(&f),
        &original,
        &bounds,
    )
    .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let grant =
        budget::model::original_mapper_loop_grant_on(&tx, &f.actor, &child.assignment_id).unwrap();
    let tokens = bounds.first_estimate() + bounds.second_estimate();
    assert_eq!(grant.tokens(), tokens);
    assert!(tokens <= 8000);
    assert_eq!(grant.requests(), 2);
    assert_eq!(grant.input().fact(), bounds.fact());
    let worker = attempts::current(&tx, &f.actor, &child.assignment_id).unwrap();
    assert_eq!(worker.lease_epoch, 1);
    assert_eq!(worker.state, "running");
    assert_eq!(grant.worker_attempt_id(), worker.id);
    let held:(i64,i64,String,String)=tx.query_row("SELECT reserved_tokens,reserved_requests,lane,capability_lease_json FROM agent_assignments WHERE id=?1",[&child.assignment_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!((held.0, held.1), (tokens, 2));
    assert_eq!(held.2, "read_only_analysis");
    assert_eq!(
        serde_json::from_str::<JsonValue>(&held.3).unwrap(),
        json!(["evidence.read", "mailbox.write"])
    );
    let mut query=tx.prepare("SELECT dimension,amount,idempotency_key,source_id FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND kind='reserve' ORDER BY dimension").unwrap();
    let rows = query
        .query_map(
            params![f.actor.root_run_id, child.assignment_id, worker.id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let model: Vec<_> = rows
        .iter()
        .filter(|row| {
            matches!(
                row.0.as_str(),
                "model_input_tokens"
                    | "model_cached_tokens"
                    | "model_output_tokens"
                    | "model_requests"
            )
        })
        .collect();
    assert_eq!(model.len(), 4);
    for (dimension, amount, key, source) in model {
        assert_eq!(
            *amount,
            if dimension == "model_requests" {
                2
            } else {
                tokens
            }
        );
        assert_eq!(key, &format!("reserve:{}:{dimension}", child.assignment_id));
        assert_eq!(source, &format!("assignment:{}", child.assignment_id));
    }
    let caps:i64=tx.query_row("SELECT count(*) FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 AND capability IN ('evidence.read','mailbox.write') AND revoked_at=''",params![child.assignment_id,child.run_id],|r|r.get(0)).unwrap();
    assert_eq!(caps, 2);
    let remaining = budget::root::remaining_child_capacity(&tx, &f.actor.root_run_id).unwrap();
    assert_eq!(remaining, (Some(60_000 - 20 - tokens), Some(17)));
    assert!(remaining.0.unwrap() >= 15000);
    budget::supervision_capacity::require_mapper_loop_reserved_headroom_on(&tx, &f.actor).unwrap();
    assert_eq!(root_tick_count(&tx, &f.actor.root_run_id, "publication"), 1);
    drop(query);
    drop(tx);
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        native_coordinator_prepare_mapper_loop_allocated(
            &db,
            &f.context,
            &f.actor,
            &decision,
            &bootstrap_mapper_task(&f),
            &original,
            &bounds
        )
        .unwrap(),
        child
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "only actual Root SDK, no Mapper SDK yet"
    );
}

#[test]
fn mapper_loop_allocation_original_inventory_profile_scope_and_missing_paid_root_refuse_without_writes(
) {
    let (f, decision, seen) = mapper_loop_allocation_fixture(true, 20);
    let decision = decision.unwrap();
    let bounds = mapper_loop_allocation_bounds(&f);
    let original = mapper_loop_original_basis(&decision);
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let mut altered = f.context.clone();
    altered
        .environment
        .api_key
        .push_str("-changed-original-model-binding");
    assert!(native_coordinator_prepare_mapper_loop_allocated(
        &db,
        &altered,
        &f.actor,
        &decision,
        &bootstrap_mapper_task(&f),
        &original,
        &bounds
    )
    .is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    let mut foreign = f.actor.clone();
    foreign.attempt_number += 1;
    assert!(native_coordinator_prepare_mapper_loop_allocated(
        &db,
        &f.context,
        &foreign,
        &decision,
        &bootstrap_mapper_task(&f),
        &original,
        &bounds
    )
    .is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    let path = f.context.target_dir.join("frontend-evidence.json");
    let bytes = fs::read(&path).unwrap();
    fs::write(
        &path,
        json!({"url":f.actor.target_key,"changed":"original inventory"}).to_string(),
    )
    .unwrap();
    assert!(native_coordinator_prepare_mapper_loop_allocated(
        &db,
        &f.context,
        &f.actor,
        &decision,
        &bootstrap_mapper_task(&f),
        &original,
        &bounds
    )
    .is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    fs::write(&path, bytes).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    let (unpaid, _, unpaid_seen) = mapper_loop_allocation_fixture(false, 20);
    let unpaid_bounds = mapper_loop_allocation_bounds(&unpaid);
    let unpaid_db = db::open(&unpaid.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&unpaid_db);
    assert!(native_coordinator_prepare_mapper_loop_allocated(
        &unpaid_db,
        &unpaid.context,
        &unpaid.actor,
        &decision,
        &bootstrap_mapper_task(&unpaid),
        &original,
        &unpaid_bounds
    )
    .is_err());
    assert_eq!(super::tests::application_table_snapshot(&unpaid_db), before);
    assert_eq!(unpaid_seen.lock().unwrap().len(), 0);
}

#[test]
fn mapper_loop_allocation_original_supervision_floor_insufficient_rolls_back() {
    // Paid Root consumes one of eight; seven remaining cannot retain six
    // original follow-ups plus two proposed Mapper requests.
    let (f, decision, seen) = mapper_loop_allocation_fixture(true, 8);
    let decision = decision.unwrap();
    let bounds = mapper_loop_allocation_bounds(&f);
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let error = native_coordinator_prepare_mapper_loop_allocated(
        &db,
        &f.context,
        &f.actor,
        &decision,
        &bootstrap_mapper_task(&f),
        &mapper_loop_original_basis(&decision),
        &bounds,
    )
    .unwrap_err();
    assert!(
        error.starts_with("mapper_loop_reviewer_supervision_headroom:"),
        "{error}"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn mapper_loop_allocation_original_v9_and_v14_cannot_use_new_issuer() {
    let (new, _, _) = mapper_loop_allocation_fixture(false, 20);
    let bounds = mapper_loop_allocation_bounds(&new);
    let (old, decision, seen) = bootstrap_paid_fixture("mapper-loop-old-v9");
    let db = db::open(&old.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        native_coordinator_prepare_mapper_loop_allocated(
            &db,
            &old.context,
            &old.actor,
            &decision,
            &bootstrap_mapper_task(&old),
            &mapper_loop_original_basis(&decision),
            &bounds
        )
        .unwrap_err(),
        "mapper_loop_original_protocol_missing"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let _real = RealSpecialistTransport::enter();
    let old = business_logic_dispatch_actual_fixture(false, true);
    let actor = &old.session.as_ref().unwrap().lease;
    let context = native_coordinator_root_context(&old.h.context, actor);
    let decision = native_coordinator_tick(&context, actor).unwrap();
    assert_eq!(
        decision.tick.basis()["bootstrapDispatch"]["schemaVersion"],
        14
    );
    let db = db::open(&old.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let requests = old.h.model_seen.lock().unwrap().len();
    assert_eq!(
        native_coordinator_prepare_mapper_loop_allocated(
            &db,
            &context,
            actor,
            &decision,
            &json!({"target":actor.target_key}),
            &mapper_loop_original_basis(&decision),
            &bounds
        )
        .unwrap_err(),
        "mapper_loop_original_protocol_missing"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(old.h.model_seen.lock().unwrap().len(), requests);
}

// Cold damage is confined to this test's disposable original database. Keep
// its path, owner and every invoice; only replace the frozen task JSON, with
// the original schema guards restored before any production reader runs.
fn mapper_loop_provider_wire_replace_fixture_task(
    db: &rusqlite::Connection,
    assignment: &str,
    task: &JsonValue,
) {
    assert!(db.is_autocommit());
    let triggers = db
        .prepare("SELECT name,sql FROM main.sqlite_master WHERE type='trigger' AND tbl_name='agent_assignments' ORDER BY name")
        .unwrap()
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    for (name, _) in &triggers {
        tx.execute_batch(&format!("DROP TRIGGER \"{}\"", name.replace('"', "\"\"")))
            .unwrap();
    }
    assert_eq!(
        tx.execute(
            "UPDATE agent_assignments SET task_slice_json=?2 WHERE id=?1",
            params![assignment, task.to_string()],
        )
        .unwrap(),
        1,
        "cold damage must affect exactly the original Mapper task"
    );
    for (_, ddl) in &triggers {
        tx.execute_batch(ddl).unwrap();
    }
    tx.commit().unwrap();
    let restored = db
        .prepare("SELECT name,sql FROM main.sqlite_master WHERE type='trigger' AND tbl_name='agent_assignments' ORDER BY name")
        .unwrap()
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(restored, triggers);
}

#[test]
fn mapper_loop_allocation_legacy_read_does_not_authorize_downgraded_live_request() {
    use crate::agent_runtime::{
        multi_agent::{budget, specialist::mapper_loop},
        store,
    };
    let (f, decision, seen) = mapper_loop_allocation_fixture(true, 20);
    let decision = decision.unwrap();
    let bounds = mapper_loop_allocation_bounds(&f);
    assert_eq!(bounds.fact()["request"].as_object().unwrap().len(), 10);
    assert_eq!(bounds.fact()["request"]["maxOutputTokens"], 512);
    assert_eq!(bounds.fact()["maxRounds"], 2);
    assert_eq!(bounds.fact()["maxLookupCalls"], 1);
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
    let original = super::tests::application_table_snapshot(&db);
    let raw: String = db
        .query_row(
            "SELECT task_slice_json FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |row| row.get(0),
        )
        .unwrap();
    let original_task: JsonValue = serde_json::from_str(&raw).unwrap();
    assert_eq!(original_task.to_string(), raw);
    let tx = db.unchecked_transaction().unwrap();
    let grant =
        budget::model::original_mapper_loop_grant_on(&tx, &f.actor, &child.assignment_id).unwrap();
    assert_eq!(grant.input().fact(), bounds.fact());
    assert_eq!(grant.requests(), 2);
    let tokens = grant.tokens();
    assert!(tokens <= 8000);
    drop(tx);

    let mut legacy_task = original_task.clone();
    assert!(legacy_task["mapperLoop"]["request"]
        .as_object_mut()
        .unwrap()
        .remove("providerOptions")
        .is_some());
    assert_eq!(
        legacy_task["mapperLoop"]["request"]
            .as_object()
            .unwrap()
            .len(),
        9
    );
    legacy_task["mapperLoopHash"] =
        json!(store::stable_hash(&legacy_task["mapperLoop"].to_string()));
    let mut restored_task = legacy_task.clone();
    restored_task["mapperLoop"] = original_task["mapperLoop"].clone();
    restored_task["mapperLoopHash"] = original_task["mapperLoopHash"].clone();
    assert_eq!(
        restored_task, original_task,
        "only the frozen wire and its hash may change"
    );
    mapper_loop_provider_wire_replace_fixture_task(&db, &child.assignment_id, &legacy_task);
    let before = super::tests::application_table_snapshot(&db);
    let calls = seen.lock().unwrap().len();
    assert_eq!(calls, 1, "only the original paid Root request exists");

    // A historical reader accepts and preserves the nine-field grant using
    // the unchanged original paid Root, worker and two-request invoices.
    let tx = db.unchecked_transaction().unwrap();
    let changes = tx.total_changes();
    let historical =
        budget::model::original_mapper_loop_grant_on(&tx, &f.actor, &child.assignment_id).unwrap();
    historical.input().require_on(&tx, &f.actor).unwrap();
    assert_eq!(historical.input().fact(), &legacy_task["mapperLoop"]);
    assert_eq!(
        historical.input().hash(),
        legacy_task["mapperLoopHash"].as_str().unwrap()
    );
    assert!(historical.input().fact()["request"]
        .get("providerOptions")
        .is_none());
    assert_eq!(historical.tokens(), tokens);
    assert_eq!(historical.requests(), 2);
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
    assert_eq!(tx.total_changes(), changes);
    drop(tx);

    // The live transport must still compare the complete original current
    // wire. Historical compatibility cannot remove a live frozen option.
    match mapper_loop::begin_first_for_transport(&db, &f.actor, &child, &bounds) {
        Err(error) => assert_eq!(error, "mapper_loop_original_input_changed"),
        Ok(_) => panic!("a downgraded historical fact must not authorize live dispatch"),
    }
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(
        seen.lock().unwrap().len(),
        calls,
        "refusal cannot send a Mapper SDK request"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_mapper_loop_journal WHERE child_run_id=?1",
            [&child.run_id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    mapper_loop_provider_wire_replace_fixture_task(&db, &child.assignment_id, &original_task);
    assert_eq!(super::tests::application_table_snapshot(&db), original);
    assert_eq!(seen.lock().unwrap().len(), calls);
}
