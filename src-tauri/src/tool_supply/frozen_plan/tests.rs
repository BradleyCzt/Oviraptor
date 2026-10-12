//! Test-only birth plumbing. Actual Fresh proof is supplied exclusively by the
//! store's real INSERT factory after original signed financial birth.
use super::*;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use base64::Engine;
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
};
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) struct TestSupplyFixture {
    directory: PathBuf,
    configuration: ToolAdministratorConfiguration,
    administrator: Ed25519KeyPair,
    approval_id: String,
    approval_hash: String,
}
impl Drop for TestSupplyFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
impl TestSupplyFixture {
    pub(crate) fn approve_on(db_path: &Path) -> Result<Self, String> {
        Self::approve_runtime_on(db_path,"offline-parser-v1",&format!("sha256:{}",hash(b"original runtime")),b"fixed offline parsing package",false)
    }
    pub(crate) fn approve_container_on(db_path: &Path, runtime_digest: &str) -> Result<Self, String> {
        let code=b"def analyze(data):\n namespace={}\n exec(compile(data['program'],'<generated-helper>','exec'),namespace)\n return namespace['analyze'](data['input'])\n";
        let mut package=crate::tool_supply::package_format::MAGIC.to_vec();
        package.extend(1u16.to_le_bytes());package.extend(1u32.to_le_bytes());
        package.extend(10u16.to_le_bytes());package.extend((code.len() as u64).to_le_bytes());
        package.push(1);package.extend(Sha256::digest(code));package.extend(b"bin/parser");package.extend(code);
        Self::approve_runtime_on(db_path,crate::tool_supply::container_parser::ADAPTER,runtime_digest,&package,true)
    }
    fn approve_runtime_on(db_path: &Path, adapter: &str, runtime_digest: &str, package: &[u8], publish: bool) -> Result<Self, String> {
        let directory = std::env::temp_dir().canonicalize().map_err(|e| e.to_string())?.join(format!(
            "oviraptor-frozen-tool-supply-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&directory).map_err(|e| e.to_string())?;
        let configured = directory.join("configuration");
        fs::create_dir(&configured).map_err(|e| e.to_string())?;
        let trusted = configured.join("tool-administrator");
        fs::create_dir(&trusted).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        for p in [&directory, &configured, &trusted] {
            fs::set_permissions(p, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
        }
        let administrator =
            Ed25519KeyPair::from_seed_unchecked(&[61; 32]).map_err(|e| e.to_string())?;
        let publisher =
            Ed25519KeyPair::from_seed_unchecked(&[62; 32]).map_err(|e| e.to_string())?;
        let anchor = |id: &str, key: &Ed25519KeyPair| json!({"id":id,"publicKeyBase64":base64::engine::general_purpose::STANDARD.encode(key.public_key().as_ref()),"publicKeySha256":hash(key.public_key().as_ref()),"revoked":false});
        let trust = json!({"schemaVersion":1,"administrators":[anchor("frozen-administrator",&administrator)],"publishers":[anchor("frozen-publisher",&publisher)]});
        let path = trusted.join("trust.json");
        fs::write(&path, trust.to_string()).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        let configuration = approval::configuration_for_test(configured);
        let payload = crate::tool_supply::ToolCapabilityManifest {
            schema_version: 1,
            id: "offline-parser".into(),
            version: "1.0.0".into(),
            platform: "macos-aarch64".into(),
            source: "fixed-release".into(),
            digest: format!("sha256:{}", hash(package)),
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
        let signature = publisher.sign(&crate::tool_supply::signed_message(&payload)?);
        let manifest = crate::tool_supply::SignedToolManifest {
            payload,
            signer_id: "frozen-publisher".into(),
            signature_base64: base64::engine::general_purpose::STANDARD.encode(signature.as_ref()),
        };
        let manifest = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
        let approval_id = uuid::Uuid::new_v4().to_string();
        let decision = json!({"kind":"approve","decision_id":approval_id,"manifest_hash":hash(&manifest),"package_digest":format!("sha256:{}",hash(package)),"platform":"macos-aarch64","runtime_adapter":adapter,"runtime_digest":runtime_digest});
        let mut signed = b"OVIRAPTOR-TOOL-ADMINISTRATOR-APPROVAL-V1\0".to_vec();
        // Administrator signatures follow the declared enum's field order,
        // not serde_json::Value's lexical key order.
        let decision_text=format!("{{\"kind\":\"approve\",\"decision_id\":{},\"manifest_hash\":{},\"package_digest\":{},\"platform\":{},\"runtime_adapter\":{},\"runtime_digest\":{}}}",decision["decision_id"],decision["manifest_hash"],decision["package_digest"],decision["platform"],decision["runtime_adapter"],decision["runtime_digest"]);
        signed.extend(decision_text.as_bytes());
        let envelope = json!({"administratorId":"frozen-administrator","decision":decision,"signatureBase64":base64::engine::general_purpose::STANDARD.encode(administrator.sign(&signed).as_ref())});
        let db = Connection::open(db_path).map_err(|e| e.to_string())?;
        db.execute_batch(approval::SCHEMA)
            .map_err(|e| e.to_string())?;
        db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        let preparation = crate::db::begin_environment_preparation(db_path)?;
        let approve = if publish {crate::tool_supply::approval_writer::approve_with_package} else {crate::tool_supply::approval_writer::approve};
        let original = approve(
            &db,
            &preparation,
            &configuration,
            &serde_json::to_vec(&envelope).map_err(|e| e.to_string())?,
            &manifest,
            package,
        )?;
        let approval_hash = original.envelope_hash().into();
        preparation.complete()?;
        Ok(Self {
            directory,
            configuration,
            administrator,
            approval_id,
            approval_hash,
        })
    }
    pub(crate) fn configuration(&self) -> &ToolAdministratorConfiguration {
        &self.configuration
    }
    pub(crate) fn approval_id(&self) -> &str {
        &self.approval_id
    }
    pub(crate) fn tool_supply(&self) -> Value {
        json!({"schemaVersion":1,"approvalIds":[self.approval_id]})
    }
    pub(crate) fn install_birth(&self, scan: &str, attempt: i64, target: &str) -> BirthGuard {
        BIRTH.with(|slot| {
            assert!(slot.borrow().is_none());
            *slot.borrow_mut() = Some(Birth {
                scan: scan.into(),
                attempt,
                target: target.into(),
                configuration_directory: self.configuration.directory_for_test(),
                calls: 0,
                observed: None,
            });
        });
        BirthGuard
    }
    pub(crate) fn revoke_on(&self, db_path: &Path) -> Result<(), String> {
        let id = uuid::Uuid::new_v4().to_string();
        let decision_text=format!("{{\"kind\":\"revoke\",\"decision_id\":{},\"approval_id\":{},\"approval_hash\":{},\"reason\":\"frozen fixture revoked\"}}",json!(id),json!(self.approval_id),json!(self.approval_hash));
        let mut signed = b"OVIRAPTOR-TOOL-ADMINISTRATOR-REVOCATION-V1\0".to_vec();
        signed.extend(decision_text.as_bytes());
        let envelope = json!({"administratorId":"frozen-administrator","decision":serde_json::from_str::<Value>(&decision_text).map_err(|e|e.to_string())?,"signatureBase64":base64::engine::general_purpose::STANDARD.encode(self.administrator.sign(&signed).as_ref())});
        let db = Connection::open(db_path).map_err(|e| e.to_string())?;
        let preparation = crate::db::begin_environment_preparation(db_path)?;
        crate::tool_supply::approval_writer::revoke(
            &db,
            &preparation,
            &self.configuration,
            &serde_json::to_vec(&envelope).map_err(|e| e.to_string())?,
        )?;
        preparation.complete()
    }
}
struct Birth {
    scan: String,
    attempt: i64,
    target: String,
    configuration_directory: PathBuf,
    calls: usize,
    observed: Option<Value>,
}
thread_local! {static BIRTH:RefCell<Option<Birth>>=const{RefCell::new(None)};}
pub(crate) struct BirthGuard;
impl BirthGuard {
    pub(crate) fn calls(&self) -> usize {
        BIRTH.with(|s| s.borrow().as_ref().unwrap().calls)
    }
    pub(crate) fn observed_fact(&self) -> Option<Value> {
        BIRTH.with(|s| s.borrow().as_ref().unwrap().observed.clone())
    }
}
impl Drop for BirthGuard {
    fn drop(&mut self) {
        BIRTH.with(|s| *s.borrow_mut() = None);
    }
}
pub(crate) fn capture_original_fresh_root_for_test(
    fresh: &NewlyInsertedNativeRoot<'_, '_>,
    writer: &mut crate::agent_runtime::web_mode::writer::ModeRootWriter<'_, '_>,
) -> Result<(), String> {
    BIRTH.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(state) = slot.as_mut() else {
            return Ok(());
        };
        let (scope, _, _) = scope_on(fresh.transaction(), fresh.id())?;
        if scope["scanId"] != state.scan
            || scope["attemptNumber"] != state.attempt
            || scope["targetUrl"] != state.target
        {
            return Err("tool_supply_test_birth_scope_mismatch".into());
        }
        state.calls += 1;
        let configuration = approval::configuration_for_test(state.configuration_directory.clone());
        state.observed = writer
            .publish_tool_supply(fresh, &configuration)?
            .map(|proof| proof.fact().clone());
        Ok(())
    })
}
