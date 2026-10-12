// O3 规模与权限（设计文档 NEST_OWNERSHIP_GATE_SMART_GROUP_REVIEW_DESIGN_2026-10-08 §8、§9、§11 第 4 步、§12）。
// - 组批量写入改为操作状态机：T0 在一个 IMMEDIATE 事务内完成幂等/CAS/哈希校验并把冻结的逐条计划写入
//   ownership_operation_staging；之后每个分块（OWNERSHIP_APPLY_CHUNK 行）一个短事务，逐行再守卫
//   （结论指纹 = 计划时、关联有效、仍在范围、证据未变），游标 last_seq 与写入同事务推进，
//   因此可续跑且不会重复写；全部分块完成后才收尾（组版本 +1、applied_operations +1、清理暂存）。
// - 组操作只写归属事实或收窄（第三方/排除 → 拒绝/不准入），永不写 allowed / exposure_eligible=1；
//   未完成批次不会向下游发布任何“整组允许”，组在收尾前也不显示为已处理。
// - 组级撤销（revert）：针对一次已完成的组操作，逐条要求当前状态 = 该操作写入后的状态，恢复旧结论；
//   旧值中的 allowed / 准入不恢复（authorization_not_restored），即后台与撤销都绝不恢复已撤销权限。
// - 撤销授权（revoke）：第一事务立即把组成员置为 revoked / 不准入（下游 ExposureScope 立刻排除），
//   第二事务才更新显示索引；中途崩溃由启动恢复补完第二步，从不重新授权。
// - 启动恢复：进程首个归属 IPC 触发一次（真正的应用启动钩子需改 lib.rs setup，见交付文档）。

const OWNERSHIP_APPLY_CHUNK: i64 = 500;
/// 同步 IPC 内最多执行分块的时间，超出后转后台线程继续，前端轮询 read_ownership_operation。
const OWNERSHIP_APPLY_IPC_BUDGET_MS: u64 = 800;
const OWNERSHIP_OPERATION_STALE_SECS: i64 = 60;
/// 同一操作反复出错（被续跑）超过该次数后标记 failed，保留暂存与审计供人工处理。
const OWNERSHIP_OPERATION_MAX_RESUMES: i64 = 5;
const OWNERSHIP_OPERATION_BUSY: &str = "OWNERSHIP_OPERATION_BUSY";
const OWNERSHIP_REVERT_DENIED: &str = "OWNERSHIP_REVERT_NOT_ALLOWED";

fn ownership_valid_operation_id(raw: &str) -> Result<String, String> {
    let operation_id = raw.trim().to_string();
    if operation_id.len() < 8
        || operation_id.len() > 80
        || !operation_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':'))
    {
        return Err(format!("{OWNERSHIP_REVIEW_INVALID}: operationId 无效"));
    }
    Ok(operation_id)
}

fn ownership_unique_error(error: rusqlite::Error) -> String {
    let text = error.to_string();
    if text.contains("ownership_operations.target_operation_id") {
        format!("{OWNERSHIP_REVERT_DENIED}: 该组操作已经撤销过（或正在撤销），不能重复撤销")
    } else if text.contains("ownership_operations.run_id") {
        format!("{OWNERSHIP_OPERATION_BUSY}: 该组已有正在分块写入的操作，请等待完成后重新预览")
    } else {
        text
    }
}

