// Actual v15 paid birth/allocation and actual Mapper SDK; diagnostics grant nothing.
fn mapper_loop_diagnostic_fixture() -> (
    RootTickFixture,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    let (f, decision, seen) = mapper_loop_allocation_fixture(true, 20);
    let decision = decision.unwrap();
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
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "one paid Root + one real Mapper SDK"
    );
    (f, child, seen)
}
fn mapper_loop_diagnostic_page(f: &RootTickFixture) -> Result<JsonValue, String> {
    crate::agent_runtime::model::diagnostics::replay::read(
        &f.context.db_path,
        &f.actor.scan_id,
        f.actor.attempt_number,
        None,
        None,
        0,
        300,
    )
    .and_then(|page| serde_json::to_value(page).map_err(|e| e.to_string()))
}
#[test]
fn mapper_loop_diagnostics_actual_sdk_source_sidecar_rows_and_legacy_root_share_cursor() {
    let (f, child, seen) = mapper_loop_diagnostic_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let page = mapper_loop_diagnostic_page(&f).unwrap();
    let rows = page["rows"].as_array().unwrap();
    let mapper: Vec<_> = rows
        .iter()
        .filter(|row| row["domain"] == "mapper_loop")
        .collect();
    assert_eq!(
        mapper
            .iter()
            .map(|row| row["stage"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "prepared",
            "sent",
            "response_received",
            "cost_saved",
            "validated",
            "terminal"
        ]
    );
    assert!(mapper.iter().all(|row| row["runId"] == child.run_id
        && row["assignmentId"] == child.assignment_id
        && row["round"] == 1));
    assert_eq!(mapper.last().unwrap()["terminalState"], "returned");
    let id = mapper[0]["ownerId"].as_str().unwrap();
    let (family,logical,hash):(String,String,String)=db.query_row("SELECT o.domain,x.logical_domain,o.identity_hash FROM native_sdk_log_owners o JOIN native_sdk_log_mapper_owners x ON x.owner_id=o.owner_id WHERE o.owner_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        (family.as_str(), logical.as_str()),
        ("specialist", "mapper_loop")
    );
    let owner = page["owners"]
        .as_array()
        .unwrap()
        .iter()
        .find(|owner| owner["ownerId"] == id)
        .unwrap();
    assert_eq!(owner["domain"], "mapper_loop");
    assert_eq!(
        db.query_row(
            "SELECT identity_hash FROM native_sdk_log_mapper_owners WHERE owner_id=?1",
            [id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        hash
    );
    let (call,worker,request):(String,String,String)=db.query_row("SELECT call_id,worker_attempt_id,request_hash FROM agent_mapper_loop_journal WHERE child_run_id=?1 AND phase='dispatch'",[&child.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        id,
        crate::agent_runtime::store::stable_hash(&json!(["mapper_loop", call]).to_string())
    );
    assert_eq!(mapper[0]["dispatchKey"], call);
    assert_eq!(mapper[0]["leaseAttemptId"], worker);
    assert_eq!(mapper[0]["requestHash"], request);
    assert!(
        rows.iter().any(|row| row["domain"] == "root"),
        "original Root domain remains readable"
    );
    assert!(rows
        .windows(2)
        .all(|pair| pair[0]["sequence"].as_i64().unwrap() < pair[1]["sequence"].as_i64().unwrap()));
    assert_eq!(mapper_loop_diagnostic_page(&f).unwrap(), page);
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 2);
}
#[test]
fn mapper_loop_diagnostics_sidecar_immutable_and_cold_removal_never_legacy_fallback() {
    let (f, _, seen) = mapper_loop_diagnostic_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    let (id, domain, hash): (String, String, String) = db
        .query_row(
            "SELECT owner_id,logical_domain,identity_hash FROM native_sdk_log_mapper_owners",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let original = super::tests::application_table_snapshot(&db);
    for sql in ["UPDATE native_sdk_log_mapper_owners SET identity_hash=printf('%064d',1)","DELETE FROM native_sdk_log_mapper_owners","INSERT OR REPLACE INTO native_sdk_log_mapper_owners SELECT * FROM native_sdk_log_mapper_owners"] {
        let error=db.execute_batch(sql).unwrap_err();
        assert!(error.to_string().contains("native_sdk_log_mapper_immutable"));
        assert_eq!(super::tests::application_table_snapshot(&db),original);
    }
    let trigger: String = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='native_sdk_log_mapper_no_delete'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("DROP TRIGGER native_sdk_log_mapper_no_delete")
        .unwrap();
    assert_eq!(
        db.execute(
            "DELETE FROM native_sdk_log_mapper_owners WHERE owner_id=?1",
            [&id]
        )
        .unwrap(),
        1
    );
    db.execute_batch(&trigger).unwrap();
    let attacked = super::tests::application_table_snapshot(&db);
    assert_eq!(
        mapper_loop_diagnostic_page(&f).unwrap_err(),
        "native_sdk_log_owner_unverified"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), attacked);
    assert_eq!(seen.lock().unwrap().len(), 2);
    db.execute("INSERT INTO native_sdk_log_mapper_owners(owner_id,logical_domain,identity_hash) VALUES(?1,?2,?3)",params![id,domain,hash]).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), original);
    assert!(mapper_loop_diagnostic_page(&f).is_ok());
}

#[test]
fn mapper_loop_diagnostics_actual_first_dispatch_cannot_use_generic_unsent_refund() {
    use crate::agent_runtime::multi_agent::{budget, specialist::mapper_loop};
    let (f, decision, seen) = mapper_loop_allocation_fixture(true, 20);
    let decision = decision.unwrap();
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
    let mapper_loop::Start::Dispatch(pending) =
        mapper_loop::begin_first_for_transport(&db, &f.actor, &child, &bounds).unwrap()
    else {
        panic!("a new original first round must dispatch")
    };
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        budget::model::release_unsent(&tx, &f.actor, &child.assignment_id).unwrap_err(),
        "mapper_loop_original_reconciliation_required"
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
    tx.commit().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "diagnostic/refund refusal cannot send Mapper SDK"
    );
    drop(pending);
}

