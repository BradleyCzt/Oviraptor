// Cold corruption changes only the public declaration in this disposable DB.
// Preserve its row identity, every immutable guard, and the original birth seal.
fn root_v2_slot_cold_replace_mode_definition(
    db: &rusqlite::Connection,
    root: &str,
    definition: &JsonValue,
) {
    use crate::agent_runtime::web_mode::root::NewRootModeDeclaration;
    let typed: NewRootModeDeclaration = serde_json::from_value(definition.clone()).unwrap();
    let canonical = serde_json::to_string(&typed).unwrap();
    let guards = |db: &rusqlite::Connection| -> Vec<(String, String)> {
        db.prepare("SELECT name,sql FROM sqlite_master WHERE type='trigger' AND tbl_name='agent_root_mode_definitions' ORDER BY name")
            .unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap().collect::<rusqlite::Result<_>>().unwrap()
    };
    let original_guards = guards(db);
    assert!(!original_guards.is_empty());
    let tx = db.unchecked_transaction().unwrap();
    for (name, _) in &original_guards {
        tx.execute_batch(&format!("DROP TRIGGER \"{}\"", name.replace('"', "\"\""))).unwrap();
    }
    assert_eq!(tx.execute(
        "UPDATE agent_root_mode_definitions SET mode_id=?1,definition_json=?2 WHERE root_run_id=?3",
        params![definition["modeId"].as_str().unwrap(), canonical, root],
    ).unwrap(), 1);
    for (_, sql) in &original_guards {
        tx.execute_batch(sql).unwrap();
    }
    tx.commit().unwrap();
    assert_eq!(guards(db), original_guards);
}

fn root_v2_slot_cold_birth_refusal_is_readonly(h: &AgentHarness, root: &str) {
    let db = db::open(&h.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert!(crate::agent_runtime::web_mode::root::read(&tx, root).unwrap().is_some(),
        "canonical, schema-valid cold corruption must reach the birth signature gate");
    let before = super::tests::application_table_snapshot(&tx);
    let changes = tx.total_changes();
    assert_eq!(native_http_budget_protocol_enabled_on(&tx, root).unwrap_err(),
        "root_mode_birth_binding_changed");
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
    assert_eq!(tx.total_changes(), changes);
    assert!(h.model_seen.lock().unwrap().is_empty());
    root_v2_slot_legacy_none_has_no_http(h);
}

#[test]
fn root_v2_slot_legacy_none_cold_mode_change_and_forged_bootstrap_fail_original_birth() {
    let h = root_v2_slot_harness("legacy-none-cold-birth", 1);
    let root = h.context.run.as_ref().unwrap().run_id.clone();
    let db = db::open(&h.db_path).unwrap();
    let original = crate::agent_runtime::web_mode::root::read(&db, &root).unwrap().unwrap();
    assert!(original.bootstrap_dispatch().is_none());
    let original = serde_json::to_value(original).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert!(!native_http_budget_protocol_enabled_on(&tx, &root).unwrap());
    drop(tx);

    // Copy only a public, valid v8 protocol shape, never a private seal or grant.
    let mut v8 = fresh_multi_production_harness("legacy-none-public-v8-shape");
    freeze_fresh_multi_production_harness_http_v8(&mut v8);
    let v8_db = db::open(&v8.db_path).unwrap();
    let v8_root = &v8.context.run.as_ref().unwrap().run_id;
    let bootstrap = crate::agent_runtime::web_mode::root::read(&v8_db, v8_root)
        .unwrap().unwrap().bootstrap_dispatch().unwrap();
    assert_eq!(bootstrap["schemaVersion"], 8);
    let initial = super::tests::application_table_snapshot(&db);
    let mut changed_mode = original.clone();
    changed_mode["modeId"] = json!(Uuid::new_v4().to_string());
    let mut forged_bootstrap = original.clone();
    forged_bootstrap["bootstrapDispatch"] = bootstrap;
    for corrupted in [changed_mode, forged_bootstrap] {
        root_v2_slot_cold_replace_mode_definition(&db, &root, &corrupted);
        root_v2_slot_cold_birth_refusal_is_readonly(&h, &root);
        root_v2_slot_cold_replace_mode_definition(&db, &root, &original);
        assert_eq!(super::tests::application_table_snapshot(&db), initial,
            "restoring the original bytes must restore every original row and seal");
    }
    assert!(h.model_seen.lock().unwrap().is_empty());
    assert!(v8.model_seen.lock().unwrap().is_empty());
    root_v2_slot_legacy_none_has_no_http(&h);
    root_v2_slot_legacy_none_has_no_http(&v8);
    drop(db);
    drop(v8_db);
    fs::remove_dir_all(h.root).unwrap();
    fs::remove_dir_all(v8.root).unwrap();
}

#[test]
fn root_v2_slot_original_v8_cold_bootstrap_deletion_cannot_become_legacy_none() {
    let mut h = fresh_multi_production_harness("v8-cold-bootstrap-deletion");
    freeze_fresh_multi_production_harness_http_v8(&mut h);
    let root = h.context.run.as_ref().unwrap().run_id.clone();
    let db = db::open(&h.db_path).unwrap();
    let original = crate::agent_runtime::web_mode::root::read(&db, &root).unwrap().unwrap();
    assert_eq!(original.bootstrap_dispatch().unwrap()["schemaVersion"], 8);
    let original = serde_json::to_value(original).unwrap();
    let initial = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let changes = tx.total_changes();
    assert!(native_http_budget_protocol_enabled_on(&tx, &root).unwrap());
    assert_eq!(super::tests::application_table_snapshot(&tx), initial);
    assert_eq!(tx.total_changes(), changes);
    drop(tx);
    let mut stripped = original.clone();
    assert!(stripped.as_object_mut().unwrap().remove("bootstrapDispatch").is_some());
    root_v2_slot_cold_replace_mode_definition(&db, &root, &stripped);
    assert!(crate::agent_runtime::web_mode::root::read(&db, &root)
        .unwrap().unwrap().bootstrap_dispatch().is_none());
    root_v2_slot_cold_birth_refusal_is_readonly(&h, &root);
    root_v2_slot_cold_replace_mode_definition(&db, &root, &original);
    assert_eq!(super::tests::application_table_snapshot(&db), initial);
    assert!(h.model_seen.lock().unwrap().is_empty());
    root_v2_slot_legacy_none_has_no_http(&h);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
