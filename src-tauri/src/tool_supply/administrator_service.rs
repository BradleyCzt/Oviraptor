//! Task-external service; approval inventory is never execution authority.
#[cfg(all(test, target_os = "macos"))]
#[path = "administrator_service/tests.rs"]
mod tests;
use super::{
    approval::{self, ToolAdministratorConfiguration, VerifiedToolApproval},
    approval_writer,
};
use crate::AppState;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use tauri::AppHandle;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApprovalInventory {
    schema_version: u32,
    approval_id: String,
    tool_id: String,
    version: String,
    platform: String,
    package_digest: String,
    runtime_adapter: String,
    runtime_digest: String,
    approval_hash: String,
    status: &'static str,
    execution_allowed: bool,
    preparation_fence_present: bool,
}
impl ApprovalInventory {
    fn original(approval: &VerifiedToolApproval) -> Self {
        Self {
            schema_version: 1,
            approval_id: approval.id().into(),
            tool_id: approval.manifest().id.clone(),
            version: approval.manifest().version.clone(),
            platform: approval.manifest().platform.clone(),
            package_digest: approval.manifest().digest.clone(),
            runtime_adapter: approval.runtime_adapter().into(),
            runtime_digest: approval.runtime_digest().into(),
            approval_hash: approval.envelope_hash().into(),
            status: "approved_inventory_only",
            execution_allowed: false,
            preparation_fence_present: false,
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RevocationInventory {
    schema_version: u32,
    revocation_id: String,
    status: &'static str,
    execution_allowed: bool,
    preparation_fence_present: bool,
}
fn preparation_fence_present(db: &Connection) -> Result<bool, String> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM main.environment_preparation_lease WHERE singleton=1)", [], |row| row.get(0))
        .map_err(|error| error.to_string())
}
fn existing(state: &AppState) -> Result<Connection, String> {
    Connection::open_with_flags(
        &state.db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|e| format!("tool_administrator_existing_database_open:{e}"))
}
fn bounded(decision: &[u8], manifest: Option<&[u8]>, package: Option<&[u8]>) -> Result<(), String> {
    if decision.len() > super::MAX_MANIFEST_BYTES
        || manifest.is_some_and(|m| m.len() > super::MAX_MANIFEST_BYTES)
        || package.is_some_and(|p| p.len() > super::MAX_OPAQUE_PACKAGE_BYTES)
    {
        return Err("tool_administrator_service_input_too_large".into());
    }
    Ok(())
}
/// Complete signature and candidate preflight happens before acquiring a
/// durable preparation fence. Once writer execution starts, errors retain the
/// fence for explicit recovery; this service cannot assert ambiguous commits.
pub(crate) fn approve(
    app: &AppHandle,
    state: &AppState,
    decision: &[u8],
    manifest: &[u8],
    package: &[u8],
) -> Result<ApprovalInventory, String> {
    bounded(decision, Some(manifest), Some(package))?;
    let configuration = ToolAdministratorConfiguration::from_app(app)?;
    let db = existing(state)?;
    approve_in(db, configuration, decision, manifest, package)
}

fn approve_in(
    db: Connection,
    configuration: ToolAdministratorConfiguration,
    decision: &[u8],
    manifest: &[u8],
    package: &[u8],
) -> Result<ApprovalInventory, String> {
    bounded(decision, Some(manifest), Some(package))?;
    if !db.is_autocommit() {
        return Err("tool_administrator_outer_transaction_forbidden".into());
    }
    let db_path = db.path().filter(|path| !path.is_empty())
        .ok_or("tool_administrator_existing_database_required")?;
    let candidate = approval::preflight_approve(&configuration, decision, manifest, package)?;
    if let Some(original) = approval::read_original_on(&db, &configuration, candidate.id())? {
        if original.envelope_hash() != candidate.envelope_hash() {
            return Err("tool_administrator_request_conflict".into());
        }
        approval::require_same_original_request_on(&db, candidate.id(), decision, manifest)?;
        match super::approved_package_source::ApprovedPackageSource::open_on(
            &db, &configuration, original.id(),
        ) {
            Ok(_) if original.runtime_adapter() != super::approved_bundle::ADAPTER => {
                let mut inventory = ApprovalInventory::original(&original);
                inventory.preparation_fence_present = preparation_fence_present(&db)?;
                return Ok(inventory);
            }
            Ok(_) => {
                if super::approved_bundle_deploy::read_original_on(
                    &db, &configuration, original.id(),
                )?.is_some() {
                    let mut inventory = ApprovalInventory::original(&original);
                    inventory.preparation_fence_present = preparation_fence_present(&db)?;
                    return Ok(inventory);
                }
            }
            Err(error) if error == "approved_package_source_missing" => (),
            Err(error) => return Err(error),
        }
        // Only the exact original signed request may prepare its missing cache.
        // Publication below refuses conflicting existing filesystem entities.
    }
    let preparation = crate::db::begin_environment_preparation(std::path::Path::new(db_path))?;
    let original = approval_writer::approve_with_package(
        &db,
        &preparation,
        &configuration,
        decision,
        manifest,
        package,
    )?;
    if original.runtime_adapter() == super::approved_bundle::ADAPTER {
        super::approved_bundle_deploy::deploy_on(
            &db, &preparation, &configuration, original.id(),
        ).map_err(|error| format!(
            "tool_administrator_receipt_saved_preparation_incomplete:{}:{error}",
            original.id(),
        ))?;
    }
    let result = ApprovalInventory::original(&original);
    preparation.complete().map_err(|e| {
        format!(
            "tool_administrator_receipt_saved_preparation_incomplete:{}:{e}",
            original.id()
        )
    })?;
    Ok(result)
}
pub(crate) fn revoke(
    app: &AppHandle,
    state: &AppState,
    decision: &[u8],
) -> Result<RevocationInventory, String> {
    bounded(decision, None, None)?;
    let configuration = ToolAdministratorConfiguration::from_app(app)?;
    let db = existing(state)?;
    if let Some(original) = approval::read_original_revocation_on(&db, &configuration, decision)? {
        return Ok(RevocationInventory {
            schema_version: 1,
            revocation_id: original.decision_id().into(),
            status: "revoked_inventory_only",
            execution_allowed: false,
            preparation_fence_present: preparation_fence_present(&db)?,
        });
    }
    approval::preflight_revoke(&db, &configuration, decision)?;
    let preparation = crate::db::begin_environment_preparation(&state.db_path)?;
    approval_writer::revoke(&db, &preparation, &configuration, decision)?;
    let receipt = approval::read_original_revocation_on(&db, &configuration, decision)?
        .ok_or("tool_administrator_insert_unconfirmed")?;
    preparation
        .complete()
        .map_err(|e| format!("tool_administrator_revocation_saved_preparation_incomplete:{e}"))?;
    Ok(RevocationInventory {
        schema_version: 1,
        revocation_id: receipt.decision_id().into(),
        status: "revoked_inventory_only",
        execution_allowed: false,
        preparation_fence_present: false,
    })
}
pub(crate) fn read(
    app: &AppHandle,
    state: &AppState,
    approval_id: &str,
) -> Result<Option<ApprovalInventory>, String> {
    if approval_id.len() != 36 {
        return Err("tool_administrator_approval_id_invalid".into());
    }
    let configuration = ToolAdministratorConfiguration::from_app(app)?;
    let db = Connection::open_with_flags(
        &state.db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|e| format!("tool_administrator_existing_database_open:{e}"))?;
    db.pragma_update(None, "query_only", true).map_err(|error| error.to_string())?;
    let snapshot = db.unchecked_transaction().map_err(|error| error.to_string())?;
    let fence = preparation_fence_present(&snapshot)?;
    Ok(approval::read_original_on(&snapshot, &configuration, approval_id)?
        .as_ref().map(|approval| {
            let mut inventory = ApprovalInventory::original(approval);
            inventory.preparation_fence_present = fence;
            inventory
        }))
}
