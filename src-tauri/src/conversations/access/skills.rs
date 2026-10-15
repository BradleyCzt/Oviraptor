use super::{files, store};
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};

pub(super) fn fingerprint(db: &Connection, id: i64) -> Result<String, String> {
    let row: Option<(String, String, String, bool)> = db
        .query_row(
            "SELECT name,description,instructions,enabled FROM agent_skills WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(store::err)?;
    let (name, description, instructions, enabled) =
        row.ok_or("conversation_access_skill_unavailable")?;
    if !enabled {
        return Err("conversation_access_skill_unavailable".into());
    }
    let generation:i64=db.query_row("SELECT COALESCE((SELECT generation FROM conversation_skill_generations WHERE skill_id=?1),0)",[id],|r|r.get(0)).map_err(store::err)?;
    let package = crate::skill_packages::package_fingerprint(db, id)?;
    Ok(format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(id, generation, name, description, instructions, package))
                .map_err(|_| "conversation_access_invalid_skills")?
        )
    ))
}
pub(super) fn page(
    text: &str,
    offset: usize,
    limit: usize,
) -> Result<(String, Option<usize>), String> {
    if limit == 0
        || limit > super::MAX_SKILL_BYTES
        || offset > text.len()
        || !text.is_char_boundary(offset)
    {
        return Err("conversation_skill_page_invalid".into());
    }
    // Scan complete bounded instructions before releasing any page.
    files::bounded_text(text.as_bytes(), text.len())?;
    let mut end = offset.saturating_add(limit).min(text.len());
    while end > offset && !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == offset && end < text.len() {
        return Err("conversation_skill_page_invalid".into());
    }
    Ok((text[offset..end].into(), (end < text.len()).then_some(end)))
}

pub(super) fn revision(
    current: &std::collections::BTreeMap<i64, Option<String>>,
) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(current).map_err(|_| "conversation_access_invalid_skills")?
        )
    ))
}
