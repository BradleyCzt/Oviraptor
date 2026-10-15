fn clean_ownership_list(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty() && value.chars().count() <= 240)
        .filter(|value| seen.insert(value.clone()))
        .take(500)
        .collect()
}

pub(crate) fn normalize_ownership_domain(value: &str) -> String {
    let mut value = value.trim().to_lowercase();
    if let Some((_, rest)) = value.split_once("://") {
        value = rest.to_string();
    }
    value = value
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_string();
    if value.matches(':').count() == 1 {
        value = value.split(':').next().unwrap_or("").to_string();
    }
    value.trim_start_matches("www.").to_string()
}

pub(crate) fn domain_matches(host: &str, rule: &str) -> bool {
    let host = normalize_ownership_domain(host);
    let rule = normalize_ownership_domain(rule);
    !host.is_empty() && !rule.is_empty() && (host == rule || host.ends_with(&format!(".{rule}")))
}

/// O1：目录域名默认精确主机（§8），不做子域泛化。
fn host_equals_rule(host: &str, rule: &str) -> bool {
    let host = normalize_ownership_domain(host);
    !host.is_empty() && host == normalize_ownership_domain(rule)
}

fn text_matches_term(text: &str, term: &str) -> bool {
    let term = term.trim().to_lowercase();
    term.chars().count() >= 2 && text.to_lowercase().contains(&term)
}

fn network_only_ownership_candidate(company:&str,domain:&str,ip:&str,candidate_domain:&str)->bool{
    company.contains('/')||company.trim().to_uppercase().starts_with("AS")||(domain.is_empty()&&!ip.is_empty()&&candidate_domain.parse::<std::net::IpAddr>().is_ok())
}

const OWNERSHIP_SCOPE_SQL:&str="pa.is_deleted=0 AND COALESCE(pa.decision,'pending') NOT IN ('rejected','not_applicable') AND COALESCE(a.probe_outcome,'') IN ('','alive_clean','web_alive','web_restricted','browser_render_required','virtual_host_required') AND COALESCE(a.content_category,'') NOT IN ('gambling','porn','custom_rule')";

