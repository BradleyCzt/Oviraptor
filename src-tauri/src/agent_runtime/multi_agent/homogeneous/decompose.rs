//! SPEC §4 / DESIGN §7：确定性降级切分。主管连续失败后禁止再问模型，改走这里。
//!
//! 重叠判定比 SPEC 伪代码的裸 `starts_with` 更严（只在 `/` 段边界成立时才算包含），
//! 只会多拒不会少拒；候选、排序与上限全部确定，可跨进程回放。

use super::decision::DecisionError;
use super::{reason, store, types};
use rusqlite::Connection;

const MAX_DETERMINISTIC_SLICES: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct Scope {
    pub kind: String,
    pub value: String,
}

impl Scope {
    pub fn new(kind: &str, value: &str) -> Self {
        Self {
            kind: kind.to_string(),
            value: value.to_string(),
        }
    }
}

/// SPEC §4：path 只在段边界上算父子关系；其它 kind 只比等值；未知 kind 防御性视为重叠。
pub fn scopes_overlap(first: &Scope, second: &Scope) -> bool {
    if first.kind != second.kind {
        return false;
    }
    match first.kind.as_str() {
        "path_prefix" => {
            first.value == second.value
                || segment_prefix_of(&first.value, &second.value)
                || segment_prefix_of(&second.value, &first.value)
        }
        "host" | "asset_id" | "source_type" => first.value == second.value,
        _ => true,
    }
}

fn segment_prefix_of(shorter: &str, longer: &str) -> bool {
    if shorter == "/" {
        return longer != "/";
    }
    let Some(rest) = longer.strip_prefix(shorter) else {
        return false;
    };
    !rest.is_empty() && rest.starts_with('/')
}

/// 规范化：去 query/fragment、统一小写 percent-free 路径段前缀、补前导 `/`、
/// 折叠重复 `/`、去掉尾随 `/`（根路径除外）、截断到 512。
pub fn normalize_path_prefix(raw: &str) -> String {
    let cut = raw
        .find(['?', '#'])
        .map(|index| &raw[..index])
        .unwrap_or(raw);
    let trimmed = cut.trim();
    if trimmed.is_empty() {
        return "/".to_string();
    }
    let without_duplicates = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<&str>>();
    let mut joined = String::new();
    for segment in without_duplicates {
        joined.push('/');
        joined.push_str(segment);
    }
    if joined.is_empty() {
        return "/".to_string();
    }
    joined.chars().take(types::MAX_SCOPE_VALUE_LEN).collect()
}

pub fn host_of(raw: &str) -> String {
    let without_scheme = raw.split_once("://").map(|rest| rest.1).unwrap_or(raw);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme);
    let without_credentials = host.rsplit('@').next().unwrap_or(host);
    let portless = without_credentials
        .split_once(':')
        .map(|pair| pair.0)
        .unwrap_or(without_credentials);
    portless.trim().to_ascii_lowercase()
}

/// 确定性 slice id：由 scope 三元组派生，与模型给出的 id 无关（DESIGN §2.4 纪律）。
pub fn slice_id_for(task_id: &str, scope_kind: &str, scope_value: &str, revision: i64) -> String {
    format!(
        "slc-{}",
        &crate::agent_runtime::store::stable_hash(&format!(
            "{task_id}:{scope_kind}:{scope_value}:{revision}"
        ))[..24]
    )
}

fn draft_for(task: &types::HomogeneousTask, kind: &str, value: &str) -> types::SliceDraft {
    types::SliceDraft {
        slice_id: slice_id_for(&task.task_id, kind, value, task.evidence_revision_base),
        scope_kind: kind.to_string(),
        scope_value: value.to_string(),
        lane: "read_only_analysis".to_string(),
        evidence_revision_base: task.evidence_revision_base,
        worker_capability_profile_id: types::PROFILE_HOMOGENEOUS_V1.to_string(),
        max_retries: 2,
    }
}

