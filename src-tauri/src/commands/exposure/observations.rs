// §Q5 — the immutable observation log and the current finding it feeds. One round records
// what it saw (append-only, unique per run/source/kind/url); the finding row stays the
// single review queue entry and gains a version whenever content or rules move.

use super::fingerprint::{finding_fingerprint, legacy_fingerprint};
use super::redaction::{
    content_hash, evidence_excerpt, redact_json, redact_text, REDACTION_VERSION,
};
use super::state::{utc_now, EXPOSURE_RULE_VERSION};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value as JsonValue};

/// A single thing a source saw. It is a statement about one round, never a summary that
/// rewrites an earlier round.
#[derive(Clone, Debug)]
pub struct NewObservation {
    pub source_key: String,
    pub kind: String,
    pub url: String,
    pub final_url: String,
    pub status_code: i64,
    pub content_type: String,
    pub body_bytes: i64,
    pub truncated: bool,
    pub boundary: String,
    pub body: Vec<u8>,
    pub headers: JsonValue,
    pub asset_id: Option<i64>,
    pub current_accessible: Option<bool>,
    pub indexed_at: String,
    pub extra: JsonValue,
    /// G3: `GET` for anything read over the network, `IMPORT` for a file a person supplied.
    pub method: String,
}

impl NewObservation {
    pub fn new(source_key: &str, kind: &str, url: &str) -> Self {
        Self {
            source_key: source_key.to_string(),
            kind: kind.to_string(),
            url: url.to_string(),
            final_url: String::new(),
            status_code: 0,
            content_type: String::new(),
            body_bytes: 0,
            truncated: false,
            boundary: String::new(),
            body: Vec::new(),
            headers: JsonValue::Null,
            asset_id: None,
            current_accessible: None,
            indexed_at: String::new(),
            extra: JsonValue::Null,
            method: "GET".to_string(),
        }
    }
}