#[test]
fn mapper_loop_diagnostics_actual_sdk_normal_signed_close_ack_release_and_cold_signature_refusal() {
    use crate::agent_runtime::multi_agent::{budget, specialist::mapper_loop};
    let (f, decision, seen) = mapper_loop_allocation_fixture(true, 20);
    let decision = decision.unwrap();
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
    let payload = json!({"summary":output.text()});
    let mut wrong = *output.usage();
    wrong.model_requests += 1;
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        complete_readonly_assessment(&db, &f.actor, &child, &wrong, &payload).unwrap_err(),
        "mapper_loop_delivery_original_output_conflict"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(
        complete_readonly_assessment(&db, &f.actor, &child, output.usage(), &payload).unwrap(),
        payload
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
    let tx = db.unchecked_transaction().unwrap();
    let receipt = mapper_loop::final_seal::read_final_original_on(&tx, &f.actor, &child)
        .unwrap()
        .unwrap();
    let receipt_id = receipt.receipt_id().to_owned();
    let message_id = receipt.message_id().to_owned();
    assert_eq!(
        tx.query_row(
            "SELECT count(*) FROM agent_mapper_loop_final_receipts WHERE child_run_id=?1",
            [&child.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_messages WHERE id=?1 AND acknowledged_at<>'' AND delivery_attempts=1 AND kind='evidence_summary'",[&message_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    for dimension in [
        "model_input_tokens",
        "model_cached_tokens",
        "model_output_tokens",
        "model_requests",
    ] {
        let balance = budget::balance(
            &tx,
            &f.actor.root_run_id,
            Some(&child.assignment_id),
            dimension,
        )
        .unwrap();
        assert_eq!(balance.reserved, 0, "{dimension}");
        assert_eq!(balance.indeterminate, 0, "{dimension}");
        if dimension == "model_requests" {
            assert_eq!(balance.consumed, 1);
        }
    }
    tx.commit().unwrap();
    let closed = super::tests::application_table_snapshot(&db);
    assert_eq!(
        complete_readonly_assessment(&db, &f.actor, &child, output.usage(), &payload).unwrap(),
        payload
    );
    assert_eq!(super::tests::application_table_snapshot(&db), closed);
    let trigger: String = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='mapper_loop_final_no_update'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    tx.execute_batch("DROP TRIGGER mapper_loop_final_no_update")
        .unwrap();
    assert_eq!(tx.execute("UPDATE agent_mapper_loop_final_receipts SET signature=zeroblob(32) WHERE receipt_id=?1",[&receipt_id]).unwrap(),1);
    tx.execute_batch(&trigger).unwrap();
    let attacked = super::tests::application_table_snapshot(&tx);
    assert_eq!(
        mapper_loop::final_seal::read_final_original_on(&tx, &f.actor, &child)
            .err()
            .unwrap(),
        "mapper_loop_final_signature_invalid"
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), attacked);
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), closed);
    assert_eq!(seen.lock().unwrap().len(), 2);
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        mapper_loop::final_seal::read_final_original_on(&tx, &f.actor, &child)
            .unwrap()
            .unwrap()
            .receipt_id(),
        receipt_id
    );
}

