// §Q3 — the run loop. Starting is one immediate transaction that either creates the plan
// or returns the active run; executing claims one work item at a time in a short
// transaction, does the network work outside any write transaction, and closes the item in
// another short transaction guarded by the claim version.

use super::cluster_repository;
use super::observations::record_pending;
use super::observations::{merge_finding, record, FindingDraft, NewObservation};
use super::scope::{ExposureDenial, ExposureScope};
use super::settings::ExposureSettings;
use super::state::{
    aggregate, aggregation_json, ensure_exposure_schema, utc_now, Aggregation, RunStatus,
    SourceOutcome, SourceStatus, WorkItemCounts, WorkItemStatus,
};
use super::transaction::ImmediateTx;
use super::work_items::{
    claim_next, counts, enumerate_gaps, finish, plan, record_usage, WorkItem, WorkItemSeed,
    DEFAULT_LEASE_SECONDS,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value as JsonValue};
use sha2::{Digest, Sha256};

pub const KIND_PASSIVE_WEB: &str = "passive_web";
pub const KIND_CERTIFICATE_TRANSPARENCY: &str = "certificate_transparency";
pub const KIND_WEB_ARCHIVE: &str = "web_archive";
/// G2: anonymous repository metadata search (one item per confirmed root domain / legal name).
pub const KIND_GITHUB_REPOSITORIES: &str = super::adapters::github::KIND_REPOSITORIES;
/// G2: code search, one item per confirmed root domain; needs a credential (IR-04).
pub const KIND_GITHUB_CODE: &str = super::adapters::github::KIND_CODE;
/// G3: one item per operator-specified public document URL.
pub const KIND_DOCUMENT_LINK: &str = super::adapters::documents::KIND_DOCUMENT_LINK;

/// What one item produced. The orchestrator persists these before the item is closed, so a
/// crash between the response and the terminal write cannot lose the evidence.
#[derive(Clone, Debug, Default)]
pub struct ItemOutcome {
    pub status: Option<WorkItemStatus>,
    pub reason: String,
    pub observations: Vec<NewObservation>,
    pub findings: Vec<FindingDraft>,
    pub pending: Vec<PendingDiscovery>,
    pub requests: i64,
    pub bytes: i64,
    pub observed: i64,
    /// What this item says about its source as a whole. The worst value across a source's
    /// items is what that source finally reports, so a broken index cannot be masked by a
    /// page walk that succeeded.
    pub source_status: Option<SourceStatus>,
    pub source_reason: String,
}

/// A name an external index returned. It is queued for review and never becomes an asset or
/// an authorized target by itself.
#[derive(Clone, Debug)]
pub struct PendingDiscovery {
    pub source_key: String,
    pub kind: String,
    pub entity: String,
    pub canonical_key: String,
    pub evidence_url: String,
}

#[derive(Clone, Debug)]
pub struct StartResult {
    pub run_id: i64,
    pub reused_active: bool,
    pub planned_items: i64,
    pub grants: usize,
    pub excluded: Vec<JsonValue>,
    pub warnings: Vec<String>,
    pub scope_hash: String,
    pub options_hash: String,
}

#[derive(Clone, Debug)]
pub struct RunReport {
    pub run_id: i64,
    pub counts: WorkItemCounts,
    pub aggregation: Aggregation,
    pub observations: i64,
    pub new_findings: i64,
}

fn hash_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn options_hash(settings: &JsonValue) -> String {
    hash_text(&settings.to_string())
}

