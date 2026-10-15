// §Q1 — read-only repository for the exposure surface. Every query is a SELECT: it never
// creates a run, never writes a row and never opens a socket. The filtered total and the
// keyset cursor come from the same predicate as the list, so the page and the summary
// cannot disagree about what "this filter" means.

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::asset_models::{ExposureFinding, ExposureRun, ExposureSourceResult};

use super::contract::{
    decode_id_cursor, encode_id_cursor, escape_like, ExposureFindingPageQuery, ExposurePage,
    ExposurePending, ExposureQueryError, ExposureSortKey, EXPOSURE_MAX_PAGE_SIZE,
    EXPOSURE_MIN_PAGE_SIZE, PASSIVE_ASSET_CAP,
};

const FINDING_RANK: &str =
    "CASE f.severity WHEN 'critical' THEN 4 WHEN 'high' THEN 3 WHEN 'medium' THEN 2 ELSE 1 END";
const FINDING_COLUMNS: &str =
    "f.id,f.project_id,f.asset_id,f.category,f.title,f.source_type,f.source_url,\
 f.evidence_excerpt,f.severity,f.confidence,f.status,f.note,\
 COALESCE(NULLIF(a.domain,''),NULLIF(a.host,''),NULLIF(a.link,''),a.ip,''),\
 COALESCE(o.exposure_eligible,0),f.first_seen_at,f.last_seen_at";
/// The filter is built once and reused by the page query, the total count and the summary
/// so no two read paths can quietly mean different things.
const FINDING_FILTER: &str = "f.project_id=?1 AND (?2='' OR f.status=?2) \
 AND (?3='' OR f.title LIKE ?3 ESCAPE '\\' OR f.source_url LIKE ?3 ESCAPE '\\' OR f.evidence_excerpt LIKE ?3 ESCAPE '\\')";

fn finding_status(query: &ExposureFindingPageQuery) -> String {
    query
        .status
        .map(|value| value.as_str().to_string())
        .unwrap_or_default()
}

fn finding_pattern(query: &ExposureFindingPageQuery) -> String {
    query.search.as_deref().map(escape_like).unwrap_or_default()
}

fn finding_from_row(row: &Row<'_>) -> rusqlite::Result<ExposureFinding> {
    Ok(ExposureFinding {
        id: row.get(0)?,
        project_id: row.get(1)?,
        asset_id: row.get(2)?,
        category: row.get(3)?,
        title: row.get(4)?,
        source_type: row.get(5)?,
        source_url: row.get(6)?,
        evidence_excerpt: row.get(7)?,
        severity: row.get(8)?,
        confidence: row.get(9)?,
        status: row.get(10)?,
        note: row.get(11)?,
        asset_label: row.get(12)?,
        exposure_eligible: row.get::<_, i64>(13)? != 0,
        first_seen_at: row.get(14)?,
        last_seen_at: row.get(15)?,
    })
}

