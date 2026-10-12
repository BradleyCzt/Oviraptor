use super::*;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use std::os::unix::fs::PermissionsExt;

struct Fixture {
    root: PathBuf,
    db_path: PathBuf,
    configuration: ToolAdministratorConfiguration,
    administrator: Ed25519KeyPair,
    publisher: Ed25519KeyPair,
    manifest: Vec<u8>,
    package: Vec<u8>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-tool-approval-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let directory = root.join("configuration");
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(directory.join("tool-administrator")).unwrap();
        fs::set_permissions(
            directory.join("tool-administrator"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let administrator = Ed25519KeyPair::from_seed_unchecked(&[51; 32]).unwrap();
        let publisher = Ed25519KeyPair::from_seed_unchecked(&[52; 32]).unwrap();
        let package = b"fixed offline parser candidate".to_vec();
        let mut manifest = SignedToolManifest {
            payload: ToolCapabilityManifest {
                schema_version: 1,
                id: "offline-parser".into(),
                version: "1.0.0".into(),
                platform: "macos-aarch64".into(),
                source: "fixed-administrator-release".into(),
                digest: format!("sha256:{}", sha(&package)),
                license: "MIT".into(),
                entrypoint: "bin/parser".into(),
                broker_capabilities: vec!["source_analysis".into()],
                network_policy: "none".into(),
                filesystem_policy: "read-only-input-and-private-temp-output".into(),
                timeout_seconds: 10,
                memory_mib: 64,
                cpu_limit: 1,
                process_limit: 1,
                output_bytes: 4096,
                input_schema_version: 1,
                output_schema_version: 1,
            },
            signer_id: "publisher-1".into(),
            signature_base64: String::new(),
        };
        manifest.signature_base64 = base64::engine::general_purpose::STANDARD.encode(
            publisher
                .sign(&signed_message(&manifest.payload).unwrap())
                .as_ref(),
        );
        let db_path = root.join("temporary.db");
        let db = Connection::open(&db_path).unwrap();
        db.execute_batch("CREATE TABLE environment_preparation_lease(singleton INTEGER PRIMARY KEY CHECK(singleton=1),owner TEXT NOT NULL,created_at TEXT DEFAULT(datetime('now','localtime'))); CREATE TABLE environment_preparation_events(id INTEGER PRIMARY KEY,owner TEXT,event TEXT,created_at TEXT DEFAULT(datetime('now','localtime'))); CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY,status TEXT);").unwrap();
        db.execute_batch(SCHEMA).unwrap();
        let fixture = Self {
            root,
            db_path,
            configuration: ToolAdministratorConfiguration { directory },
            administrator,
            publisher,
            manifest: serde_json::to_vec(&manifest).unwrap(),
            package,
        };
        fixture.write_trust(true);
        fixture
    }
    fn write_trust(&self, include_admin: bool) {
        let anchor = |id: &str, key: &Ed25519KeyPair| serde_json::json!({"id":id,"publicKeyBase64":base64::engine::general_purpose::STANDARD.encode(key.public_key().as_ref()),"publicKeySha256":sha(key.public_key().as_ref()),"revoked":false});
        let admins = if include_admin {
            vec![anchor("administrator-1", &self.administrator)]
        } else {
            Vec::new()
        };
        let raw = serde_json::json!({"schemaVersion":1,"administrators":admins,"publishers":[anchor("publisher-1",&self.publisher)]});
        let path = self
            .configuration
            .directory
            .join("tool-administrator/trust.json");
        fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fn approve(&self) -> SignedAdministratorDecision {
        let decision = AdministratorDecision::Approve {
            decision_id: uuid::Uuid::new_v4().to_string(),
            manifest_hash: sha(canonical_manifest(&self.manifest).unwrap().as_bytes()),
            package_digest: format!("sha256:{}", sha(&self.package)),
            platform: "macos-aarch64".into(),
            runtime_adapter: "offline-parser-v1".into(),
            runtime_digest: format!("sha256:{}", sha(b"fixed runtime")),
        };
        self.sign(decision)
    }
    fn sign(&self, decision: AdministratorDecision) -> SignedAdministratorDecision {
        let domain = match decision {
            AdministratorDecision::Approve { .. } => APPROVAL_DOMAIN,
            AdministratorDecision::Revoke { .. } => REVOCATION_DOMAIN,
        };
        let mut bytes = domain.to_vec();
        bytes.extend(serde_json::to_vec(&decision).unwrap());
        SignedAdministratorDecision {
            administrator_id: "administrator-1".into(),
            signature_base64: base64::engine::general_purpose::STANDARD
                .encode(self.administrator.sign(&bytes).as_ref()),
            decision,
        }
    }
    fn id(decision: &SignedAdministratorDecision) -> &str {
        match &decision.decision {
            AdministratorDecision::Approve { decision_id, .. }
            | AdministratorDecision::Revoke { decision_id, .. } => decision_id,
        }
    }
}
fn snapshot(db: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
    let mut result = Vec::new();
    for table in [
        "environment_preparation_lease",
        "environment_preparation_events",
        "sentinel_scans",
        "tool_administrator_decisions",
    ] {
        let mut statement = db
            .prepare(&format!("SELECT rowid,* FROM main.{table} ORDER BY rowid"))
            .unwrap();
        let count = statement.column_count();
        result.extend(
            statement
                .query_map([], |row| {
                    (0..count)
                        .map(|i| row.get(i))
                        .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
                })
                .unwrap()
                .map(Result::unwrap),
        );
    }
    result
}
fn rejection(
    f: &Fixture,
    lease: &crate::db::EnvironmentPreparationLease,
    decision: &SignedAdministratorDecision,
    manifest: &[u8],
    expected: &str,
) {
    let mut db = Connection::open(&f.db_path).unwrap();
    let tx = db.transaction().unwrap();
    let before = snapshot(&tx);
    let error = approve_on(
        &tx,
        lease,
        &f.configuration,
        &serde_json::to_vec(decision).unwrap(),
        manifest,
        &f.package,
    )
    .err()
    .unwrap();
    assert_eq!(error, expected);
    assert_eq!(snapshot(&tx), before);
    tx.rollback().unwrap();
}
#[test]
fn administrator_approval_original_signed_inventory_and_revocation_are_exact() {
    let f = Fixture::new();
    let lease = crate::db::begin_environment_preparation(&f.db_path).unwrap();
    let decision = f.approve();
    let mut db = Connection::open(&f.db_path).unwrap();
    let tx = db.transaction().unwrap();
    let approval = approve_on(
        &tx,
        &lease,
        &f.configuration,
        &serde_json::to_vec(&decision).unwrap(),
        &f.manifest,
        &f.package,
    )
    .unwrap();
    assert_eq!(approval.id(), Fixture::id(&decision));
    assert_eq!(
        approval.manifest().digest,
        format!("sha256:{}", sha(&f.package))
    );
    assert_eq!(approval.runtime_adapter(), "offline-parser-v1");
    assert_eq!(
        approval.runtime_digest(),
        format!("sha256:{}", sha(b"fixed runtime"))
    );
    let hash = approval.envelope_hash().to_string();
    tx.commit().unwrap();
    let before = snapshot(&db);
    assert!(read_original_on(&db, &f.configuration, approval.id())
        .unwrap()
        .is_some());
    assert_eq!(snapshot(&db), before);
    let revoke = f.sign(AdministratorDecision::Revoke {
        decision_id: uuid::Uuid::new_v4().to_string(),
        approval_id: approval.id().into(),
        approval_hash: hash,
        reason: "administrator withdrew runtime".into(),
    });
    let tx = db.transaction().unwrap();
    revoke_on(
        &tx,
        &lease,
        &f.configuration,
        &serde_json::to_vec(&revoke).unwrap(),
    )
    .unwrap();
    tx.commit().unwrap();
    let before = snapshot(&db);
    assert_eq!(
        read_original_on(&db, &f.configuration, approval.id())
            .err()
            .unwrap(),
        "tool_administrator_approval_revoked"
    );
    assert_eq!(snapshot(&db), before);
    assert!(db
        .execute(
            "UPDATE tool_administrator_decisions SET canonical_manifest='changed'",
            []
        )
        .is_err());
    assert!(db
        .execute("DELETE FROM tool_administrator_decisions", [])
        .is_err());
    assert_eq!(snapshot(&db), before);
}
#[test]
fn administrator_approval_unconfigured_missing_anchor_and_wrong_signature_do_not_write() {
    let f = Fixture::new();
    let lease = crate::db::begin_environment_preparation(&f.db_path).unwrap();
    let decision = f.approve();
    fs::remove_file(
        f.configuration
            .directory
            .join("tool-administrator/trust.json"),
    )
    .unwrap();
    rejection(
        &f,
        &lease,
        &decision,
        &f.manifest,
        "tool_administrator_trust_unconfigured",
    );
    f.write_trust(false);
    rejection(
        &f,
        &lease,
        &decision,
        &f.manifest,
        "tool_administrator_configuration_invalid",
    );
    f.write_trust(true);
    let mut wrong = f.approve();
    wrong.signature_base64 = base64::engine::general_purpose::STANDARD.encode([0u8; 64]);
    rejection(
        &f,
        &lease,
        &wrong,
        &f.manifest,
        "tool_administrator_signature_invalid",
    );
}
#[test]
fn administrator_approval_manifest_runtime_and_package_drift_do_not_write() {
    let f = Fixture::new();
    let lease = crate::db::begin_environment_preparation(&f.db_path).unwrap();
    let decision = f.approve();
    let mut manifest: SignedToolManifest = serde_json::from_slice(&f.manifest).unwrap();
    manifest.payload.memory_mib += 1;
    // Re-sign the publisher release: old administrator approval still binds old policy.
    manifest.signature_base64 = base64::engine::general_purpose::STANDARD.encode(
        f.publisher
            .sign(&signed_message(&manifest.payload).unwrap())
            .as_ref(),
    );
    rejection(
        &f,
        &lease,
        &decision,
        &serde_json::to_vec(&manifest).unwrap(),
        "tool_administrator_candidate_binding_changed",
    );
    let mut drift = f.approve();
    if let AdministratorDecision::Approve { runtime_digest, .. } = &mut drift.decision {
        *runtime_digest = format!("sha256:{}", sha(b"changed runtime"));
    }
    rejection(
        &f,
        &lease,
        &drift,
        &f.manifest,
        "tool_administrator_signature_invalid",
    );
    let mut db = Connection::open(&f.db_path).unwrap();
    let tx = db.transaction().unwrap();
    let before = snapshot(&tx);
    assert_eq!(
        approve_on(
            &tx,
            &lease,
            &f.configuration,
            &serde_json::to_vec(&decision).unwrap(),
            &f.manifest,
            b"changed package"
        )
        .err()
        .unwrap(),
        "tool_package_digest_mismatch"
    );
    assert_eq!(snapshot(&tx), before);
}
#[test]
fn administrator_approval_requires_own_held_lease_and_rejects_active_scan() {
    let f = Fixture::new();
    let own = crate::db::begin_environment_preparation(&f.db_path).unwrap();
    let other = Fixture::new();
    let other_lease = crate::db::begin_environment_preparation(&other.db_path).unwrap();
    let decision = f.approve();
    rejection(
        &f,
        &other_lease,
        &decision,
        &f.manifest,
        "tool_supply_preparation_owner_mismatch",
    );
    let db = Connection::open(&f.db_path).unwrap();
    db.execute(
        "UPDATE environment_preparation_lease SET owner='foreign-owner'",
        [],
    )
    .unwrap();
    rejection(
        &f,
        &own,
        &decision,
        &f.manifest,
        "tool_supply_preparation_owner_mismatch",
    );
    let f = Fixture::new();
    let own = crate::db::begin_environment_preparation(&f.db_path).unwrap();
    let decision = f.approve();
    let db = Connection::open(&f.db_path).unwrap();
    db.execute(
        "INSERT INTO sentinel_scans(id,status) VALUES('fixture-active','scanning')",
        [],
    )
    .unwrap();
    rejection(
        &f,
        &own,
        &decision,
        &f.manifest,
        "tool_supply_preparation_owner_mismatch",
    );
}

#[test]
fn administrator_approval_temporary_fence_and_scan_shadows_are_rejected() {
    let f = Fixture::new();
    let lease = crate::db::begin_environment_preparation(&f.db_path).unwrap();
    let decision = f.approve();
    let mut db = Connection::open(&f.db_path).unwrap();
    db.execute_batch("CREATE TEMP TABLE environment_preparation_lease AS SELECT * FROM main.environment_preparation_lease; UPDATE main.environment_preparation_lease SET owner='foreign-owner';").unwrap();
    let tx = db.transaction().unwrap();
    let before = snapshot(&tx);
    assert_eq!(
        approve_on(
            &tx,
            &lease,
            &f.configuration,
            &serde_json::to_vec(&decision).unwrap(),
            &f.manifest,
            &f.package
        )
        .err()
        .unwrap(),
        "tool_supply_preparation_owner_mismatch"
    );
    assert_eq!(snapshot(&tx), before);
    tx.rollback().unwrap();
    let f = Fixture::new();
    let lease = crate::db::begin_environment_preparation(&f.db_path).unwrap();
    let decision = f.approve();
    let mut db = Connection::open(&f.db_path).unwrap();
    db.execute_batch("INSERT INTO main.sentinel_scans(id,status) VALUES('active','scanning'); CREATE TEMP TABLE sentinel_scans(id TEXT,status TEXT);").unwrap();
    let tx = db.transaction().unwrap();
    let before = snapshot(&tx);
    assert_eq!(
        approve_on(
            &tx,
            &lease,
            &f.configuration,
            &serde_json::to_vec(&decision).unwrap(),
            &f.manifest,
            &f.package
        )
        .err()
        .unwrap(),
        "tool_supply_preparation_owner_mismatch"
    );
    assert_eq!(snapshot(&tx), before);
}
#[test]
fn administrator_approval_same_named_database_replacement_is_rejected() {
    let f = Fixture::new();
    let lease = crate::db::begin_environment_preparation(&f.db_path).unwrap();
    let decision = f.approve();
    let saved = f.root.join("original-database.db");
    fs::rename(&f.db_path, &saved).unwrap();
    fs::copy(&saved, &f.db_path).unwrap();
    // Even an exact byte copy containing the original owner row is not the
    // database entity held when this private lease was born.
    rejection(
        &f,
        &lease,
        &decision,
        &f.manifest,
        "tool_supply_preparation_owner_mismatch",
    );
}
#[test]
fn administrator_approval_extra_insert_triggers_cannot_write_projects_or_owner() {
    for body in [
        "UPDATE projects SET name='sidewrite';",
        "UPDATE environment_preparation_lease SET owner='sidewrite';",
    ] {
        let f = Fixture::new();
        let lease = crate::db::begin_environment_preparation(&f.db_path).unwrap();
        let decision = f.approve();
        let mut db = Connection::open(&f.db_path).unwrap();
        db.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT); INSERT INTO projects VALUES(1,'original');").unwrap();
        db.execute_batch(&format!("CREATE TRIGGER malicious_approval AFTER INSERT ON tool_administrator_decisions BEGIN {body} END;")).unwrap();
        let tx = db.transaction().unwrap();
        let before = snapshot(&tx);
        assert_eq!(
            approve_on(
                &tx,
                &lease,
                &f.configuration,
                &serde_json::to_vec(&decision).unwrap(),
                &f.manifest,
                &f.package
            )
            .err()
            .unwrap(),
            "tool_administrator_inventory_schema_changed"
        );
        assert_eq!(snapshot(&tx), before);
        assert_eq!(
            tx.query_row("SELECT name FROM main.projects WHERE id=1", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "original"
        );
    }
}

#[test]
fn administrator_owned_writer_approves_revokes_and_preserves_borrowed_authorizer() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let f = Fixture::new();
    let lease = crate::db::begin_environment_preparation(&f.db_path).unwrap();
    let decision = f.approve();
    let db = Connection::open(&f.db_path).unwrap();
    // A restrictive borrowed hook would deny any attempt to insert through
    // this connection. The production writer must neither remove nor use it.
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Insert { .. } | AuthAction::Update { .. } | AuthAction::Delete { .. } => {
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }))
    .unwrap();
    let approval = crate::tool_supply::approval_writer::approve(
        &db,
        &lease,
        &f.configuration,
        &serde_json::to_vec(&decision).unwrap(),
        &f.manifest,
        &f.package,
    )
    .unwrap();
    assert!(db
        .prepare("UPDATE main.environment_preparation_lease SET owner=owner WHERE 0")
        .is_err());
    assert!(read_original_on(&db, &f.configuration, approval.id())
        .unwrap()
        .is_some());
    let revoke = f.sign(AdministratorDecision::Revoke {
        decision_id: uuid::Uuid::new_v4().to_string(),
        approval_id: approval.id().into(),
        approval_hash: approval.envelope_hash().into(),
        reason: "original administrator revoked".into(),
    });
    crate::tool_supply::approval_writer::revoke(
        &db,
        &lease,
        &f.configuration,
        &serde_json::to_vec(&revoke).unwrap(),
    )
    .unwrap();
    assert!(db
        .prepare("UPDATE main.environment_preparation_lease SET owner=owner WHERE 0")
        .is_err());
    assert_eq!(
        read_original_on(&db, &f.configuration, approval.id())
            .err()
            .unwrap(),
        "tool_administrator_approval_revoked"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM main.tool_administrator_decisions",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}
#[test]
fn administrator_owned_writer_extra_trigger_rolls_back_and_outer_transaction_is_rejected() {
    for body in [
        "UPDATE projects SET name='sidewrite';",
        "UPDATE environment_preparation_lease SET owner='sidewrite';",
    ] {
        let f = Fixture::new();
        let lease = crate::db::begin_environment_preparation(&f.db_path).unwrap();
        let decision = f.approve();
        let db = Connection::open(&f.db_path).unwrap();
        db.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT); INSERT INTO projects VALUES(1,'original');").unwrap();
        db.execute_batch(&format!("CREATE TRIGGER malicious_writer AFTER INSERT ON tool_administrator_decisions BEGIN {body} END;")).unwrap();
        let before = snapshot(&db);
        assert_eq!(
            crate::tool_supply::approval_writer::approve(
                &db,
                &lease,
                &f.configuration,
                &serde_json::to_vec(&decision).unwrap(),
                &f.manifest,
                &f.package
            )
            .err()
            .unwrap(),
            "tool_administrator_inventory_schema_changed"
        );
        assert_eq!(snapshot(&db), before);
        assert_eq!(
            db.query_row("SELECT name FROM main.projects WHERE id=1", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "original"
        );
        assert!(db.is_autocommit());
    }
    let f = Fixture::new();
    let lease = crate::db::begin_environment_preparation(&f.db_path).unwrap();
    let decision = f.approve();
    let mut db = Connection::open(&f.db_path).unwrap();
    let tx = db.transaction().unwrap();
    let before = snapshot(&tx);
    assert_eq!(
        crate::tool_supply::approval_writer::approve(
            &tx,
            &lease,
            &f.configuration,
            &serde_json::to_vec(&decision).unwrap(),
            &f.manifest,
            &f.package
        )
        .err()
        .unwrap(),
        "tool_administrator_outer_transaction_forbidden"
    );
    let arbitrary_revoke = f.sign(AdministratorDecision::Revoke {
        decision_id: uuid::Uuid::new_v4().to_string(),
        approval_id: Fixture::id(&decision).into(),
        approval_hash: "0".repeat(64),
        reason: "not issued".into(),
    });
    assert_eq!(
        crate::tool_supply::approval_writer::revoke(
            &tx,
            &lease,
            &f.configuration,
            &serde_json::to_vec(&arbitrary_revoke).unwrap()
        )
        .err()
        .unwrap(),
        "tool_administrator_outer_transaction_forbidden"
    );
    assert_eq!(snapshot(&tx), before);
}

#[test]
fn administrator_owned_writer_rejects_moved_original_connection_even_with_new_lease() {
    let f = Fixture::new();
    let old = Connection::open(&f.db_path).unwrap();
    old.query_row(
        "SELECT count(*) FROM tool_administrator_decisions",
        [],
        |r| r.get::<_, i64>(0),
    )
    .unwrap();
    let saved = f.root.join("original-opened.db");
    fs::rename(&f.db_path, &saved).unwrap();
    fs::copy(&saved, &f.db_path).unwrap();
    let lease = crate::db::begin_environment_preparation(&f.db_path).unwrap();
    let decision = f.approve();
    let fresh = Connection::open(&f.db_path).unwrap();
    let before = snapshot(&fresh);
    assert_eq!(
        crate::tool_supply::approval_writer::approve(
            &old,
            &lease,
            &f.configuration,
            &serde_json::to_vec(&decision).unwrap(),
            &f.manifest,
            &f.package
        )
        .err()
        .unwrap(),
        "tool_administrator_original_database_changed"
    );
    assert_eq!(snapshot(&fresh), before);
}
#[test]
fn administrator_original_revocation_exact_reply_loss_replay_and_conflict_are_pure() {
    let f = Fixture::new();
    let lease = crate::db::begin_environment_preparation(&f.db_path).unwrap();
    let decision = f.approve();
    let db = Connection::open(&f.db_path).unwrap();
    let approval = crate::tool_supply::approval_writer::approve(
        &db,
        &lease,
        &f.configuration,
        &serde_json::to_vec(&decision).unwrap(),
        &f.manifest,
        &f.package,
    )
    .unwrap();
    let revoke = f.sign(AdministratorDecision::Revoke {
        decision_id: uuid::Uuid::new_v4().to_string(),
        approval_id: approval.id().into(),
        approval_hash: approval.envelope_hash().into(),
        reason: "original revoke".into(),
    });
    let raw = serde_json::to_vec(&revoke).unwrap();
    assert!(read_original_revocation_on(&db, &f.configuration, &raw)
        .unwrap()
        .is_none());
    crate::tool_supply::approval_writer::revoke(&db, &lease, &f.configuration, &raw).unwrap();
    let before = snapshot(&db);
    for _ in 0..2 {
        assert_eq!(
            read_original_revocation_on(&db, &f.configuration, &raw)
                .unwrap()
                .unwrap()
                .decision_id(),
            Fixture::id(&revoke)
        );
        assert_eq!(snapshot(&db), before);
    }
    let conflicting = f.sign(AdministratorDecision::Revoke {
        decision_id: uuid::Uuid::new_v4().to_string(),
        approval_id: approval.id().into(),
        approval_hash: approval.envelope_hash().into(),
        reason: "new signed intent".into(),
    });
    assert_eq!(
        read_original_revocation_on(
            &db,
            &f.configuration,
            &serde_json::to_vec(&conflicting).unwrap()
        )
        .err()
        .unwrap(),
        "tool_administrator_revocation_request_conflict"
    );
    assert_eq!(snapshot(&db), before);
    let mut wrong = revoke;
    wrong.signature_base64 = base64::engine::general_purpose::STANDARD.encode([0; 64]);
    assert_eq!(
        read_original_revocation_on(&db, &f.configuration, &serde_json::to_vec(&wrong).unwrap())
            .err()
            .unwrap(),
        "tool_administrator_signature_invalid"
    );
    assert_eq!(snapshot(&db), before);
}
