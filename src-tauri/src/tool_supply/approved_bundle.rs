//! Explicit new-adapter parsing of current signed, held package bytes. This is
//! not deployment, worker authority, a sandbox, or an execution receipt.
use super::{
    approval::{self, ToolAdministratorConfiguration},
    approved_package_source::ApprovedPackageSource,
    package_format::{parse_flat_bundle_v1, VerifiedFlatBundle},
    ToolCapabilityManifest,
};
use rusqlite::Connection;
use std::io::Cursor;

pub(crate) const ADAPTER: &str = "offline-parser-bundle-v1";
pub(crate) struct ApprovedFlatBundle {
    source: ApprovedPackageSource,
    bytes: Vec<u8>,
    parsed: VerifiedFlatBundle,
    approval_hash: String,
    runtime_digest: String,
    runtime_adapter: String,
}
impl ApprovedFlatBundle {
    pub(crate) fn manifest(&self) -> &ToolCapabilityManifest {
        self.source.manifest()
    }
    pub(crate) fn metadata(&self) -> &VerifiedFlatBundle {
        &self.parsed
    }
    pub(crate) fn original_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn approval_id(&self) -> &str {
        self.source.approval_id()
    }
    pub(crate) fn require_on(
        &mut self,
        db: &Connection,
        configuration: &ToolAdministratorConfiguration,
    ) -> Result<(), String> {
        if db.is_autocommit() {
            let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return self.require_on(&tx, configuration);
        }
        self.source.require_on(db, configuration)?;
        let current = approval::read_original_on(db, configuration, self.source.approval_id())?
            .ok_or("approved_bundle_approval_missing")?;
        if current.envelope_hash() != self.approval_hash
            || current.runtime_adapter() != self.runtime_adapter
            || current.runtime_digest() != self.runtime_digest
        {
            return Err("approved_bundle_original_approval_changed".into());
        }
        Ok(())
    }
}

pub(crate) fn open_on(
    db: &Connection,
    configuration: &ToolAdministratorConfiguration,
    approval_id: &str,
) -> Result<ApprovedFlatBundle, String> {
    if db.is_autocommit() {
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return open_on(&tx, configuration, approval_id);
    }
    let original = approval::read_original_on(db, configuration, approval_id)?
        .ok_or("approved_bundle_approval_missing")?;
    // Never reinterpret old opaque approval, even if its body has the new magic.
    if original.runtime_adapter() != ADAPTER
        && original.runtime_adapter() != super::container_parser::ADAPTER
    {
        return Err("approved_bundle_explicit_adapter_required".into());
    }
    let mut source = ApprovedPackageSource::open_on(db, configuration, approval_id)?;
    let bytes = source.read_verified_bytes_on(db, configuration)?;
    let parsed = parse_flat_bundle_v1(&mut Cursor::new(&bytes), &original.manifest().entrypoint)?;
    // The parser hash is over exact encoded bytes, not only individual entries.
    let encoded_hash = format!(
        "sha256:{}",
        parsed
            .sha256()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    if encoded_hash != original.manifest().digest
        || parsed.encoded_bytes() != source.package_size()
        || source.manifest() != original.manifest()
    {
        return Err("approved_bundle_original_bytes_changed".into());
    }
    source.require_on(db, configuration)?;
    let mut bundle = ApprovedFlatBundle {
        source,
        bytes,
        parsed,
        approval_hash: original.envelope_hash().into(),
        runtime_digest: original.runtime_digest().into(),
        runtime_adapter: original.runtime_adapter().into(),
    };
    bundle.require_on(db, configuration)?;
    Ok(bundle)
}

#[cfg(all(test, target_os = "macos"))]
#[path = "approved_bundle/tests.rs"]
pub(crate) mod tests;