fn disposition_clause(filter: Option<&'static str>) -> String {
    match filter.and_then(super::classify::disposition_filter_values) {
        None => String::new(),
        Some(values) => format!(
            " AND COALESCE((SELECT d.disposition FROM exposure_finding_dispositions d WHERE d.finding_id=f.id),'unclassified') IN ({})",
            values
                .iter()
                .map(|value| format!("'{value}'"))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn page_error(error: rusqlite::Error) -> ExposureQueryError {
    ExposureQueryError::storage(&error.to_string())
}

/// One page of findings plus the cursor for the page after it. A row whose keyset tuple
/// ties on every sort column is still ordered by id, so two pages can never repeat or
/// skip a row.
pub fn page_exposure_findings(
    connection: &Connection,
    query: &ExposureFindingPageQuery,
) -> Result<ExposurePage<ExposureFinding>, ExposureQueryError> {
    // G1-3 the disposition clause is only compiled when a filter is set, so an unfiltered page
    // never depends on the classification tables (older databases stay readable).
    let disposition = disposition_clause(query.disposition);
    let total: i64 = connection
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM exposure_findings f \
                 LEFT JOIN assets a ON a.id=f.asset_id \
                 LEFT JOIN asset_ownership_decisions o ON o.project_id=f.project_id AND o.asset_id=f.asset_id \
                 WHERE {FINDING_FILTER}{disposition}"
            ),
            params![query.project_id, finding_status(query), finding_pattern(query)],
            |row| row.get(0),
        )
        .map_err(page_error)?;
    let sql = format!(
        "SELECT {FINDING_COLUMNS},{FINDING_RANK} \
         FROM exposure_findings f \
         LEFT JOIN assets a ON a.id=f.asset_id \
         LEFT JOIN asset_ownership_decisions o ON o.project_id=f.project_id AND o.asset_id=f.asset_id \
         WHERE {FINDING_FILTER}{disposition} \
         AND ({FINDING_RANK},f.confidence,f.last_seen_at,f.id) < (?4,?5,?6,?7) \
         ORDER BY {FINDING_RANK} DESC,f.confidence DESC,f.last_seen_at DESC,f.id DESC \
         LIMIT ?8"
    );
    let mut statement = connection.prepare(&sql).map_err(page_error)?;
    let rows = statement
        .query_map(
            params![
                query.project_id,
                finding_status(query),
                finding_pattern(query),
                query.after.rank,
                query.after.confidence,
                query.after.last_seen,
                query.after.id,
                query.page_size + 1
            ],
            finding_from_row,
        )
        .map_err(page_error)?;
    let mut items: Vec<ExposureFinding> = rows
        .map(|row| row.map_err(page_error))
        .collect::<Result<_, _>>()?;
    let has_more = items.len() as i64 > query.page_size;
    if has_more {
        items.pop();
    }
    let next_cursor = items
        .last()
        .map(|row| {
            ExposureSortKey {
                rank: rank_of_severity(&row.severity),
                confidence: row.confidence,
                last_seen: row.last_seen_at.clone(),
                id: row.id,
            }
            .encode()
        })
        .unwrap_or_default();
    Ok(ExposurePage {
        items,
        next_cursor: if has_more { next_cursor } else { String::new() },
        has_more,
        total,
        page_size: query.page_size,
    })
}

fn rank_of_severity(severity: &str) -> i64 {
    match severity {
        "critical" => 4,
        "high" => 3,
        "medium" => 2,
        _ => 1,
    }
}

/// Runs are ordered newest first, so the keyset is a single strictly-smaller id.
pub fn page_exposure_runs(
    connection: &Connection,
    project_id: i64,
    page_size: i64,
    cursor: &str,
) -> Result<ExposurePage<ExposureRun>, ExposureQueryError> {
    let after_id = decode_id_cursor(cursor)?;
    if project_id <= 0 || !(EXPOSURE_MIN_PAGE_SIZE..=EXPOSURE_MAX_PAGE_SIZE).contains(&page_size) {
        return Err(ExposureQueryError::invalid_page_size(page_size));
    }
    let total: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_runs WHERE project_id=?1",
            [project_id],
            |row| row.get(0),
        )
        .map_err(page_error)?;
    let mut statement = connection
        .prepare(
            "SELECT id,project_id,status,eligible_assets,scanned_assets,fetched_resources,findings,\
             stage,current_source,error,created_at,started_at,completed_at \
             FROM exposure_runs WHERE project_id=?1 AND id<?2 ORDER BY id DESC LIMIT ?3",
        )
        .map_err(page_error)?;
    let rows = statement
        .query_map(params![project_id, after_id, page_size + 1], |row| {
            Ok(ExposureRun {
                id: row.get(0)?,
                project_id: row.get(1)?,
                status: row.get(2)?,
                eligible_assets: row.get(3)?,
                scanned_assets: row.get(4)?,
                fetched_resources: row.get(5)?,
                findings: row.get(6)?,
                stage: row.get(7)?,
                current_source: row.get(8)?,
                error: row.get(9)?,
                created_at: row.get(10)?,
                started_at: row.get(11)?,
                completed_at: row.get(12)?,
            })
        })
        .map_err(page_error)?;
    let mut items: Vec<ExposureRun> = rows
        .map(|row| row.map_err(page_error))
        .collect::<Result<_, _>>()?;
    let has_more = items.len() as i64 > page_size;
    if has_more {
        items.pop();
    }
    Ok(ExposurePage {
        next_cursor: items
            .last()
            .map(|row| encode_id_cursor(row.id))
            .unwrap_or_default(),
        has_more,
        items,
        total,
        page_size,
    })
}