/// Writes the observation and answers whether it was new to this round. Re-observing the
/// same URL in the same round refreshes the counters but never duplicates the row.
pub fn record(
    connection: &rusqlite::Connection,
    run_id: i64,
    project_id: i64,
    work_item_id: Option<i64>,
    observation: &NewObservation,
) -> Result<(i64, bool), String> {
    if observation.source_key.trim().is_empty()
        || observation.kind.trim().is_empty()
        || observation.url.trim().is_empty()
    {
        return Err("观察缺少来源、类型或 URL".to_string());
    }
    let existing: Option<i64> = connection
        .query_row(
            "SELECT id FROM exposure_observations WHERE run_id=?1 AND source_key=?2 AND kind=?3 AND url=?4",
            params![run_id, observation.source_key, observation.kind, observation.url],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let headers = redact_json(&if observation.headers.is_null() {
        json!({})
    } else {
        observation.headers.clone()
    });
    let evidence = redact_json(&if observation.extra.is_null() {
        json!({})
    } else {
        observation.extra.clone()
    });
    let payload = json!({
        "contentType": observation.content_type,
        "boundary": observation.boundary,
        "truncated": observation.truncated,
        "headers": headers,
        "detail": evidence,
    });
    // §E3: the cluster and cache passes group on the canonical URL and the registered domain.
    // Both are stored at write time so a page or a re-run never re-parses every row.
    let canonical = super::fingerprint::canonical_url(&observation.url);
    // G1-1: the boundary comes from the shared PSL and its basis is stored with it, so a
    // later snapshot or the legacy rule never merges with these rows.
    let (root_domain, root_basis) = super::clusters::domain_boundary_of(&canonical);
    connection
        .execute(
            "INSERT INTO exposure_observations(run_id,project_id,work_item_id,source_key,asset_id,kind,url,final_url,status_code,content_type,content_hash,headers_hash,body_bytes,truncated,rule_version,redaction_version,evidence_json,boundary,current_accessible,indexed_at,queried_at,canonical_url,root_domain,root_basis,method)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)
             ON CONFLICT(run_id,source_key,kind,url) DO UPDATE SET
                work_item_id=COALESCE(excluded.work_item_id,exposure_observations.work_item_id),
                final_url=excluded.final_url,status_code=excluded.status_code,
                content_hash=excluded.content_hash,headers_hash=excluded.headers_hash,
                body_bytes=excluded.body_bytes,truncated=excluded.truncated,
                evidence_json=excluded.evidence_json,rule_version=excluded.rule_version,
                redaction_version=excluded.redaction_version,
                canonical_url=excluded.canonical_url,root_domain=excluded.root_domain,root_basis=excluded.root_basis,
                current_accessible=excluded.current_accessible,indexed_at=excluded.indexed_at,
                queried_at=excluded.queried_at",
            params![
                run_id,
                project_id,
                work_item_id,
                observation.source_key,
                observation.asset_id,
                observation.kind,
                observation.url,
                observation.final_url,
                observation.status_code,
                observation.content_type,
                content_hash(&observation.body),
                content_hash(headers.to_string().as_bytes()),
                observation.body_bytes,
                i64::from(observation.truncated),
                EXPOSURE_RULE_VERSION,
                REDACTION_VERSION,
                payload.to_string(),
                observation.boundary,
                match observation.current_accessible {
                    Some(value) => i64::from(value),
                    None => -1,
                },
                observation.indexed_at,
                utc_now(),
                canonical,
                root_domain,
                root_basis,
                observation.method
            ],
        )
        .map_err(|error| format!("观察写入失败：{error}"))?;
    let id: i64 = connection
        .query_row(
            "SELECT id FROM exposure_observations WHERE run_id=?1 AND source_key=?2 AND kind=?3 AND url=?4",
            params![run_id, observation.source_key, observation.kind, observation.url],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok((id, existing.is_none()))
}

/// The candidate a source wants reviewed. It is a statement about a pattern plus a location,
/// never a verdict.
#[derive(Clone, Debug)]
pub struct FindingDraft {
    pub category: String,
    pub title: String,
    pub url: String,
    pub source_type: String,
    pub evidence: String,
    pub severity: String,
    pub confidence: i64,
    pub asset_id: Option<i64>,
    pub content_hash: String,
}

/// Merges a candidate into the single review queue row for this project, keeps the
/// observation provenance, and records a version when content or rules moved. A moved
/// version means a human conclusion has to be revisited; the earlier status is kept and is
/// reported through `review_needed`, never silently reused as if it still applied.
/// Returns `(finding_id, is_new_row)`.
#[allow(clippy::too_many_arguments)]
pub fn merge_finding(
    connection: &rusqlite::Connection,
    run_id: i64,
    project_id: i64,
    draft: &FindingDraft,
    observation_id: i64,
) -> Result<(i64, bool), String> {
    if draft.title.trim().is_empty() || draft.url.trim().is_empty() {
        return Err("线索缺少标题或来源地址".to_string());
    }
    let fingerprint = finding_fingerprint(&draft.category, &draft.url, &draft.title);
    let legacy = legacy_fingerprint(&draft.category, &draft.url, &draft.title);
    let existing: Option<(i64, String, i64, String, i64)> = connection
        .query_row(
            "SELECT id,fingerprint,confidence,status,COALESCE(asset_id,0) FROM exposure_findings
             WHERE project_id=?1 AND (fingerprint=?2 OR fingerprint=?3)",
            params![project_id, fingerprint, legacy],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let evidence = evidence_excerpt(&redact_text(&draft.evidence), 4000);
    let excerpt_hash = content_hash(evidence.as_bytes());
    let (finding_id, is_new) = match &existing {
        Some((id, current_fingerprint, _, _, _)) => {
            connection
                .execute(
                    "UPDATE exposure_findings SET asset_id=COALESCE(?2,asset_id),evidence_excerpt=?3,
                        severity=?4,confidence=MAX(confidence,?5),last_seen_at=?6,updated_at=?7
                     WHERE id=?1",
                    params![id, draft.asset_id, evidence, draft.severity, draft.confidence, utc_now(), utc_now()],
                )
                .map_err(|error| error.to_string())?;
            if *current_fingerprint != fingerprint {
                // History keeps the old key; the new one is recorded so a later round can find
                // this row without rewriting the historical fingerprint.
                connection
                    .execute(
                        "INSERT INTO exposure_review_events(project_id,finding_id,from_status,to_status,note,actor,created_at)
                         VALUES(?1,?2,'','','fingerprint_v2:'+?3,'system',?4)",
                        params![project_id, id, fingerprint, utc_now()],
                    )
                    .map_err(|error| error.to_string())?;
            }
            (*id, false)
        }
        None => {
            connection
                .execute(
                    "INSERT INTO exposure_findings(project_id,asset_id,category,title,source_type,source_url,evidence_excerpt,severity,confidence,status,fingerprint,first_seen_at,last_seen_at,updated_at)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'new',?10,?11,?11,?11)",
                    params![
                        project_id,
                        draft.asset_id,
                        draft.category,
                        draft.title,
                        draft.source_type,
                        draft.url,
                        evidence,
                        draft.severity,
                        draft.confidence,
                        fingerprint,
                        utc_now()
                    ],
                )
                .map_err(|error| format!("线索写入失败：{error}"))?;
            (connection.last_insert_rowid(), true)
        }
    };
    connection
        .execute(
            "INSERT INTO exposure_finding_observations(finding_id,observation_id,run_id,project_id)
             VALUES(?1,?2,?3,?4) ON CONFLICT(finding_id,observation_id) DO NOTHING",
            params![finding_id, observation_id, run_id, project_id],
        )
        .map_err(|error| error.to_string())?;
    version_finding(
        connection,
        finding_id,
        run_id,
        &excerpt_hash,
        &draft.severity,
        &evidence,
    )?;
    Ok((finding_id, is_new))
}

/// Appends a version row and flags that a human conclusion needs revisiting when the stored
/// content moved. The previous manual status is left untouched.
fn version_finding(
    connection: &rusqlite::Connection,
    finding_id: i64,
    run_id: i64,
    content: &str,
    severity: &str,
    evidence: &str,
) -> Result<i64, String> {
    let previous: Option<(i64, String, String, String)> = connection
        .query_row(
            "SELECT revision,content_hash,rule_version,manual_status FROM exposure_finding_versions
             WHERE finding_id=?1 ORDER BY revision DESC LIMIT 1",
            [finding_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((revision, last_content, last_rule, manual_status)) = previous else {
        connection
            .execute(
                "INSERT INTO exposure_finding_versions(finding_id,run_id,revision,content_hash,rule_version,redaction_version,evidence_excerpt,severity,confidence,manual_status,review_needed,created_at)
                 VALUES(?1,?2,1,?3,?4,?5,?6,?7,(SELECT confidence FROM exposure_findings WHERE id=?1),'',0,?8)",
                params![finding_id, run_id, content, EXPOSURE_RULE_VERSION, REDACTION_VERSION, evidence, severity, utc_now()],
            )
            .map_err(|error| error.to_string())?;
        return Ok(1);
    };
    let moved = last_content != content || last_rule != EXPOSURE_RULE_VERSION;
    let next = revision + 1;
    if !moved {
        return Ok(revision);
    }
    connection
        .execute(
            "INSERT INTO exposure_finding_versions(finding_id,run_id,revision,content_hash,rule_version,redaction_version,evidence_excerpt,severity,confidence,manual_status,review_needed,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,(SELECT confidence FROM exposure_findings WHERE id=?1),?9,1,?10)",
            params![finding_id, run_id, next, content, EXPOSURE_RULE_VERSION, REDACTION_VERSION, evidence, severity, manual_status, utc_now()],
        )
        .map_err(|error| error.to_string())?;
    Ok(next)
}

pub fn review_needed(
    connection: &rusqlite::Connection,
    finding_id: i64,
) -> Result<Option<(i64, String)>, String> {
    let row: Option<(i64, String)> = connection
        .query_row(
            "SELECT revision,rule_version FROM exposure_finding_versions
             WHERE finding_id=?1 AND review_needed=1 ORDER BY revision DESC LIMIT 1",
            [finding_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(row)
}

/// A human decision becomes an event, so the review trail survives later merges.
pub fn note_review_event(
    connection: &rusqlite::Connection,
    project_id: i64,
    finding_id: i64,
    from_status: &str,
    to_status: &str,
    note: &str,
) -> Result<(), String> {
    let top = connection
        .query_row(
            "SELECT COALESCE(MAX(revision),1) FROM exposure_finding_versions WHERE finding_id=?1",
            [finding_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(1);
    connection
        .execute(
            "INSERT INTO exposure_review_events(project_id,finding_id,from_status,to_status,note,actor,created_at)
             VALUES(?1,?2,?3,?4,?5,'user',?6)",
            params![project_id, finding_id, from_status, to_status, redact_text(note), utc_now()],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE exposure_finding_versions SET manual_status=?2 WHERE finding_id=?1 AND revision=?3",
            params![finding_id, to_status, top],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Queues a name an external index returned. Idempotent per project/source/kind/key, and it
/// never creates an asset or grants authorization.
pub fn record_pending(
    connection: &rusqlite::Connection,
    project_id: i64,
    run_id: i64,
    pending: &super::orchestrator::PendingDiscovery,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO exposure_pending_discoveries(project_id,run_id,source_key,kind,entity,canonical_key,evidence_url,status)
             VALUES(?1,?2,?3,?4,?5,?6,?7,'unreviewed')
             ON CONFLICT(project_id,source_key,kind,canonical_key) DO UPDATE SET
                run_id=excluded.run_id,entity=excluded.entity,evidence_url=excluded.evidence_url,updated_at=datetime('now','localtime')",
            params![
                project_id,
                run_id,
                pending.source_key,
                pending.kind,
                pending.entity,
                pending.canonical_key,
                pending.evidence_url
            ],
        )
        .map_err(|error| format!("待归并发现写入失败：{error}"))?;
    Ok(())
}

pub fn observations_for_run(
    connection: &rusqlite::Connection,
    run_id: i64,
    limit: i64,
) -> Result<Vec<(String, String, String, i64)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT source_key,kind,url,status_code FROM exposure_observations WHERE run_id=?1 ORDER BY id LIMIT ?2",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![run_id, limit], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|error| error.to_string())?;
    Ok(rows.flatten().collect())
}

pub fn finding_observation_count(
    connection: &rusqlite::Connection,
    finding_id: i64,
) -> Result<i64, String> {
    connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_finding_observations WHERE finding_id=?1",
            [finding_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}
