#[test]
fn mapper_loop_expired_delivery_live_paid_output_cannot_fabricate_expiry_or_sign() {
    use crate::agent_runtime::multi_agent::specialist::mapper_loop;
    let (f, child, seen) = mapper_loop_diagnostic_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let output = mapper_loop::verified_original_output_on(&tx, &f.actor, &child).unwrap();
    let usage = *output.usage();
    let payload = serde_json::json!({"summary":output.text()});
    tx.rollback().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let calls = seen.lock().unwrap().len();
    assert_eq!(
        complete_expired_mapper_assessment(&db, &f.actor, &child, &usage, &payload).unwrap_err(),
        "mapper_loop_expired_original_worker_required",
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), calls);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_mapper_loop_expired_receipts", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
}