/// The sources of one run. `expected_project_id` is verified against the row that
/// actually owns the run, because a numeric id is not a scope check (§4.2).
pub fn read_exposure_source_results(
    connection: &Connection,
    run_id: i64,
    expected_project_id: i64,
) -> Result<Vec<ExposureSourceResult>, ExposureQueryError> {
    if !exposure_run_in_project(connection, run_id, expected_project_id)? {
        return Err(ExposureQueryError::outside_project());
    }
    let mut statement = connection
        .prepare(
            "SELECT run_id,source_key,status,item_count,error,completed_at \
             FROM exposure_source_results WHERE run_id=?1 ORDER BY id",
        )
        .map_err(page_error)?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok(ExposureSourceResult {
                run_id: row.get(0)?,
                source_key: row.get(1)?,
                status: row.get(2)?,
                item_count: row.get(3)?,
                error: row.get(4)?,
                completed_at: row.get(5)?,
            })
        })
        .map_err(page_error)?;
    rows.map(|row| row.map_err(page_error)).collect()
}

pub fn exposure_run_in_project(
    connection: &Connection,
    run_id: i64,
    project_id: i64,
) -> Result<bool, ExposureQueryError> {
    if run_id <= 0 || project_id <= 0 {
        return Err(ExposureQueryError::new(
            "exposure_run_id_invalid",
            "缺少有效的任务或工作空间 id",
        ));
    }
    let found: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_runs WHERE id=?1 AND project_id=?2",
            params![run_id, project_id],
            |row| row.get(0),
        )
        .map_err(page_error)?;
    Ok(found > 0)
}

pub fn load_exposure_finding(
    connection: &Connection,
    finding_id: i64,
    expected_project_id: i64,
) -> Result<ExposureFinding, ExposureQueryError> {
    if finding_id <= 0 {
        return Err(ExposureQueryError::new(
            "exposure_finding_id_invalid",
            "缺少有效的线索 id",
        ));
    }
    let mut statement = connection
        .prepare(&format!(
            "SELECT {FINDING_COLUMNS} FROM exposure_findings f \
             LEFT JOIN assets a ON a.id=f.asset_id \
             LEFT JOIN asset_ownership_decisions o ON o.project_id=f.project_id AND o.asset_id=f.asset_id \
             WHERE f.id=?1 AND f.project_id=?2"
        ))
        .map_err(page_error)?;
    let found = statement
        .query_map(params![finding_id, expected_project_id], finding_from_row)
        .map_err(page_error)?
        .next()
        .transpose()
        .map_err(page_error)?;
    found.ok_or_else(ExposureQueryError::outside_project)
}

/// How many assets the *current* collector would take: the ownership-eligible members of
/// an active project, capped the way the collector caps them. Same predicate, two counts,
/// so the page can show the difference instead of hiding it.
pub fn eligible_asset_scope(
    connection: &Connection,
    project_id: i64,
) -> Result<(i64, i64, bool), ExposureQueryError> {
    let project_active: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM projects WHERE id=?1 AND status='active'",
            [project_id],
            |row| row.get(0),
        )
        .map_err(page_error)?;
    let total: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM assets a \
             JOIN asset_ownership_decisions o ON o.asset_id=a.id AND o.project_id=?1 \
             JOIN project_assets pa ON pa.asset_id=a.id AND pa.project_id=?1 \
             WHERE o.exposure_eligible=1 AND pa.is_deleted=0",
            [project_id],
            |row| row.get(0),
        )
        .map_err(page_error)?;
    Ok((total, total.min(PASSIVE_ASSET_CAP), project_active > 0))
}

