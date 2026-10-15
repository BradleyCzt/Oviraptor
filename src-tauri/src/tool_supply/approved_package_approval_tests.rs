//! Actual task-external owned writer tests; no manually fabricated lease.
use super::*;
use aws_lc_rs::signature::Ed25519KeyPair;
use rusqlite::Connection;
use serde_json::{json, Value};
struct Fixture {
    root: PathBuf,
    path: PathBuf,
    _supply: frozen_plan::tests::TestSupplyFixture,
    config: approval::ToolAdministratorConfiguration,
    decision: Vec<u8>,
    manifest: Vec<u8>,
    package: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("oviraptor-owned-package-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let path = crate::db::initialize(&root).unwrap();
        let supply = frozen_plan::tests::TestSupplyFixture::approve_on(&path).unwrap();
        let db = Connection::open(&path).unwrap();
        let (d,m):(String,String)=db.query_row("SELECT canonical_envelope,canonical_manifest FROM tool_administrator_decisions WHERE decision_id=?1",[supply.approval_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let config = approval::configuration_for_test(
            supply
                .configuration()
                .directory_for_test()
                .canonicalize()
                .unwrap(),
        );
        Self {
            root,
            path,
            _supply: supply,
            config,
            decision: d.into_bytes(),
            manifest: m.into_bytes(),
            package: b"fixed offline parsing package".to_vec(),
        }
    }
    // A NEW decision is signed by the actual fixture administrator key, never
    // replacing an existing signed id or manufacturing a persisted proof.
    fn new_signed_decision(&self) -> Vec<u8> {
        let mut envelope: Value = serde_json::from_slice(&self.decision).unwrap();
        let d = &mut envelope["decision"];
        d["decision_id"] = json!(uuid::Uuid::new_v4().to_string());
        let text=format!("{{\"kind\":\"approve\",\"decision_id\":{},\"manifest_hash\":{},\"package_digest\":{},\"platform\":{},\"runtime_adapter\":{},\"runtime_digest\":{}}}",d["decision_id"],d["manifest_hash"],d["package_digest"],d["platform"],d["runtime_adapter"],d["runtime_digest"]);
        let key = Ed25519KeyPair::from_seed_unchecked(&[61; 32]).unwrap();
        let mut bytes = b"OVIRAPTOR-TOOL-ADMINISTRATOR-APPROVAL-V1\0".to_vec();
        bytes.extend(text.as_bytes());
        envelope["signatureBase64"] =
            json!(base64::engine::general_purpose::STANDARD.encode(key.sign(&bytes).as_ref()));
        serde_json::to_vec(&envelope).unwrap()
    }
    #[cfg(target_os = "macos")]
    fn owned_approve(&self, decision: &[u8]) -> Result<String, String> {
        let db = Connection::open(&self.path).map_err(|e| e.to_string())?;
        let preparation = crate::db::begin_environment_preparation(&self.path)?;
        let result = approval_writer::approve_with_package(
            &db,
            &preparation,
            &self.config,
            decision,
            &self.manifest,
            &self.package,
        )
        .map(|v| v.id().to_owned());
        preparation.complete()?;
        result
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
#[cfg(target_os = "macos")]
fn approved_package_owned_writer_actual_new_decision_and_source_readonly() {
    let f = Fixture::new();
    let decision = f.new_signed_decision();
    let declared: Value = serde_json::from_slice(&decision).unwrap();
    let approval_id = f.owned_approve(&decision).unwrap();
    assert_eq!(approval_id, declared["decision"]["decision_id"]);
    let db = Connection::open(&f.path).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM tool_administrator_decisions",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
    let before = db.total_changes();
    let mut source =
        approved_package_source::ApprovedPackageSource::open_on(&db, &f.config, &approval_id)
            .unwrap();
    source.require_on(&db, &f.config).unwrap();
    assert_eq!(
        source.read_verified_bytes_on(&db, &f.config).unwrap(),
        f.package
    );
    assert_eq!(db.total_changes(), before);
    let original: String = db
        .query_row(
            "SELECT canonical_envelope FROM tool_administrator_decisions WHERE decision_id=?1",
            [&approval_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(f.owned_approve(&decision).unwrap(), approval_id);
    let after: String = db
        .query_row(
            "SELECT canonical_envelope FROM tool_administrator_decisions WHERE decision_id=?1",
            [&approval_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(original, after);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM tool_administrator_decisions",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        count
    );
}
#[test]
#[cfg(target_os = "macos")]
fn approved_package_owned_writer_actual_original_missing_cache_reuses_immutable_decision() {
    let f = Fixture::new();
    assert!(!f.config.original_package_cache_root().exists());
    let db = Connection::open(&f.path).unwrap();
    let rows:Vec<(String,String,String)>=db.prepare("SELECT decision_id,canonical_envelope,canonical_manifest FROM tool_administrator_decisions ORDER BY decision_id").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    let approved = f.owned_approve(&f.decision).unwrap();
    assert!(
        approved_package_source::ApprovedPackageSource::open_on(&db, &f.config, &approved).is_ok()
    );
    let after:Vec<(String,String,String)>=db.prepare("SELECT decision_id,canonical_envelope,canonical_manifest FROM tool_administrator_decisions ORDER BY decision_id").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(rows, after);
}

#[cfg(all(unix, not(target_os = "macos")))]
include!("approved_package_approval_platform_tests.rs");
