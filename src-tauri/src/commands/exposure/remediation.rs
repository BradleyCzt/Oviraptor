// §E5 — human review, remediation and retest, deterministic only. A review decision is an
// event with an actor, a reason and references; it never rewrites an observation. A remediation
// binds the finding, the observation and the round it came from, and a retest starts a *new*
// round under a re-checked authorization instead of marking the old evidence "gone". Nothing
// here closes anything on its own: the machine only ever reports a comparison basis.

use super::contract::{
    decode_id_cursor, encode_id_cursor, ExposurePage, ExposureQueryError, EXPOSURE_MAX_PAGE_SIZE,
    EXPOSURE_MIN_PAGE_SIZE,
};
use super::orchestrator::{start_or_reuse, StartResult};
use super::redaction::redact_text;
use super::scope::ExposureScope;
use super::state::utc_now;
use super::transaction::ImmediateTx;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value as JsonValue};

pub const REVIEW_STATUSES: &[&str] = &["new", "reviewing", "confirmed", "dismissed"];
pub const REMEDIATION_STATUSES: &[&str] =
    &["open", "fixing", "awaiting_retest", "closed", "rejected"];

/// The current top revision of a finding, i.e. the value a reviewer must have read.
pub fn top_revision(connection: &Connection, finding_id: i64) -> Result<i64, String> {
    connection
        .query_row(
            "SELECT COALESCE(MAX(revision),0) FROM exposure_finding_versions WHERE finding_id=?1",
            [finding_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

/// Applies a human decision. The caller sends the revision it actually read: a queue that has
/// moved on is refused instead of overwriting somebody else's conclusion.
#[allow(clippy::too_many_arguments)]
pub fn review_scoped(
    connection: &Connection,
    project_id: i64,
    finding_id: i64,
    expected_revision: i64,
    status: &str,
    actor: &str,
    reason: &str,
    references: &[i64],
) -> Result<i64, String> {
    review_scoped_cas(connection, project_id, finding_id, expected_revision, None, status, actor, reason, references)
}

/// FIX3-R2：复核不会推进 finding 的版本号，只比 revision 时两名复核员读同一版本都会“成功”，
/// 后写的静默覆盖先写的结论。`expected_review_event` 是客户端读到的最新复核事件 id（没有为 0）；
/// 与库里最新事件不一致即冲突、零写入。命令入口必须带上它。
#[allow(clippy::too_many_arguments)]
pub fn review_scoped_cas(
    connection: &Connection,
    project_id: i64,
    finding_id: i64,
    expected_revision: i64,
    expected_review_event: Option<i64>,
    status: &str,
    actor: &str,
    reason: &str,
    references: &[i64],
) -> Result<i64, String> {
    if !REVIEW_STATUSES.contains(&status) {
        return Err(format!(
            "exposure_review_status_invalid：复核状态 {status} 不在 {} 里",
            REVIEW_STATUSES.join("/")
        ));
    }
    let actor = actor.trim();
    if actor.is_empty() || actor.chars().count() > 80 {
        return Err("exposure_review_actor_invalid：复核必须有 1~80 字的操作者".to_string());
    }
    if reason.trim().is_empty() {
        return Err(
            "exposure_review_reason_required：人工结论必须写明依据，不能只点一个按钮".to_string(),
        );
    }
    // FIX3-R1 归属、原状态、版本和复核事件都在同一个写事务里读：读与写之间没有可插入的窗口。
    let transaction = ImmediateTx::begin(connection)?;
    let owned: Option<(i64, String)> = transaction
        .connection()
        .query_row(
            "SELECT project_id,status FROM exposure_findings WHERE id=?1",
            [finding_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((row_project, before)) = owned else {
        return Err(format!(
            "exposure_finding_missing:线索 {finding_id} 不存在，不能凭空复核"
        ));
    };
    if row_project != project_id {
        return Err("exposure_scope_mismatch：这条线索不属于当前工作空间，已拒绝复核".to_string());
    }
    // P1-2 the revision check and the write share one write transaction: a collector that
    // advances the finding between "check" and "write" makes this review a conflict.
    let top = top_revision(transaction.connection(), finding_id)?;
    if top != expected_revision {
        return Err(format!(
            "exposure_review_conflict：队列已前进到 r{top}，编辑器读到的是 r{expected_revision}，未写入任何结论"
        ));
    }
    if let Some(expected_event) = expected_review_event {
        let latest: i64 = transaction
            .connection()
            .query_row(
                "SELECT COALESCE(MAX(id),0) FROM exposure_review_events WHERE project_id=?1 AND finding_id=?2",
                params![project_id, finding_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if latest != expected_event {
            return Err(format!(
                "exposure_review_conflict：同一版本 r{top} 已有别的复核结论（事件 {latest}），编辑器读到的是事件 {expected_event}，未写入任何结论"
            ));
        }
    }
    transaction.connection().execute(
        "UPDATE exposure_findings SET status=?1,note=?2,updated_at=datetime('now','localtime') WHERE id=?3 AND project_id=?4",
        params![status, redact_text(reason), finding_id, project_id],
    )
    .map_err(|error| error.to_string())?;
    transaction.connection().execute(
        "INSERT INTO exposure_review_events(project_id,finding_id,from_status,to_status,note,actor,expected_revision,references_json,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            project_id,
            finding_id,
            before,
            status,
            redact_text(reason),
            actor,
            expected_revision,
            json!(references).to_string(),
            utc_now()
        ],
    )
    .map_err(|error| error.to_string())?;
    if top > 0 {
        transaction.connection().execute(
            "UPDATE exposure_finding_versions SET manual_status=?1 WHERE finding_id=?2 AND revision=?3",
            params![status, finding_id, top],
        )
        .map_err(|error| error.to_string())?;
    }
    transaction.commit()?;
    Ok(top)
}

/// Who decided what, when, and against which evidence. Read-only history.
pub fn review_trail(
    connection: &Connection,
    project_id: i64,
    finding_id: i64,
) -> Result<Vec<JsonValue>, String> {
    let mut statement = connection
        .prepare(
            "SELECT from_status,to_status,note,actor,expected_revision,references_json,created_at,id \
             FROM exposure_review_events WHERE project_id=?1 AND finding_id=?2 ORDER BY id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![project_id, finding_id], |row| {
            let references: String = row.get(5)?;
            Ok(json!({
                "fromStatus": row.get::<_, String>(0)?,
                "toStatus": row.get::<_, String>(1)?,
                "note": row.get::<_, String>(2)?,
                "actor": row.get::<_, String>(3)?,
                "expectedRevision": row.get::<_, i64>(4)?,
                "references": serde_json::from_str::<JsonValue>(&references).unwrap_or(json!([])),
                "createdAt": row.get::<_, String>(6)?,
                "eventId": row.get::<_, i64>(7)?,
            }))
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

/// Records that somebody is fixing a finding, bound to the exact observation and round the
/// conclusion came from. Recording the same evidence again updates the row instead of
/// duplicating it.
#[allow(clippy::too_many_arguments)]
pub fn record_remediation(
    connection: &Connection,
    project_id: i64,
    finding_id: i64,
    observation_id: Option<i64>,
    run_id: i64,
    actor: &str,
    note: &str,
    status: &str,
) -> Result<i64, String> {
    if !REMEDIATION_STATUSES.contains(&status) {
        return Err(format!(
            "exposure_remediation_status_invalid：整改状态 {status} 不在 {} 里",
            REMEDIATION_STATUSES.join("/")
        ));
    }
    let actor = actor.trim();
    if actor.is_empty() || note.trim().is_empty() {
        return Err("exposure_remediation_incomplete：整改必须写明操作者与说明".to_string());
    }
    let finding: Option<i64> = connection
        .query_row(
            "SELECT id FROM exposure_findings WHERE id=?1 AND project_id=?2",
            params![finding_id, project_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if finding.is_none() {
        return Err(
            "exposure_scope_mismatch：这条线索不属于当前工作空间，不能挂整改记录".to_string(),
        );
    }
    // G1-3 an auto-excluded finding (public procurement / public material) is not a risk to
    // remediate. Only a human conclusion can put it back; the reclassification channel is
    // IR-07, so until then the refusal names the reason instead of creating busywork.
    if let Some(view) = super::classify::disposition_of(connection, project_id, finding_id)? {
        let human_confirmed: bool = connection
            .query_row(
                "SELECT status='confirmed' FROM exposure_findings WHERE id=?1",
                [finding_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if super::classify::is_excluded(&view.disposition) && !human_confirmed {
            return Err(format!(
                "exposure_remediation_excluded：这条线索已自动判为 {}（{}），不是待整改风险；如需改判请走人工复核（IR-07 集成后提供）",
                view.disposition, view.reason
            ));
        }
    }
    if let Some(observation) = observation_id {
        let linked: Option<i64> = connection
            .query_row(
                "SELECT o.id FROM exposure_observations o \
                 JOIN exposure_finding_observations l ON l.observation_id=o.id AND l.finding_id=?3 \
                 WHERE o.id=?1 AND o.project_id=?2",
                params![observation, project_id, finding_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if linked.is_none() {
            return Err(format!(
                "exposure_remediation_evidence_missing：观察 {observation} 不存在、不属于本项目或不是线索 {finding_id} 的证据"
            ));
        }
    }
    if run_id != 0 {
        let owned_run: Option<i64> = connection
            .query_row(
                "SELECT id FROM exposure_runs WHERE id=?1 AND project_id=?2",
                params![run_id, project_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if owned_run.is_none() {
            return Err(format!(
                "exposure_remediation_run_invalid：轮次 {run_id} 不属于当前工作空间"
            ));
        }
    }
    // P1-2 a closed or rejected remediation is final: registering the same evidence again
    // must not reopen it from a stale page.
    let terminal: Option<String> = connection
        .query_row(
            "SELECT status FROM exposure_remediations WHERE project_id=?1 AND finding_id=?2 AND observation_id IS ?3 \
             AND status IN ('closed','rejected')",
            params![project_id, finding_id, observation_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(terminal) = terminal {
        return Err(format!(
            "exposure_remediation_terminal：整改已是 {terminal}，不能被重新登记或改写"
        ));
    }
    connection
        .execute(
            "INSERT INTO exposure_remediations(project_id,finding_id,observation_id,run_id,actor,status,note)
             VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(project_id,finding_id,observation_id) DO UPDATE SET
                status=excluded.status, note=excluded.note, actor=excluded.actor, run_id=excluded.run_id",
            params![
                project_id,
                finding_id,
                observation_id,
                run_id,
                actor,
                status,
                redact_text(note)
            ],
        )
        .map_err(|error| error.to_string())?;
    let id: i64 = connection
        .query_row(
            "SELECT id FROM exposure_remediations WHERE project_id=?1 AND finding_id=?2 AND observation_id IS ?3",
            params![project_id, finding_id, observation_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(id)
}

/// A retest is always a new round: the authorization is re-checked against the current scope,
/// the current settings are frozen into the new plan, and the old observation is left exactly
/// as it was. A revoked scope therefore cannot "retest" its way back in.
pub fn start_retest(
    connection: &Connection,
    project_id: i64,
    remediation_id: i64,
    settings: &JsonValue,
) -> Result<StartResult, String> {
    let row: Option<(i64, Option<i64>, String)> = connection
        .query_row(
            "SELECT finding_id,observation_id,status FROM exposure_remediations WHERE id=?1 AND project_id=?2",
            params![remediation_id, project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((finding_id, observation_id, status)) = row else {
        return Err(format!(
            "exposure_remediation_missing:整改 {remediation_id} 不属于当前工作空间"
        ));
    };
    if matches!(status.as_str(), "closed" | "rejected") {
        return Err(format!(
            "exposure_remediation_terminal：整改已是 {status}，不能再复测"
        ));
    }
    let observation = observation_id.ok_or_else(|| {
        "exposure_retest_reference_missing：整改没有绑定可复测的观察，复测无法指向原证据"
            .to_string()
    })?;
    let (url, observed_asset): (String, Option<i64>) = connection
        .query_row(
            "SELECT url,asset_id FROM exposure_observations WHERE id=?1 AND project_id=?2",
            params![observation, project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("exposure_retest_reference_lost：原观察已不可读：{error}"))?;
    // Re-authorization, the same gate any other round has to pass.
    let scope = ExposureScope::load(connection, project_id, settings)
        .map_err(|denial| format!("{}:{}", denial.code, denial.message))?;
    if scope.grants.is_empty() {
        return Err(
            "exposure_retest_unauthorized：当前工作空间没有准入资产，复测不能凭旧授权启动"
                .to_string(),
        );
    }
    // P1-3 / T30 the asset the evidence came from must itself still be granted: another
    // authorized asset in the workspace is not a licence to re-query a revoked one.
    if let Some(asset) = observed_asset {
        if !scope.grants.iter().any(|grant| grant.asset_id == Some(asset)) {
            return Err(format!(
                "exposure_retest_unauthorized：原证据所在资产 {asset} 已不在准入范围（撤权、排除或归档），复测不发任何请求"
            ));
        }
    }
    let started = start_or_reuse(connection, project_id, settings)?;
    connection
        .execute(
            "INSERT INTO exposure_remediation_retests(remediation_id,project_id,run_id,finding_id,observation_url)
             VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(remediation_id,run_id) DO NOTHING",
            params![remediation_id, project_id, started.run_id, finding_id, url],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "UPDATE exposure_remediations SET status='awaiting_retest' WHERE id=?1",
            [remediation_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(started)
}

/// What the newest retest round actually saw, compared with the bytes the conclusion was based
/// on. This is a report, never a verdict: "not seen this round" is not proof of removal.
pub fn retest_outcome(
    connection: &Connection,
    project_id: i64,
    remediation_id: i64,
) -> Result<JsonValue, String> {
    let retest: Option<(i64, String)> = connection
        .query_row(
            "SELECT t.run_id,t.observation_url FROM exposure_remediation_retests t \
             WHERE t.remediation_id=?1 AND t.project_id=?2 ORDER BY t.id DESC LIMIT 1",
            params![remediation_id, project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((run_id, url)) = retest else {
        return Ok(json!({
            "basis": "no_retest_round",
            "canClose": false,
            "note": "还没有复测轮次，整改不能被记为已验证",
        }));
    };
    let round_status: String = connection
        .query_row(
            "SELECT status FROM exposure_runs WHERE id=?1",
            [run_id],
            |row| row.get(0),
        )
        .unwrap_or_default();
    let before: Option<String> = connection
        .query_row(
            "SELECT o.content_hash FROM exposure_remediations r \
             JOIN exposure_observations o ON o.id=r.observation_id WHERE r.id=?1",
            [remediation_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let seen: Option<(String, String, i64)> = connection
        .query_row(
            "SELECT content_hash,queried_at,status_code FROM exposure_observations \
             WHERE run_id=?1 AND url=?2 AND project_id=?3 ORDER BY id DESC LIMIT 1",
            params![run_id, url, project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let (basis, can_close, note) = match (&seen, &before) {
        (None, _)
            if matches!(
                round_status.as_str(),
                "failed" | "interrupted" | "cancelled"
            ) =>
        {
            (
                "retest_round_incomplete",
                false,
                format!("复测轮 {run_id} 状态是 {round_status}，没有可比对的完整结果"),
            )
        }
        (None, _) => (
            "retest_not_seen",
            false,
            "复测轮里没有这条 URL 的观察：只能说明本轮没取到，不能证明已经下线".to_string(),
        ),
        // G1-4: a transport failure is recorded with status 0 and the hash of an empty body, so
        // it must be judged *before* the byte comparison; otherwise "could not fetch" reads as
        // "different bytes" and would let a remediation close (spec T28–T30).
        (Some((_, _, status_code)), Some(_))
            if !((200..300).contains(status_code) || matches!(*status_code, 404 | 410)) =>
        (
            "retest_unreachable",
            false,
            format!("复测没能取到该资源（HTTP {}），无法判断整改是否生效", seen.as_ref().map(|row| row.2).unwrap_or_default()),
        ),
        // P1-4 a round that failed, was cancelled/interrupted or is still open is not a basis
        // for closing, even if it left an observation with other bytes behind.
        (Some((hash, _, _)), Some(previous))
            if hash != previous && !matches!(round_status.as_str(), "completed" | "partial") =>
        (
            "retest_round_incomplete",
            false,
            format!("复测轮 {run_id} 状态是 {round_status}，未完成的轮次不能作为关闭依据"),
        ),
        (Some((hash, _, _)), Some(previous)) if hash != previous => (
            "retest_content_changed",
            true,
            "复测取回了不同的字节，原结论需要按新证据重新复核".to_string(),
        ),
        (Some(_), Some(_)) => (
            "retest_content_identical",
            false,
            "复测看到同一份字节，整改不能算已验证".to_string(),
        ),
        (Some(_), None) => (
            "retest_without_baseline",
            false,
            "整改绑定的原观察已不可读，缺少可比对的基线".to_string(),
        ),
    };
    Ok(json!({
        "basis": basis,
        "canClose": can_close,
        "retestRunId": run_id,
        "roundStatus": round_status,
        "url": url,
        "previousHash": before.unwrap_or_default(),
        "currentHash": seen.clone().map(|row| row.0).unwrap_or_default(),
        "retestObservedAt": seen.map(|row| row.1).unwrap_or_default(),
        "note": note,
    }))
}

/// A human closes or rejects a remediation, and only with a basis the retest report can support.
/// The old observation rows are never touched, and the finding is moved to `reviewing` so the
/// conclusion has to be re-made against the new evidence.
pub fn mark_remediation(
    connection: &Connection,
    project_id: i64,
    remediation_id: i64,
    status: &str,
    actor: &str,
    reason: &str,
) -> Result<JsonValue, String> {
    if !REMEDIATION_STATUSES.contains(&status) {
        return Err(format!(
            "exposure_remediation_status_invalid：整改状态 {status} 不在 {} 里",
            REMEDIATION_STATUSES.join("/")
        ));
    }
    let actor = actor.trim();
    if actor.is_empty() || reason.trim().is_empty() {
        return Err(
            "exposure_remediation_incomplete：变更整改状态必须写明操作者与理由".to_string(),
        );
    }
    // FIX3-R7 终态检查、复测依据和写入在同一个写事务里；写入只在状态仍是读到的那个值时生效。
    // 两人同时驳回/关闭同一条整改时，后到者看到终态，零写入。
    let transaction = ImmediateTx::begin(connection)?;
    let current: Option<(i64, String)> = transaction
        .connection()
        .query_row(
            "SELECT finding_id,status FROM exposure_remediations WHERE id=?1 AND project_id=?2",
            params![remediation_id, project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((finding_id, current_status)) = current else {
        return Err(format!(
            "exposure_remediation_missing:整改 {remediation_id} 不属于当前工作空间"
        ));
    };
    if matches!(current_status.as_str(), "closed" | "rejected") {
        return Err(format!(
            "exposure_remediation_terminal：整改已是 {current_status}，不能再改为 {status}"
        ));
    }
    let outcome = retest_outcome(transaction.connection(), project_id, remediation_id)?;
    if status == "closed" && outcome["canClose"] != json!(true) {
        return Err(format!(
            "exposure_remediation_unclosed：{}，不能凭感觉把整改记为已验证",
            outcome["basis"].as_str().unwrap_or("没有复测依据")
        ));
    }
    let changed = transaction.connection().execute(
        "UPDATE exposure_remediations SET status=?1, closed_at=?2 WHERE id=?3 AND project_id=?4 AND status=?5",
        params![
            status,
            if status == "closed" { utc_now() } else { String::new() },
            remediation_id,
            project_id,
            current_status
        ],
    )
    .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("exposure_remediation_conflict：整改状态已被他人改变，未写入".to_string());
    }
    transaction.commit()?;
    if status == "closed" {
        review_scoped(
            connection,
            project_id,
            finding_id,
            top_revision(connection, finding_id)?,
            "reviewing",
            actor,
            &format!(
                "复测轮 {} 依据 {}：{reason}",
                outcome["retestRunId"].as_i64().unwrap_or_default(),
                outcome["basis"].as_str().unwrap_or_default()
            ),
            &[finding_id],
        )?;
    }
    Ok(outcome)
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemediationRow {
    pub id: i64,
    pub finding_id: i64,
    pub observation_id: Option<i64>,
    pub run_id: i64,
    pub actor: String,
    pub status: String,
    pub note: String,
    pub created_at: String,
    pub closed_at: String,
    pub retest_run_ids: Vec<i64>,
    pub finding_title: String,
    pub finding_status: String,
}

fn storage(error: rusqlite::Error) -> ExposureQueryError {
    ExposureQueryError::storage(&error.to_string())
}

/// One page of remediation records, newest first by id keyset, with the retest rounds attached.
pub fn page_remediations(
    connection: &Connection,
    project_id: i64,
    status: &str,
    cursor: &str,
    page_size: i64,
) -> Result<ExposurePage<RemediationRow>, ExposureQueryError> {
    if !(EXPOSURE_MIN_PAGE_SIZE..=EXPOSURE_MAX_PAGE_SIZE).contains(&page_size) {
        return Err(ExposureQueryError::invalid_page_size(page_size));
    }
    let status = status.trim().to_ascii_lowercase();
    if !(status.is_empty() || REMEDIATION_STATUSES.contains(&status.as_str())) {
        return Err(ExposureQueryError::new(
            "exposure_remediation_status_invalid",
            format!("未知整改状态 {status}，已拒绝而不是当作无筛选"),
        ));
    }
    let before_id = if cursor.trim().is_empty() {
        i64::MAX
    } else {
        decode_id_cursor(cursor)?
    };
    let filter = "WHERE r.project_id=?1 AND (?2='' OR r.status=?2) AND r.id<?3";
    let total: i64 = connection
        .query_row(
            &format!("SELECT COUNT(*) FROM exposure_remediations r {filter}"),
            params![project_id, status, before_id],
            |row| row.get(0),
        )
        .map_err(storage)?;
    let sql = format!(
        "SELECT r.id,r.finding_id,r.observation_id,r.run_id,r.actor,r.status,r.note,r.created_at,r.closed_at,\
                f.title,f.status \
         FROM exposure_remediations r JOIN exposure_findings f ON f.id=r.finding_id \
         {filter} ORDER BY r.id DESC LIMIT ?4"
    );
    let mut statement = connection.prepare(&sql).map_err(storage)?;
    let fetched = statement
        .query_map(
            params![project_id, status, before_id, page_size + 1],
            |row| {
                Ok(RemediationRow {
                    id: row.get(0)?,
                    finding_id: row.get(1)?,
                    observation_id: row.get(2)?,
                    run_id: row.get(3)?,
                    actor: row.get(4)?,
                    status: row.get(5)?,
                    note: row.get(6)?,
                    created_at: row.get(7)?,
                    closed_at: row.get(8)?,
                    retest_run_ids: Vec::new(),
                    finding_title: row.get(9)?,
                    finding_status: row.get(10)?,
                })
            },
        )
        .map_err(storage)?;
    let mut items = fetched.collect::<Result<Vec<_>, _>>().map_err(storage)?;
    let has_more = items.len() > page_size as usize;
    items.truncate(page_size as usize);
    for item in items.iter_mut() {
        let mut statement = connection
            .prepare(
                "SELECT run_id FROM exposure_remediation_retests WHERE remediation_id=?1 ORDER BY id",
            )
            .map_err(storage)?;
        item.retest_run_ids = statement
            .query_map([item.id], |row| row.get::<_, i64>(0))
            .map_err(storage)?
            .flatten()
            .collect();
    }
    Ok(ExposurePage {
        next_cursor: items
            .last()
            .map(|row| encode_id_cursor(row.id))
            .unwrap_or_default(),
        has_more,
        total,
        page_size,
        items,
    })
}

/// §E5 the classes the page has to keep apart. Every number comes from rows that exist, so
/// "no evidence" can never be read as "no problem".
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaxonomyRow {
    pub code: String,
    pub label: String,
    pub count: i64,
    pub meaning: String,
}

fn scalar(connection: &Connection, sql: &str, project_id: i64) -> i64 {
    connection
        .query_row(sql, [project_id], |row| row.get(0))
        .unwrap_or(0)
}

pub fn taxonomy(connection: &Connection, project_id: i64) -> Result<Vec<TaxonomyRow>, String> {
    let rows = vec![
        (
            "confirmed_exposure",
            "已确认暴露",
            scalar(
                connection,
                "SELECT COUNT(*) FROM exposure_findings WHERE project_id=?1 AND status='confirmed'",
                project_id,
            ),
            "人工确认过的公开暴露线索，结论仍绑定原观察",
        ),
        (
            "review_pending",
            "待复核",
            scalar(
                connection,
                "SELECT COUNT(*) FROM exposure_findings WHERE project_id=?1 AND status IN ('new','reviewing')",
                project_id,
            ),
            "规则命中或人工正在看，尚未定性",
        ),
        (
            "source_unreachable",
            "来源不可达",
            scalar(
                connection,
                "SELECT COUNT(DISTINCT s.source_key) FROM exposure_source_results s \
                 JOIN exposure_runs r ON r.id=s.run_id WHERE r.project_id=?1 \
                 AND s.status IN ('failed','parse_error','unsupported','unconfigured')",
                project_id,
            ),
            "这些来源在本轮没取到东西，不能算覆盖",
        ),
        (
            "not_covered",
            "未覆盖",
            scalar(
                connection,
                "SELECT COUNT(*) FROM exposure_work_items w JOIN exposure_runs r ON r.id=w.run_id \
                 WHERE r.project_id=?1 AND w.status IN ('blocked_quota','blocked_authorization','skipped_unconfigured','orphaned','cancelled')",
                project_id,
            ),
            "计划里有、本轮没采到的工作项",
        ),
        (
            "rule_hit",
            "规则命中",
            scalar(
                connection,
                "SELECT COUNT(DISTINCT v.finding_id) FROM exposure_finding_versions v \
                 JOIN exposure_findings f ON f.id=v.finding_id \
                 WHERE f.project_id=?1 AND v.rule_version<>''",
                project_id,
            ),
            "确定性规则的匹配结果，不是模型判断",
        ),
        (
            "unknown",
            "证据不足",
            scalar(
                connection,
                "SELECT COUNT(*) FROM exposure_findings f WHERE f.project_id=?1 \
                 AND NOT EXISTS(SELECT 1 FROM exposure_finding_observations l WHERE l.finding_id=f.id)",
                project_id,
            ),
            "没有任何观察支撑的条目，一律不能定性为漏洞",
        ),
        (
            "dismissed",
            "已排除",
            scalar(
                connection,
                "SELECT COUNT(*) FROM exposure_findings WHERE project_id=?1 AND status='dismissed'",
                project_id,
            ),
            "人工排除，操作者与理由留在复核轨迹里",
        ),
        (
            "auto_excluded",
            "自动排除",
            scalar(
                connection,
                "SELECT COUNT(*) FROM exposure_finding_dispositions WHERE project_id=?1 \
                 AND disposition IN ('excluded_public_procurement','excluded_public_material')",
                project_id,
            ),
            "公开招采公告或正常公开材料（G1-3/T08 规则 oviraptor.exposure.classify.v2），仍可查看，不计入高风险待办",
        ),
    ];
    Ok(rows
        .into_iter()
        .map(|(code, label, count, meaning)| TaxonomyRow {
            code: code.to_string(),
            label: label.to_string(),
            count,
            meaning: meaning.to_string(),
        })
        .collect())
}