/// §Q4 — the pending queue, paged with a real total. Read-only: opening this page never
/// promotes a name into an asset and never grants anything.
pub fn page_pending_discoveries(
    connection: &Connection,
    input: &(i64, String, i64, Option<i64>),
) -> Result<ExposurePage<ExposurePending>, ExposureQueryError> {
    let (project_id, status, page_size, after) = input.clone();
    let after_value = after.unwrap_or(i64::MAX);
    let mut items = {
        let mut statement = connection
            .prepare(
                "SELECT id,project_id,run_id,source_key,kind,entity,canonical_key,evidence_url,status,created_at
                 FROM exposure_pending_discoveries
                 WHERE project_id=?1 AND (?2='' OR status=?2) AND id<?3
                 ORDER BY id DESC LIMIT ?4",
            )
            .map_err(page_error)?;
        let rows = statement
            .query_map(
                params![project_id, status, after_value, page_size + 1],
                |row| {
                    Ok(ExposurePending {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        run_id: row.get(2)?,
                        source_key: row.get(3)?,
                        kind: row.get(4)?,
                        entity: row.get(5)?,
                        canonical_key: row.get(6)?,
                        evidence_url: row.get(7)?,
                        status: row.get(8)?,
                        created_at: row.get(9)?,
                    })
                },
            )
            .map_err(page_error)?;
        rows.map(|row| row.map_err(page_error))
            .collect::<Result<Vec<_>, ExposureQueryError>>()?
    };
    let has_more = items.len() as i64 > page_size;
    if has_more {
        items.pop();
    }
    let total: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_pending_discoveries WHERE project_id=?1 AND (?2='' OR status=?2)",
            params![project_id, status],
            |row| row.get(0),
        )
         .map_err(page_error)?;
    let next_cursor = if has_more {
        items
            .last()
            .map(|row| encode_id_cursor(row.id))
            .unwrap_or_default()
    } else {
        String::new()
    };
    Ok(ExposurePage {
        items,
        next_cursor,
        has_more,
        total,
        page_size,
    })
}

