//! A private attempt signature over the exact fresh Root's immutable birth.
//! Readers never mint or repair a missing signature.
use super::*;

fn binding(db: &Connection, root: &str) -> Result<String, String> {
    let d = read(db, root)?.ok_or("root_mode_birth_definition_missing")?;
    let (rowid, created, plan_hash): (i64, String, String) = db.query_row(
        "SELECT rowid,created_at,native_plan_text_hash FROM agent_root_mode_definitions WHERE root_run_id=?1",
        [root], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    let (run_rowid, run_created): (i64, String) = db
        .query_row(
            "SELECT rowid,created_at FROM agent_runs WHERE id=?1",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    Ok(
        serde_json::json!({"domain":"oviraptor.root-mode-birth.v1","rootRunId":root,
        "rootRowid":run_rowid,"rootCreatedAt":run_created,"definitionRowid":rowid,
        "definitionCreatedAt":created,"nativePlanTextHash":plan_hash,"definition":d})
        .to_string(),
    )
}

pub(super) fn issue(
    fresh: &store::NewlyInsertedNativeRoot<'_, '_>,
    d: &NewRootModeDeclaration,
) -> Result<(), String> {
    let proof = d
        .authorization
        .as_ref()
        .ok_or("web_mode_private_creation_missing")?;
    if proof.fact() != d.fact() {
        return Err("root_mode_birth_original_mode_changed".into());
    }
    let raw = binding(fresh.transaction(), fresh.id())?;
    let tag = proof.sign_private(raw.as_bytes());
    let n = fresh.transaction().execute(
        "INSERT INTO agent_root_mode_birth_seals(root_run_id,binding_json,signature) VALUES(?1,?2,?3)",
        params![fresh.id(),raw,tag]).map_err(|e|e.to_string())?;
    if n != 1 {
        return Err("root_mode_birth_write_unconfirmed".into());
    }
    verify_birth(fresh.transaction(), fresh.id(), proof)
}

pub(crate) fn verify_birth(
    db: &Connection,
    root: &str,
    proof: &VerifiedMode,
) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("root_mode_birth_snapshot_required".into());
    }
    let d = read(db, root)?.ok_or("root_mode_birth_definition_missing")?;
    if proof.fact() != d.fact() {
        return Err("root_mode_birth_original_mode_changed".into());
    }
    let (raw, tag): (String, Vec<u8>) = db
        .query_row(
            "SELECT binding_json,signature FROM agent_root_mode_birth_seals WHERE root_run_id=?1",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "root_mode_birth_seal_missing")?;
    if raw != binding(db, root)? || tag.len() != 32 {
        return Err("root_mode_birth_binding_changed".into());
    }
    proof.verify_private(raw.as_bytes(), &tag)
}