/// 状态快照：与 OWNERSHIP_DECISION_STATE_SQL 同口径；没有结论行时得到“未判断”的状态。
fn ownership_state_of(
    connection: &rusqlite::Connection,
    project_id: i64,
    asset_id: i64,
) -> Result<String, String> {
    connection
        .query_row(
            &format!("SELECT {OWNERSHIP_DECISION_STATE_SQL} FROM (SELECT 1) LEFT JOIN asset_ownership_decisions o ON o.project_id=?1 AND o.asset_id=?2"),
            params![project_id, asset_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipOperationView {
    pub operation_id: String,
    pub project_id: i64,
    pub run_id: i64,
    pub group_id: i64,
    pub kind: String,
    pub action: String,
    pub status: String,
    pub target_operation_id: String,
    pub total_items: i64,
    pub done_items: i64,
    pub applied: i64,
    pub skipped: i64,
    pub conflicted: i64,
    pub failed: i64,
    pub reasons: std::collections::BTreeMap<String, i64>,
    pub resume_count: i64,
    pub error: String,
    pub created_at: String,
    pub updated_at: String,
    /// 本进程是否正持有该操作（否则为待恢复或已结束）。
    pub owned_by_live_worker: bool,
}

fn ownership_read_operation_view(
    connection: &rusqlite::Connection,
    operation_id: &str,
) -> Result<Option<OwnershipOperationView>, String> {
    let now = ownership_now_epoch(connection)?;
    let row = connection
        .query_row(
            "SELECT operation_id,project_id,run_id,group_id,kind,action,status,target_operation_id,total_items,done_items,resume_count,error,created_at,updated_at,worker_token,heartbeat_epoch FROM ownership_operations WHERE operation_id=?1",
            [operation_id],
            |row| {
                Ok((
                    OwnershipOperationView {
                        operation_id: row.get(0)?,
                        project_id: row.get(1)?,
                        run_id: row.get(2)?,
                        group_id: row.get(3)?,
                        kind: row.get(4)?,
                        action: row.get(5)?,
                        status: row.get(6)?,
                        target_operation_id: row.get(7)?,
                        total_items: row.get(8)?,
                        done_items: row.get(9)?,
                        applied: 0,
                        skipped: 0,
                        conflicted: 0,
                        failed: 0,
                        reasons: Default::default(),
                        resume_count: row.get(10)?,
                        error: row.get(11)?,
                        created_at: row.get(12)?,
                        updated_at: row.get(13)?,
                        owned_by_live_worker: false,
                    },
                    row.get::<_, String>(14)?,
                    row.get::<_, i64>(15)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((mut view, token, heartbeat)) = row else {
        return Ok(None);
    };
    view.owned_by_live_worker = !token.is_empty()
        && token.starts_with(&ownership_token_prefix())
        && heartbeat >= now - OWNERSHIP_OPERATION_STALE_SECS;
    let mut statement = connection
        .prepare("SELECT outcome,reason,COUNT(*) FROM ownership_operation_items WHERE operation_id=?1 GROUP BY outcome,reason")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([operation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for (outcome, reason, count) in rows {
        match outcome.as_str() {
            "applied" => view.applied += count,
            "skipped" => view.skipped += count,
            "conflicted" => view.conflicted += count,
            _ => view.failed += count,
        }
        if !reason.is_empty() {
            *view.reasons.entry(reason).or_insert(0) += count;
        }
    }
    Ok(Some(view))
}

fn ownership_result_from_view(
    view: &OwnershipOperationView,
    group_version: i64,
    replayed: bool,
) -> OwnershipGroupDecisionResult {
    OwnershipGroupDecisionResult {
        operation_id: view.operation_id.clone(),
        project_id: view.project_id,
        run_id: view.run_id,
        group_id: view.group_id,
        action: view.action.clone(),
        applied: view.applied,
        skipped: view.skipped,
        conflicted: view.conflicted,
        failed: view.failed,
        reasons: view.reasons.clone(),
        group_version,
        replayed,
        status: view.status.clone(),
        total_items: view.total_items,
        done_items: view.done_items,
    }
}

fn ownership_group_version_of(
    connection: &rusqlite::Connection,
    run_id: i64,
    group_id: i64,
) -> Result<i64, String> {
    Ok(connection
        .query_row(
            "SELECT group_version FROM ownership_review_groups WHERE id=?1 AND run_id=?2",
            params![group_id, run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or(0))
}

/// 认领一个处于 applying 且无人持有（令牌为空 / 属于旧进程 / 心跳过期）的操作。
fn ownership_claim_operation(
    connection: &rusqlite::Connection,
    operation_id: &str,
) -> Result<Option<String>, String> {
    let token = ownership_worker_token();
    let now = ownership_now_epoch(connection)?;
    let claimed = connection
        .execute(
            "UPDATE ownership_operations SET worker_token=?2,heartbeat_epoch=?3,resume_count=resume_count+1,updated_at=datetime('now','localtime') WHERE operation_id=?1 AND status='applying' AND (worker_token='' OR heartbeat_epoch<?4 OR substr(worker_token,1,length(?5))<>?5)",
            params![operation_id, token, now, now - OWNERSHIP_OPERATION_STALE_SECS, ownership_token_prefix()],
        )
        .map_err(|error| error.to_string())?;
    Ok((claimed == 1).then_some(token))
}

enum OwnershipBegin {
    /// 本次调用持有该操作，可以驱动分块。
    Owned(String),
    /// 已完成（原样重放）或正由其他存活的工作线程推进。
    Existing,
}

/// 同一 operationId 已存在：payload 不同直接拒绝；applying 且无人持有时接管续跑。
fn ownership_existing_operation(
    connection: &rusqlite::Connection,
    operation_id: &str,
    payload_hash: &str,
    project_id: i64,
) -> Result<Option<OwnershipBegin>, String> {
    let Some((stored_hash, stored_project, status)) = connection
        .query_row(
            "SELECT payload_hash,project_id,status FROM ownership_operations WHERE operation_id=?1",
            [operation_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    if stored_hash != payload_hash || stored_project != project_id {
        return Err(format!("{OWNERSHIP_OPERATION_MISMATCH}: operationId {operation_id} 已用于另一份批量操作，本次未写入；请重新预览以获得新的操作编号"));
    }
    if status == "applying" {
        if let Some(token) = ownership_claim_operation(connection, operation_id)? {
            return Ok(Some(OwnershipBegin::Owned(token)));
        }
    }
    Ok(Some(OwnershipBegin::Existing))
}

/// T0：幂等、CAS、哈希校验，并把冻结的逐条计划写入暂存（与校验同一事务，计划即预览所见）。
fn ownership_begin_group_decision(
    connection: &mut rusqlite::Connection,
    input: &OwnershipGroupDecisionApply,
    operation_id: &str,
) -> Result<OwnershipBegin, String> {
    let payload_hash = ownership_operation_payload_hash(input);
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if let Some(existing) =
        ownership_existing_operation(&transaction, operation_id, &payload_hash, input.project_id)?
    {
        transaction.commit().map_err(|error| error.to_string())?;
        return Ok(existing);
    }
    let request = OwnershipGroupDecisionRequest {
        project_id: input.project_id,
        run_id: input.run_id,
        group_id: input.group_id,
        action: input.action.clone(),
        note: input.note.clone(),
    };
    let evaluation = ownership_evaluate_group(&transaction, &request)?;
    let versions = &evaluation.run.versions;
    if evaluation.group_version != input.group_version
        || versions.policy_version != input.policy_version
        || versions.directory_version != input.directory_version
        || versions.binding_version != input.binding_version
    {
        return Err(format!("{OWNERSHIP_PREVIEW_STALE}: 组版本或策略/目录/绑定版本与预览不一致（组 v{} / 预览 v{}），未写入；请重新预览", evaluation.group_version, input.group_version));
    }
    if evaluation.hash != input.expected_action_hash {
        return Err(format!("{OWNERSHIP_PREVIEW_STALE}: 预览之后成员发生删除、撤权、证据或人工结论变化，未写入；请重新预览"));
    }
    let token = ownership_worker_token();
    let now = ownership_now_epoch(&transaction)?;
    let note = ownership_normalize_note(&input.action, &input.note);
    transaction
        .execute(
            "INSERT INTO ownership_operations(operation_id,project_id,run_id,group_id,kind,action,status,payload_hash,expected_action_hash,total_items,worker_token,heartbeat_epoch,result_json) VALUES(?1,?2,?3,?4,'group_decision',?5,'applying',?6,?7,?8,?9,?10,json_object('note',?11))",
            params![operation_id, input.project_id, evaluation.run.id, input.group_id, input.action, payload_hash, input.expected_action_hash, evaluation.checks.len() as i64, token, now, note],
        )
        .map_err(ownership_unique_error)?;
    {
        let mut insert = transaction
            .prepare("INSERT INTO ownership_operation_staging(operation_id,seq,asset_id,planned,reason,expected_state,evidence_hash,old_json,evidence_json,evidence_ref) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)")
            .map_err(|error| error.to_string())?;
        for (index, check) in evaluation.checks.iter().enumerate() {
            let planned = match check.outcome {
                "applicable" => "apply",
                "skipped" => "skipped",
                _ => "conflicted",
            };
            insert
                .execute(params![
                    operation_id,
                    index as i64 + 1,
                    check.asset_id,
                    planned,
                    check.reason,
                    check.current_state,
                    check.evidence_hash,
                    check.old.to_string(),
                    check.evidence_json,
                    format!(
                        "run:{}/group:{}/evidence:{}",
                        evaluation.run.id, input.group_id, check.evidence_hash
                    )
                ])
                .map_err(|error| error.to_string())?;
        }
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(OwnershipBegin::Owned(token))
}

struct OwnershipStagedRow {
    seq: i64,
    asset_id: i64,
    planned: String,
    reason: String,
    expected_state: String,
    evidence_hash: String,
    restore: JsonValue,
    evidence_json: String,
    evidence_ref: String,
    active: bool,
    in_scope: bool,
    current_state: String,
    current_evidence: String,
    exists: bool,
    current: JsonValue,
    authorization: String,
    eligible: i64,
}

fn ownership_read_staged(
    connection: &rusqlite::Connection,
    operation_id: &str,
    project_id: i64,
    after_seq: i64,
    limit: i64,
) -> Result<Vec<OwnershipStagedRow>, String> {
    let mut statement = connection
        .prepare(&format!("SELECT s.seq,s.asset_id,s.planned,s.reason,s.expected_state,s.evidence_hash,s.restore_json,s.evidence_json,s.evidence_ref,pa.asset_id IS NOT NULL AND a.id IS NOT NULL AND COALESCE(pa.is_deleted,1)=0,CASE WHEN pa.asset_id IS NOT NULL AND a.id IS NOT NULL AND {OWNERSHIP_SCOPE_SQL} THEN 1 ELSE 0 END,{OWNERSHIP_DECISION_STATE_SQL},COALESCE(a.company,''),COALESCE(a.host,''),COALESCE(a.link,''),COALESCE(a.domain,''),COALESCE(a.title,''),COALESCE(a.ip,''),COALESCE(a.extra_json,'{{}}'),o.asset_id IS NOT NULL,COALESCE(o.status,'unreviewed'),COALESCE(o.source,''),COALESCE(o.confidence,0),COALESCE(o.authorization_status,'unknown'),COALESCE(o.exposure_eligible,0),COALESCE(o.reason,'') FROM ownership_operation_staging s LEFT JOIN project_assets pa ON pa.project_id=?2 AND pa.asset_id=s.asset_id LEFT JOIN assets a ON a.id=s.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=?2 AND o.asset_id=s.asset_id WHERE s.operation_id=?1 AND s.seq>?3 ORDER BY s.seq LIMIT ?4"))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![operation_id, project_id, after_seq, limit],
            |row| {
                let restore: String = row.get(6)?;
                let exists = row.get::<_, i64>(19)? != 0;
                let authorization: String = row.get(23)?;
                let eligible: i64 = row.get(24)?;
                Ok(OwnershipStagedRow {
                    seq: row.get(0)?,
                    asset_id: row.get(1)?,
                    planned: row.get(2)?,
                    reason: row.get(3)?,
                    expected_state: row.get(4)?,
                    evidence_hash: row.get(5)?,
                    restore: serde_json::from_str(&restore).unwrap_or(JsonValue::Null),
                    evidence_json: row.get(7)?,
                    evidence_ref: row.get(8)?,
                    active: row.get::<_, i64>(9)? != 0,
                    in_scope: row.get::<_, i64>(10)? != 0,
                    current_state: row.get(11)?,
                    current_evidence: ownership_evidence_hash(
                        &row.get::<_, String>(12)?,
                        &row.get::<_, String>(13)?,
                        &row.get::<_, String>(14)?,
                        &row.get::<_, String>(15)?,
                        &row.get::<_, String>(16)?,
                        &row.get::<_, String>(17)?,
                        &row.get::<_, String>(18)?,
                    ),
                    exists,
                    current: serde_json::json!({"exists": exists, "status": row.get::<_, String>(20)?, "source": row.get::<_, String>(21)?, "confidence": row.get::<_, i64>(22)?, "authorizationStatus": authorization, "exposureEligible": eligible, "reason": row.get::<_, String>(25)?}),
                    authorization,
                    eligible,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

struct OwnershipOperationHead {
    project_id: i64,
    run_id: i64,
    group_id: i64,
    kind: String,
    action: String,
    status: String,
    token: String,
    last_seq: i64,
    note: String,
}

fn ownership_operation_head(
    connection: &rusqlite::Connection,
    operation_id: &str,
) -> Result<Option<OwnershipOperationHead>, String> {
    connection
        .query_row(
            "SELECT project_id,run_id,group_id,kind,action,status,worker_token,last_seq,COALESCE(json_extract(result_json,'$.note'),'') FROM ownership_operations WHERE operation_id=?1",
            [operation_id],
            |row| {
                Ok(OwnershipOperationHead {
                    project_id: row.get(0)?,
                    run_id: row.get(1)?,
                    group_id: row.get(2)?,
                    kind: row.get(3)?,
                    action: row.get(4)?,
                    status: row.get(5)?,
                    token: row.get(6)?,
                    last_seq: row.get(7)?,
                    note: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())
}

/// 审计行：(资产, 结果, 原因, 旧值, 新值, 证据引用, 写前状态, 写后状态)。
type OwnershipItemRow = (i64, &'static str, String, String, String, String, String, String);

/// 一个分块：同一 IMMEDIATE 事务内读取计划行与当前状态、逐行守卫写入、写审计、推进游标。
fn ownership_operation_step(
    connection: &mut rusqlite::Connection,
    operation_id: &str,
    token: &str,
    chunk: i64,
) -> Result<OwnershipStep, String> {
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let Some(head) = ownership_operation_head(&transaction, operation_id)? else {
        return Ok(OwnershipStep::Lost);
    };
    if head.status != "applying" || head.token != token {
        return Ok(OwnershipStep::Lost);
    }
    let rows = ownership_read_staged(
        &transaction,
        operation_id,
        head.project_id,
        head.last_seq,
        chunk.max(1),
    )?;
    if rows.is_empty() {
        drop(transaction);
        return ownership_finalize_operation(connection, operation_id, token);
    }
    let revert = head.kind == "group_revert";
    let narrowing = matches!(head.action.as_str(), "third_party" | "excluded");
    let status = match head.action.as_str() {
        "confirm_ownership" => "confirmed",
        other => other,
    };
    let confidence = if head.action == "related" { 70 } else { 100 };
    let reason_text = format!(
        "{}（复核组 #{}，快照 #{}，操作 {operation_id}）",
        if revert {
            head.note.clone()
        } else {
            ownership_normalize_note(&head.action, &head.note)
        },
        head.group_id,
        head.run_id
    );
    let mut items: Vec<OwnershipItemRow> = Vec::with_capacity(rows.len());
    let next_seq = rows.last().map(|row| row.seq).unwrap_or(head.last_seq);
    for row in &rows {
        let current_json = row.current.to_string();
        let unchanged = |outcome: &'static str, reason: String| -> OwnershipItemRow {
            (
                row.asset_id,
                outcome,
                reason,
                current_json.clone(),
                "{}".into(),
                row.evidence_ref.clone(),
                row.current_state.clone(),
                row.current_state.clone(),
            )
        };
        if row.planned != "apply" {
            let outcome = if row.planned == "skipped" {
                "skipped"
            } else {
                "conflicted"
            };
            items.push(unchanged(outcome, row.reason.clone()));
            continue;
        }
        let conflict = if !row.active {
            Some("deleted_or_unlinked")
        } else if row.current_state != row.expected_state {
            Some(if revert {
                "changed_after_operation"
            } else {
                "decision_changed"
            })
        } else if !revert && !row.in_scope {
            Some("left_review_scope")
        } else if !revert && row.current_evidence != row.evidence_hash {
            Some("evidence_changed")
        } else {
            None
        };
        if let Some(reason) = conflict {
            items.push(unchanged("conflicted", reason.into()));
            continue;
        }
        let (written, new_json, item_reason) = if revert {
            ownership_revert_row(&transaction, head.project_id, row, &reason_text)?
        } else {
            // 只写归属事实；收窄动作沿用既有“拒绝 + 不准入”。非收窄动作的授权与准入保持原值，绝不置为允许。
            let written = if row.exists {
                transaction
                    .execute(
                        &format!("UPDATE asset_ownership_decisions AS o SET status=?3,confidence=?4,source='manual',reason=?5,authorization_status=CASE WHEN ?6=1 THEN 'denied' ELSE o.authorization_status END,exposure_eligible=CASE WHEN ?6=1 THEN 0 ELSE o.exposure_eligible END,updated_at=datetime('now','localtime') WHERE o.project_id=?1 AND o.asset_id=?2 AND {OWNERSHIP_DECISION_STATE_SQL}=?7"),
                        params![head.project_id, row.asset_id, status, confidence, reason_text, i64::from(narrowing), row.expected_state],
                    )
                    .map_err(|error| error.to_string())?
            } else {
                transaction
                    .execute(
                        "INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,authorization_status,exposure_eligible,source,reason,evidence_json,negative_evidence_json) VALUES(?1,?2,?3,?4,?5,0,'manual',?6,?7,'[]') ON CONFLICT(project_id,asset_id) DO NOTHING",
                        params![head.project_id, row.asset_id, status, confidence, if narrowing { "denied" } else { "unknown" }, reason_text, row.evidence_json],
                    )
                    .map_err(|error| error.to_string())?
            };
            let (authorization, eligible) = if narrowing {
                ("denied".to_string(), 0)
            } else if row.exists {
                (row.authorization.clone(), row.eligible)
            } else {
                ("unknown".to_string(), 0)
            };
            if written == 1 {
                transaction
                    .execute(
                        "INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'ownership',?3)",
                        params![head.project_id, row.asset_id, format!("{status}: {reason_text}")],
                    )
                    .map_err(|error| error.to_string())?;
            }
            (
                written,
                serde_json::json!({"status": status, "source": "manual", "confidence": confidence, "authorizationStatus": authorization, "exposureEligible": eligible, "reason": reason_text}),
                String::new(),
            )
        };
        if written != 1 {
            items.push(unchanged("failed", "write_guard_missed".into()));
            continue;
        }
        let after = ownership_state_of(&transaction, head.project_id, row.asset_id)?;
        items.push((
            row.asset_id,
            "applied",
            item_reason,
            current_json.clone(),
            new_json.to_string(),
            row.evidence_ref.clone(),
            row.current_state.clone(),
            after,
        ));
    }
    {
        let mut insert = transaction
            .prepare("INSERT INTO ownership_operation_items(operation_id,asset_id,outcome,reason,old_json,new_json,evidence_ref,before_state,after_state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)")
            .map_err(|error| error.to_string())?;
        for (asset_id, outcome, reason, old, new, evidence_ref, before, after) in &items {
            insert
                .execute(params![
                    operation_id,
                    asset_id,
                    outcome,
                    reason,
                    old,
                    new,
                    evidence_ref,
                    before,
                    after
                ])
                .map_err(|error| error.to_string())?;
        }
    }
    let now = ownership_now_epoch(&transaction)?;
    let advanced = transaction
        .execute(
            "UPDATE ownership_operations SET last_seq=?3,done_items=done_items+?4,heartbeat_epoch=?5,updated_at=datetime('now','localtime') WHERE operation_id=?1 AND worker_token=?2 AND status='applying' AND last_seq=?6",
            params![operation_id, token, next_seq, items.len() as i64, now, head.last_seq],
        )
        .map_err(|error| error.to_string())?;
    if advanced != 1 {
        // 游标已被他人推进：整个分块回滚，不重复写。
        return Ok(OwnershipStep::Lost);
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(OwnershipStep::Continued)
}

/// 撤销一行：恢复旧结论；旧值中的 allowed / 准入不恢复（只会更窄，不会更宽）。
fn ownership_revert_row(
    connection: &rusqlite::Connection,
    project_id: i64,
    row: &OwnershipStagedRow,
    reason_text: &str,
) -> Result<(usize, JsonValue, String), String> {
    let restore = &row.restore;
    let existed = restore
        .get("exists")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let old_authorization = restore
        .get("authorizationStatus")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown")
        .to_string();
    let old_eligible = restore
        .get("exposureEligible")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    let mut not_restored = false;
    let authorization = if old_authorization == "allowed" && row.authorization != "allowed" {
        not_restored = true;
        "unknown".to_string()
    } else {
        old_authorization
    };
    if old_eligible > row.eligible {
        not_restored = true;
    }
    let eligible = old_eligible.min(row.eligible);
    let written = if !existed {
        connection
            .execute(
                &format!("DELETE FROM asset_ownership_decisions AS o WHERE o.project_id=?1 AND o.asset_id=?2 AND {OWNERSHIP_DECISION_STATE_SQL}=?3"),
                params![project_id, row.asset_id, row.expected_state],
            )
            .map_err(|error| error.to_string())?
    } else {
        connection
            .execute(
                &format!("UPDATE asset_ownership_decisions AS o SET status=?3,source=?4,confidence=?5,reason=?6,authorization_status=?7,exposure_eligible=?8,updated_at=datetime('now','localtime') WHERE o.project_id=?1 AND o.asset_id=?2 AND {OWNERSHIP_DECISION_STATE_SQL}=?9"),
                params![
                    project_id,
                    row.asset_id,
                    restore.get("status").and_then(JsonValue::as_str).unwrap_or("unreviewed"),
                    restore.get("source").and_then(JsonValue::as_str).unwrap_or("auto"),
                    restore.get("confidence").and_then(JsonValue::as_i64).unwrap_or(0),
                    restore.get("reason").and_then(JsonValue::as_str).unwrap_or(""),
                    authorization,
                    eligible,
                    row.expected_state
                ],
            )
            .map_err(|error| error.to_string())?
    };
    if written == 1 {
        connection
            .execute(
                "INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'ownership',?3)",
                params![project_id, row.asset_id, format!("revert: {reason_text}")],
            )
            .map_err(|error| error.to_string())?;
    }
    let new = if existed {
        serde_json::json!({"exists": true, "status": restore.get("status"), "source": restore.get("source"), "confidence": restore.get("confidence"), "authorizationStatus": authorization, "exposureEligible": eligible, "reason": restore.get("reason")})
    } else {
        serde_json::json!({"exists": false})
    };
    Ok((
        written,
        new,
        if not_restored {
            "authorization_not_restored".into()
        } else {
            String::new()
        },
    ))
}

/// 收尾：全部计划行都已处理后，按审计行真实计数生成结果；组版本 +1（使旧预览失效）并清理暂存。
fn ownership_finalize_operation(
    connection: &mut rusqlite::Connection,
    operation_id: &str,
    token: &str,
) -> Result<OwnershipStep, String> {
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let Some(head) = ownership_operation_head(&transaction, operation_id)? else {
        return Ok(OwnershipStep::Lost);
    };
    if head.status != "applying" || head.token != token {
        return Ok(OwnershipStep::Lost);
    }
    let (total, done, items, remaining): (i64, i64, i64, i64) = transaction
        .query_row(
            "SELECT total_items,done_items,(SELECT COUNT(*) FROM ownership_operation_items WHERE operation_id=?1),(SELECT COUNT(*) FROM ownership_operation_staging WHERE operation_id=?1 AND seq>?2) FROM ownership_operations WHERE operation_id=?1",
            params![operation_id, head.last_seq],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|error| error.to_string())?;
    if remaining != 0 || done != total || items != total {
        return Err(format!(
            "操作 {operation_id} 对账失败：计划 {total}，已处理 {done}，审计 {items}，剩余 {remaining}"
        ));
    }
    let group_sql = if head.kind == "group_revert" {
        "UPDATE ownership_review_groups SET group_version=group_version+1,applied_operations=MAX(applied_operations-1,0) WHERE id=?1 AND run_id=?2"
    } else {
        "UPDATE ownership_review_groups SET group_version=group_version+1,applied_operations=applied_operations+1 WHERE id=?1 AND run_id=?2"
    };
    transaction
        .execute(group_sql, params![head.group_id, head.run_id])
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE ownership_operations SET status='completed',worker_token='',updated_at=datetime('now','localtime') WHERE operation_id=?1",
            [operation_id],
        )
        .map_err(|error| error.to_string())?;
    let view = ownership_read_operation_view(&transaction, operation_id)?.ok_or("操作不存在")?;
    let group_version = ownership_group_version_of(&transaction, head.run_id, head.group_id)?;
    let result = ownership_result_from_view(&view, group_version, false);
    let mut stored = serde_json::to_value(&result).map_err(|error| error.to_string())?;
    stored["note"] = JsonValue::String(head.note.clone());
    transaction
        .execute(
            "UPDATE ownership_operations SET applied=?2,skipped=?3,conflicted=?4,failed=?5,result_json=?6 WHERE operation_id=?1",
            params![operation_id, result.applied, result.skipped, result.conflicted, result.failed, stored.to_string()],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "DELETE FROM ownership_operation_staging WHERE operation_id=?1",
            [operation_id],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(OwnershipStep::Completed)
}

/// 驱动分块直至完成、超出时间预算或 max_steps（测试用于模拟崩溃）。出错时释放令牌供恢复接管。
fn ownership_drive_operation(
    connection: &mut rusqlite::Connection,
    operation_id: &str,
    token: &str,
    chunk: i64,
    deadline: Option<Instant>,
    max_steps: Option<usize>,
) -> Result<OwnershipStep, String> {
    let mut steps = 0usize;
    loop {
        if max_steps.is_some_and(|limit| steps >= limit)
            || deadline.is_some_and(|limit| Instant::now() >= limit)
        {
            return Ok(OwnershipStep::Continued);
        }
        steps += 1;
        match ownership_operation_step(connection, operation_id, token, chunk) {
            Ok(OwnershipStep::Continued) => continue,
            Ok(outcome) => return Ok(outcome),
            Err(error) => {
                let _ = connection.execute(
                    "UPDATE ownership_operations SET worker_token='',error=?3,status=CASE WHEN resume_count>=?4 THEN 'failed' ELSE status END,updated_at=datetime('now','localtime') WHERE operation_id=?1 AND worker_token=?2",
                    params![operation_id, token, error.chars().take(500).collect::<String>(), OWNERSHIP_OPERATION_MAX_RESUMES],
                );
                return Err(error);
            }
        }
    }
}

fn ownership_operation_worker(db_path: PathBuf, operation_id: String, token: String) {
    if let Ok(mut connection) = db::open(&db_path) {
        let _ = ownership_drive_operation(
            &mut connection,
            &operation_id,
            &token,
            OWNERSHIP_APPLY_CHUNK,
            None,
            None,
        );
    }
}

fn ownership_current_result(
    connection: &rusqlite::Connection,
    operation_id: &str,
    replayed: bool,
) -> Result<OwnershipGroupDecisionResult, String> {
    let view = ownership_read_operation_view(connection, operation_id)?.ok_or("操作不存在")?;
    let group_version = ownership_group_version_of(connection, view.run_id, view.group_id)?;
    Ok(ownership_result_from_view(&view, group_version, replayed))
}

/// 在时间预算内驱动；返回 (当前结果, 需要后台继续时的令牌)。budget=None 时同步做完。
fn ownership_drive_begin(
    connection: &mut rusqlite::Connection,
    operation_id: &str,
    begin: OwnershipBegin,
    budget: Option<Duration>,
    chunk: i64,
) -> Result<(OwnershipGroupDecisionResult, Option<String>), String> {
    let token = match begin {
        OwnershipBegin::Existing => {
            let stored: Option<String> = connection
                .query_row(
                    "SELECT result_json FROM ownership_operations WHERE operation_id=?1 AND status='completed'",
                    [operation_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            let mut result = match stored
                .and_then(|text| serde_json::from_str::<OwnershipGroupDecisionResult>(&text).ok())
            {
                Some(result) => result,
                None => ownership_current_result(connection, operation_id, true)?,
            };
            result.replayed = true;
            return Ok((result, None));
        }
        OwnershipBegin::Owned(token) => token,
    };
    let deadline = budget.map(|limit| Instant::now() + limit);
    let outcome =
        ownership_drive_operation(connection, operation_id, &token, chunk, deadline, None)?;
    let result = ownership_current_result(connection, operation_id, false)?;
    Ok((
        result,
        (outcome == OwnershipStep::Continued).then_some(token),
    ))
}

fn apply_ownership_group_decision_budgeted(
    connection: &mut rusqlite::Connection,
    input: OwnershipGroupDecisionApply,
    budget: Option<Duration>,
    chunk: i64,
) -> Result<(OwnershipGroupDecisionResult, Option<String>), String> {
    let operation_id = ownership_valid_operation_id(&input.operation_id)?;
    let begin = ownership_begin_group_decision(connection, &input, &operation_id)?;
    ownership_drive_begin(connection, &operation_id, begin, budget, chunk)
}

/// 同步做完整个操作并返回最终结果（测试用；与 O2 合同一致）。
#[cfg(test)]
fn apply_ownership_group_decision_for_connection(
    connection: &mut rusqlite::Connection,
    input: OwnershipGroupDecisionApply,
) -> Result<OwnershipGroupDecisionResult, String> {
    apply_ownership_group_decision_budgeted(connection, input, None, OWNERSHIP_APPLY_CHUNK)
        .map(|(result, _)| result)
}

#[tauri::command]
pub fn apply_ownership_group_decision(
    state: State<AppState>,
    input: OwnershipGroupDecisionApply,
) -> Result<OwnershipGroupDecisionResult, String> {
    ownership_ensure_startup_recovery(&state.db_path);
    let operation_id = input.operation_id.trim().to_string();
    let (result, token) = apply_ownership_group_decision_budgeted(
        &mut db::open(&state.db_path)?,
        input,
        Some(Duration::from_millis(OWNERSHIP_APPLY_IPC_BUDGET_MS)),
        OWNERSHIP_APPLY_CHUNK,
    )?;
    if let Some(token) = token {
        let db_path = state.db_path.clone();
        thread::spawn(move || ownership_operation_worker(db_path, operation_id, token));
    }
    Ok(result)
}

fn read_ownership_operation_for_connection(
    connection: &rusqlite::Connection,
    project_id: i64,
    operation_id: &str,
) -> Result<OwnershipOperationView, String> {
    ownership_read_operation_view(connection, operation_id)?
        .filter(|view| view.project_id == project_id)
        .ok_or_else(|| format!("{OWNERSHIP_REVIEW_INVALID}: 操作不属于当前工作空间"))
}

#[tauri::command]
pub fn read_ownership_operation(
    state: State<AppState>,
    project_id: i64,
    operation_id: String,
) -> Result<OwnershipOperationView, String> {
    ownership_ensure_startup_recovery(&state.db_path);
    read_ownership_operation_for_connection(&db::open(&state.db_path)?, project_id, &operation_id)
}

// ---------------------------------------------------------------------------------------------
// 组级撤销（revert）：撤销一次已完成的组归属操作。
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OwnershipRevertRequest {
    pub project_id: i64,
    pub target_operation_id: String,
    pub note: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OwnershipRevertApply {
    pub operation_id: String,
    pub project_id: i64,
    pub target_operation_id: String,
    pub note: String,
    pub expected_revert_hash: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipRevertPreview {
    pub project_id: i64,
    pub target_operation_id: String,
    pub target_action: String,
    pub run_id: i64,
    pub group_id: i64,
    pub note: String,
    pub applied_in_target: i64,
    pub revertible: i64,
    pub conflicted: i64,
    pub reasons: std::collections::BTreeMap<String, i64>,
    /// 旧值含“允许/准入”而当前已不是：撤销不会恢复这些权限。
    pub authorization_not_restored: i64,
    pub expected_revert_hash: String,
    pub notes: Vec<String>,
}

struct OwnershipRevertCheck {
    asset_id: i64,
    planned: &'static str,
    reason: &'static str,
    expected_state: String,
    restore_json: String,
    current_json: String,
    not_restored: bool,
}

struct OwnershipRevertEvaluation {
    run_id: i64,
    group_id: i64,
    action: String,
    checks: Vec<OwnershipRevertCheck>,
    hash: String,
}

fn ownership_revert_note(note: &str) -> String {
    let note: String = note.trim().chars().take(500).collect();
    if note.is_empty() {
        "组复核：撤销此前的组操作".into()
    } else {
        note
    }
}

type OwnershipTargetRow = (i64, i64, i64, String, String, String);

fn ownership_evaluate_revert(
    connection: &rusqlite::Connection,
    request: &OwnershipRevertRequest,
) -> Result<OwnershipRevertEvaluation, String> {
    let target = request.target_operation_id.trim();
    let (project_id, run_id, group_id, kind, action, status): OwnershipTargetRow = connection
        .query_row(
            "SELECT project_id,run_id,group_id,kind,action,status FROM ownership_operations WHERE operation_id=?1",
            [target],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("{OWNERSHIP_REVIEW_INVALID}: 要撤销的操作不存在"))?;
    if project_id != request.project_id {
        return Err(format!("{OWNERSHIP_REVIEW_INVALID}: 要撤销的操作不属于当前工作空间"));
    }
    if kind != "group_decision" || status != "completed" {
        return Err(format!("{OWNERSHIP_REVERT_DENIED}: 只能撤销已完成的组归属操作（当前 {kind}/{status}）"));
    }
    let already: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM ownership_operations WHERE kind='group_revert' AND target_operation_id=?1)",
            [target],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if already {
        return Err(format!("{OWNERSHIP_REVERT_DENIED}: 该组操作已经撤销过（或正在撤销）"));
    }
    let mut statement = connection
        .prepare(&format!("SELECT i.asset_id,i.after_state,i.old_json,pa.asset_id IS NOT NULL AND COALESCE(pa.is_deleted,1)=0,{OWNERSHIP_DECISION_STATE_SQL},o.asset_id IS NOT NULL,COALESCE(o.status,'unreviewed'),COALESCE(o.source,''),COALESCE(o.confidence,0),COALESCE(o.authorization_status,'unknown'),COALESCE(o.exposure_eligible,0),COALESCE(o.reason,'') FROM ownership_operation_items i LEFT JOIN project_assets pa ON pa.project_id=?2 AND pa.asset_id=i.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=?2 AND o.asset_id=i.asset_id WHERE i.operation_id=?1 AND i.outcome='applied' ORDER BY i.asset_id"))
        .map_err(|error| error.to_string())?;
    let mut rows = statement
        .query(params![target, project_id])
        .map_err(|error| error.to_string())?;
    let note = ownership_revert_note(&request.note);
    let mut digest = Sha256::new();
    digest.update(format!("{project_id}|{target}|{note}\n").as_bytes());
    let mut checks = Vec::new();
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let text = |index: usize| -> Result<String, String> {
            row.get::<_, String>(index).map_err(|error| error.to_string())
        };
        let number = |index: usize| -> Result<i64, String> {
            row.get::<_, i64>(index).map_err(|error| error.to_string())
        };
        let asset_id = number(0)?;
        let after_state = text(1)?;
        let restore_json = text(2)?;
        let active = number(3)? != 0;
        let current_state = text(4)?;
        let authorization = text(9)?;
        let eligible = number(10)?;
        let current_json = serde_json::json!({"exists": number(5)? != 0, "status": text(6)?, "source": text(7)?, "confidence": number(8)?, "authorizationStatus": authorization, "exposureEligible": eligible, "reason": text(11)?}).to_string();
        let restore: JsonValue = serde_json::from_str(&restore_json).unwrap_or(JsonValue::Null);
        let not_restored = (restore.get("authorizationStatus").and_then(JsonValue::as_str)
            == Some("allowed")
            && authorization != "allowed")
            || restore
                .get("exposureEligible")
                .and_then(JsonValue::as_i64)
                .unwrap_or(0)
                > eligible;
        let (planned, reason) = if !active {
            ("conflicted", "deleted_or_unlinked")
        } else if after_state.is_empty() || current_state != after_state {
            ("conflicted", "changed_after_operation")
        } else {
            ("apply", "")
        };
        digest.update(format!("{asset_id}|{planned}|{reason}|{current_state}\n").as_bytes());
        checks.push(OwnershipRevertCheck {
            asset_id,
            planned,
            reason,
            expected_state: after_state,
            restore_json,
            current_json,
            not_restored: planned == "apply" && not_restored,
        });
    }
    let hash = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(OwnershipRevertEvaluation {
        run_id,
        group_id,
        action,
        checks,
        hash,
    })
}

fn preview_ownership_group_revert_for_connection(
    connection: &rusqlite::Connection,
    request: OwnershipRevertRequest,
) -> Result<OwnershipRevertPreview, String> {
    let evaluation = ownership_evaluate_revert(connection, &request)?;
    let mut reasons = std::collections::BTreeMap::new();
    let (mut revertible, mut conflicted, mut not_restored) = (0i64, 0i64, 0i64);
    for check in &evaluation.checks {
        if check.planned == "apply" {
            revertible += 1;
            not_restored += i64::from(check.not_restored);
        } else {
            conflicted += 1;
            *reasons.entry(check.reason.to_string()).or_insert(0) += 1;
        }
    }
    Ok(OwnershipRevertPreview {
        project_id: request.project_id,
        target_operation_id: request.target_operation_id.trim().to_string(),
        target_action: evaluation.action,
        run_id: evaluation.run_id,
        group_id: evaluation.group_id,
        note: ownership_revert_note(&request.note),
        applied_in_target: evaluation.checks.len() as i64,
        revertible,
        conflicted,
        reasons,
        authorization_not_restored: not_restored,
        expected_revert_hash: evaluation.hash,
        notes: vec![
            "只撤销该操作写入、且之后未被其他操作改动的成员；之后有变化的成员保留现状".into(),
            "撤销不会恢复任何授权或暴露面准入：旧值是“允许/准入”而当前不是的，恢复为未授权、不准入".into(),
        ],
    })
}

#[tauri::command]
pub fn preview_ownership_group_revert(
    state: State<AppState>,
    input: OwnershipRevertRequest,
) -> Result<OwnershipRevertPreview, String> {
    preview_ownership_group_revert_for_connection(&db::open(&state.db_path)?, input)
}

fn ownership_revert_payload_hash(input: &OwnershipRevertApply) -> String {
    ownership_sha(
        &serde_json::json!([
            "revert",
            input.project_id,
            input.target_operation_id.trim(),
            ownership_revert_note(&input.note),
            input.expected_revert_hash
        ])
        .to_string(),
    )
}

fn ownership_begin_revert(
    connection: &mut rusqlite::Connection,
    input: &OwnershipRevertApply,
    operation_id: &str,
) -> Result<OwnershipBegin, String> {
    let payload_hash = ownership_revert_payload_hash(input);
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if let Some(existing) =
        ownership_existing_operation(&transaction, operation_id, &payload_hash, input.project_id)?
    {
        transaction.commit().map_err(|error| error.to_string())?;
        return Ok(existing);
    }
    let evaluation = ownership_evaluate_revert(
        &transaction,
        &OwnershipRevertRequest {
            project_id: input.project_id,
            target_operation_id: input.target_operation_id.clone(),
            note: input.note.clone(),
        },
    )?;
    if evaluation.hash != input.expected_revert_hash {
        return Err(format!("{OWNERSHIP_PREVIEW_STALE}: 撤销预览之后成员已变化，未写入；请重新预览撤销"));
    }
    let token = ownership_worker_token();
    let now = ownership_now_epoch(&transaction)?;
    transaction
        .execute(
            "INSERT INTO ownership_operations(operation_id,project_id,run_id,group_id,kind,action,status,target_operation_id,payload_hash,expected_action_hash,total_items,worker_token,heartbeat_epoch,result_json) VALUES(?1,?2,?3,?4,'group_revert','revert','applying',?5,?6,?7,?8,?9,?10,json_object('note',?11))",
            params![operation_id, input.project_id, evaluation.run_id, evaluation.group_id, input.target_operation_id.trim(), payload_hash, input.expected_revert_hash, evaluation.checks.len() as i64, token, now, ownership_revert_note(&input.note)],
        )
        .map_err(ownership_unique_error)?;
    {
        let mut insert = transaction
            .prepare("INSERT INTO ownership_operation_staging(operation_id,seq,asset_id,planned,reason,expected_state,old_json,restore_json,evidence_ref) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)")
            .map_err(|error| error.to_string())?;
        for (index, check) in evaluation.checks.iter().enumerate() {
            insert
                .execute(params![
                    operation_id,
                    index as i64 + 1,
                    check.asset_id,
                    check.planned,
                    check.reason,
                    check.expected_state,
                    check.current_json,
                    check.restore_json,
                    format!("revert:{}", input.target_operation_id.trim())
                ])
                .map_err(|error| error.to_string())?;
        }
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(OwnershipBegin::Owned(token))
}

fn apply_ownership_group_revert_budgeted(
    connection: &mut rusqlite::Connection,
    input: OwnershipRevertApply,
    budget: Option<Duration>,
    chunk: i64,
) -> Result<(OwnershipGroupDecisionResult, Option<String>), String> {
    let operation_id = ownership_valid_operation_id(&input.operation_id)?;
    let begin = ownership_begin_revert(connection, &input, &operation_id)?;
    ownership_drive_begin(connection, &operation_id, begin, budget, chunk)
}

#[tauri::command]
pub fn apply_ownership_group_revert(
    state: State<AppState>,
    input: OwnershipRevertApply,
) -> Result<OwnershipGroupDecisionResult, String> {
    ownership_ensure_startup_recovery(&state.db_path);
    let operation_id = input.operation_id.trim().to_string();
    let (result, token) = apply_ownership_group_revert_budgeted(
        &mut db::open(&state.db_path)?,
        input,
        Some(Duration::from_millis(OWNERSHIP_APPLY_IPC_BUDGET_MS)),
        OWNERSHIP_APPLY_CHUNK,
    )?;
    if let Some(token) = token {
        let db_path = state.db_path.clone();
        thread::spawn(move || ownership_operation_worker(db_path, operation_id, token));
    }
    Ok(result)
}

// ---------------------------------------------------------------------------------------------
// 撤销授权（revoke）：先阻断下游，再更新显示索引。
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OwnershipRevokeRequest {
    pub operation_id: String,
    pub project_id: i64,
    pub run_id: i64,
    pub group_id: i64,
    pub note: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipRevokeResult {
    pub operation_id: String,
    pub project_id: i64,
    pub run_id: i64,
    pub group_id: i64,
    /// blocked：下游已阻断、显示索引待更新；completed：两步都已完成。
    pub status: String,
    pub revoked: i64,
    pub already_revoked: i64,
    pub without_decision: i64,
    pub group_version: i64,
    pub replayed: bool,
}

fn ownership_revoke_payload_hash(input: &OwnershipRevokeRequest) -> String {
    ownership_sha(
        &serde_json::json!([
            "revoke",
            input.project_id,
            input.run_id,
            input.group_id,
            input.note.trim()
        ])
        .to_string(),
    )
}

fn ownership_revoke_result(
    connection: &rusqlite::Connection,
    operation_id: &str,
    replayed: bool,
) -> Result<OwnershipRevokeResult, String> {
    let view = ownership_read_operation_view(connection, operation_id)?.ok_or("操作不存在")?;
    Ok(OwnershipRevokeResult {
        operation_id: view.operation_id.clone(),
        project_id: view.project_id,
        run_id: view.run_id,
        group_id: view.group_id,
        status: view.status.clone(),
        revoked: view.applied,
        already_revoked: view.reasons.get("already_revoked").copied().unwrap_or(0),
        without_decision: view.reasons.get("no_decision_row").copied().unwrap_or(0),
        group_version: ownership_group_version_of(connection, view.run_id, view.group_id)?,
        replayed,
    })
}

/// 第一步（单个 IMMEDIATE 事务）：记录操作与逐条旧值，并以集合式 UPDATE 立即把组成员置为
/// revoked / 不准入。提交后 ExposureScope::load 即排除这些资产（not eligible → authorization_revoked）。
/// 返回已存在操作的状态（幂等重放）；新建时返回 None。
fn ownership_revoke_block(
    connection: &mut rusqlite::Connection,
    input: &OwnershipRevokeRequest,
    operation_id: &str,
) -> Result<Option<String>, String> {
    let payload_hash = ownership_revoke_payload_hash(input);
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if let Some((stored_hash, stored_project, status)) = transaction
        .query_row(
            "SELECT payload_hash,project_id,status FROM ownership_operations WHERE operation_id=?1",
            [operation_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
    {
        if stored_hash != payload_hash || stored_project != input.project_id {
            return Err(format!("{OWNERSHIP_OPERATION_MISMATCH}: operationId {operation_id} 已用于另一份操作，本次未写入"));
        }
        transaction.commit().map_err(|error| error.to_string())?;
        return Ok(Some(status));
    }
    // 撤销授权是收窄动作：快照过期也允许执行（只要快照属于本项目且成员索引仍在）。
    ownership_snapshot_for(&transaction, input.project_id, input.run_id)?;
    let exists: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM ownership_review_groups WHERE id=?1 AND run_id=?2 AND project_id=?3)",
            params![input.group_id, input.run_id, input.project_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !exists {
        return Err(format!("{OWNERSHIP_REVIEW_INVALID}: 复核组不属于该快照或当前工作空间"));
    }
    let note: String = input.note.trim().chars().take(500).collect();
    let note = if note.is_empty() {
        "组复核：撤销授权".to_string()
    } else {
        note
    };
    transaction
        .execute(
            "INSERT INTO ownership_operations(operation_id,project_id,run_id,group_id,kind,action,status,payload_hash,expected_action_hash,result_json) VALUES(?1,?2,?3,?4,'authorization_revoke','revoke_authorization','blocked',?5,'',json_object('note',?6))",
            params![operation_id, input.project_id, input.run_id, input.group_id, payload_hash, note],
        )
        .map_err(ownership_unique_error)?;
    transaction
        .execute(
            &format!("INSERT INTO ownership_operation_items(operation_id,asset_id,outcome,reason,old_json,new_json,evidence_ref,before_state,after_state) SELECT ?1,m.asset_id,CASE WHEN o.asset_id IS NULL OR (o.authorization_status='revoked' AND o.exposure_eligible=0) THEN 'skipped' ELSE 'applied' END,CASE WHEN o.asset_id IS NULL THEN 'no_decision_row' WHEN o.authorization_status='revoked' AND o.exposure_eligible=0 THEN 'already_revoked' ELSE '' END,json_object('exists',o.asset_id IS NOT NULL,'status',COALESCE(o.status,'unreviewed'),'source',COALESCE(o.source,''),'confidence',COALESCE(o.confidence,0),'authorizationStatus',COALESCE(o.authorization_status,'unknown'),'exposureEligible',COALESCE(o.exposure_eligible,0),'reason',COALESCE(o.reason,'')),'{{}}','revoke:run:'||m.run_id||'/group:'||m.group_id,{OWNERSHIP_DECISION_STATE_SQL},{OWNERSHIP_DECISION_STATE_SQL} FROM ownership_review_group_members m LEFT JOIN asset_ownership_decisions o ON o.project_id=m.project_id AND o.asset_id=m.asset_id WHERE m.run_id=?2 AND m.group_id=?3 AND m.project_id=?4"),
            params![operation_id, input.run_id, input.group_id, input.project_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE asset_ownership_decisions SET authorization_status='revoked',exposure_eligible=0,updated_at=datetime('now','localtime') WHERE project_id=?2 AND asset_id IN (SELECT asset_id FROM ownership_operation_items WHERE operation_id=?1 AND outcome='applied')",
            params![operation_id, input.project_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            &format!("UPDATE ownership_operation_items AS i SET new_json=json_object('authorizationStatus','revoked','exposureEligible',0),after_state=(SELECT {OWNERSHIP_DECISION_STATE_SQL} FROM asset_ownership_decisions o WHERE o.project_id=?2 AND o.asset_id=i.asset_id) WHERE i.operation_id=?1 AND i.outcome='applied'"),
            params![operation_id, input.project_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "INSERT INTO asset_events(project_id,asset_id,event_type,summary) SELECT ?2,asset_id,'authorization',?3 FROM ownership_operation_items WHERE operation_id=?1 AND outcome='applied'",
            params![operation_id, input.project_id, format!("revoked: {note}（操作 {operation_id}）")],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE ownership_operations SET total_items=(SELECT COUNT(*) FROM ownership_operation_items WHERE operation_id=?1),done_items=(SELECT COUNT(*) FROM ownership_operation_items WHERE operation_id=?1),updated_at=datetime('now','localtime') WHERE operation_id=?1",
            [operation_id],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(None)
}

/// 第二步：更新显示索引（组版本 +1 使旧预览失效，记录撤销数），操作标记 completed。只碰显示数据。
fn ownership_revoke_display(
    connection: &mut rusqlite::Connection,
    operation_id: &str,
) -> Result<(), String> {
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let Some(head) = ownership_operation_head(&transaction, operation_id)? else {
        return Ok(());
    };
    if head.kind != "authorization_revoke" || head.status != "blocked" {
        return Ok(());
    }
    let revoked: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM ownership_operation_items WHERE operation_id=?1 AND outcome='applied'",
            [operation_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE ownership_review_groups SET group_version=group_version+1,authorization_revoked=authorization_revoked+?3 WHERE id=?1 AND run_id=?2",
            params![head.group_id, head.run_id, revoked],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE ownership_operations SET status='completed',applied=?2,skipped=total_items-?2,updated_at=datetime('now','localtime') WHERE operation_id=?1 AND status='blocked'",
            params![operation_id, revoked],
        )
        .map_err(|error| error.to_string())?;
    let result = ownership_revoke_result(&transaction, operation_id, false)?;
    let mut stored = serde_json::to_value(&result).map_err(|error| error.to_string())?;
    stored["note"] = JsonValue::String(head.note);
    transaction
        .execute(
            "UPDATE ownership_operations SET result_json=?2 WHERE operation_id=?1",
            params![operation_id, stored.to_string()],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(())
}

fn revoke_ownership_group_authorization_for_connection(
    connection: &mut rusqlite::Connection,
    input: OwnershipRevokeRequest,
    stop_after_block: bool,
) -> Result<OwnershipRevokeResult, String> {
    let operation_id = ownership_valid_operation_id(&input.operation_id)?;
    let existing = ownership_revoke_block(connection, &input, &operation_id)?;
    if !stop_after_block {
        ownership_revoke_display(connection, &operation_id)?;
    }
    ownership_revoke_result(connection, &operation_id, existing.is_some())
}

#[tauri::command]
pub fn revoke_ownership_group_authorization(
    state: State<AppState>,
    input: OwnershipRevokeRequest,
) -> Result<OwnershipRevokeResult, String> {
    ownership_ensure_startup_recovery(&state.db_path);
    revoke_ownership_group_authorization_for_connection(
        &mut db::open(&state.db_path)?,
        input,
        false,
    )
}

// ---------------------------------------------------------------------------------------------
// 启动恢复
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipRecoveryReport {
    pub runs_resumed: Vec<i64>,
    pub operations_resumed: Vec<String>,
    pub revocations_completed: Vec<String>,
    pub errors: Vec<String>,
}

/// 接管所有未完成的分析任务与组操作（同步执行，供后台线程与测试调用）：
/// - 分析：queued，或 running 且令牌属于旧进程 / 心跳过期 → 认领后从 last_asset_id 继续；
/// - 组操作 applying：从 last_seq 继续，逐行守卫保证已撤权 / 已变化的成员不会被覆盖；
/// - 撤销授权 blocked：只补做显示索引，决不重新授权。
fn ownership_recover_for_connection(
    connection: &mut rusqlite::Connection,
) -> Result<OwnershipRecoveryReport, String> {
    let mut report = OwnershipRecoveryReport::default();
    let now = ownership_now_epoch(connection)?;
    let prefix = ownership_token_prefix();
    let runs: Vec<i64> = {
        let mut statement = connection
            .prepare("SELECT id FROM ownership_analysis_runs WHERE cancel_requested=0 AND (status='queued' OR (status='running' AND (heartbeat_epoch<?1 OR substr(worker_token,1,length(?2))<>?2))) ORDER BY id")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![now - OWNERSHIP_ANALYSIS_STALE_SECS, prefix], |row| {
                row.get(0)
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    for run_id in runs {
        match ownership_claim_run(connection, run_id) {
            Ok(Some(token)) => {
                report.runs_resumed.push(run_id);
                if let Err(error) =
                    ownership_drive_run(connection, run_id, &token, OWNERSHIP_ANALYSIS_CHUNK, None)
                {
                    report.errors.push(format!("run {run_id}: {error}"));
                }
            }
            Ok(None) => {}
            Err(error) => report.errors.push(format!("run {run_id}: {error}")),
        }
    }
    let operations: Vec<(String, String)> = {
        let mut statement = connection
            .prepare("SELECT operation_id,status FROM ownership_operations WHERE status='blocked' OR (status='applying' AND (worker_token='' OR heartbeat_epoch<?1 OR substr(worker_token,1,length(?2))<>?2)) ORDER BY created_at,operation_id")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(
                params![now - OWNERSHIP_OPERATION_STALE_SECS, prefix],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    for (operation_id, status) in operations {
        if status == "blocked" {
            match ownership_revoke_display(connection, &operation_id) {
                Ok(()) => report.revocations_completed.push(operation_id),
                Err(error) => report.errors.push(format!("{operation_id}: {error}")),
            }
            continue;
        }
        match ownership_claim_operation(connection, &operation_id) {
            Ok(Some(token)) => {
                report.operations_resumed.push(operation_id.clone());
                if let Err(error) = ownership_drive_operation(
                    connection,
                    &operation_id,
                    &token,
                    OWNERSHIP_APPLY_CHUNK,
                    None,
                    None,
                ) {
                    report.errors.push(format!("{operation_id}: {error}"));
                }
            }
            Ok(None) => {}
            Err(error) => report.errors.push(format!("{operation_id}: {error}")),
        }
    }
    Ok(report)
}

/// 每个进程只触发一次：首个归属 IPC 到来时在后台线程执行恢复，不阻塞该 IPC。
fn ownership_ensure_startup_recovery(db_path: &Path) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_ok() {
        let path = db_path.to_path_buf();
        thread::spawn(move || {
            if let Ok(mut connection) = db::open(&path) {
                let _ = ownership_recover_for_connection(&mut connection);
            }
        });
    }
}
