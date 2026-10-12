use super::*;
use crate::tool_supply::{approval_writer, signed_message, SignedToolManifest};
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) struct Fixture {
    root: PathBuf,
    db_path: PathBuf,
    config: ToolAdministratorConfiguration,
    administrator: Ed25519KeyPair,
    id: String,
    hash: String,
    manifest: ToolCapabilityManifest,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn bytes() -> Vec<u8> {
    let mut b = super::super::package_format::MAGIC.to_vec();
    b.extend(1u16.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    let content = b"fixed signed parser bytes, not executed";
    b.extend(10u16.to_le_bytes());
    b.extend(u64::try_from(content.len()).unwrap().to_le_bytes());
    b.push(1);
    b.extend(Sha256::digest(content));
    b.extend(b"bin/parser");
    b.extend(content);
    b
}
impl Fixture {
    pub(crate) fn configuration(&self) -> &ToolAdministratorConfiguration {
        &self.config
    }
    pub(crate) fn database_path(&self) -> &std::path::Path {
        &self.db_path
    }
    pub(crate) fn approval_id(&self) -> &str {
        &self.id
    }
    pub(crate) fn new(adapter: &str, package: Vec<u8>) -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "oviraptor-approved-bundle-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let db_path = crate::db::initialize(&root).unwrap();
        let config_dir = root.join("configuration");
        let trust_dir = config_dir.join("tool-administrator");
        fs::create_dir(&config_dir).unwrap();
        fs::create_dir(&trust_dir).unwrap();
        for path in [&config_dir, &trust_dir] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let administrator = Ed25519KeyPair::from_seed_unchecked(&[91; 32]).unwrap();
        let publisher = Ed25519KeyPair::from_seed_unchecked(&[92; 32]).unwrap();
        let anchor = |id: &str, key: &Ed25519KeyPair| json!({"id":id,"publicKeyBase64":base64::engine::general_purpose::STANDARD.encode(key.public_key().as_ref()),"publicKeySha256":sha(key.public_key().as_ref()),"revoked":false});
        let trust = json!({"schemaVersion":1,"administrators":[anchor("bundle-admin",&administrator)],"publishers":[anchor("bundle-publisher",&publisher)]});
        let path = trust_dir.join("trust.json");
        fs::write(&path, trust.to_string()).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        let config = approval::configuration_for_test(config_dir);
        let manifest = ToolCapabilityManifest {
            schema_version: 1,
            id: "fixed-bundle-parser".into(),
            version: "1.0.0".into(),
            platform: format!("macos-{}", std::env::consts::ARCH),
            source: "fixed-local-release".into(),
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
        };
        let signed = SignedToolManifest {
            payload: manifest.clone(),
            signer_id: "bundle-publisher".into(),
            signature_base64: base64::engine::general_purpose::STANDARD
                .encode(publisher.sign(&signed_message(&manifest).unwrap()).as_ref()),
        };
        let manifest_bytes = serde_json::to_vec(&signed).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let decision_text = format!("{{\"kind\":\"approve\",\"decision_id\":{},\"manifest_hash\":{},\"package_digest\":{},\"platform\":{},\"runtime_adapter\":{},\"runtime_digest\":{}}}",json!(id),json!(sha(&manifest_bytes)),json!(manifest.digest),json!(manifest.platform),json!(adapter),json!(format!("sha256:{}",sha(b"fixed future runtime identity"))));
        let mut domain = b"OVIRAPTOR-TOOL-ADMINISTRATOR-APPROVAL-V1\0".to_vec();
        domain.extend(decision_text.as_bytes());
        let envelope = json!({"administratorId":"bundle-admin","decision":serde_json::from_str::<Value>(&decision_text).unwrap(),"signatureBase64":base64::engine::general_purpose::STANDARD.encode(administrator.sign(&domain).as_ref())});
        let db = Connection::open(&db_path).unwrap();
        let lease = crate::db::begin_environment_preparation(&db_path).unwrap();
        let original = approval_writer::approve_with_package(
            &db,
            &lease,
            &config,
            &serde_json::to_vec(&envelope).unwrap(),
            &manifest_bytes,
            &package,
        )
        .unwrap();
        let hash = original.envelope_hash().into();
        lease.complete().unwrap();
        Self {
            root,
            db_path,
            config,
            administrator,
            id,
            hash,
            manifest,
        }
    }
    fn directory(&self) -> PathBuf {
        self.config
            .original_package_cache_root()
            .join(&self.manifest.id)
            .join(crate::tool_supply::candidate_cache_key(&self.manifest).unwrap())
    }
    pub(crate) fn revoke(&self) {
        let text = format!("{{\"kind\":\"revoke\",\"decision_id\":{},\"approval_id\":{},\"approval_hash\":{},\"reason\":\"original bundle revoked\"}}",json!(uuid::Uuid::new_v4().to_string()),json!(self.id),json!(self.hash));
        let mut signed = b"OVIRAPTOR-TOOL-ADMINISTRATOR-REVOCATION-V1\0".to_vec();
        signed.extend(text.as_bytes());
        let envelope = json!({"administratorId":"bundle-admin","decision":serde_json::from_str::<Value>(&text).unwrap(),"signatureBase64":base64::engine::general_purpose::STANDARD.encode(self.administrator.sign(&signed).as_ref())});
        let db = Connection::open(&self.db_path).unwrap();
        let lease = crate::db::begin_environment_preparation(&self.db_path).unwrap();
        approval_writer::revoke(
            &db,
            &lease,
            &self.config,
            &serde_json::to_vec(&envelope).unwrap(),
        )
        .unwrap();
        lease.complete().unwrap();
    }
}
fn snapshot(db: &Connection) -> Vec<(String, Vec<Vec<String>>)> {
    let names: Vec<String> = db
        .prepare("SELECT name FROM main.sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let sql = format!("SELECT * FROM main.\"{}\"", name.replace('"', "\"\""));
            let mut stmt = db.prepare(&sql).unwrap();
            let count = stmt.column_count();
            let mut rows = stmt.query([]).unwrap();
            let mut values = Vec::new();
            while let Some(row) = rows.next().unwrap() {
                values.push(
                    (0..count)
                        .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                        .collect(),
                );
            }
            values.sort();
            (name, values)
        })
        .collect()
}
#[test]
fn approved_bundle_actual_new_signed_published_bytes_parse_without_writes() {
    let f = Fixture::new(ADAPTER, bytes());
    let db = Connection::open(&f.db_path).unwrap();
    let before = snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let mut parsed = open_on(&tx, &f.config, &f.id).unwrap();
    assert_eq!(parsed.approval_id(), f.id);
    assert_eq!(parsed.original_bytes(), bytes());
    assert_eq!(parsed.metadata().entries()[0].path(), "bin/parser");
    assert_eq!(parsed.manifest(), &f.manifest);
    parsed.require_on(&tx, &f.config).unwrap();
    assert_eq!(snapshot(&tx), before);
    tx.commit().unwrap();
    assert_eq!(snapshot(&db), before);
}
#[test]
fn approved_bundle_actual_old_signed_adapter_with_new_magic_is_not_upgraded() {
    let f = Fixture::new("offline-parser-v1", bytes());
    let db = Connection::open(&f.db_path).unwrap();
    let before = snapshot(&db);
    assert_eq!(
        open_on(&db, &f.config, &f.id).err().unwrap(),
        "approved_bundle_explicit_adapter_required"
    );
    assert_eq!(snapshot(&db), before);
}
#[test]
fn approved_bundle_actual_revocation_invalidates_held_source_without_writes() {
    let f = Fixture::new(ADAPTER, bytes());
    let db = Connection::open(&f.db_path).unwrap();
    let mut held = open_on(&db, &f.config, &f.id).unwrap();
    f.revoke();
    let before = snapshot(&db);
    assert_eq!(
        held.require_on(&db, &f.config).unwrap_err(),
        "tool_administrator_approval_revoked"
    );
    assert_eq!(
        open_on(&db, &f.config, &f.id).err().unwrap(),
        "tool_administrator_approval_revoked"
    );
    assert_eq!(snapshot(&db), before);
}
#[test]
fn approved_bundle_actual_cached_signature_and_signed_malformed_format_are_rejected() {
    let f = Fixture::new(ADAPTER, bytes());
    let db = Connection::open(&f.db_path).unwrap();
    let path = f.directory().join("manifest.json");
    let original = fs::read(&path).unwrap();
    let mut manifest: Value = serde_json::from_slice(&original).unwrap();
    manifest["signatureBase64"] =
        json!(base64::engine::general_purpose::STANDARD.encode([0u8; 64]));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&path, manifest.to_string()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    let before = snapshot(&db);
    assert_eq!(
        open_on(&db, &f.config, &f.id).err().unwrap(),
        "tool_signature_invalid"
    );
    assert_eq!(snapshot(&db), before);
    let malformed = Fixture::new(ADAPTER, b"signed but not flat bundle bytes".to_vec());
    let db = Connection::open(&malformed.db_path).unwrap();
    let before = snapshot(&db);
    assert_eq!(
        open_on(&db, &malformed.config, &malformed.id)
            .err()
            .unwrap(),
        "flat_bundle_magic_invalid"
    );
    assert_eq!(snapshot(&db), before);
}