/// SPEC §4 的候选优先级：asset_id → path_prefix → host → source_type。
/// 每支都用确定性的取数与排序，保证同输入同输出（DESIGN §6.7 回放要求）。
pub fn deterministic_candidates(
    connection: &Connection,
    task: &types::HomogeneousTask,
) -> Result<Vec<types::SliceDraft>, DecisionError> {
    // 1) 冻结资产（sentinel_targets.asset_id：扫描目标表，不属 exposure 业务代码）
    let assets: Vec<String> = connection
        .prepare(
            "SELECT DISTINCT CAST(asset_id AS TEXT) FROM sentinel_targets WHERE scan_id=?1 AND asset_id IS NOT NULL ORDER BY 1",
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?
        .query_map([task.scan_id.as_str()], |row| row.get::<_, String>(0))
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    if !assets.is_empty() {
        return Ok(assets
            .into_iter()
            .take(MAX_DETERMINISTIC_SLICES)
            .map(|asset| draft_for(task, "asset_id", &asset))
            .collect());
    }

    // 2) 已发现的 path 前缀（只读侦察材料：agent_evidence_nodes kind='endpoint'）
    let paths: Vec<String> = connection
        .prepare(
            "SELECT DISTINCT COALESCE(json_extract(payload_json,'$.path'), json_extract(payload_json,'$.url')) AS discovered \
             FROM agent_evidence_nodes WHERE root_run_id=?1 AND kind='endpoint' AND discovered IS NOT NULL ORDER BY discovered",
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?
        .query_map([task.root_run_id.as_str()], |row| row.get::<_, Option<String>>(0))
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?
        .filter_map(Result::ok)
        .flatten()
        .map(|raw| normalize_path_prefix(&raw))
        .collect();
    if !paths.is_empty() {
        let mut ordered = drop_overlapping(
            &paths
                .into_iter()
                .map(|path| Scope::new("path_prefix", &path))
                .collect::<Vec<Scope>>(),
        );
        ordered.sort_by(|left, right| {
            right
                .value
                .len()
                .cmp(&left.value.len())
                .then_with(|| left.value.cmp(&right.value))
        });
        return Ok(ordered
            .into_iter()
            .take(MAX_DETERMINISTIC_SLICES)
            .map(|scope| draft_for(task, "path_prefix", &scope.value))
            .collect());
    }

    // 3) 冻结主机
    let hosts: Vec<String> = connection
        .prepare("SELECT url FROM sentinel_targets WHERE scan_id=?1 ORDER BY url")
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?
        .query_map([task.scan_id.as_str()], |row| row.get::<_, String>(0))
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    let mut unique_hosts: Vec<String> = Vec::new();
    for raw in hosts {
        let host = host_of(&raw);
        if !host.is_empty() && !unique_hosts.contains(&host) {
            unique_hosts.push(host);
        }
    }
    if !unique_hosts.is_empty() {
        return Ok(unique_hosts
            .into_iter()
            .take(MAX_DETERMINISTIC_SLICES)
            .map(|host| draft_for(task, "host", &host))
            .collect());
    }

    // 4) 来源类型：Nest 侧无现成集合（现状 source_types 属 appsec 校验域，语义不同），
    //    Phase 1 不启用该分支，返回空候选 → 调用方走 PauseForUser。
    Ok(Vec::new())
}

/// SPEC §4 drop_overlapping：更长的 path 优先保留，平长度按字典序，结果稳定。
pub fn drop_overlapping(scopes: &[Scope]) -> Vec<Scope> {
    let mut ordered = scopes.to_vec();
    ordered.sort_by(|left, right| {
        right
            .value
            .len()
            .cmp(&left.value.len())
            .then_with(|| left.value.cmp(&right.value))
    });
    let mut kept: Vec<Scope> = Vec::new();
    for candidate in ordered {
        if kept
            .iter()
            .any(|existing| scopes_overlap(existing, &candidate))
        {
            continue;
        }
        kept.push(candidate);
    }
    kept.sort_by(|left, right| left.value.cmp(&right.value));
    kept
}

#[derive(Clone, Debug, PartialEq)]
pub enum DecomposeOutcome {
    /// 降级成功：切片已按确定性算法落库（decomposition_model='deterministic'）。
    Degraded(Vec<String>),
    /// 无候选可切分：任务保持原状态并提示用户给 path/asset（不假装成功）。
    PauseForUser(String),
    /// §J：所有冻结方向终态时通过既有 stop 收尾，禁止空候选反复付费。
    Stopped(String),
}

/// DESIGN §7.4/§7.5：降级分解 + 标注 + 串行化（Phase 1 只派第一个 pending 切片）。
pub fn deterministic_decompose(
    connection: &Connection,
    task: &types::HomogeneousTask,
) -> Result<DecomposeOutcome, DecisionError> {
    let drafts = deterministic_candidates(connection, task)?;
    if drafts.is_empty() {
        return Ok(DecomposeOutcome::PauseForUser(
            "无法确定性切分，请用户指定 path 或 asset".to_string(),
        ));
    }
    let ids = store::insert_slices(connection, &task.task_id, &drafts, "deterministic")?;
    store::set_task_state(
        connection,
        &task.task_id,
        "decomposed",
        task.decided_by_lease_epoch,
    )?;
    store::set_task_degraded(connection, &task.task_id, reason::DEGRADED_PATH_USED)?;
    // Phase 1 串行：把该 root_run 的只读 lane 配额压到 1。
    store::upsert_quota(connection, &task.root_run_id, "read_only_analysis", 1)?;
    Ok(DecomposeOutcome::Degraded(ids))
}
