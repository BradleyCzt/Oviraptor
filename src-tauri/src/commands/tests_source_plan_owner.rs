// These tests use the real Source publisher and original Root issuer. They do
// not repair or rewrite the fixtures consumed by existing Source regressions.
pub(super) fn source_plan_owner_frozen_fixture() -> (
    PathBuf,
    rusqlite::Connection,
    WorkbenchStartRecord,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    source_tool_true_born_fixture_model(None)
}

fn source_plan_owner_assert_redacted(
    db: &rusqlite::Connection,
    record: &WorkbenchStartRecord,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    use crate::agent_runtime::multi_agent::homogeneous::{
        execution_policy, material_policy, model_worker,
    };
    let before = db.total_changes();
    assert!(execution_policy::Binding::capture(db, &record.scan_id, 1)
        .unwrap()
        .is_none());
    assert!(!model_worker::enabled(db, &record.scan_id, 1).unwrap());
    assert!(!material_policy::for_root(db, &actor.root_run_id).unwrap());
    let text = "Cookie: session=source-owner-fixture";
    assert_ne!(
        material_policy::text(db, &actor.root_run_id, text).unwrap(),
        text
    );
    assert_eq!(
        db.total_changes(),
        before,
        "owner recognition must remain read-only"
    );
}

#[test]
fn source_plan_owner_uses_published_source_hash_without_homogeneous_capabilities() {
    let (root, db, record, actor) = source_plan_owner_frozen_fixture();
    let (text, hash): (String, String) = db.query_row(
        "SELECT plan_json,plan_hash FROM native_scan_plans WHERE scan_id=?1 AND attempt_number=1",
        [&record.scan_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    let plan: JsonValue = serde_json::from_str(&text).unwrap();
    assert_ne!(
        crate::agent_runtime::store::stable_hash(&text),
        hash,
        "Source intentionally excludes attemptNumber from its own hash"
    );
    assert!(
        NativeSourcePlan::verifies_source_owner(&db, &record.scan_id, 1, &plan, &hash).unwrap()
    );
    source_plan_owner_assert_redacted(&db, &record, &actor);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_plan_owner_history_needs_neither_current_attempt_nor_live_repository() {
    let (root, db, record, actor) = source_plan_owner_frozen_fixture();
    db.execute(
        "UPDATE sentinel_scans SET status='completed',attempt_count=2 WHERE id=?1",
        [&record.scan_id],
    )
    .unwrap();
    db.execute("UPDATE sentinel_scan_attempts SET status='completed' WHERE scan_id=?1 AND attempt_number=1", [&record.scan_id]).unwrap();
    fs::remove_dir_all(&record.source_path).unwrap();
    // Legacy scope version zero still proves ownership; it never grants the
    // upgraded Source runtime or any homogeneous/plaintext capability.
    db.execute_batch(
        "SAVEPOINT legacy_scope_owner; DROP TRIGGER source_scope_no_update;
        UPDATE source_scope_contracts SET analysis_policy_version=0;",
    )
    .unwrap();
    source_plan_owner_assert_redacted(&db, &record, &actor);
    db.execute_batch("ROLLBACK TO legacy_scope_owner; RELEASE legacy_scope_owner")
        .unwrap();
    source_plan_owner_assert_redacted(&db, &record, &actor);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_plan_owner_rejects_changed_material_identity_and_foreign_protocol_fields() {
    use crate::agent_runtime::multi_agent::homogeneous::{
        execution_policy, material_policy, model_worker,
    };
    let (root, db, record, actor) = source_plan_owner_frozen_fixture();
    let original = NativeSourcePlan::load(&db, &record.scan_id, 1)
        .unwrap()
        .unwrap();
    let original_json = original.as_json();
    let original_hash = original.hash();
    let mut cases = vec![
        (
            "tree_hash",
            {
                let mut v = original_json.clone();
                v["snapshot"]["treeHash"] = json!("changed");
                v
            },
            false,
        ),
        (
            "allowed_tools",
            {
                let mut v = original_json.clone();
                v["allowedTools"] = json!(["shell"]);
                v
            },
            false,
        ),
        (
            "scan_identity",
            {
                let mut v = original_json.clone();
                v["scanId"] = json!("another-scan");
                v
            },
            true,
        ),
        (
            "attempt_identity",
            {
                let mut v = original_json.clone();
                v["attemptNumber"] = json!(2);
                v
            },
            true,
        ),
        (
            "source_to_web",
            json!({"workerExecution":"model_read_only_v1","credentialVisibility":"plaintext",
            "scheduling_model":"homogeneous_workers","scanId":record.scan_id,"attemptNumber":1,
            "lane":"read_only_analysis","allowedParallel":3}),
            true,
        ),
    ];
    for (key, value) in [
        ("workerExecution", json!(execution_policy::MODE)),
        ("credentialVisibility", json!("plaintext")),
        ("scheduling_model", json!("homogeneous_workers")),
    ] {
        let mut changed = original_json.clone();
        changed[key] = value;
        cases.push((key, changed, false));
    }
    for (label, changed, rehash) in cases {
        db.execute_batch("SAVEPOINT source_owner_mutation").unwrap();
        let hash = if rehash {
            NativeSourcePlan::from_json(&changed)
                .map(|plan| plan.hash())
                .unwrap_or_else(|| crate::agent_runtime::store::stable_hash(&changed.to_string()))
        } else {
            original_hash.clone()
        };
        db.execute("UPDATE native_scan_plans SET plan_json=?1,plan_hash=?2 WHERE scan_id=?3 AND attempt_number=1",
            params![changed.to_string(),hash,record.scan_id]).unwrap();
        let before = db.total_changes();
        assert!(
            execution_policy::Binding::capture(&db, &record.scan_id, 1).is_err(),
            "{label}"
        );
        assert!(
            model_worker::enabled(&db, &record.scan_id, 1).is_err(),
            "{label}"
        );
        assert!(
            material_policy::for_root(&db, &actor.root_run_id).is_err(),
            "{label}"
        );
        assert_eq!(db.total_changes(), before, "{label} must not write");
        db.execute_batch("ROLLBACK TO source_owner_mutation; RELEASE source_owner_mutation")
            .unwrap();
    }
    for mutation in [
        "UPDATE native_scan_plans SET scan_type='greybox'",
        "UPDATE native_scan_plans SET backend='other'",
        "UPDATE native_scan_plans SET plan_hash='changed'",
    ] {
        db.execute_batch("SAVEPOINT source_owner_row").unwrap();
        db.execute_batch(mutation).unwrap();
        assert!(
            execution_policy::Binding::capture(&db, &record.scan_id, 1).is_err(),
            "{mutation}"
        );
        assert!(
            model_worker::enabled(&db, &record.scan_id, 1).is_err(),
            "{mutation}"
        );
        db.execute_batch("ROLLBACK TO source_owner_row; RELEASE source_owner_row")
            .unwrap();
    }
    source_plan_owner_assert_redacted(&db, &record, &actor);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_plan_owner_requires_original_scope_receipt_even_with_a_correct_source_hash() {
    use crate::agent_runtime::multi_agent::homogeneous::{execution_policy, model_worker};
    let (root, db, record, _) = source_plan_owner_frozen_fixture();
    let source = NativeSourcePlan::load(&db, &record.scan_id, 1)
        .unwrap()
        .unwrap();
    let unowned = db::open(&db::initialize(&root.join("unowned-plan")).unwrap()).unwrap();
    unowned.execute("INSERT INTO native_scan_plans(scan_id,attempt_number,scan_type,backend,plan_json,plan_hash)
        VALUES(?1,1,?2,'native',?3,?4)",params![record.scan_id,source.scan_type,source.as_json().to_string(),source.hash()]).unwrap();
    let before = unowned.total_changes();
    assert!(execution_policy::Binding::capture(&unowned, &record.scan_id, 1).is_err());
    assert!(model_worker::enabled(&unowned, &record.scan_id, 1).is_err());
    assert_eq!(unowned.total_changes(), before);
    drop(unowned);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_plan_owner_recognition_never_authorizes_changed_source_material() {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{
            homogeneous::{execution_policy, model_worker},
            source,
        },
    };
    let (root, db, record, actor) = source_plan_owner_frozen_fixture();
    let mut plan = NativeSourcePlan::load(&db, &record.scan_id, 1)
        .unwrap()
        .unwrap();
    plan.tree_hash = "rehashed-unrelated-source-material".into();
    db.execute("UPDATE native_scan_plans SET plan_json=?1,plan_hash=?2 WHERE scan_id=?3 AND attempt_number=1",
        params![plan.as_json().to_string(),plan.hash(),record.scan_id]).unwrap();
    let before = db.total_changes();
    assert!(execution_policy::Binding::capture(&db, &record.scan_id, 1)
        .unwrap()
        .is_none());
    assert!(!model_worker::enabled(&db, &record.scan_id, 1).unwrap());
    assert!(
        source::task_slice(&db, &actor, AgentRole::SourceAnalyst).is_err(),
        "the Source owner still binds executable material to its original sealed snapshot"
    );
    assert_eq!(db.total_changes(), before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_plan_owner_missing_owner_store_fails_closed_for_web_shaped_rows() {
    use crate::agent_runtime::multi_agent::homogeneous::{execution_policy, model_worker};
    // Production and supported callers use db::initialize's complete schema.
    // An unavailable owner store cannot prove that a mutable Web-shaped row
    // did not replace a Source plan; it must not become a permissive fallback.
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE native_scan_plans(scan_id TEXT,attempt_number INTEGER,
        scan_type TEXT,backend TEXT,plan_json TEXT,plan_hash TEXT)",
    )
    .unwrap();
    let plan = json!({"schemaVersion":1,"scanId":"minimal-web","attemptNumber":1,
        "scheduling_model":"homogeneous_workers","workerExecution":"model_read_only_v1",
        "allowedParallel":3,"lane":"read_only_analysis","credentialVisibility":"plaintext"});
    db.execute(
        "INSERT INTO native_scan_plans VALUES('minimal-web',1,'web','native',?1,?2)",
        params![
            plan.to_string(),
            crate::agent_runtime::store::stable_hash(&plan.to_string())
        ],
    )
    .unwrap();
    let before = db.total_changes();
    assert!(execution_policy::Binding::capture(&db, "minimal-web", 1).is_err());
    assert!(model_worker::enabled(&db, "minimal-web", 1).is_err());
    assert_eq!(db.total_changes(), before);
}