/// §Q5 — the evidence package behind one finding: the immutable observations that produced
/// it, the version trail, and whether a human conclusion now needs revisiting. Everything
/// returned here is already redacted at write time; nothing hands back a raw body.
pub fn exposure_evidence(
    connection: &Connection,
    finding_id: i64,
    expected_project_id: i64,
) -> Result<serde_json::Value, ExposureQueryError> {
    let row: Option<(i64, String, String, i64, String)> = connection
        .query_row(
            "SELECT id,category,title,confidence,status FROM exposure_findings WHERE id=?1 AND project_id=?2",
            params![finding_id, expected_project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()
         .map_err(page_error)?;
    let Some((id, category, title, confidence, status)) = row else {
        return Err(ExposureQueryError::new(
            "exposure_scope_mismatch",
            "该线索不属于当前工作空间，或不存在",
        ));
    };
    let mut statement = connection
        .prepare(
            "SELECT o.id,o.run_id,o.source_key,o.kind,o.url,o.final_url,o.status_code,o.content_type,o.content_hash,
                    o.headers_hash,o.body_bytes,o.truncated,o.rule_version,o.redaction_version,o.boundary,
                    o.current_accessible,o.indexed_at,o.queried_at,o.evidence_json
             FROM exposure_observations o
             JOIN exposure_finding_observations f ON f.observation_id=o.id
             WHERE f.finding_id=?1 ORDER BY o.id",
        )
         .map_err(page_error)?;
    let observations = statement
        .query_map([id], |row| {
            let accessible: i64 = row.get(15)?;
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "runId": row.get::<_, i64>(1)?,
                "sourceKey": row.get::<_, String>(2)?,
                "kind": row.get::<_, String>(3)?,
                "url": row.get::<_, String>(4)?,
                "finalUrl": row.get::<_, String>(5)?,
                "statusCode": row.get::<_, i64>(6)?,
                "contentType": row.get::<_, String>(7)?,
                "contentHash": row.get::<_, String>(8)?,
                "headersHash": row.get::<_, String>(9)?,
                "bodyBytes": row.get::<_, i64>(10)?,
                "truncated": row.get::<_, i64>(11)? != 0,
                "ruleVersion": row.get::<_, String>(12)?,
                "redactionVersion": row.get::<_, String>(13)?,
                "boundary": row.get::<_, String>(14)?,
                "currentAccessible": match accessible { 1 => serde_json::json!(true), 0 => serde_json::json!(false), _ => serde_json::Value::Null },
                "indexedAt": row.get::<_, String>(16)?,
                "queriedAt": row.get::<_, String>(17)?,
                "evidence": serde_json::from_str::<serde_json::Value>(&row.get::<_, String>(18)?).unwrap_or_else(|_| serde_json::Value::Object(Default::default())),
            }))
        })
         .map_err(page_error)?
        .filter_map(|row| row.ok())
        .collect::<Vec<_>>();
    let mut versions = connection
        .prepare(
            "SELECT revision,run_id,content_hash,rule_version,redaction_version,severity,manual_status,review_needed,created_at
             FROM exposure_finding_versions WHERE finding_id=?1 ORDER BY revision DESC LIMIT 20",
        )
         .map_err(page_error)?
        .query_map([id], |row| {
            Ok(serde_json::json!({
                "revision": row.get::<_, i64>(0)?,
                "runId": row.get::<_, i64>(1)?,
                "contentHash": row.get::<_, String>(2)?,
                "ruleVersion": row.get::<_, String>(3)?,
                "redactionVersion": row.get::<_, String>(4)?,
                "severity": row.get::<_, String>(5)?,
                "manualStatus": row.get::<_, String>(6)?,
                "reviewNeeded": row.get::<_, i64>(7)? != 0,
                "createdAt": row.get::<_, String>(8)?,
            }))
        })
         .map_err(page_error)?
        .filter_map(|row| row.ok())
        .collect::<Vec<_>>();
    versions.reverse();
    let mut events = connection
        .prepare(
            "SELECT from_status,to_status,note,actor,created_at FROM exposure_review_events
             WHERE finding_id=?1 AND project_id=?2 ORDER BY id DESC LIMIT 50",
        )
        .map_err(page_error)?
        .query_map(params![id, expected_project_id], |row| {
            Ok(serde_json::json!({
                "fromStatus": row.get::<_, String>(0)?,
                "toStatus": row.get::<_, String>(1)?,
                "note": row.get::<_, String>(2)?,
                "actor": row.get::<_, String>(3)?,
                "createdAt": row.get::<_, String>(4)?,
            }))
        })
        .map_err(page_error)?
        .filter_map(|row| row.ok())
        .collect::<Vec<_>>();
    events.reverse();
    let review_needed = super::observations::review_needed(connection, id)
        .ok()
        .flatten()
        .map(|(revision, rule)| serde_json::json!({"revision": revision, "ruleVersion": rule}));
    // G1-3 classification trail; a read failure is reported, never shown as "unclassified".
    let (disposition, classifications, classification_error) = match (
        super::classify::disposition_of(connection, expected_project_id, id),
        super::classify::classifications_of(connection, expected_project_id, id),
    ) {
        (Ok(view), Ok(rows)) => (view, rows, None),
        (Err(error), _) | (_, Err(error)) => (None, Vec::new(), Some(error)),
    };
    // G3-2 intel grade: the import-time grade unless a person changed it; a read failure is
    // reported instead of being shown as "no grade".
    let (intel_grade, intel_grade_error) = if category == "intel_claim" {
        match super::imports::current_grade(connection, expected_project_id, id) {
            Ok(grade) => (grade, None),
            Err(error) => (None, Some(error)),
        }
    } else {
        (None, None)
    };
    Ok(serde_json::json!({
        "intelGrade": intel_grade,
        "intelGradeError": intel_grade_error,
        "finding": {"id": id, "category": category, "title": title, "confidence": confidence, "status": status},
        "disposition": disposition,
        "classifications": classifications,
        "classificationError": classification_error,
        "classifyPolicyVersion": super::classify::CLASSIFY_POLICY_VERSION,
        "observations": observations,
        "versions": versions,
        "reviewEvents": events,
        "reviewNeeded": review_needed,
    }))
}
