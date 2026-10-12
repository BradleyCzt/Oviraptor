//! Task-external administrator decisions. Publisher signatures are not approval.
//! This leaf neither extracts a candidate nor grants a role execution permission.
use super::*;
use rusqlite::{Connection, OptionalExtension, Transaction};
use tauri::Manager;

pub(crate) const SCHEMA: &str = include_str!("approval/schema.sql");
const APPROVAL_DOMAIN: &[u8] = b"OVIRAPTOR-TOOL-ADMINISTRATOR-APPROVAL-V1\0";
const REVOCATION_DOMAIN: &[u8] = b"OVIRAPTOR-TOOL-ADMINISTRATOR-REVOCATION-V1\0";
const MAX_TRUST_BYTES: u64 = 64 * 1024;

/// The only production constructor resolves the application's own directory.
/// No Agent tool accepts this type, a path, an anchor vector, or a consent bool.
#[derive(Clone)]
pub(crate) struct ToolAdministratorConfiguration {
    directory: PathBuf,
}
impl ToolAdministratorConfiguration {
    /// Fixed task-external package location. Approval alone does not populate it.
    pub(super) fn original_package_cache_root(&self) -> PathBuf {
        self.directory.join("tool-supply-cache")
    }
    pub(super) fn original_deployed_package_root(&self) -> PathBuf {
        self.directory.join("tool-supply-deployed")
    }

