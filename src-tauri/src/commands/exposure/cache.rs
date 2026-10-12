// §Q6 — the analysis cache. A key binds project, frozen scope version, privacy class, content
// hash, rule version and analysis kind, so identical bytes under the same contract are
// analysed once and any move in any of those dimensions misses instead of reusing.

use super::state::EXPOSURE_RULE_VERSION;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheKey {
    pub project_id: i64,
    pub scope_version: i64,
    pub privacy_class: String,
    pub content_hash: String,
    pub rule_version: String,
    pub kind: String,
    /// §E3 the same bytes under another source, another URL or another configuration version
    /// are different questions, so they are different keys.
    pub source_key: String,
    pub canonical_url: String,
    pub settings_version: i64,
}

impl CacheKey {
    pub fn new(
        project_id: i64,
        scope_version: i64,
        privacy_class: &str,
        content_hash: &str,
        kind: &str,
    ) -> Self {
        Self {
            project_id,
            scope_version,
            privacy_class: privacy_class.to_ascii_lowercase(),
            content_hash: content_hash.to_string(),
            rule_version: EXPOSURE_RULE_VERSION.to_string(),
            kind: kind.to_string(),
            source_key: String::new(),
            canonical_url: String::new(),
            settings_version: 0,
        }
    }

    /// Binds the identity dimensions the analysis actually depended on. Leaving them unset is
    /// only possible for a legacy caller; a production write always sets them.
    pub fn with_identity(
        mut self,
        source_key: &str,
        canonical_url: &str,
        settings_version: i64,
    ) -> Self {
        self.source_key = source_key.to_string();
        self.canonical_url = canonical_url.to_string();
        self.settings_version = settings_version;
        self
    }

    /// A key is a digest, not a concatenation that could be re-split by a crafted hash.
    pub fn as_text(&self) -> String {
        let joined = [
            &self.project_id.to_string()[..],
            &self.scope_version.to_string(),
            &self.privacy_class,
            &self.content_hash,
            &self.rule_version,
            &self.kind,
            &self.source_key,
            &self.canonical_url,
            &self.settings_version.to_string(),
        ]
        .join("\u{1}");
        format!("{:x}", Sha256::digest(joined.as_bytes()))
    }
}

#[derive(Clone, Debug)]
pub struct CacheHit {
    pub key: String,
    pub result: String,
    pub hits: i64,
    pub stored_at: String,
    pub expires_at: String,
}

/// Reads a cached result and counts the reuse. A miss is a normal answer, not an error.
pub fn lookup(connection: &Connection, key: &CacheKey) -> Result<Option<CacheHit>, String> {
    lookup_guarded(connection, key, || true)
}

/// Reads a cached result only while the caller can still prove the question is the same one:
/// `authorized` is the current authorization check. A revoked or expired scope gets a miss
/// and the reuse counter does not move, so a cache row can never stand in for permission to
/// fetch or to conclude.
pub fn lookup_guarded(
    connection: &Connection,
    key: &CacheKey,
    authorized: impl FnOnce() -> bool,
) -> Result<Option<CacheHit>, String> {
    if !authorized() {
        return Ok(None);
    }
    let text = key.as_text();
    let row: Option<(String, String, i64, String, String)> = connection
        .query_row(
            "SELECT cache_key,result_json,hits,updated_at,expires_at FROM exposure_analysis_cache WHERE cache_key=?1",
            [text.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((cache_key, result, hits, stored_at, expires_at)) = row else {
        return Ok(None);
    };
    if !expires_at.trim().is_empty() && expires_at.as_str() <= super::state::utc_now().as_str() {
        // Expired: the row stays as history, but it answers nothing and reuses nothing.
        return Ok(None);
    }
    connection
        .execute(
            "UPDATE exposure_analysis_cache SET hits=hits+1 WHERE cache_key=?1",
            [cache_key.as_str()],
        )
        .map_err(|error| error.to_string())?;
    Ok(Some(CacheHit {
        key: cache_key,
        result,
        hits: hits + 1,
        stored_at,
        expires_at,
    }))
}

pub fn store(connection: &Connection, key: &CacheKey, result_json: &str) -> Result<(), String> {
    store_expiring(connection, key, result_json, "")
}

/// `expires_at` is the deadline the entry stops answering by; empty means the entry carries
/// no self-imposed deadline and only a key dimension (scope, rules, configuration) can miss.
pub fn store_expiring(
    connection: &Connection,
    key: &CacheKey,
    result_json: &str,
    expires_at: &str,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO exposure_analysis_cache(cache_key,project_id,scope_version,privacy_class,content_hash,rule_version,kind,source_key,canonical_url,settings_version,result_json,hits,expires_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,0,?12)
             ON CONFLICT(cache_key) DO UPDATE SET result_json=excluded.result_json,expires_at=excluded.expires_at,updated_at=datetime('now','localtime')",
            params![
                key.as_text(),
                key.project_id,
                key.scope_version,
                key.privacy_class,
                key.content_hash,
                key.rule_version,
                key.kind,
                key.source_key,
                key.canonical_url,
                key.settings_version,
                result_json,
                expires_at
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Content changed: only the entries for that hash become unusable, and the caller re-analyses
/// the affected objects instead of the whole workspace.
pub fn invalidate_content(
    connection: &Connection,
    project_id: i64,
    content_hash: &str,
) -> Result<i64, String> {
    let removed = connection
        .execute(
            "DELETE FROM exposure_analysis_cache WHERE project_id=?1 AND content_hash=?2",
            params![project_id, content_hash],
        )
        .map_err(|error| error.to_string())?;
    Ok(removed as i64)
}

/// A rule-pack change invalidates every entry produced under the previous rules.
pub fn invalidate_before_rule(
    connection: &Connection,
    project_id: i64,
    rule_version: &str,
) -> Result<i64, String> {
    let removed = connection
        .execute(
            "DELETE FROM exposure_analysis_cache WHERE project_id=?1 AND rule_version<>?2",
            params![project_id, rule_version],
        )
        .map_err(|error| error.to_string())?;
    Ok(removed as i64)
}

pub fn entry_count(connection: &Connection, project_id: i64) -> Result<i64, String> {
    connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_analysis_cache WHERE project_id=?1",
            [project_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

/// Privacy guard: evidence from a non-public class can never answer a public lookup, because
/// the privacy class is part of the key.
pub fn crosses_privacy_class(public: &CacheKey, other: &CacheKey) -> bool {
    public.privacy_class != other.privacy_class
}