/// §E2 the round's own budget and item ceiling, read back from the frozen settings so a
/// later edit of the current configuration cannot move an in-flight or interrupted round.
/// G1-0: what `run_plan` reads. A round without a plan row, or with an empty/corrupt frozen
/// value, fails with a stable code instead of collecting under today's defaults.
pub fn frozen_settings_strict(
    connection: &Connection,
    run_id: i64,
    project_id: i64,
) -> Result<ExposureSettings, String> {
    let row: Option<(i64, String)> = connection
        .query_row(
            "SELECT COALESCE(settings_version,1),COALESCE(settings_json,'') FROM exposure_run_plans WHERE run_id=?1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("[exposure_plan_unreadable] 读取第 {run_id} 轮计划失败：{error}"))?;
    let Some((version, raw)) = row else {
        return Err(format!(
            "[exposure_plan_missing] 第 {run_id} 轮没有冻结计划，拒绝按默认配置执行"
        ));
    };
    ExposureSettings::from_frozen_strict(project_id, version, &raw)
}

/// Lenient legacy reader kept for the E2 tests that inspect an already-frozen value.
pub fn frozen_settings(connection: &Connection, run_id: i64, project_id: i64) -> ExposureSettings {
    let row: Option<(i64, String)> = connection
        .query_row(
            "SELECT COALESCE(settings_version,1),COALESCE(settings_json,'{}') FROM exposure_run_plans WHERE run_id=?1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .unwrap_or(None);
    let (version, raw) = row.unwrap_or((1, String::from("{}")));
    ExposureSettings::from_frozen(project_id, version, &raw)
}

/// G1-0: prefix of a scope hash that names the scope's identity only.
pub const SCOPE_HASH_PREFIX: &str = "s2:";

/// The identity of a frozen scope. G1-0: `issuedAt`/`validUntil` are the moment the scope was
/// loaded, so G0's hash changed every second and a resume in a later second was always
/// refused as "授权范围已变化". The authorization window itself is part of the options hash.
pub fn scope_hash_of(scope: &ExposureScope) -> String {
    let mut identity = scope.as_json();
    if let Some(map) = identity.as_object_mut() {
        map.remove("issuedAt");
        map.remove("validUntil");
    }
    format!("{SCOPE_HASH_PREFIX}{}", hash_text(&identity.to_string()))
}

/// The objects this round will cover: one per authorized origin, plus one per planned
/// external index domain. Nothing is silently cut: the budget turns the tail into
/// `blocked_quota` items, which the aggregation reports as a named gap.
pub fn planned_seeds(
    connection: &Connection,
    project_id: i64,
    scope: &ExposureScope,
    settings: &JsonValue,
) -> Result<Vec<WorkItemSeed>, String> {
    let mut seeds = Vec::new();
    for grant in &scope.grants {
        seeds.push(WorkItemSeed {
            work_key: format!("{KIND_PASSIVE_WEB}|{}", grant.origin),
            kind: KIND_PASSIVE_WEB.to_string(),
            source_key: KIND_PASSIVE_WEB.to_string(),
            asset_id: grant.asset_id,
            origin: grant.origin.clone(),
            entity: grant.label.clone(),
            input_hash: super::fingerprint::input_hash(&[
                "passive_web",
                &grant.origin,
                &project_id.to_string(),
            ]),
        });
    }
    let mut approved_domains: Vec<String> = connection
        .query_row(
            "SELECT approved_domains_json FROM asset_ownership_profiles WHERE project_id=?1",
            [project_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default();
    approved_domains.sort();
    approved_domains.dedup();
    if scope
        .sources
        .iter()
        .any(|row| row == KIND_CERTIFICATE_TRANSPARENCY)
    {
        for domain in approved_domains.iter() {
            let domain = domain.trim().trim_start_matches("*.").to_ascii_lowercase();
            if domain.is_empty() {
                continue;
            }
            seeds.push(WorkItemSeed {
                work_key: format!("{KIND_CERTIFICATE_TRANSPARENCY}|{domain}"),
                kind: KIND_CERTIFICATE_TRANSPARENCY.to_string(),
                source_key: KIND_CERTIFICATE_TRANSPARENCY.to_string(),
                asset_id: None,
                origin: String::new(),
                entity: domain.clone(),
                input_hash: super::fingerprint::input_hash(&["ct", &domain]),
            });
        }
    }
    if scope.sources.iter().any(|row| row == KIND_WEB_ARCHIVE) {
        for domain in approved_domains.iter() {
            let domain = domain.trim().trim_start_matches("*.").to_ascii_lowercase();
            if domain.is_empty() {
                continue;
            }
            seeds.push(WorkItemSeed {
                work_key: format!("{KIND_WEB_ARCHIVE}|{domain}"),
                kind: KIND_WEB_ARCHIVE.to_string(),
                source_key: KIND_WEB_ARCHIVE.to_string(),
                asset_id: None,
                origin: String::new(),
                entity: domain.clone(),
                input_hash: super::fingerprint::input_hash(&["archive", &domain]),
            });
        }
    }
    // G2: the GitHub queries come only from confirmed names — the approved root domains and
    // the profile's legal name. Shared keywords are one query; employee names or loose short
    // words are never expanded into queries.
    if scope.sources.iter().any(|row| row == KIND_GITHUB_REPOSITORIES) {
        let legal_name: String = connection
            .query_row(
                "SELECT COALESCE(legal_name,'') FROM asset_ownership_profiles WHERE project_id=?1",
                [project_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        let mut names: Vec<String> = approved_domains
            .iter()
            .map(|domain| domain.trim().trim_start_matches("*.").to_ascii_lowercase())
            .filter(|domain| !domain.is_empty())
            .collect();
        let legal_name = legal_name.trim().to_string();
        if legal_name.chars().count() >= 2 {
            names.push(legal_name);
        }
        names.dedup();
        for name in names {
            seeds.push(WorkItemSeed {
                work_key: format!("{KIND_GITHUB_REPOSITORIES}|{name}"),
                kind: KIND_GITHUB_REPOSITORIES.to_string(),
                source_key: KIND_GITHUB_REPOSITORIES.to_string(),
                asset_id: None,
                origin: String::new(),
                entity: name.clone(),
                input_hash: super::fingerprint::input_hash(&[
                    "github_repositories",
                    super::adapters::github::ADAPTER_VERSION,
                    &name,
                ]),
            });
        }
    }
    if scope.sources.iter().any(|row| row == KIND_GITHUB_CODE) {
        for domain in approved_domains.iter() {
            let domain = domain.trim().trim_start_matches("*.").to_ascii_lowercase();
            if domain.is_empty() {
                continue;
            }
            seeds.push(WorkItemSeed {
                work_key: format!("{KIND_GITHUB_CODE}|{domain}"),
                kind: KIND_GITHUB_CODE.to_string(),
                source_key: KIND_GITHUB_CODE.to_string(),
                asset_id: None,
                origin: String::new(),
                entity: domain.clone(),
                input_hash: super::fingerprint::input_hash(&[
                    "github_code",
                    super::adapters::github::ADAPTER_VERSION,
                    &domain,
                ]),
            });
        }
    }
    // G3: only the exact URLs frozen in this round's settings. Enabled without any URL, the
    // source plans one placeholder item so the gap is reported by name, not as "no items".
    if scope.sources.iter().any(|row| row == KIND_DOCUMENT_LINK) {
        let entries = super::settings::parse_document_urls(settings.get("exposureDocumentUrls"))?;
        if entries.is_empty() {
            let entity = super::adapters::documents::UNCONFIGURED_ENTITY.to_string();
            seeds.push(WorkItemSeed {
                work_key: format!("{KIND_DOCUMENT_LINK}|{entity}"),
                kind: KIND_DOCUMENT_LINK.to_string(),
                source_key: KIND_DOCUMENT_LINK.to_string(),
                asset_id: None,
                origin: String::new(),
                entity,
                input_hash: super::fingerprint::input_hash(&["document_link", "unconfigured"]),
            });
        }
        for entry in entries {
            seeds.push(WorkItemSeed {
                work_key: format!("{KIND_DOCUMENT_LINK}|{}", entry.url),
                kind: KIND_DOCUMENT_LINK.to_string(),
                source_key: KIND_DOCUMENT_LINK.to_string(),
                asset_id: None,
                origin: String::new(),
                entity: entry.url.clone(),
                input_hash: super::fingerprint::input_hash(&[
                    "document_link",
                    super::adapters::documents::ADAPTER_VERSION,
                    &entry.url,
                    if entry.public_material { "public" } else { "" },
                    &entry.note,
                ]),
            });
        }
    }
    Ok(seeds)
}

/// Atomically either creates the plan or returns the run that is already active. The
/// database's partial unique index is the tie-breaker, so two simultaneous starters cannot
/// both get a run even if the pre-check races.
pub fn start_or_reuse(
    connection: &Connection,
    project_id: i64,
    settings: &JsonValue,
) -> Result<StartResult, String> {
    let warnings = ensure_exposure_schema(connection)?;
    // §E2: the settings are validated and frozen with a version; an invalid field stops the
    // round instead of being quietly replaced by a default.
    let effective = ExposureSettings::from_json(project_id, settings)?;
    let frozen = effective.to_settings_json();
    let scope = ExposureScope::load(connection, project_id, &frozen)
        .map_err(|denial| format!("{}:{}", denial.code, denial.message))?;
    let settings = &frozen;
    let seeds = planned_seeds(connection, project_id, &scope, settings)?;
    let scope_hash = scope_hash_of(&scope);
    let options = options_hash(settings);
    let excluded: Vec<JsonValue> = scope
        .excluded
        .iter()
        .map(|row| json!({"assetId": row.asset_id, "reason": row.reason}))
        .collect();

    if let Some(existing) = active_run(connection, project_id)? {
        return Ok(StartResult {
            run_id: existing,
            reused_active: true,
            planned_items: counts(connection, existing)?.total,
            grants: scope.grants.len(),
            excluded,
            warnings,
            scope_hash,
            options_hash: options,
        });
    }

    let transaction = ImmediateTx::begin(connection)?;
    let inserted = transaction.connection().execute(
        "INSERT INTO exposure_runs(project_id,status,eligible_assets,stage,created_at)
         VALUES(?1,'queued',?2,'planned',?3)",
        params![project_id, seeds.len() as i64, utc_now()],
    );
    let run_id = match inserted {
        Ok(_) => transaction.insert_row_id(),
        Err(error)
            if error
                .to_string()
                .contains("UNIQUE constraint failed: exposure_runs")
                || error.to_string().contains("ux_exposure_runs_single_active") =>
        {
            // Another starter won the race; hand back its run instead of a second one.
            let existing = active_run(transaction.connection(), project_id)?
                .ok_or_else(|| format!("并发启动冲突但找不到活跃任务：{error}"))?;
            let existing_counts = counts(transaction.connection(), existing)?.total;
            transaction.commit()?;
            return Ok(StartResult {
                run_id: existing,
                reused_active: true,
                // The winning starter may still be writing its plan; the page re-reads it from
                // the run endpoints, so a smaller count here is not a claim of emptiness.
                planned_items: existing_counts,
                grants: scope.grants.len(),
                excluded,
                warnings,
                scope_hash,
                options_hash: options,
            });
        }
        Err(error) => return Err(format!("采集任务创建失败：{error}")),
    };
    transaction.connection().execute(
        "INSERT INTO exposure_run_plans(run_id,project_id,attempt_number,scope_version,scope_hash,options_hash,sources_json,budget_requests,budget_bytes,planned_items,created_at,settings_version,settings_json)
         VALUES(?1,?2,1,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            run_id,
            project_id,
            scope.scope_version,
            scope_hash,
            options,
            json!(scope.sources).to_string(),
            scope.max_requests,
            scope.max_total_bytes,
            seeds.len() as i64,
            utc_now(),
            effective.version,
            frozen.to_string()
        ],
    )
    .map_err(|error| format!("计划冻结失败：{error}"))?;
    transaction.connection().execute(
        "INSERT INTO exposure_scope_receipts(run_id,project_id,scope_version,scope_hash,scope_json,coarse_legacy,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(run_id) DO UPDATE SET scope_version=excluded.scope_version,scope_hash=excluded.scope_hash,scope_json=excluded.scope_json",
        params![
            run_id,
            project_id,
            scope.scope_version,
            scope_hash,
            scope.as_json().to_string(),
            i64::from(scope.coarse_legacy),
            utc_now()
        ],
    )
    .map_err(|error| format!("授权收据写入失败：{error}"))?;
    plan(transaction.connection(), run_id, project_id, 1, &seeds)?;
    transaction.commit()?;
    Ok(StartResult {
        run_id,
        reused_active: false,
        planned_items: seeds.len() as i64,
        grants: scope.grants.len(),
        excluded,
        warnings,
        scope_hash,
        options_hash: options,
    })
}

pub fn active_run(connection: &Connection, project_id: i64) -> Result<Option<i64>, String> {
    connection
        .query_row(
            "SELECT id FROM exposure_runs WHERE project_id=?1 AND status IN ('queued','running') ORDER BY id LIMIT 1",
            [project_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn planned_sources(connection: &Connection, run_id: i64) -> Result<Vec<String>, String> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT sources_json FROM exposure_run_plans WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    // G1-0: a missing column value means "no sources were frozen"; unreadable JSON is an
    // error, not an empty list that would let aggregation call a broken plan complete.
    match raw {
        None => Ok(Vec::new()),
        Some(text) if text.trim().is_empty() => Ok(Vec::new()),
        Some(text) => serde_json::from_str::<Vec<String>>(&text).map_err(|error| {
            format!("[exposure_plan_sources_corrupt] 第 {run_id} 轮冻结的来源列表无效：{error}")
        }),
    }
}

pub fn cancel_requested(connection: &Connection, run_id: i64) -> bool {
    connection
        .query_row(
            "SELECT cancel_requested FROM exposure_runs WHERE id=?1",
            [run_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(1)
        != 0
}

pub fn begin(connection: &Connection, run_id: i64) -> Result<(), String> {
    connection
        .execute(
            "UPDATE exposure_runs SET status='running',stage='executing',started_at=CASE WHEN started_at='' THEN ?2 ELSE started_at END,error='' WHERE id=?1 AND status IN ('queued','running')",
            params![run_id, utc_now()],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn mark_stage(connection: &Connection, run_id: i64, stage: &str, source: &str) {
    let _ = connection.execute(
        "UPDATE exposure_runs SET stage=?2,current_source=?3 WHERE id=?1",
        params![run_id, stage, source],
    );
}

/// Drains the plan. `executor` performs the work for one claimed item (and is the only place
/// that touches the network); every database write around it stays short.
pub fn run_plan(
    connection: &Connection,
    run_id: i64,
    owner: &str,
    executor: &mut dyn FnMut(&WorkItem) -> Result<ItemOutcome, String>,
) -> Result<RunReport, String> {
    begin(connection, run_id)?;
    // G1-0: project, frozen settings and scope version are read strictly. G0 fell back to
    // project 0 / today's defaults / scope v1, so a broken plan still "collected".
    let plan_settings = {
        let project_id = connection
            .query_row(
                "SELECT project_id FROM exposure_runs WHERE id=?1",
                [run_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("[exposure_run_unreadable] 读取第 {run_id} 轮失败：{error}"))?
            .ok_or_else(|| format!("[exposure_run_missing] 第 {run_id} 轮不存在"))?;
        frozen_settings_strict(connection, run_id, project_id)?
    };
    // §E3 the cache key has to name the scope version this round was frozen against, so the
    // same bytes under another scope version are a different question.
    let plan_scope_version: i64 = connection
        .query_row(
            "SELECT scope_version FROM exposure_run_plans WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("[exposure_plan_unreadable] 读取第 {run_id} 轮范围版本失败：{error}"))?
        .ok_or_else(|| format!("[exposure_plan_missing] 第 {run_id} 轮没有冻结计划"))?;
    let mut handled = 0i64;
    let mut source_reports: std::collections::BTreeMap<String, (SourceStatus, String, i64)> =
        std::collections::BTreeMap::new();
    loop {
        if cancel_requested(connection, run_id) {
            break;
        }
        let Some(item) = claim_next(connection, run_id, owner, DEFAULT_LEASE_SECONDS)? else {
            break;
        };
        // §E2 item ceiling: the tail is closed as an explained quota gap, never dropped, so
        // "planned 600, collected 500" stays visible instead of looking like full coverage.
        if handled >= plan_settings.max_items {
            let remaining = counts(connection, run_id)?.open;
            if finish(
                connection,
                item.id,
                owner,
                item.claim_version,
                WorkItemStatus::BlockedQuota,
                &format!(
                    "本轮工作项额度 {}，剩余 {remaining} 项未采集",
                    plan_settings.max_items
                ),
                0,
                0,
                0,
            )? {
                continue;
            }
            break;
        }
        handled += 1;
        let outcome = executor(&item)?;
        let status = outcome.status.unwrap_or(WorkItemStatus::Completed);
        let mut stored = 0i64;
        let mut recorded_ids: Vec<i64> = Vec::new();
        // Evidence first, terminal state second: a crash after the response but before the
        // close must not throw away what was actually seen.
        for observation in &outcome.observations {
            let (id, _) = record(
                connection,
                run_id,
                item.project_id,
                Some(item.id),
                observation,
            )?;
            stored += 1;
            recorded_ids.push(id);
            // G1-3 the page itself (title, structure, host) decides procurement, not the URL.
            let context = super::classify::page_context(
                &observation.url,
                &observation.body,
                &observation.content_type,
            );
            for draft in outcome
                .findings
                .iter()
                .filter(|draft| draft.url == observation.url)
            {
                let (finding_id, _) = merge_finding(connection, run_id, item.project_id, draft, id)?;
                let verdict = super::classify::classify(draft, &item.source_key, &context);
                super::classify::record(connection, item.project_id, finding_id, Some(id), run_id, &verdict)?;
            }
        }
        for draft in outcome
            .findings
            .iter()
            .filter(|draft| !outcome.observations.iter().any(|row| row.url == draft.url))
        {
            let synthetic = NewObservation {
                source_key: item.source_key.clone(),
                kind: draft.category.clone(),
                url: draft.url.clone(),
                final_url: draft.url.clone(),
                asset_id: draft.asset_id,
                body: draft.evidence.as_bytes().to_vec(),
                body_bytes: draft.evidence.chars().count() as i64,
                ..NewObservation::new(&item.source_key, &draft.category, &draft.url)
            };
            let (id, _) = record(
                connection,
                run_id,
                item.project_id,
                Some(item.id),
                &synthetic,
            )?;
            stored += 1;
            recorded_ids.push(id);
            let (finding_id, _) = merge_finding(connection, run_id, item.project_id, draft, id)?;
            // An index row has no page body of its own: only its address can be judged.
            let context = super::classify::page_context(&draft.url, &[], "application/octet-stream");
            let verdict = super::classify::classify(draft, &item.source_key, &context);
            super::classify::record(connection, item.project_id, finding_id, Some(id), run_id, &verdict)?;
        }
        for pending in &outcome.pending {
            record_pending(connection, item.project_id, run_id, pending)?;
        }
        // §E3 the cluster graph is written from the rows this item just observed, before the
        // item is allowed to reach a terminal state. A failure here leaves the item un-closed
        // and the round failed: an inherited conclusion is never claimed on a graph that was
        // not persisted.
        cluster_repository::persist_for_observations(
            connection,
            run_id,
            item.project_id,
            &recorded_ids,
            plan_settings.version,
        )?;
        cache_classified_verdicts(
            connection,
            &item,
            &outcome,
            plan_settings.version,
            plan_settings.authorization_days,
            plan_scope_version,
        )?;
        if let Some(status) = outcome.source_status {
            let entry = source_reports
                .entry(item.source_key.clone())
                .or_insert_with(|| (status, String::new(), 0i64));
            if status.severity_rank() >= entry.0.severity_rank() {
                entry.0 = status;
                entry.1 = outcome.source_reason.clone();
            }
            entry.2 += outcome.observed.max(0);
        }
        record_usage(connection, run_id, outcome.requests, outcome.bytes)?;
        if !finish(
            connection,
            item.id,
            owner,
            item.claim_version,
            status,
            &outcome.reason,
            stored,
            outcome.requests,
            outcome.bytes,
        )? {
            // The claim was taken over while this worker was away. The evidence above stays,
            // but a worker that no longer owns the item must not decide its terminal state.
            continue;
        }
        if cancel_requested(connection, run_id) {
            break;
        }
    }
    for (source_key, (status, reason, observed)) in &source_reports {
        save_source_result(connection, run_id, source_key, *status, reason, *observed)?;
    }
    // G4-0 incident proposals for what this round touched. A failure is never hidden: the
    // round ends with it as an infrastructure reason instead of "completed".
    let proposal = connection
        .query_row("SELECT project_id FROM exposure_runs WHERE id=?1", [run_id], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())
        .and_then(|project_id| super::incidents::propose(connection, project_id, Some(run_id)));
    match proposal {
        Ok(_) => finalize(connection, run_id, None),
        Err(error) => finalize(
            connection,
            run_id,
            Some(&format!("[exposure_incident_proposal_failed] 事件建议未完成：{error}")),
        ),
    }
}

/// §E3 the classification actually applied to these bytes is recorded under this round's own
/// contract, so an identical resource under the same project, scope version, source,
/// canonical URL, configuration and rule version is the same question — and anything else is
/// a miss. A cached verdict never authorizes a fetch: the caller still has to pass the scope
/// gate, and only the deterministic fields are kept, never the evidence text.
fn cache_classified_verdicts(
    connection: &Connection,
    item: &WorkItem,
    outcome: &ItemOutcome,
    settings_version: i64,
    authorization_days: i64,
    scope_version: i64,
) -> Result<(), String> {
    use super::cache::{store_expiring, CacheKey};
    use super::fingerprint::canonical_url;
    use super::redaction::content_hash;
    for observation in &outcome.observations {
        let Some(draft) = outcome
            .findings
            .iter()
            .find(|row| row.url == observation.url)
        else {
            continue;
        };
        let key = CacheKey::new(
            item.project_id,
            scope_version,
            "public",
            &content_hash(&observation.body),
            "classification",
        )
        .with_identity(
            &item.source_key,
            &canonical_url(&observation.url),
            settings_version,
        );
        // The verdict may not outlive the authorization window that produced it.
        let expires_at = if authorization_days > 0 {
            super::state::shift_seconds_forward(authorization_days * 86_400)
        } else {
            String::new()
        };
        store_expiring(
            connection,
            &key,
            &json!({
                "category": draft.category,
                "severity": draft.severity,
                "confidence": draft.confidence,
                "ruleVersion": super::state::EXPOSURE_RULE_VERSION,
                "settingsVersion": settings_version,
                "scopeVersion": scope_version,
                "sourceKey": item.source_key,
                "provenance": "deterministic-rule-classification",
                "note": "确定性规则结论，不是模型裁决，也不构成授权",
            })
            .to_string(),
            &expires_at,
        )?;
    }
    Ok(())
}

pub fn save_source_result(
    connection: &Connection,
    run_id: i64,
    source_key: &str,
    status: SourceStatus,
    reason: &str,
    observed: i64,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO exposure_source_results(run_id,source_key,status,item_count,error)
             VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(run_id,source_key) DO UPDATE SET status=excluded.status,item_count=excluded.item_count,error=excluded.error,completed_at=datetime('now','localtime')",
            params![run_id, source_key, status.as_str(), observed, reason],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn read_source_outcomes(
    connection: &Connection,
    run_id: i64,
) -> Result<Vec<SourceOutcome>, String> {
    let mut statement = connection
        .prepare("SELECT source_key,status,error,item_count FROM exposure_source_results WHERE run_id=?1 ORDER BY source_key")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([run_id], |row| {
            let status: String = row.get(1)?;
            Ok(SourceOutcome {
                source_key: row.get(0)?,
                status: SourceStatus::parse(&status).unwrap_or(SourceStatus::Failed),
                reason: row.get(2)?,
                observed: row.get(3)?,
            })
        })
        .map_err(|error| error.to_string())?;
    Ok(rows.flatten().collect())
}

/// The only place that decides the final state of a run, from the plan, the source
/// outcomes and the work-item counters — never from "the thread returned normally".
pub fn finalize(
    connection: &Connection,
    run_id: i64,
    infra_failure: Option<&str>,
) -> Result<RunReport, String> {
    let item_counts = counts(connection, run_id)?;
    // G1-0: an unreadable source list cannot be aggregated, but the round still has to reach
    // a terminal `failed` with that reason instead of staying `running` forever.
    let sources = match planned_sources(connection, run_id) {
        Ok(rows) => rows,
        Err(error) => {
            let reason = match infra_failure {
                Some(first) if !first.contains("exposure_plan_sources_corrupt") => format!("{first}；{error}"),
                _ => error.clone(),
            };
            connection
                .execute(
                    "UPDATE exposure_runs SET status='failed',stage='failed',current_source='',error=?2,completed_at=?3 WHERE id=?1",
                    params![run_id, reason, utc_now()],
                )
                .map_err(|write| format!("{error}；且无法写入失败状态：{write}"))?;
            return Err(error);
        }
    };
    let outcomes = read_source_outcomes(connection, run_id)?;
    let cancelled = cancel_requested(connection, run_id);
    for source in &sources {
        if !outcomes.iter().any(|row| row.source_key == *source) {
            // The source was planned but produced no work item at all: that is "未配置",
            // not a silent success.
            save_source_result(
                connection,
                run_id,
                source,
                SourceStatus::Unconfigured,
                "该来源本轮没有可执行的工作项（缺少已批准域名或资产）",
                0,
            )?;
        }
    }
    let outcomes = read_source_outcomes(connection, run_id)?;
    let aggregation = aggregate(&sources, &outcomes, &item_counts, cancelled, infra_failure);
    let gaps = enumerate_gaps(connection, run_id, 20)?;
    let mut detail: Vec<String> = aggregation.reasons.clone();
    if !gaps.is_empty() {
        detail.push(format!(
            "未闭合工作项：{}",
            gaps.iter()
                .map(|(key, status, reason)| format!("{key}={status}({reason})"))
                .collect::<Vec<_>>()
                .join("，")
        ));
    }
    let observations: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let findings_new: i64 = connection
        .query_row(
            "SELECT COUNT(DISTINCT finding_id) FROM exposure_finding_observations WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let error_text = if aggregation.status == RunStatus::Completed {
        String::new()
    } else {
        let json = aggregation_json(&Aggregation {
            status: aggregation.status,
            reasons: detail.clone(),
            gaps: aggregation.gaps.clone(),
        });
        json.to_string()
    };
    connection
        .execute(
            "UPDATE exposure_runs SET status=?2,stage=?3,current_source='',scanned_assets=?4,fetched_resources=?5,findings=?6,error=?7,completed_at=?8,eligible_assets=MAX(?9,eligible_assets) WHERE id=?1",
            params![
                run_id,
                aggregation.status.as_str(),
                aggregation.status.as_str(),
                item_counts.terminal,
                observations,
                findings_new,
                error_text,
                utc_now(),
                item_counts.total
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(RunReport {
        run_id,
        counts: item_counts,
        aggregation,
        observations,
        new_findings: findings_new,
    })
}

/// A denial that a caller may turn into a `blocked_authorization` item instead of a crash.
pub fn authorization_gap(denial: &ExposureDenial) -> (WorkItemStatus, String) {
    (
        WorkItemStatus::BlockedAuthorization,
        format!("{}:{}", denial.code, denial.message),
    )
}