#[test]
fn mapper_loop_diagnostics_actual_paid_final_insert_fault_rolls_back_ack_and_recovers_without_sdk()
{
    use crate::agent_runtime::multi_agent::specialist::mapper_loop;
    let (f, child, seen) = mapper_loop_diagnostic_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let output = mapper_loop::verified_original_output_on(&tx, &f.actor, &child).unwrap();
    tx.commit().unwrap();
    let payload = json!({"summary":output.text()});
    assert_eq!(output.usage().model_requests, 1);
    db.execute_batch("CREATE TRIGGER actual_mapper_final_fault BEFORE INSERT ON agent_mapper_loop_final_receipts BEGIN SELECT RAISE(IGNORE); END").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        complete_readonly_assessment(&db, &f.actor, &child, output.usage(), &payload).unwrap_err(),
        "mapper_loop_final_insert_unconfirmed"
    );
    assert_eq!(
        super::tests::application_table_snapshot(&db),
        before,
        "same completion TX must restore closure, release and ACK while keeping original paid bill"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_mapper_loop_final_receipts WHERE child_run_id=?1",
            [&child.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='evidence_summary'",[&child.assignment_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_mapper_loop_journal WHERE child_run_id=?1 AND phase='received'",[&child.run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            Some(&child.assignment_id),
            "model_requests"
        )
        .unwrap()
        .consumed,
        1
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
    db.execute_batch("DROP TRIGGER actual_mapper_final_fault")
        .unwrap();
    assert_eq!(
        complete_readonly_assessment(&db, &f.actor, &child, output.usage(), &payload).unwrap(),
        payload
    );
    let closed = super::tests::application_table_snapshot(&db);
    assert_eq!(
        complete_readonly_assessment(&db, &f.actor, &child, output.usage(), &payload).unwrap(),
        payload
    );
    assert_eq!(super::tests::application_table_snapshot(&db), closed);
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "local recovery uses the original paid result; never a new SDK"
    );
    let tx = db.unchecked_transaction().unwrap();
    let receipt = mapper_loop::final_seal::read_final_original_on(&tx, &f.actor, &child)
        .unwrap()
        .unwrap();
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_messages WHERE id=?1 AND acknowledged_at<>'' AND delivery_attempts=1",[receipt.message_id()],|r|r.get::<_,i64>(0)).unwrap(),1);
}
