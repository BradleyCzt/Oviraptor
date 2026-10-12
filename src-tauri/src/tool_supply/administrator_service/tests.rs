//! Actual private production service core with new, initially unapproved
//! signed inputs. No App mock, original-row deletion, or synthetic invoice.
use super::*;
use crate::tool_supply::{
    approved_bundle, package_format::MAGIC, signed_message, SignedToolManifest,
    ToolCapabilityManifest,
};
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn bundle() -> Vec<u8> {
    let content = b"actual fixed parser deployment fixture";
    let mut b = MAGIC.to_vec();
    b.extend(1u16.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(10u16.to_le_bytes());
    b.extend(u64::try_from(content.len()).unwrap().to_le_bytes());
    b.push(1);
    b.extend(Sha256::digest(content));
    b.extend(b"bin/parser");
    b.extend(content);
    b
}
struct Unpublished {
    root: PathBuf,
    path: PathBuf,
    config: ToolAdministratorConfiguration,
    decision: Vec<u8>,
    manifest: Vec<u8>,
    package: Vec<u8>,
    id: String,
    payload: ToolCapabilityManifest,
}
impl Drop for Unpublished {
    fn drop(&mut self) {
        fn writable(path: &Path) {
            if let Ok(m) = fs::symlink_metadata(path) {
                if m.is_dir() && !m.file_type().is_symlink() {
                    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
                    if let Ok(entries) = fs::read_dir(path) {
                        for e in entries.flatten() {
                            writable(&e.path());
                        }
                    }
                }
            }
        }
        writable(&self.root);
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Unpublished {
    fn new(package: Vec<u8>) -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("oviraptor-bundle-service-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let path = crate::db::initialize(&root).unwrap();
        let configured = root.join("configuration");
        let trustdir = configured.join("tool-administrator");
        fs::create_dir(&configured).unwrap();
        fs::create_dir(&trustdir).unwrap();
        for p in [&configured, &trustdir] {
            fs::set_permissions(p, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let admin = Ed25519KeyPair::from_seed_unchecked(&[95; 32]).unwrap();
        let publisher = Ed25519KeyPair::from_seed_unchecked(&[96; 32]).unwrap();
        let anchor = |id: &str, key: &Ed25519KeyPair| json!({"id":id,"publicKeyBase64":base64::engine::general_purpose::STANDARD.encode(key.public_key().as_ref()),"publicKeySha256":sha(key.public_key().as_ref()),"revoked":false});
        let trust = json!({"schemaVersion":1,"administrators":[anchor("service-admin",&admin)],"publishers":[anchor("service-publisher",&publisher)]});
        let trustpath = trustdir.join("trust.json");
        fs::write(&trustpath, trust.to_string()).unwrap();
        fs::set_permissions(trustpath, fs::Permissions::from_mode(0o600)).unwrap();
        let config = approval::configuration_for_test(configured);
        let payload = ToolCapabilityManifest {
            schema_version: 1,
            id: "service-fixed-parser".into(),
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
        let manifest = serde_json::to_vec(&SignedToolManifest {
            payload: payload.clone(),
            signer_id: "service-publisher".into(),
            signature_base64: base64::engine::general_purpose::STANDARD
                .encode(publisher.sign(&signed_message(&payload).unwrap()).as_ref()),
        })
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let text=format!("{{\"kind\":\"approve\",\"decision_id\":{},\"manifest_hash\":{},\"package_digest\":{},\"platform\":{},\"runtime_adapter\":{},\"runtime_digest\":{}}}",json!(id),json!(sha(&manifest)),json!(payload.digest),json!(payload.platform),json!(approved_bundle::ADAPTER),json!(format!("sha256:{}",sha(b"fixed future runtime"))));
        let mut domain = b"OVIRAPTOR-TOOL-ADMINISTRATOR-APPROVAL-V1\0".to_vec();
        domain.extend(text.as_bytes());
        let decision=serde_json::to_vec(&json!({"administratorId":"service-admin","decision":serde_json::from_str::<Value>(&text).unwrap(),"signatureBase64":base64::engine::general_purpose::STANDARD.encode(admin.sign(&domain).as_ref())})).unwrap();
        let f = Self {
            root,
            path,
            config,
            decision,
            manifest,
            package,
            id,
            payload,
        };
        assert_eq!(f.count("tool_administrator_decisions"), 0);
        assert!(!f.config.original_package_cache_root().exists());
        f
    }
    fn approve(&self) -> Result<ApprovalInventory, String> {
        approve_in(
            Connection::open(&self.path).unwrap(),
            self.config.clone(),
            &self.decision,
            &self.manifest,
            &self.package,
        )
    }
    fn count(&self, table: &str) -> i64 {
        Connection::open(&self.path)
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM main.\"{table}\""), [], |r| {
                r.get(0)
            })
            .unwrap()
    }
    fn cache(&self) -> PathBuf {
        self.config
            .original_package_cache_root()
            .join(&self.payload.id)
            .join(crate::tool_supply::candidate_cache_key(&self.payload).unwrap())
    }
    fn deployed(&self) -> PathBuf {
        self.config
            .original_deployed_package_root()
            .join(&self.payload.id)
            .join(crate::tool_supply::candidate_cache_key(&self.payload).unwrap())
    }
    fn recover(&self) {
        let owner: String = Connection::open(&self.path)
            .unwrap()
            .query_row(
                "SELECT owner FROM environment_preparation_lease WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        crate::db::recover_environment_preparation(&self.path, &owner, "已确认安装子进程停止")
            .unwrap();
    }
}
fn snapshot(path: &Path) -> Vec<(String, Vec<Vec<String>>)> {
    let db = Connection::open(path).unwrap();
    let names: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut stmt = db
                .prepare(&format!(
                    "SELECT * FROM main.\"{}\"",
                    name.replace('"', "\"\"")
                ))
                .unwrap();
            let count = stmt.column_count();
            let mut rows = stmt.query([]).unwrap();
            let mut result: Vec<Vec<String>> = Vec::new();
            while let Some(r) = rows.next().unwrap() {
                result.push(
                    (0..count)
                        .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                        .collect(),
                );
            }
            result.sort();
            (name, result)
        })
        .collect()
}
#[test]
fn administrator_service_actual_new_bundle_completes_and_exact_repeat_is_whole_table_free() {
    let f = Unpublished::new(bundle());
    let receipt = f.approve().unwrap();
    assert_eq!(receipt.approval_id, f.id);
    assert!(!receipt.execution_allowed);
    assert!(!receipt.preparation_fence_present);
    assert_eq!(f.count("tool_administrator_decisions"), 1);
    assert_eq!(f.count("environment_preparation_lease"), 0);
    let entry = f.deployed().join("bin/parser");
    let before = fs::metadata(&entry).unwrap();
    let cached = fs::metadata(f.cache().join("package.blob")).unwrap();
    let original = snapshot(&f.path);
    let repeated = f.approve().unwrap();
    assert_eq!(repeated.approval_hash, receipt.approval_hash);
    assert_eq!(snapshot(&f.path), original);
    let after = fs::metadata(&entry).unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    let after = fs::metadata(f.cache().join("package.blob")).unwrap();
    assert_eq!((cached.dev(), cached.ino()), (after.dev(), after.ino()));
    // An unrelated live preparation fence is reported and never auto-cleared.
    let lease = crate::db::begin_environment_preparation(&f.path).unwrap();
    let original = snapshot(&f.path);
    assert!(f.approve().unwrap().preparation_fence_present);
    assert_eq!(snapshot(&f.path), original);
    lease.complete().unwrap();
}
#[test]
fn administrator_service_actual_signed_malformed_bundle_saves_original_and_holds_fence() {
    let f = Unpublished::new(b"signed malformed format content".to_vec());
    let error = f.approve().err().unwrap();
    assert_eq!(
        error,
        format!(
            "tool_administrator_receipt_saved_preparation_incomplete:{}:flat_bundle_magic_invalid",
            f.id
        )
    );
    assert_eq!(f.count("tool_administrator_decisions"), 1);
    assert_eq!(f.count("environment_preparation_lease"), 1);
    assert_eq!(fs::read(f.cache().join("package.blob")).unwrap(), f.package);
    assert!(!f.deployed().exists());
    let original = snapshot(&f.path);
    let error = f.approve().err().unwrap();
    assert_eq!(error, "flat_bundle_magic_invalid");
    assert_eq!(snapshot(&f.path), original);
    f.recover();
    let recovered = snapshot(&f.path);
    let error = f.approve().err().unwrap();
    assert_eq!(error, "flat_bundle_magic_invalid");
    assert_eq!(snapshot(&f.path), recovered);
    assert_eq!(f.count("environment_preparation_lease"), 0);
    assert_eq!(f.count("tool_administrator_decisions"), 1);
    assert_eq!(fs::read(f.cache().join("package.blob")).unwrap(), f.package);
}
#[test]
fn administrator_service_actual_deploy_fault_saved_then_explicit_recovery_reuses_original_approval()
{
    let f = Unpublished::new(bundle());
    let root = f.config.original_deployed_package_root();
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let id = root.join(&f.payload.id);
    fs::create_dir(&id).unwrap();
    fs::set_permissions(&id, fs::Permissions::from_mode(0o500)).unwrap();
    let error = f.approve().err().unwrap();
    assert_eq!(error,format!("tool_administrator_receipt_saved_preparation_incomplete:{}:tool_bundle_deploy_stage_create_failed",f.id));
    assert_eq!(f.count("tool_administrator_decisions"), 1);
    assert_eq!(f.count("environment_preparation_lease"), 1);
    assert!(!f.deployed().exists());
    let cache = fs::metadata(f.cache().join("package.blob")).unwrap();
    f.recover();
    fs::set_permissions(&id, fs::Permissions::from_mode(0o700)).unwrap();
    let receipt = f.approve().unwrap();
    assert_eq!(receipt.approval_id, f.id);
    assert_eq!(f.count("tool_administrator_decisions"), 1);
    assert_eq!(f.count("environment_preparation_lease"), 0);
    let after = fs::metadata(f.cache().join("package.blob")).unwrap();
    assert_eq!((cache.dev(), cache.ino()), (after.dev(), after.ino()));
    assert_eq!(
        fs::read(f.deployed().join("bin/parser")).unwrap(),
        b"actual fixed parser deployment fixture"
    );
}
#[test]
fn administrator_service_actual_bad_signature_and_corrupt_existing_tree_write_nothing() {
    let f = Unpublished::new(bundle());
    let mut changed: Value = serde_json::from_slice(&f.decision).unwrap();
    changed["signatureBase64"] = json!(base64::engine::general_purpose::STANDARD.encode([0u8; 64]));
    let original = snapshot(&f.path);
    let error = approve_in(
        Connection::open(&f.path).unwrap(),
        f.config.clone(),
        &serde_json::to_vec(&changed).unwrap(),
        &f.manifest,
        &f.package,
    )
    .err()
    .unwrap();
    assert_eq!(error, "tool_administrator_signature_invalid");
    assert_eq!(snapshot(&f.path), original);
    assert!(!f.config.original_package_cache_root().exists());
    f.approve().unwrap();
    let path = f.deployed();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(path.join("extra"), b"preserve attacker bytes").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).unwrap();
    let original = snapshot(&f.path);
    assert_eq!(
        f.approve().err().unwrap(),
        "tool_bundle_deploy_tree_changed"
    );
    assert_eq!(snapshot(&f.path), original);
    assert_eq!(
        fs::read(path.join("extra")).unwrap(),
        b"preserve attacker bytes"
    );
}