fn ownership_profile_from_row(row: &Row<'_>) -> rusqlite::Result<AssetOwnershipProfile> {
    let strings = |raw: String| {
        serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default()
    };
    Ok(AssetOwnershipProfile {
        project_id: row.get(0)?,
        legal_name: row.get(1)?,
        jurisdiction: row.get(2)?,
        jurisdictions: strings(row.get(3)?),
        excluded_jurisdictions: strings(row.get(4)?),
        aliases: strings(row.get(5)?),
        approved_domains: strings(row.get(6)?),
        shared_domains: strings(row.get(7)?),
        excluded_names: strings(row.get(8)?),
        excluded_domains: strings(row.get(9)?),
        notes: row.get(10)?,
        policy_version: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

const OWNERSHIP_PROFILE_SELECT: &str = "SELECT project_id,legal_name,jurisdiction,jurisdictions_json,excluded_jurisdictions_json,aliases_json,approved_domains_json,shared_domains_json,excluded_names_json,excluded_domains_json,notes,policy_version,updated_at FROM asset_ownership_profiles WHERE project_id=?1";

/// O0：档案保存冲突的稳定错误码，前端据此保留草稿并提示载入最新版本。
const OWNERSHIP_PROFILE_CONFLICT: &str = "OWNERSHIP_PROFILE_VERSION_CONFLICT";
/// O0：批量归属结论的前置条件（项目、成员资格、候选资格、结论版本）未满足。
const OWNERSHIP_DECISION_PRECONDITION: &str = "OWNERSHIP_DECISION_PRECONDITION_FAILED";

fn read_ownership_profile(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<AssetOwnershipProfile, String> {
    if let Some(profile) = connection
        .query_row(OWNERSHIP_PROFILE_SELECT, [project_id], ownership_profile_from_row)
        .optional()
        .map_err(|error| error.to_string())?
    {
        return Ok(profile);
    }
    connection
        .query_row("SELECT 1 FROM projects WHERE id=?1", [project_id], |row| row.get::<_, i64>(0))
        .map_err(|_| "工作空间不存在".to_string())?;
    // 未保存档案：updated_at 为空即“尚无已保存版本”，保存时 expectedVersion 应传 0。
    // O1：项目标题不一定是法人名称（例如“中国移动历史资产”），不再预填 legal_name；
    // 标题只在 preview_project_ownership_suggestions 中作为待确认建议出现。
    Ok(AssetOwnershipProfile {
        project_id,
        legal_name: String::new(),
        jurisdictions: Vec::new(),
        excluded_jurisdictions: Vec::new(),
        aliases: Vec::new(),
        approved_domains: Vec::new(),
        shared_domains: Vec::new(),
        excluded_names: Vec::new(),
        excluded_domains: Vec::new(),
        jurisdiction: String::new(),
        notes: String::new(),
        policy_version: 1,
        updated_at: String::new(),
    })
}

#[tauri::command]
pub fn get_asset_ownership_profile(
    state: State<AppState>,
    project_id: i64,
) -> Result<AssetOwnershipProfile, String> {
    // O1 预填优先级：已保存档案 → （前端草稿）→ 已确认组织目录 → 空；项目标题不作为法人名称。
    // 目录预填不改变版本：未保存时 updated_at 仍为空，保存的 expectedVersion 仍为 0。
    Ok(ownership_bootstrap_for_connection(&db::open(&state.db_path)?, project_id)?.profile)
}

fn normalize_ownership_profile_input(input: &mut AssetOwnershipProfile) {
    input.legal_name = input.legal_name.trim().chars().take(240).collect();
    input.jurisdiction = input.jurisdiction.trim().chars().take(120).collect();
    input.jurisdictions = clean_ownership_list(std::mem::take(&mut input.jurisdictions));
    input.excluded_jurisdictions = clean_ownership_list(std::mem::take(&mut input.excluded_jurisdictions));
    input.aliases = clean_ownership_list(std::mem::take(&mut input.aliases));
    input.approved_domains = clean_ownership_list(std::mem::take(&mut input.approved_domains))
        .into_iter()
        .map(|value| normalize_ownership_domain(&value))
        .filter(|value| !value.is_empty())
        .collect();
    input.shared_domains = clean_ownership_list(std::mem::take(&mut input.shared_domains)).into_iter().map(|value| normalize_ownership_domain(&value)).filter(|value|!value.is_empty()).collect();
    input.excluded_names = clean_ownership_list(std::mem::take(&mut input.excluded_names));
    input.excluded_domains = clean_ownership_list(std::mem::take(&mut input.excluded_domains))
        .into_iter()
        .map(|value| normalize_ownership_domain(&value))
        .filter(|value| !value.is_empty())
        .collect();
    input.notes = input.notes.trim().chars().take(2000).collect();
}

/// O0 保存 CAS：`expected_version` 是页面加载档案时看到的 policy_version，0 表示页面基于“尚未保存”的预填。
/// 版本不一致直接拒绝且不写入；读取当前版本、写入与回读同在一个 IMMEDIATE 事务内，回传的版本就是本次写入的版本。
fn save_ownership_profile_for_connection(
    connection: &mut rusqlite::Connection,
    mut input: AssetOwnershipProfile,
    expected_version: i64,
) -> Result<AssetOwnershipProfile, String> {
    normalize_ownership_profile_input(&mut input);
    if input.legal_name.is_empty() {
        return Err("请填写目标主体名称".into());
    }
    if expected_version < 0 {
        return Err("归属档案版本无效，未写入".into());
    }
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let project_exists: bool = transaction
        .query_row("SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)", [input.project_id], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if !project_exists {
        return Err("工作空间不存在，未写入".into());
    }
    let current_version: i64 = transaction
        .query_row("SELECT policy_version FROM asset_ownership_profiles WHERE project_id=?1", [input.project_id], |row| row.get(0))
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or(0);
    if current_version != expected_version {
        let current = if current_version == 0 { "尚未保存".to_string() } else { format!("v{current_version}") };
        let seen = if expected_version == 0 { "未保存的预填".to_string() } else { format!("v{expected_version}") };
        return Err(format!("{OWNERSHIP_PROFILE_CONFLICT}: 归属档案已被其他操作更新（当前 {current}，页面基于 {seen}），本次未写入；页面草稿仍保留，请核对最新档案后再保存"));
    }
    transaction
        .execute(
            "INSERT INTO asset_ownership_profiles(project_id,legal_name,jurisdiction,jurisdictions_json,excluded_jurisdictions_json,aliases_json,approved_domains_json,shared_domains_json,excluded_names_json,excluded_domains_json,notes,policy_version) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,1) ON CONFLICT(project_id) DO UPDATE SET legal_name=excluded.legal_name,jurisdiction=excluded.jurisdiction,jurisdictions_json=excluded.jurisdictions_json,excluded_jurisdictions_json=excluded.excluded_jurisdictions_json,aliases_json=excluded.aliases_json,approved_domains_json=excluded.approved_domains_json,shared_domains_json=excluded.shared_domains_json,excluded_names_json=excluded.excluded_names_json,excluded_domains_json=excluded.excluded_domains_json,notes=excluded.notes,policy_version=asset_ownership_profiles.policy_version+1,updated_at=datetime('now','localtime')",
            params![input.project_id,input.legal_name,input.jurisdiction,serde_json::to_string(&input.jurisdictions).unwrap_or_else(|_| "[]".into()),serde_json::to_string(&input.excluded_jurisdictions).unwrap_or_else(|_| "[]".into()),serde_json::to_string(&input.aliases).unwrap_or_else(|_| "[]".into()),serde_json::to_string(&input.approved_domains).unwrap_or_else(|_| "[]".into()),serde_json::to_string(&input.shared_domains).unwrap_or_else(|_| "[]".into()),serde_json::to_string(&input.excluded_names).unwrap_or_else(|_| "[]".into()),serde_json::to_string(&input.excluded_domains).unwrap_or_else(|_| "[]".into()),input.notes],
        )
        .map_err(|error| error.to_string())?;
    let saved = read_ownership_profile(&transaction, input.project_id)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(saved)
}

#[tauri::command]
pub fn save_asset_ownership_profile(
    state: State<AppState>,
    input: AssetOwnershipProfile,
    expected_version: i64,
) -> Result<AssetOwnershipProfile, String> {
    save_ownership_profile_for_connection(&mut db::open(&state.db_path)?, input, expected_version)
}

fn asset_ownership_summary(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<AssetOwnershipSummary, String> {
    connection
        .query_row(
            &format!("SELECT COUNT(*),COALESCE(SUM(CASE WHEN COALESCE(o.status,'unreviewed')='unreviewed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN o.status='attributed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN o.status='confirmed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN o.status='related' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN o.status='third_party' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN o.status='excluded' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN o.exposure_eligible=1 THEN 1 ELSE 0 END),0) FROM project_assets pa JOIN assets a ON a.id=pa.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id=?1 AND {OWNERSHIP_SCOPE_SQL}"),
            [project_id],
            |row| Ok(AssetOwnershipSummary { all: row.get(0)?, unreviewed: row.get(1)?, attributed: row.get(2)?, confirmed: row.get(3)?, related: row.get(4)?, third_party: row.get(5)?, excluded: row.get(6)?, exposure_eligible: row.get(7)? }),
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_asset_ownership_summary(
    state: State<AppState>,
    project_id: i64,
) -> Result<AssetOwnershipSummary, String> {
    asset_ownership_summary(&db::open(&state.db_path)?, project_id)
}

#[tauri::command]
pub fn assess_asset_ownership(
    state: State<AppState>,
    project_id: i64,
) -> Result<AssetOwnershipAssessmentResult, String> {
    assess_asset_ownership_for_connection(&mut db::open(&state.db_path)?, project_id)
}

fn assess_asset_ownership_for_connection(
    connection: &mut rusqlite::Connection,
    project_id: i64,
) -> Result<AssetOwnershipAssessmentResult, String> {
    let profile = read_ownership_profile(connection, project_id)?;
    let rules = {
        let mut statement = connection
            .prepare("SELECT rule_type,pattern,action FROM asset_ownership_rules WHERE project_id=?1 AND enabled=1")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([project_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    // O1：已绑定组织目录时，本项目范围内的目录实体名称/别名/确认域名作为确定性线索；只产生“疑似归属”，不授权。
    let directory = project_directory_terms(connection, project_id)?;
    let assets = {
        let mut statement = connection
            .prepare(&format!("SELECT a.id,a.company,a.host,a.link,a.domain,a.title,a.ip,a.extra_json FROM assets a JOIN project_assets pa ON pa.asset_id=a.id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id=?1 AND {OWNERSHIP_SCOPE_SQL} AND COALESCE(o.source,'auto')<>'manual'"))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([project_id], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,String>(6)?,row.get::<_,String>(7)?)))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    let transaction = connection.transaction().map_err(|error| error.to_string())?;
    let mut assessed = 0i64;
    for (asset_id, company, host, link, domain, title, ip, extra_json) in assets {
        let candidate_domain = [domain.as_str(), host.as_str(), link.as_str()]
            .into_iter()
            .map(normalize_ownership_domain)
            .find(|value| !value.is_empty())
            .unwrap_or_default();
        let extra = serde_json::from_str::<JsonValue>(&extra_json).unwrap_or_else(|_| JsonValue::Object(Default::default()));let org=extra.get("org").and_then(JsonValue::as_str).unwrap_or("");let region=["country","country_name","region","province"].iter().filter_map(|key|extra.get(*key).and_then(JsonValue::as_str)).collect::<Vec<_>>().join(" ");
        let combined = format!("{company} {title} {org}");
        let mut approved_domains = profile.approved_domains.clone();
        let mut excluded_domains = profile.excluded_domains.clone();
        let mut excluded_names = profile.excluded_names.clone();
        // O3：rule_type='host' 是精确主机规则（默认的学习范围），只匹配完全相同的主机。
        let (mut approved_hosts, mut excluded_hosts) = (Vec::new(), Vec::new());
        for (rule_type, pattern, action) in &rules {
            match (rule_type.as_str(), action.as_str()) {
                ("domain", "approve") => approved_domains.push(pattern.clone()),
                ("domain", "exclude") => excluded_domains.push(pattern.clone()),
                ("host", "approve") => approved_hosts.push(pattern.clone()),
                ("host", "exclude") => excluded_hosts.push(pattern.clone()),
                ("name", "exclude") => excluded_names.push(pattern.clone()),
                _ => {}
            }
        }
        let excluded_domain = excluded_domains.iter().find(|rule| domain_matches(&candidate_domain, rule)).or_else(|| excluded_hosts.iter().find(|rule| host_equals_rule(&candidate_domain, rule)));
        let approved_domain = approved_domains.iter().find(|rule| domain_matches(&candidate_domain, rule)).or_else(|| approved_hosts.iter().find(|rule| host_equals_rule(&candidate_domain, rule)));
        let shared_domain = profile.shared_domains.iter().find(|rule| domain_matches(&candidate_domain,rule));
        let excluded_name = excluded_names.iter().find(|rule| text_matches_term(&combined, rule));
        let alias_match = std::iter::once(&profile.legal_name)
            .chain(profile.aliases.iter())
            .find(|term| text_matches_term(&company, term) || text_matches_term(&title, term)||text_matches_term(org,term));
        let directory_shared = directory.shared_domains.iter().find(|(rule, _)| host_equals_rule(&candidate_domain, rule));
        let directory_domain = directory.official_domains.iter().find(|(rule, _)| host_equals_rule(&candidate_domain, rule));
        let directory_alias = directory.names.iter().find(|(term, _)| text_matches_term(&company, term) || text_matches_term(&title, term) || text_matches_term(org, term));
        let cidr_or_asn=network_only_ownership_candidate(&company,&domain,&ip,&candidate_domain);
        let excluded_region=profile.excluded_jurisdictions.iter().find(|item|text_matches_term(&region,item));let allowed_region=profile.jurisdictions.iter().find(|item|text_matches_term(&region,item));
        let (status, confidence, reason, evidence, negative) = if let Some(rule) = excluded_domain {
            ("excluded", 100, format!("命中排除域名 {rule}"), Vec::<String>::new(), vec![format!("排除域名：{rule}")])
        } else if let Some(rule) = excluded_name {
            ("excluded", 94, format!("命中排除主体 {rule}"), Vec::<String>::new(), vec![format!("排除主体：{rule}")])
        } else if let Some(rule) = shared_domain {
            ("attributed", 52, format!("命中集团共享域名 {rule}；不能据此认定属于当前跨境主体"), vec![format!("集团域名线索：{rule}")], vec!["共享域名可能同时承载其他地区或子公司".into()])
        } else if let Some((rule, entity)) = directory_shared {
            ("attributed", 52, format!("命中目录实体「{entity}」的共享域名 {rule}；共享域名不能单独认定归属"), vec![format!("目录共享域名：{rule}")], vec!["共享域名可能同时承载其他单位".into()])
        } else if let Some(rule) = approved_domain {
            ("attributed", 92, format!("属于确认根域名 {rule}；仍需确认授权范围"), vec![format!("确认域名：{rule}")], Vec::<String>::new())
        } else if let Some((rule, entity)) = directory_domain {
            ("attributed", 92, format!("精确主机 {rule} 为目录实体「{entity}」的已确认域名；仍需确认授权范围"), vec![format!("目录确认域名：{rule}")], Vec::<String>::new())
        } else if cidr_or_asn&&(alias_match.is_some()||directory_alias.is_some()) {
            let region_note=if let Some(value)=excluded_region{format!("；地域 {value} 位于排除范围")}else if let Some(value)=allowed_region{format!("；地域线索命中 {value}")}else{"；未取得可靠地域证据".into()};
            ("attributed",if allowed_region.is_some(){55}else{45},format!("ASN/IP 组织字段命中 {org}，仅证明网络登记或运营关系，不等于业务资产{region_note}"),vec![format!("网络组织：{org}"),format!("IP：{ip}")],vec!["ASN、CIDR、WHOIS 组织均属于弱归属证据".into()])
        } else if let Some(term) = alias_match {
            ("attributed", 58, format!("名称命中 {term}，缺少技术控制证据"), vec![format!("名称线索：{term}")], Vec::<String>::new())
        } else if let Some((term, entity)) = directory_alias {
            ("attributed", 58, format!("名称命中目录实体「{entity}」（{term}），缺少技术控制证据"), vec![format!("目录名称线索：{term}")], Vec::<String>::new())
        } else {
            ("unreviewed", 0, "没有足够的实体归属证据".into(), Vec::<String>::new(), Vec::<String>::new())
        };
        transaction.execute(
            "INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,authorization_status,exposure_eligible,source,reason,evidence_json,negative_evidence_json) VALUES(?1,?2,?3,?4,'unknown',0,'auto',?5,?6,?7) ON CONFLICT(project_id,asset_id) DO UPDATE SET status=excluded.status,confidence=excluded.confidence,authorization_status='unknown',exposure_eligible=0,source='auto',reason=excluded.reason,evidence_json=excluded.evidence_json,negative_evidence_json=excluded.negative_evidence_json,updated_at=datetime('now','localtime') WHERE asset_ownership_decisions.source<>'manual'",
            params![project_id,asset_id,status,confidence,reason,serde_json::to_string(&evidence).unwrap(),serde_json::to_string(&negative).unwrap()],
        ).map_err(|error| error.to_string())?;
        assessed += 1;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(AssetOwnershipAssessmentResult { assessed, summary: asset_ownership_summary(connection, project_id)? })
}

/// O0：页面加载该行时看到的归属结论。服务端在写入前逐字段比对，任何变化都视为结论版本已变。
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetOwnershipDecisionExpectation {
    pub asset_id: i64,
    pub status: String,
    pub source: String,
    pub confidence: i64,
    pub authorization_status: String,
    pub exposure_eligible: bool,
    pub reason: String,
}

/// O0/O3 共用的前置条件校验（必须在调用方的 IMMEDIATE 事务内调用）：项目、成员资格、
/// 需要时的复核范围、以及页面所见结论版本逐字段一致。任何一条不满足则整批拒绝。
fn ownership_check_decision_preconditions(
    connection: &rusqlite::Connection,
    expected_project_id: i64,
    asset_ids: &[i64],
    expectations: &HashMap<i64, AssetOwnershipDecisionExpectation>,
    grants_exposure: bool,
) -> Result<(), String> {
    let (mut missing, mut out_of_scope, mut stale, mut unversioned) = (0usize, 0usize, 0usize, 0usize);
    {
        let mut statement = connection
            .prepare(&format!("SELECT CASE WHEN {OWNERSHIP_SCOPE_SQL} THEN 1 ELSE 0 END,COALESCE(o.status,'unreviewed'),COALESCE(o.source,''),COALESCE(o.confidence,0),COALESCE(o.authorization_status,'unknown'),COALESCE(o.exposure_eligible,0),COALESCE(o.reason,'') FROM project_assets pa JOIN assets a ON a.id=pa.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id=?1 AND pa.asset_id=?2 AND pa.is_deleted=0"))
            .map_err(|error| error.to_string())?;
        for asset_id in asset_ids {
            let Some(expected) = expectations.get(asset_id) else {
                unversioned += 1;
                continue;
            };
            let current = statement
                .query_row(params![expected_project_id, asset_id], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })
                .optional()
                .map_err(|error| error.to_string())?;
            let Some((in_scope, status, source, confidence, authorization, eligible, reason)) = current else {
                missing += 1;
                continue;
            };
            if grants_exposure && in_scope == 0 {
                out_of_scope += 1;
                continue;
            }
            if status != expected.status
                || source != expected.source
                || confidence != expected.confidence
                || authorization != expected.authorization_status
                || (eligible != 0) != expected.exposure_eligible
                || reason != expected.reason
            {
                stale += 1;
            }
        }
    }
    if missing + out_of_scope + stale + unversioned > 0 {
        return Err(format!(
            "{OWNERSHIP_DECISION_PRECONDITION}: 批量操作已拒绝，未写入任何结论：{missing} 条已不属于当前工作空间或已删除；{out_of_scope} 条已不在归属复核范围（授权要求仍在范围内）；{stale} 条归属结论在页面加载后已变化；{unversioned} 条缺少页面看到的结论版本。请刷新复核队列后重新选择。"
        ));
    }
    Ok(())
}

/// O0 批量归属结论：先在 IMMEDIATE 事务内校验全部前置条件，任何一条不满足则整批拒绝、不写入。
/// 1. 每条选择必须属于页面当前加载的项目（expected_project_id）；
/// 2. 必须是未删除的 project_assets 关系（撤销/降级动作同样校验）；
/// 3. “确认归属并授权”会写入 authorization_status=allowed、exposure_eligible=1，因此额外要求资产仍在当前归属复核范围；
/// 4. 现存结论必须与页面看到的版本一致，防止覆盖重新评估或其他人工结论。
fn apply_asset_ownership_decisions(
    connection: &mut rusqlite::Connection,
    input: AssetOwnershipDecisionInput,
    expected_project_id: i64,
    expected_decisions: Vec<AssetOwnershipDecisionExpectation>,
) -> Result<i64, String> {
    if !["unreviewed", "confirmed", "related", "third_party", "excluded"].contains(&input.status.as_str()) {
        return Err("归属结论无效".into());
    }
    if input.selections.is_empty() || input.selections.len() > 20_000 || expected_decisions.len() > 20_000 {
        return Err("请选择 1–20000 条资产".into());
    }
    if expected_project_id <= 0 {
        return Err(format!("{OWNERSHIP_DECISION_PRECONDITION}: 缺少页面当前工作空间，未写入任何结论"));
    }
    let foreign = input.selections.iter().filter(|item| item.project_id != expected_project_id).count();
    if foreign > 0 {
        return Err(format!("{OWNERSHIP_DECISION_PRECONDITION}: 批量操作已拒绝，未写入任何结论：{foreign} 条选择不属于当前工作空间（可能是切换项目前的旧选择），请刷新后重新选择"));
    }
    let expectations: HashMap<i64, AssetOwnershipDecisionExpectation> = expected_decisions
        .into_iter()
        .map(|item| (item.asset_id, item))
        .collect();
    let mut unique = HashSet::new();
    let asset_ids: Vec<i64> = input
        .selections
        .iter()
        .map(|item| item.asset_id)
        .filter(|asset_id| unique.insert(*asset_id))
        .collect();
    let grants_exposure = input.status == "confirmed";
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    ownership_check_decision_preconditions(&transaction, expected_project_id, &asset_ids, &expectations, grants_exposure)?;
    let mut changed = 0i64;
    for asset_id in asset_ids {
        let project_id = expected_project_id;
        if input.status == "unreviewed" {
            changed += transaction.execute(
                "DELETE FROM asset_ownership_decisions WHERE project_id=?1 AND asset_id=?2",
                params![project_id, asset_id],
            ).map_err(|error| error.to_string())? as i64;
            transaction.execute(
                "INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'ownership','撤销人工归属结论，恢复自动判断')",
                params![project_id, asset_id],
            ).map_err(|error| error.to_string())?;
            continue;
        }
        let (authorization, eligible, confidence) = match input.status.as_str() {
            "confirmed" => ("allowed", 1, 100),
            "excluded" | "third_party" => ("denied", 0, 100),
            _ => ("unknown", 0, 70),
        };
        changed += transaction.execute(
            "INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,authorization_status,exposure_eligible,source,reason,evidence_json,negative_evidence_json) VALUES(?1,?2,?3,?4,?5,?6,'manual',?7,'[]','[]') ON CONFLICT(project_id,asset_id) DO UPDATE SET status=excluded.status,confidence=excluded.confidence,authorization_status=excluded.authorization_status,exposure_eligible=excluded.exposure_eligible,source='manual',reason=excluded.reason,updated_at=datetime('now','localtime')",
            params![project_id,asset_id,input.status,confidence,authorization,eligible,input.note],
        ).map_err(|error| error.to_string())? as i64;
        transaction.execute("INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'ownership',?3)", params![project_id,asset_id,format!("{}: {}",input.status,input.note)]).map_err(|error| error.to_string())?;
        if input.learn_domain_rule && matches!(input.status.as_str(), "confirmed" | "excluded") {
            // O3：默认只学习精确主机（rule_type='host'），不再隐式泛化到子域。
            let action = if input.status == "confirmed" { "approve" } else { "exclude" };
            ownership_learn_host_rule(&transaction, project_id, asset_id, action, "exact_host")?;
        }
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(changed)
}

#[tauri::command]
pub fn update_asset_ownership(
    state: State<AppState>,
    input: AssetOwnershipDecisionInput,
    expected_project_id: i64,
    expected_decisions: Vec<AssetOwnershipDecisionExpectation>,
) -> Result<i64, String> {
    apply_asset_ownership_decisions(&mut db::open(&state.db_path)?, input, expected_project_id, expected_decisions)
}

#[cfg(test)]
mod asset_ownership_tests {
    use super::{domain_matches, network_only_ownership_candidate, normalize_ownership_domain, text_matches_term};

    #[test]
    fn ownership_domain_matching_respects_dns_label_boundaries() {
        assert!(domain_matches("api.cmhk.com", "cmhk.com"));
        assert!(domain_matches("https://www.cmhk.com:443/a", "cmhk.com"));
        assert!(!domain_matches("evilcmhk.com", "cmhk.com"));
        assert!(!domain_matches("cmhk.com.example.org", "cmhk.com"));
    }

    #[test]
    fn ownership_normalization_strips_transport_details_only() {
        assert_eq!(
            normalize_ownership_domain("HTTPS://WWW.CMHK.COM:443/path?q=1"),
            "cmhk.com"
        );
        assert_eq!(normalize_ownership_domain("mobile.com.hk."), "mobile.com.hk");
    }

    #[test]
    fn ownership_name_terms_reject_blank_and_single_character_noise() {
        assert!(text_matches_term("香港移动客户服务", "香港移动"));
        assert!(!text_matches_term("中国移动", "移"));
        assert!(!text_matches_term("中国移动", ""));
    }

    #[test]
    fn asn_cidr_and_ip_only_assets_are_weak_network_evidence() {
        assert!(network_only_ownership_candidate("182.239.124.0/23","","182.239.125.17","182.239.125.17"));
        assert!(network_only_ownership_candidate("AS58453","","1.1.1.1","1.1.1.1"));
        assert!(!network_only_ownership_candidate("CMHK","cmhk.com","1.1.1.1","cmhk.com"));
    }
}

#[cfg(test)]
mod asset_ownership_o0_tests {
    use super::*;

    fn fixture() -> (PathBuf, rusqlite::Connection) {
        let root = std::env::temp_dir().join(format!("oviraptor-ownership-o0-{}", Uuid::new_v4()));
        let path = db::initialize(&root).unwrap();
        let connection = db::open(&path).unwrap();
        connection
            .execute_batch(
                "INSERT INTO projects(id,name) VALUES(91,'中国移动历史资产'),(92,'香港移动');
                 INSERT INTO assets(id,asset_key,host,domain) VALUES(1,'o0-1','a.example.cn','a.example.cn'),(2,'o0-2','b.example.cn','b.example.cn'),(3,'o0-3','c.example.cn','c.example.cn'),(4,'o0-4','d.example.hk','d.example.hk');
                 INSERT INTO project_assets(project_id,asset_id,is_deleted) VALUES(91,1,0),(91,2,1),(91,3,0),(92,4,0);
                 UPDATE assets SET probe_outcome='dead' WHERE id=3;",
            )
            .unwrap();
        (root, connection)
    }

    fn profile(project_id: i64, legal_name: &str) -> AssetOwnershipProfile {
        AssetOwnershipProfile { project_id, legal_name: legal_name.into(), ..Default::default() }
    }

    fn untouched(asset_id: i64) -> AssetOwnershipDecisionExpectation {
        AssetOwnershipDecisionExpectation {
            asset_id,
            status: "unreviewed".into(),
            source: String::new(),
            confidence: 0,
            authorization_status: "unknown".into(),
            exposure_eligible: false,
            reason: String::new(),
        }
    }

    fn decision(project_id: i64, asset_ids: &[i64], status: &str) -> AssetOwnershipDecisionInput {
        AssetOwnershipDecisionInput {
            selections: asset_ids.iter().map(|asset_id| AssetSelection { project_id, asset_id: *asset_id }).collect(),
            status: status.into(),
            note: "o0 test".into(),
            learn_domain_rule: false,
        }
    }

    fn decision_rows(connection: &rusqlite::Connection) -> Vec<(i64, i64, String, String, i64)> {
        let mut statement = connection
            .prepare("SELECT project_id,asset_id,status,authorization_status,exposure_eligible FROM asset_ownership_decisions ORDER BY project_id,asset_id")
            .unwrap();
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows
    }

    #[test]
    fn profile_save_requires_the_version_the_page_loaded() {
        let (root, mut connection) = fixture();
        let unsaved = read_ownership_profile(&connection, 91).unwrap();
        assert!(unsaved.updated_at.is_empty(), "unsaved prefill must be distinguishable from saved v1");
        let first = save_ownership_profile_for_connection(&mut connection, profile(91, "中国移动通信集团有限公司"), 0).unwrap();
        assert_eq!(first.policy_version, 1);
        assert!(!first.updated_at.is_empty());
        // A second page that also loaded the unsaved prefill must not overwrite the first save.
        let error = save_ownership_profile_for_connection(&mut connection, profile(91, "旧页面草稿"), 0).unwrap_err();
        assert!(error.starts_with(OWNERSHIP_PROFILE_CONFLICT), "{error}");
        assert_eq!(read_ownership_profile(&connection, 91).unwrap().legal_name, "中国移动通信集团有限公司");
        let second = save_ownership_profile_for_connection(&mut connection, profile(91, "中国移动通信集团有限公司 2"), 1).unwrap();
        assert_eq!(second.policy_version, 2);
        let stale = save_ownership_profile_for_connection(&mut connection, profile(91, "基于 v1 的草稿"), 1).unwrap_err();
        assert!(stale.contains("当前 v2") && stale.contains("页面基于 v1"), "{stale}");
        assert_eq!(read_ownership_profile(&connection, 91).unwrap().policy_version, 2);
        // Another project's version is independent.
        assert_eq!(save_ownership_profile_for_connection(&mut connection, profile(92, "香港移动通讯有限公司"), 0).unwrap().policy_version, 1);
        assert!(save_ownership_profile_for_connection(&mut connection, profile(404, "不存在"), 0).is_err());
        assert!(save_ownership_profile_for_connection(&mut connection, profile(91, "x"), -1).is_err());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn decisions_reject_selections_from_another_project_without_writing() {
        let (root, mut connection) = fixture();
        let error = apply_asset_ownership_decisions(&mut connection, decision(92, &[4], "confirmed"), 91, vec![untouched(4)]).unwrap_err();
        assert!(error.starts_with(OWNERSHIP_DECISION_PRECONDITION) && error.contains("1 条选择不属于当前工作空间"), "{error}");
        // A forged selection claiming project 91 for an asset only linked to 92 is not a member.
        let error = apply_asset_ownership_decisions(&mut connection, decision(91, &[4], "excluded"), 91, vec![untouched(4)]).unwrap_err();
        assert!(error.contains("1 条已不属于当前工作空间或已删除"), "{error}");
        assert!(apply_asset_ownership_decisions(&mut connection, decision(91, &[1], "confirmed"), 0, vec![untouched(1)]).is_err());
        assert!(decision_rows(&connection).is_empty());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn decisions_validate_membership_scope_and_version_atomically() {
        let (root, mut connection) = fixture();
        // Deleted link (2) and missing version (3) poison the whole batch, including the valid asset 1.
        let error = apply_asset_ownership_decisions(&mut connection, decision(91, &[1, 2, 3], "related"), 91, vec![untouched(1), untouched(2)]).unwrap_err();
        assert!(error.contains("1 条已不属于当前工作空间或已删除") && error.contains("1 条缺少页面看到的结论版本"), "{error}");
        assert!(decision_rows(&connection).is_empty(), "rejected batch must not write any row");
        // Granting exposure requires the asset to still be inside the ownership review scope.
        let error = apply_asset_ownership_decisions(&mut connection, decision(91, &[3], "confirmed"), 91, vec![untouched(3)]).unwrap_err();
        assert!(error.contains("1 条已不在归属复核范围"), "{error}");
        assert!(decision_rows(&connection).is_empty());
        // Downgrades on an out-of-scope but still linked asset remain possible.
        assert_eq!(apply_asset_ownership_decisions(&mut connection, decision(91, &[3], "excluded"), 91, vec![untouched(3)]).unwrap(), 1);
        assert_eq!(apply_asset_ownership_decisions(&mut connection, decision(91, &[1, 1], "confirmed"), 91, vec![untouched(1)]).unwrap(), 1);
        assert_eq!(
            decision_rows(&connection),
            vec![(91, 1, "confirmed".into(), "allowed".into(), 1), (91, 3, "excluded".into(), "denied".into(), 0)]
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn decisions_reject_when_the_row_changed_after_the_page_loaded() {
        let (root, mut connection) = fixture();
        // The page saw asset 1 as unreviewed; a re-assessment then marked it attributed.
        connection
            .execute("INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,source,reason) VALUES(91,1,'attributed',92,'auto','属于确认根域名 example.cn')", [])
            .unwrap();
        let error = apply_asset_ownership_decisions(&mut connection, decision(91, &[1], "confirmed"), 91, vec![untouched(1)]).unwrap_err();
        assert!(error.contains("1 条归属结论在页面加载后已变化"), "{error}");
        assert_eq!(decision_rows(&connection), vec![(91, 1, "attributed".into(), "unknown".into(), 0)]);
        let seen = AssetOwnershipDecisionExpectation {
            asset_id: 1,
            status: "attributed".into(),
            source: "auto".into(),
            confidence: 92,
            authorization_status: "unknown".into(),
            exposure_eligible: false,
            reason: "属于确认根域名 example.cn".into(),
        };
        assert_eq!(apply_asset_ownership_decisions(&mut connection, decision(91, &[1], "confirmed"), 91, vec![seen.clone()]).unwrap(), 1);
        // Replaying the same click with the now-stale view is rejected instead of silently re-applied.
        assert!(apply_asset_ownership_decisions(&mut connection, decision(91, &[1], "unreviewed"), 91, vec![seen]).is_err());
        let confirmed = AssetOwnershipDecisionExpectation {
            asset_id: 1,
            status: "confirmed".into(),
            source: "manual".into(),
            confidence: 100,
            authorization_status: "allowed".into(),
            exposure_eligible: true,
            reason: "o0 test".into(),
        };
        assert_eq!(apply_asset_ownership_decisions(&mut connection, decision(91, &[1], "unreviewed"), 91, vec![confirmed]).unwrap(), 1);
        assert!(decision_rows(&connection).is_empty());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    /// 与 AssetOwnershipGate.vue loadQueue 发出的查询完全一致（只换 ownershipView）。
    fn gate_queue_total(connection: &rusqlite::Connection, project_id: i64, view: &str) -> i64 {
        let query = AssetQuery {
            project_id: Some(project_id),
            page: 1,
            page_size: 100,
            deleted_view: "active".into(),
            probe_view: "browser_review".into(),
            probe_outcome_view: "all".into(),
            sentinel_view: "all".into(),
            decision_view: "ownership_scope".into(),
            ownership_view: view.into(),
            sort_by: "priority".into(),
            sort_direction: "desc".into(),
            ..Default::default()
        };
        let (filter, values) = asset_filter(&query, true);
        connection
            .query_row(
                &format!("SELECT COUNT(*) FROM assets a JOIN project_assets pa ON pa.asset_id=a.id JOIN projects p ON p.id=pa.project_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=a.id WHERE {filter}"),
                params_from_iter(values.iter()),
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn gate_queue_totals_match_the_summary_cards() {
        let (root, connection) = fixture();
        connection
            .execute_batch(
                "INSERT INTO assets(id,asset_key,host,domain) VALUES(5,'o0-5','e.example.cn','e.example.cn'),(6,'o0-6','f.example.cn','f.example.cn'),(7,'o0-7','g.example.cn','g.example.cn'),(8,'o0-8','h.example.cn','h.example.cn');
                 INSERT INTO project_assets(project_id,asset_id,is_deleted,decision) VALUES(91,5,0,'pending'),(91,6,0,'pending'),(91,7,0,'rejected'),(91,8,0,'pending'),(92,1,0,'pending');
                 INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,authorization_status,exposure_eligible,source) VALUES
                   (91,1,'confirmed',100,'allowed',1,'manual'),
                   (91,5,'attributed',58,'unknown',0,'auto'),
                   (91,6,'related',70,'unknown',1,'manual'),
                   (91,7,'confirmed',100,'allowed',1,'manual'),
                   (91,8,'excluded',100,'denied',0,'manual'),
                   (92,1,'confirmed',100,'allowed',1,'manual');",
            )
            .unwrap();
        let summary = asset_ownership_summary(&connection, 91).unwrap();
        // 91: 1(confirmed,eligible) 3(dead → out of scope) 5(attributed) 6(legacy eligible related) 7(rejected → out) 8(excluded); 2 deleted.
        assert_eq!((summary.all, summary.unreviewed, summary.attributed, summary.confirmed), (4, 0, 1, 1));
        assert_eq!((summary.related, summary.excluded, summary.exposure_eligible), (1, 1, 2));
        assert_eq!(gate_queue_total(&connection, 91, ""), summary.all);
        assert_eq!(gate_queue_total(&connection, 91, "review"), summary.unreviewed + summary.attributed);
        assert_eq!(gate_queue_total(&connection, 91, "confirmed"), summary.confirmed);
        assert_eq!(gate_queue_total(&connection, 91, "related"), summary.related);
        assert_eq!(gate_queue_total(&connection, 91, "excluded"), summary.excluded);
        // The eligible preview lists exactly what the card counts and what exposure consumes,
        // including a legacy eligible row that is not confirmed+allowed.
        assert_eq!(gate_queue_total(&connection, 91, "eligible"), summary.exposure_eligible);
        // Project 92's decision on shared asset 1 never leaks into project 91's numbers.
        assert_eq!(asset_ownership_summary(&connection, 92).unwrap().exposure_eligible, 1);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
