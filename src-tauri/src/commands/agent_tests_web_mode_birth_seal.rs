#[test]
fn web_mode_birth_seal_is_private_immutable_and_cold_protocol_cannot_be_upgraded() {
    use crate::agent_runtime::web_mode::root;
    let fixture = web_mode_fixture("multi", "https://mode.example.test/app");
    web_mode_test_record(&fixture, true, true).unwrap();
    let db = db::open(&fixture.path).unwrap();
    let proof = private_web_mode_on(&db, &fixture.scan, 1).unwrap();
    assert!(!format!("{proof:?}").contains("private_key"));
    let id: String = db
        .query_row(
            "SELECT root_run_id FROM agent_root_mode_birth_seals",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    root::verify_birth(&tx, &id, &proof).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
    drop(tx);
    for sql in ["UPDATE agent_root_mode_birth_seals SET signature=zeroblob(32)", "DELETE FROM agent_root_mode_birth_seals", "INSERT OR REPLACE INTO agent_root_mode_birth_seals SELECT * FROM agent_root_mode_birth_seals"] {
        let before = super::tests::application_table_snapshot(&db);
        assert!(db.execute_batch(sql).is_err());
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    // Cold DB attacker can recalculate public canonical JSON, but cannot sign.
    let tx = db.unchecked_transaction().unwrap();
    tx.execute_batch(
        "DROP TRIGGER root_mode_immutable_update; DROP TRIGGER root_mode_birth_no_update;",
    )
    .unwrap();
    let raw: String = tx
        .query_row(
            "SELECT definition_json FROM agent_root_mode_definitions WHERE root_run_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    let mut definition: JsonValue = serde_json::from_str(&raw).unwrap();
    definition["newEvidenceSupervision"] = json!({"schemaVersion":1,"source":"closed_web_executor_original_http_receipt","deduplication":"original_node_physical_fact","permittedStep":"assess:new_evidence","executionGrant":false});
    let d: root::NewRootModeDeclaration = serde_json::from_value(definition).unwrap();
    let canonical = serde_json::to_string(&d).unwrap();
    tx.execute(
        "UPDATE agent_root_mode_definitions SET definition_json=?1 WHERE root_run_id=?2",
        params![canonical, id],
    )
    .unwrap();
    let raw: String = tx
        .query_row(
            "SELECT binding_json FROM agent_root_mode_birth_seals WHERE root_run_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    let mut binding: JsonValue = serde_json::from_str(&raw).unwrap();
    binding["definition"] = serde_json::from_str(&canonical).unwrap();
    tx.execute(
        "UPDATE agent_root_mode_birth_seals SET binding_json=?1 WHERE root_run_id=?2",
        params![binding.to_string(), id],
    )
    .unwrap();
    assert!(root::read(&tx, &id).unwrap().is_some());
    let before = super::tests::application_table_snapshot(&tx);
    assert_eq!(
        root::verify_birth(&tx, &id, &proof).unwrap_err(),
        "root_mode_birth_signature_invalid"
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
    drop(tx);
    let tx = db.unchecked_transaction().unwrap();
    root::verify_birth(&tx, &id, &proof).unwrap();
}
