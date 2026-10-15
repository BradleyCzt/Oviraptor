// O3 第 5 项：把逐条“确认归属并授权”拆成两步合同。
// - confirm_asset_ownership：只写归属事实（confirmed / manual / 100），保留每行现有授权状态与暴露面准入，
//   不创建任何执行授权；可选学习域名规则，默认精确主机（rule_type='host'），
//   “包含子域”必须由人明确选择，且拒绝公共后缀、托管平台根、共享域名与 IP。
// - authorize_asset_ownership：授权仍走既有受控授权合同 apply_asset_ownership_decisions（status='confirmed'
//   → allowed / exposure_eligible=1，IMMEDIATE 事务 + 项目/成员/范围/逐字段 CAS 校验，代码未改）；
//   本入口只额外要求页面所见的每一行已经是人工确认归属（manual + confirmed），CAS 保证事务内仍然如此。
// - 路径范围不提供：下游暴露面授权以 origin 为粒度（ExposureGrant 为 scheme+host+port），
//   路径级授权无法在下游表达，详见交付文档。

const OWNERSHIP_AUTHORIZATION_REQUIRES_CONFIRMED: &str =
    "OWNERSHIP_AUTHORIZATION_REQUIRES_CONFIRMED";
const OWNERSHIP_RULE_SCOPE_DENIED: &str = "OWNERSHIP_RULE_SCOPE_NOT_ALLOWED";
const OWNERSHIP_RULE_SCOPES: [&str; 3] = ["none", "exact_host", "subdomains"];

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipConfirmInput {
    pub selections: Vec<AssetSelection>,
    #[serde(default)]
    pub note: String,
    /// none（默认，不学习规则）/ exact_host（精确主机）/ subdomains（包含子域，需人明确选择）。
    #[serde(default)]
    pub domain_rule_scope: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipAuthorizeInput {
    pub selections: Vec<AssetSelection>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipConfirmResult {
    pub changed: i64,
    /// 写入后的逐行结论版本：向导第二步“授权”直接用它作为 CAS 期望值。
    pub expectations: Vec<AssetOwnershipDecisionExpectation>,
    pub rule_scope: String,
    pub learned_rules: Vec<String>,
}

/// 学习一条域名规则。exact_host → rule_type='host'（只匹配该主机）；subdomains → rule_type='domain'
/// （匹配该主机及其子域），后者拒绝 IP、公共后缀（含 private 区托管平台根）与共享域名。
fn ownership_learn_host_rule(
    connection: &rusqlite::Connection,
    project_id: i64,
    asset_id: i64,
    action: &str,
    scope: &str,
) -> Result<Option<String>, String> {
    let (link, host): (String, String) = connection
        .query_row(
            "SELECT link,host FROM assets WHERE id=?1",
            [asset_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or_default();
    let pattern = ownership_endpoint_host(&link, &host);
    if pattern.is_empty() || pattern.parse::<std::net::IpAddr>().is_ok() {
        if scope == "subdomains" {
            return Err(format!("{OWNERSHIP_RULE_SCOPE_DENIED}: 资产 #{asset_id} 没有可泛化的域名（IP 或空主机），未写入任何结论"));
        }
        return Ok(None);
    }
    let rule_type = match scope {
        "exact_host" => "host",
        "subdomains" => {
            let parts = ownership_domain_parts(&pattern);
            if parts.as_ref().and_then(|parts| parts.registrable.as_ref()).is_none() {
                return Err(format!("{OWNERSHIP_RULE_SCOPE_DENIED}: {pattern} 是公共后缀或托管平台根，不能按子域泛化，未写入任何结论"));
            }
            let profile_shared: Vec<String> = read_ownership_profile(connection, project_id)?.shared_domains;
            let directory_shared: Vec<String> = {
                let mut statement = connection
                    .prepare("SELECT value FROM org_entity_identifiers WHERE kind='shared_domain'")
                    .map_err(|error| error.to_string())?;
                let rows = statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|error| error.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                rows
            };
            if let Some(shared) = profile_shared
                .iter()
                .chain(directory_shared.iter())
                .find(|shared| domain_matches(shared, &pattern) || domain_matches(&pattern, shared))
            {
                return Err(format!("{OWNERSHIP_RULE_SCOPE_DENIED}: {pattern} 与共享域名 {shared} 重叠，不能按子域泛化（共享域名只按精确主机），未写入任何结论"));
            }
            "domain"
        }
        _ => return Ok(None),
    };
    connection
        .execute(
            "INSERT INTO asset_ownership_rules(project_id,rule_type,pattern,action,source) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(project_id,rule_type,pattern,action) DO UPDATE SET enabled=1,source=excluded.source,updated_at=datetime('now','localtime')",
            params![project_id, rule_type, pattern, action, OWNERSHIP_LEARNED_RULE_SOURCE_V2],
        )
        .map_err(|error| error.to_string())?;
    Ok(Some(format!("{rule_type}:{pattern}")))
}

fn ownership_selection_ids(
    selections: &[AssetSelection],
    expected_project_id: i64,
    expected_decisions: usize,
) -> Result<Vec<i64>, String> {
    if selections.is_empty() || selections.len() > 20_000 || expected_decisions > 20_000 {
        return Err("请选择 1–20000 条资产".into());
    }
    if expected_project_id <= 0 {
        return Err(format!("{OWNERSHIP_DECISION_PRECONDITION}: 缺少页面当前工作空间，未写入任何结论"));
    }
    let foreign = selections
        .iter()
        .filter(|item| item.project_id != expected_project_id)
        .count();
    if foreign > 0 {
        return Err(format!("{OWNERSHIP_DECISION_PRECONDITION}: 批量操作已拒绝，未写入任何结论：{foreign} 条选择不属于当前工作空间（可能是切换项目前的旧选择），请刷新后重新选择"));
    }
    let mut unique = HashSet::new();
    Ok(selections
        .iter()
        .map(|item| item.asset_id)
        .filter(|asset_id| unique.insert(*asset_id))
        .collect())
}

fn ownership_expectation_of(
    connection: &rusqlite::Connection,
    project_id: i64,
    asset_id: i64,
) -> Result<AssetOwnershipDecisionExpectation, String> {
    connection
        .query_row(
            "SELECT COALESCE(o.status,'unreviewed'),COALESCE(o.source,''),COALESCE(o.confidence,0),COALESCE(o.authorization_status,'unknown'),COALESCE(o.exposure_eligible,0),COALESCE(o.reason,'') FROM (SELECT 1) LEFT JOIN asset_ownership_decisions o ON o.project_id=?1 AND o.asset_id=?2",
            params![project_id, asset_id],
            |row| {
                Ok(AssetOwnershipDecisionExpectation {
                    asset_id,
                    status: row.get(0)?,
                    source: row.get(1)?,
                    confidence: row.get(2)?,
                    authorization_status: row.get(3)?,
                    exposure_eligible: row.get::<_, i64>(4)? != 0,
                    reason: row.get(5)?,
                })
            },
        )
        .map_err(|error| error.to_string())
}

/// 第一步：只确认归属。与 O0 合同同样的前置校验（项目、成员资格、逐字段 CAS）；不要求范围，因为不授予任何权限。
fn confirm_asset_ownership_for_connection(
    connection: &mut rusqlite::Connection,
    input: OwnershipConfirmInput,
    expected_project_id: i64,
    expected_decisions: Vec<AssetOwnershipDecisionExpectation>,
) -> Result<OwnershipConfirmResult, String> {
    let scope = if input.domain_rule_scope.trim().is_empty() {
        "none".to_string()
    } else {
        input.domain_rule_scope.trim().to_string()
    };
    if !OWNERSHIP_RULE_SCOPES.contains(&scope.as_str()) {
        return Err(format!("{OWNERSHIP_RULE_SCOPE_DENIED}: 不支持的规则范围 {scope}（路径范围不可用：下游授权以 origin 为粒度）"));
    }
    let asset_ids = ownership_selection_ids(&input.selections, expected_project_id, expected_decisions.len())?;
    let expectations: HashMap<i64, AssetOwnershipDecisionExpectation> = expected_decisions
        .into_iter()
        .map(|item| (item.asset_id, item))
        .collect();
    let note: String = input.note.trim().chars().take(500).collect();
    let note = if note.is_empty() {
        "人工确认：属于当前主体（仅确认归属，未授权）".to_string()
    } else {
        note
    };
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    ownership_check_decision_preconditions(&transaction, expected_project_id, &asset_ids, &expectations, false)?;
    let mut changed = 0i64;
    let mut learned = Vec::new();
    for asset_id in &asset_ids {
        // 授权状态与暴露面准入保持原值（新行为 unknown / 0）：归属结论本身不创建执行授权。
        changed += transaction
            .execute(
                "INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,authorization_status,exposure_eligible,source,reason,evidence_json,negative_evidence_json) VALUES(?1,?2,'confirmed',100,'unknown',0,'manual',?3,'[]','[]') ON CONFLICT(project_id,asset_id) DO UPDATE SET status='confirmed',confidence=100,source='manual',reason=excluded.reason,updated_at=datetime('now','localtime')",
                params![expected_project_id, asset_id, note],
            )
            .map_err(|error| error.to_string())? as i64;
        transaction
            .execute(
                "INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'ownership',?3)",
                params![expected_project_id, asset_id, format!("confirmed (ownership only): {note}")],
            )
            .map_err(|error| error.to_string())?;
        if scope != "none" {
            if let Some(rule) = ownership_learn_host_rule(&transaction, expected_project_id, *asset_id, "approve", &scope)? {
                if !learned.contains(&rule) {
                    learned.push(rule);
                }
            }
        }
    }
    let mut written = Vec::with_capacity(asset_ids.len());
    for asset_id in &asset_ids {
        written.push(ownership_expectation_of(&transaction, expected_project_id, *asset_id)?);
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(OwnershipConfirmResult {
        changed,
        expectations: written,
        rule_scope: scope,
        learned_rules: learned,
    })
}

#[tauri::command]
pub fn confirm_asset_ownership(
    state: State<AppState>,
    input: OwnershipConfirmInput,
    expected_project_id: i64,
    expected_decisions: Vec<AssetOwnershipDecisionExpectation>,
) -> Result<OwnershipConfirmResult, String> {
    confirm_asset_ownership_for_connection(
        &mut db::open(&state.db_path)?,
        input,
        expected_project_id,
        expected_decisions,
    )
}

/// 第二步：授权。只接受页面所见为人工确认归属的行，然后原样调用既有受控授权合同。
fn authorize_asset_ownership_for_connection(
    connection: &mut rusqlite::Connection,
    input: OwnershipAuthorizeInput,
    expected_project_id: i64,
    expected_decisions: Vec<AssetOwnershipDecisionExpectation>,
) -> Result<i64, String> {
    let asset_ids = ownership_selection_ids(&input.selections, expected_project_id, expected_decisions.len())?;
    let by_asset: HashMap<i64, &AssetOwnershipDecisionExpectation> = expected_decisions
        .iter()
        .map(|item| (item.asset_id, item))
        .collect();
    let unconfirmed = asset_ids
        .iter()
        .filter(|asset_id| {
            by_asset
                .get(asset_id)
                .is_none_or(|item| item.status != "confirmed" || item.source != "manual")
        })
        .count();
    if unconfirmed > 0 {
        return Err(format!("{OWNERSHIP_AUTHORIZATION_REQUIRES_CONFIRMED}: 授权已拒绝，未写入：{unconfirmed} 条资产尚未人工确认归属；请先执行“确认归属”，再单独授权"));
    }
    let note: String = input.note.trim().chars().take(500).collect();
    apply_asset_ownership_decisions(
        connection,
        AssetOwnershipDecisionInput {
            selections: input.selections,
            status: "confirmed".into(),
            note: if note.is_empty() {
                "人工授权：已确认归属的资产授予暴露面准入".into()
            } else {
                note
            },
            learn_domain_rule: false,
        },
        expected_project_id,
        expected_decisions,
    )
}

#[tauri::command]
pub fn authorize_asset_ownership(
    state: State<AppState>,
    input: OwnershipAuthorizeInput,
    expected_project_id: i64,
    expected_decisions: Vec<AssetOwnershipDecisionExpectation>,
) -> Result<i64, String> {
    authorize_asset_ownership_for_connection(
        &mut db::open(&state.db_path)?,
        input,
        expected_project_id,
        expected_decisions,
    )
}
