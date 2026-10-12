// Temporary real original Root/artifact fixtures; helper-only, no Mapper SDK.
#[test]
fn mapper_lookup_original_inventory_scope_strict_args_and_drift_are_readonly() {
    use super::mapper_evidence_lookup::{MapperEvidenceInventory, MapperLookupLimits};
    let f = root_tick_fixture("mapper-original-lookup", "http://127.0.0.1:65530/v1");
    let db = db::open(&f.context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    let inventory = MapperEvidenceInventory::capture_on(
        &tx,
        &f.actor,
        &f.context.target_dir,
        1,
        MapperLookupLimits::new(256 * 1024, 24_000).unwrap(),
    )
    .unwrap();
    assert_eq!(inventory.fact()["evidenceRevision"], 1);
    assert_eq!(inventory.fact()["limits"]["sourceMaxBytes"], 256 * 1024);
    assert_eq!(
        inventory.hash(),
        crate::agent_runtime::store::stable_hash(&inventory.fact().to_string())
    );
    let observation = inventory
        .lookup_on(&tx, &f.actor, &json!({"evidenceId":"frontend-evidence"}))
        .unwrap();
    assert_eq!(observation.value()["ok"], true);
    assert_eq!(
        observation.value()["sha256"],
        inventory.fact()["entries"][0]["sha256"]
    );
    assert!(observation.value().to_string().len() <= 24_000);
    for (arguments, code) in [
        (
            json!({"evidenceId":"../outside"}),
            "mapper_lookup_evidence_id_invalid",
        ),
        (
            json!({"evidenceId":"frontend-evidence","path":"/etc/passwd"}),
            "mapper_lookup_arguments_invalid",
        ),
        (json!({"evidenceId":5}), "mapper_lookup_arguments_invalid"),
    ] {
        let audit = inventory.lookup_on(&tx, &f.actor, &arguments).unwrap();
        assert_eq!(audit.value()["code"], code);
        assert_eq!(audit.value()["executed"], false);
        assert!(!audit.value().to_string().contains("passwd"));
    }
    let mut foreign = f.actor.clone();
    foreign.attempt_number += 1;
    assert!(
        matches!(inventory.lookup_on(&tx,&foreign,&json!({"evidenceId":"frontend-evidence"})),Err(e) if e=="mapper_lookup_actor_changed")
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
    fs::write(
        f.context.target_dir.join("frontend-evidence.json"),
        json!({"url":f.actor.target_key,"changed":true}).to_string(),
    )
    .unwrap();
    assert!(
        matches!(inventory.lookup_on(&tx,&f.actor,&json!({"evidenceId":"frontend-evidence"})),Err(e) if e=="mapper_lookup_source_changed")
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
}

#[test]
fn mapper_lookup_new_limits_refuse_oversize_and_bound_utf8_observation() {
    use super::mapper_evidence_lookup::{MapperEvidenceInventory, MapperLookupLimits};
    assert!(MapperLookupLimits::new(256 * 1024 + 1, 24_000).is_err());
    assert!(MapperLookupLimits::new(256 * 1024, 24_001).is_err());
    let f = root_tick_fixture("mapper-bounded-lookup", "http://127.0.0.1:65530/v1");
    let file = f.context.target_dir.join("frontend-evidence.json");
    fs::write(
        &file,
        json!({"url":f.actor.target_key,"notes":"省市公司流程线索".repeat(2000)}).to_string(),
    )
    .unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let inventory = MapperEvidenceInventory::capture_on(
        &tx,
        &f.actor,
        &f.context.target_dir,
        1,
        MapperLookupLimits::new(256 * 1024, 512).unwrap(),
    )
    .unwrap();
    let observation = inventory
        .lookup_on(&tx, &f.actor, &json!({"evidenceId":"frontend-evidence"}))
        .unwrap();
    assert_eq!(observation.value()["truncated"], true);
    assert!(observation.value().to_string().len() <= 512);
    assert!(observation.value()["preview"]
        .as_str()
        .unwrap()
        .is_char_boundary(observation.value()["preview"].as_str().unwrap().len()));
    assert!(MapperEvidenceInventory::capture_on(
        &tx,
        &f.actor,
        &f.context.target_dir,
        1,
        MapperLookupLimits::new(16, 512).unwrap()
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn mapper_lookup_rejects_frontend_symlink_without_following_outside_artifact() {
    use super::mapper_evidence_lookup::{MapperEvidenceInventory, MapperLookupLimits};
    let f = root_tick_fixture("mapper-symlink-lookup", "http://127.0.0.1:65530/v1");
    let file = f.context.target_dir.join("frontend-evidence.json");
    let outside = f.f.root.join("outside-front-end.json");
    fs::write(
        &outside,
        json!({"url":f.actor.target_key,"private":"must not read"}).to_string(),
    )
    .unwrap();
    fs::remove_file(&file).unwrap();
    std::os::unix::fs::symlink(&outside, &file).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    assert!(MapperEvidenceInventory::capture_on(
        &tx,
        &f.actor,
        &f.context.target_dir,
        1,
        MapperLookupLimits::new(256 * 1024, 24_000).unwrap()
    )
    .is_err());
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
}