    pub(crate) fn from_app(app: &tauri::AppHandle) -> Result<Self, String> {
        if let Some(root)=crate::startup_profile::acceptance_root()? {
            return Ok(Self{directory:root.join("configuration")});
        }
        Ok(Self {
            directory: app.path().app_config_dir().map_err(|e| e.to_string())?,
        })
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConfiguredAnchor {
    id: String,
    public_key_base64: String,
    public_key_sha256: String,
    revoked: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrustFile {
    schema_version: u32,
    administrators: Vec<ConfiguredAnchor>,
    publishers: Vec<ConfiguredAnchor>,
}
struct AdministratorTrustStore {
    administrators: Vec<ConfiguredAnchor>,
    publishers: Vec<ToolTrustAnchor>,
    digest: String,
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn anchor_key(anchor: &ConfiguredAnchor) -> Result<Vec<u8>, String> {
    let key = base64::engine::general_purpose::STANDARD
        .decode(&anchor.public_key_base64)
        .map_err(|_| "tool_administrator_anchor_invalid")?;
    if !valid_token(&anchor.id) || key.len() != 32 || sha(&key) != anchor.public_key_sha256 {
        return Err("tool_administrator_anchor_invalid".into());
    }
    Ok(key)
}
fn check_private_metadata(metadata: &fs::Metadata, directory: bool) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o022 != 0
            || (!directory && metadata.nlink() != 1)
        {
            return Err("tool_administrator_configuration_permissions".into());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (metadata, directory);
        return Err("unsupported_tool_administrator_permissions".into());
    }
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err("tool_administrator_configuration_invalid".into());
    }
    Ok(())
}
impl AdministratorTrustStore {
    fn load(configuration: &ToolAdministratorConfiguration) -> Result<Self, String> {
        for part in [
            &configuration.directory,
            &configuration.directory.join("tool-administrator"),
        ] {
            let metadata = fs::symlink_metadata(part).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    "tool_administrator_trust_unconfigured".to_string()
                } else {
                    "tool_administrator_configuration_unreadable".to_string()
                }
            })?;
            if metadata.file_type().is_symlink() {
                return Err("tool_administrator_configuration_symlink".into());
            }
            check_private_metadata(&metadata, true)?;
        }
        let path = configuration
            .directory
            .join("tool-administrator/trust.json");
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "tool_administrator_trust_unconfigured".to_string()
            } else {
                "tool_administrator_configuration_unreadable".to_string()
            }
        })?;
        let metadata = file
            .metadata()
            .map_err(|_| "tool_administrator_configuration_unreadable")?;
        check_private_metadata(&metadata, false)?;
        if metadata.len() > MAX_TRUST_BYTES {
            return Err("tool_administrator_configuration_too_large".into());
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_TRUST_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "tool_administrator_configuration_unreadable")?;
        if bytes.len() as u64 > MAX_TRUST_BYTES {
            return Err("tool_administrator_configuration_too_large".into());
        }
        let config: TrustFile = serde_json::from_slice(&bytes)
            .map_err(|_| "tool_administrator_configuration_invalid")?;
        if config.schema_version != 1
            || config.administrators.is_empty()
            || config.administrators.len() > 32
            || config.publishers.is_empty()
            || config.publishers.len() > 64
        {
            return Err("tool_administrator_configuration_invalid".into());
        }
        for anchors in [&config.administrators, &config.publishers] {
            for (i, anchor) in anchors.iter().enumerate() {
                anchor_key(anchor)?;
                if anchors[..i].iter().any(|a| a.id == anchor.id) {
                    return Err("tool_administrator_anchor_duplicate".into());
                }
            }
        }
        let publishers = config
            .publishers
            .iter()
            .map(|a| {
                Ok(ToolTrustAnchor {
                    signer_id: a.id.clone(),
                    public_key: anchor_key(a)?,
                    revoked: a.revoked,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            administrators: config.administrators,
            publishers,
            digest: sha(&bytes),
        })
    }
    fn verify(&self, envelope: &SignedAdministratorDecision) -> Result<(), String> {
        let anchor = self
            .administrators
            .iter()
            .find(|a| a.id == envelope.administrator_id && !a.revoked)
            .ok_or("tool_administrator_not_trusted_or_revoked")?;
        let key = anchor_key(anchor)?;
        let mut message = match envelope.decision {
            AdministratorDecision::Approve { .. } => APPROVAL_DOMAIN,
            AdministratorDecision::Revoke { .. } => REVOCATION_DOMAIN,
        }
        .to_vec();
        message.extend(
            serde_json::to_vec(&envelope.decision)
                .map_err(|_| "tool_administrator_decision_encoding")?,
        );
        let signature = base64::engine::general_purpose::STANDARD
            .decode(&envelope.signature_base64)
            .map_err(|_| "tool_administrator_signature_invalid")?;
        if signature.len() != 64 {
            return Err("tool_administrator_signature_invalid".into());
        }
        UnparsedPublicKey::new(&ED25519, key)
            .verify(&message, &signature)
            .map_err(|_| "tool_administrator_signature_invalid".into())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum AdministratorDecision {
    Approve {
        decision_id: String,
        manifest_hash: String,
        package_digest: String,
        platform: String,
        runtime_adapter: String,
        runtime_digest: String,
    },
    Revoke {
        decision_id: String,
        approval_id: String,
        approval_hash: String,
        reason: String,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignedAdministratorDecision {
    administrator_id: String,
    decision: AdministratorDecision,
    signature_base64: String,
}
/// A verified inventory entry is not a lease and does not permit execution.
pub(crate) struct VerifiedToolApproval {
    id: String,
    manifest: ToolCapabilityManifest,
    runtime_adapter: String,
    runtime_digest: String,
    envelope_hash: String,
}
impl VerifiedToolApproval {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn manifest(&self) -> &ToolCapabilityManifest {
        &self.manifest
    }
    pub(crate) fn runtime_adapter(&self) -> &str {
        &self.runtime_adapter
    }
    pub(crate) fn runtime_digest(&self) -> &str {
        &self.runtime_digest
    }
    pub(crate) fn envelope_hash(&self) -> &str {
        &self.envelope_hash
    }
}
fn uuid_exact(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}
fn digest_exact(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|v| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
fn parse_envelope(raw: &[u8]) -> Result<SignedAdministratorDecision, String> {
    if raw.len() > MAX_MANIFEST_BYTES {
        return Err("tool_administrator_decision_too_large".into());
    }
    let envelope: SignedAdministratorDecision =
        serde_json::from_slice(raw).map_err(|_| "tool_administrator_decision_invalid")?;
    if !valid_token(&envelope.administrator_id) {
        return Err("tool_administrator_decision_invalid".into());
    }
    Ok(envelope)
}
fn canonical_manifest(raw: &[u8]) -> Result<String, String> {
    let signed: SignedToolManifest =
        serde_json::from_slice(raw).map_err(|_| "tool_manifest_invalid_json")?;
    serde_json::to_string(&signed).map_err(|e| e.to_string())
}
fn verify_approval(
    store: &AdministratorTrustStore,
    envelope: &SignedAdministratorDecision,
    manifest_raw: &[u8],
) -> Result<VerifiedToolApproval, String> {
    store.verify(envelope)?;
    let AdministratorDecision::Approve {
        decision_id,
        manifest_hash,
        package_digest,
        platform,
        runtime_adapter,
        runtime_digest,
    } = &envelope.decision
    else {
        return Err("tool_administrator_approval_required".into());
    };
    let canonical = canonical_manifest(manifest_raw)?;
    let manifest = verify_signed_tool_manifest(canonical.as_bytes(), &store.publishers, platform)?;
    if !uuid_exact(decision_id)
        || sha(canonical.as_bytes()) != *manifest_hash
        || manifest.digest != *package_digest
        || !valid_token(runtime_adapter)
        || !digest_exact(runtime_digest)
    {
        return Err("tool_administrator_candidate_binding_changed".into());
    }
    // First approved runtime class is offline. Approval never turns on network.
    if manifest.network_policy != "none"
        || manifest
            .broker_capabilities
            .iter()
            .any(|c| !matches!(c.as_str(), "evidence.read" | "source_analysis"))
    {
        return Err("tool_administrator_runtime_policy_unsupported".into());
    }
    let encoded = serde_json::to_vec(envelope).map_err(|e| e.to_string())?;
    Ok(VerifiedToolApproval {
        id: decision_id.clone(),
        manifest,
        runtime_adapter: runtime_adapter.clone(),
        runtime_digest: runtime_digest.clone(),
        envelope_hash: sha(&encoded),
    })
}

/// Caller must hold the existing task-external preparation lease. This same
/// transaction checks the caller's opaque owner guard and durable fence.
pub(super) fn approve_on(
    tx: &Transaction<'_>,
    preparation: &crate::db::EnvironmentPreparationLease,
    configuration: &ToolAdministratorConfiguration,
    decision: &[u8],
    manifest: &[u8],
    package: &[u8],
) -> Result<VerifiedToolApproval, String> {
    require_preparation(tx, preparation)?;
    let store = AdministratorTrustStore::load(configuration)?;
    let envelope = parse_envelope(decision)?;
    let approval = verify_approval(&store, &envelope, manifest)?;
    verify_staged_tool_blob(
        manifest,
        package,
        &store.publishers,
        &approval.manifest.platform,
    )?;
    if AdministratorTrustStore::load(configuration)?.digest != store.digest {
        return Err("tool_administrator_configuration_changed".into());
    }
    let canonical = serde_json::to_string(&envelope).map_err(|e| e.to_string())?;
    let changed = tx.execute("INSERT INTO main.tool_administrator_decisions(decision_id,kind,approval_id,administrator_id,canonical_envelope,envelope_hash,canonical_manifest) VALUES(?1,'approve',?1,?2,?3,?4,?5)",rusqlite::params![approval.id,envelope.administrator_id,canonical,approval.envelope_hash,canonical_manifest(manifest)?]).map_err(|e|e.to_string())?;
    if changed != 1 {
        return Err("tool_administrator_insert_unconfirmed".into());
    }
    preparation.require_tool_supply_on(tx)?;
    read_original_on(tx, configuration, &approval.id)?
        .ok_or_else(|| "tool_administrator_insert_unconfirmed".into())
}
fn require_schema(db: &Connection) -> Result<(), String> {
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    for name in [
        "tool_administrator_decisions",
        "tool_administrator_one_approval",
        "tool_administrator_one_revocation",
        "tool_administrator_no_update",
        "tool_administrator_no_delete",
        "tool_administrator_no_replace",
    ] {
        let trusted: String = expected
            .query_row(
                "SELECT sql FROM main.sqlite_master WHERE name=?1",
                [name],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let actual: String = db
            .query_row(
                "SELECT sql FROM main.sqlite_master WHERE name=?1",
                [name],
                |r| r.get(0),
            )
            .map_err(|_| "tool_administrator_inventory_schema_changed")?;
        let shadow: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name=?1 OR tbl_name='tool_administrator_decisions')", [name], |r| r.get(0)).map_err(|e| e.to_string())?;
        if shadow || actual != trusted {
            return Err("tool_administrator_inventory_schema_changed".into());
        }
    }
    let extras: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM main.sqlite_master WHERE type='trigger' AND tbl_name='tool_administrator_decisions' AND name NOT IN ('tool_administrator_no_update','tool_administrator_no_delete','tool_administrator_no_replace'))", [], |r|r.get(0)).map_err(|e|e.to_string())?;
    if extras {
        return Err("tool_administrator_inventory_schema_changed".into());
    }
    Ok(())
}
fn require_preparation(
    tx: &Transaction<'_>,
    preparation: &crate::db::EnvironmentPreparationLease,
) -> Result<(), String> {
    require_schema(tx)?;
    preparation.require_tool_supply_on(tx)
}
/// Historical inventory proof; a revocation or untrusted key always denies use.
pub(crate) fn read_original_on(
    db: &Connection,
    configuration: &ToolAdministratorConfiguration,
    id: &str,
) -> Result<Option<VerifiedToolApproval>, String> {
    let Some((approval, revocation)) = read_inventory_on(db, configuration, id)? else {
        return Ok(None);
    };
    if revocation.is_some() {
        return Err("tool_administrator_approval_revoked".into());
    }
    Ok(Some(approval))
}
pub(super) struct VerifiedToolRevocation {
    decision_id: String,
    canonical_envelope: String,
}
impl VerifiedToolRevocation {
    pub(super) fn decision_id(&self) -> &str {
        &self.decision_id
    }
}
fn read_inventory_on(
    db: &Connection,
    configuration: &ToolAdministratorConfiguration,
    id: &str,
) -> Result<Option<(VerifiedToolApproval, Option<VerifiedToolRevocation>)>, String> {
    require_schema(db)?;
    if !uuid_exact(id) {
        return Err("tool_administrator_approval_id_invalid".into());
    }
    let store = AdministratorTrustStore::load(configuration)?;
    let raw:Option<(String,String,String,String,String)>=db.query_row("SELECT canonical_envelope,envelope_hash,canonical_manifest,administrator_id,approval_id FROM main.tool_administrator_decisions WHERE decision_id=?1 AND kind='approve'",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|e|e.to_string())?;
    let Some((raw, hash, manifest, administrator, approval_id)) = raw else {
        return Ok(None);
    };
    let envelope = parse_envelope(raw.as_bytes())?;
    let canonical = serde_json::to_string(&envelope).map_err(|e| e.to_string())?;
    let approval = verify_approval(&store, &envelope, manifest.as_bytes())?;
    if canonical != raw
        || sha(raw.as_bytes()) != hash
        || approval.id != id
        || approval_id != id
        || envelope.administrator_id != administrator
    {
        return Err("tool_administrator_saved_decision_changed".into());
    }
    let revoke:Option<(String,String,String,String,String)>=db.query_row("SELECT canonical_envelope,envelope_hash,decision_id,administrator_id,canonical_manifest FROM main.tool_administrator_decisions WHERE approval_id=?1 AND kind='revoke'",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|e|e.to_string())?;
    let mut verified_revocation = None;
    if let Some((raw, hash, revoke_id, administrator, revoke_manifest)) = revoke {
        let revoke = parse_envelope(raw.as_bytes())?;
        store.verify(&revoke)?;
        let encoded = serde_json::to_string(&revoke).map_err(|e| e.to_string())?;
        let AdministratorDecision::Revoke {
            decision_id,
            approval_id,
            approval_hash,
            reason,
        } = &revoke.decision
        else {
            return Err("tool_administrator_saved_revocation_changed".into());
        };
        if !revoke_manifest.is_empty()
            || raw != encoded
            || sha(raw.as_bytes()) != hash
            || decision_id != &revoke_id
            || !uuid_exact(decision_id)
            || approval_id != id
            || approval_hash != &approval.envelope_hash
            || reason.is_empty()
            || reason.len() > 512
            || administrator != revoke.administrator_id
        {
            return Err("tool_administrator_saved_revocation_changed".into());
        }
        verified_revocation = Some(VerifiedToolRevocation {
            decision_id: revoke_id,
            canonical_envelope: raw,
        });
    }
    if AdministratorTrustStore::load(configuration)?.digest != store.digest {
        return Err("tool_administrator_configuration_changed".into());
    }
    Ok(Some((approval, verified_revocation)))
}
pub(super) fn revoke_on(
    tx: &Transaction<'_>,
    preparation: &crate::db::EnvironmentPreparationLease,
    configuration: &ToolAdministratorConfiguration,
    decision: &[u8],
) -> Result<(), String> {
    require_preparation(tx, preparation)?;
    let store = AdministratorTrustStore::load(configuration)?;
    let envelope = parse_envelope(decision)?;
    store.verify(&envelope)?;
    let AdministratorDecision::Revoke {
        decision_id,
        approval_id,
        approval_hash,
        reason,
    } = &envelope.decision
    else {
        return Err("tool_administrator_revocation_required".into());
    };
    if !uuid_exact(decision_id)
        || decision_id == approval_id
        || reason.is_empty()
        || reason.len() > 512
    {
        return Err("tool_administrator_revocation_invalid".into());
    }
    let original = read_original_on(tx, configuration, approval_id)?
        .ok_or("tool_administrator_approval_missing")?;
    if &original.envelope_hash != approval_hash {
        return Err("tool_administrator_revocation_binding_changed".into());
    }
    let canonical = serde_json::to_string(&envelope).map_err(|e| e.to_string())?;
    let changed=tx.execute("INSERT INTO main.tool_administrator_decisions(decision_id,kind,approval_id,administrator_id,canonical_envelope,envelope_hash,canonical_manifest) VALUES(?1,'revoke',?2,?3,?4,?5,'')",rusqlite::params![decision_id,approval_id,envelope.administrator_id,canonical,sha(canonical.as_bytes())]).map_err(|e|e.to_string())?;
    if changed != 1 {
        return Err("tool_administrator_insert_unconfirmed".into());
    }
    preparation.require_tool_supply_on(tx)?;
    match read_original_on(tx, configuration, approval_id) {
        Err(e) if e == "tool_administrator_approval_revoked" => Ok(()),
        Err(e) => Err(e),
        Ok(_) => Err("tool_administrator_insert_unconfirmed".into()),
    }
}

#[cfg(all(test, unix))]
#[path = "approval/tests.rs"]
mod tests;

/// Pure preflight only: writer re-loads trust and repeats all verification.
/// Publisher verification only; neither cache publication nor administrator approval.
pub(super) fn verify_package_manifest_on(
    configuration: &ToolAdministratorConfiguration,
    manifest: &[u8],
    platform: &str,
) -> Result<(ToolCapabilityManifest, String), String> {
    let store = AdministratorTrustStore::load(configuration)?;
    let verified = verify_signed_tool_manifest(manifest, &store.publishers, platform)?;
    if AdministratorTrustStore::load(configuration)?.digest != store.digest {
        return Err("tool_administrator_configuration_changed".into());
    }
    Ok((verified, store.digest))
}

pub(super) fn preflight_approve(
    configuration: &ToolAdministratorConfiguration,
    decision: &[u8],
    manifest: &[u8],
    package: &[u8],
) -> Result<VerifiedToolApproval, String> {
    let store = AdministratorTrustStore::load(configuration)?;
    let envelope = parse_envelope(decision)?;
    let original = verify_approval(&store, &envelope, manifest)?;
    verify_staged_tool_blob(
        manifest,
        package,
        &store.publishers,
        &original.manifest.platform,
    )?;
    if AdministratorTrustStore::load(configuration)?.digest != store.digest {
        return Err("tool_administrator_configuration_changed".into());
    }
    Ok(original)
}
pub(super) fn preflight_revoke(
    db: &Connection,
    configuration: &ToolAdministratorConfiguration,
    decision: &[u8],
) -> Result<(), String> {
    let store = AdministratorTrustStore::load(configuration)?;
    let envelope = parse_envelope(decision)?;
    store.verify(&envelope)?;
    let AdministratorDecision::Revoke {
        decision_id,
        approval_id,
        approval_hash,
        reason,
    } = &envelope.decision
    else {
        return Err("tool_administrator_revocation_required".into());
    };
    if !uuid_exact(decision_id)
        || decision_id == approval_id
        || reason.is_empty()
        || reason.len() > 512
    {
        return Err("tool_administrator_revocation_invalid".into());
    }
    let original = read_original_on(db, configuration, approval_id)?
        .ok_or("tool_administrator_approval_missing")?;
    if &original.envelope_hash != approval_hash {
        return Err("tool_administrator_revocation_binding_changed".into());
    }
    if AdministratorTrustStore::load(configuration)?.digest != store.digest {
        return Err("tool_administrator_configuration_changed".into());
    }
    Ok(())
}
/// Call only after read_original_on has verified current revocation and trust.
/// Repeated UUIDs must be the same complete signed request, not just a release.
pub(super) fn require_same_original_request_on(
    db: &Connection,
    id: &str,
    decision: &[u8],
    manifest: &[u8],
) -> Result<(), String> {
    require_schema(db)?;
    let envelope = parse_envelope(decision)?;
    let incoming = serde_json::to_string(&envelope).map_err(|e| e.to_string())?;
    let manifest = canonical_manifest(manifest)?;
    let (saved,saved_manifest):(String,String)=db.query_row("SELECT canonical_envelope,canonical_manifest FROM main.tool_administrator_decisions WHERE decision_id=?1 AND kind='approve'",[id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    if saved != incoming || saved_manifest != manifest {
        return Err("tool_administrator_request_conflict".into());
    }
    Ok(())
}

/// Exact signed revocation replay; no generic revoked error is a success proof.
pub(super) fn read_original_revocation_on(
    db: &Connection,
    configuration: &ToolAdministratorConfiguration,
    decision: &[u8],
) -> Result<Option<VerifiedToolRevocation>, String> {
    let store = AdministratorTrustStore::load(configuration)?;
    let incoming = parse_envelope(decision)?;
    store.verify(&incoming)?;
    let AdministratorDecision::Revoke {
        decision_id,
        approval_id,
        approval_hash,
        reason,
    } = &incoming.decision
    else {
        return Err("tool_administrator_revocation_required".into());
    };
    if !uuid_exact(decision_id)
        || decision_id == approval_id
        || reason.is_empty()
        || reason.len() > 512
    {
        return Err("tool_administrator_revocation_invalid".into());
    }
    let Some((approval, revocation)) = read_inventory_on(db, configuration, approval_id)? else {
        return Err("tool_administrator_approval_missing".into());
    };
    if &approval.envelope_hash != approval_hash {
        return Err("tool_administrator_revocation_binding_changed".into());
    }
    let Some(original) = revocation else {
        return Ok(None);
    };
    let encoded = serde_json::to_string(&incoming).map_err(|e| e.to_string())?;
    if original.canonical_envelope != encoded || original.decision_id != *decision_id {
        return Err("tool_administrator_revocation_request_conflict".into());
    }
    if AdministratorTrustStore::load(configuration)?.digest != store.digest {
        return Err("tool_administrator_configuration_changed".into());
    }
    Ok(Some(original))
}

pub(super) fn configuration_digest_on(
    configuration: &ToolAdministratorConfiguration,
) -> Result<String, String> {
    Ok(AdministratorTrustStore::load(configuration)?.digest)
}

#[cfg(test)]
pub(super) fn configuration_for_test(directory: PathBuf) -> ToolAdministratorConfiguration {
    ToolAdministratorConfiguration { directory }
}

#[cfg(test)]
impl ToolAdministratorConfiguration {
    pub(super) fn directory_for_test(&self) -> PathBuf {
        self.directory.clone()
    }
}
