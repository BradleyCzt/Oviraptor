// O2 后台聚合复核闭环（设计文档 NEST_OWNERSHIP_GATE_SMART_GROUP_REVIEW_DESIGN_2026-10-08 §6–§10、§11 第 3 步）。
// - ownership_analysis_runs：持久化分析任务 queued/running/completed/cancelled/failed，绑定 project/scope/data/
//   policy/directory/binding/algorithm 版本；按稳定主键分块读取、短事务写中间结果；可取消；同项目不重复启动。
// - ownership_review_group_members：主组成员索引，PRIMARY KEY(run_id, asset_id) 保证一个资产在一个快照内只属于一个主组。
// - ownership_review_groups：按 §6 关键条件（候选单位/关系、域名或托管边界、支持与反证、人工结论、授权状态）拆组；
//   反证、冲突、未知、人工结论各自成组，不为减少卡片强制合并。
// - ownership_operations / ownership_operation_items：组批量预览/确认（提交 groupId，不传全量 ID），operationId 幂等，
//   CAS 校验快照/组版本/策略/目录/绑定/expectedActionHash，逐条审计旧值与新值。
// - 归属确认只写归属事实：不创建授权、不设暴露面准入；授权仍走既有合同（第三方/排除沿用既有“拒绝 + 不准入”的收窄语义）。

const OWNERSHIP_GROUP_ALGORITHM: &str = "o2-review-groups-v2-endpoint-host";
const OWNERSHIP_ANALYSIS_CHUNK: i64 = 500;
/// 心跳超过该秒数的 running 任务视为工作线程已丢失，可被重新认领并从 last_asset_id 继续。
const OWNERSHIP_ANALYSIS_STALE_SECS: i64 = 120;
const OWNERSHIP_GROUP_PAGE_MAX: i64 = 200;
const OWNERSHIP_MEMBER_PAGE_MAX: i64 = 500;
const OWNERSHIP_GROUP_SAMPLE_LIMIT: usize = 5;
const OWNERSHIP_PREVIEW_EXCLUDED_SAMPLE: usize = 20;
/// 保留成员索引的已完成快照数（当前 + 上一份），更早的快照只保留任务元数据。
const OWNERSHIP_SNAPSHOTS_KEPT: i64 = 2;

const OWNERSHIP_REVIEW_INVALID: &str = "OWNERSHIP_REVIEW_INPUT_INVALID";
const OWNERSHIP_SNAPSHOT_STALE: &str = "OWNERSHIP_SNAPSHOT_STALE";
const OWNERSHIP_PREVIEW_STALE: &str = "OWNERSHIP_PREVIEW_STALE";
const OWNERSHIP_OPERATION_MISMATCH: &str = "OWNERSHIP_OPERATION_PAYLOAD_MISMATCH";
const OWNERSHIP_GROUP_ACTION_DENIED: &str = "OWNERSHIP_GROUP_ACTION_NOT_ALLOWED";

const OWNERSHIP_GROUP_ACTIONS: [&str; 4] =
    ["confirm_ownership", "related", "third_party", "excluded"];

/// 现有归属结论的完整状态（与 O0 页面逐字段比对的字段一致，外加 updated_at）。
/// Rust 端不另行拼接，统一由这条 SQL 产生，保证快照、预览、确认和增量计数使用同一口径。
const OWNERSHIP_DECISION_STATE_SQL: &str = "COALESCE(o.status,'unreviewed')||'|'||COALESCE(o.source,'')||'|'||COALESCE(o.confidence,0)||'|'||COALESCE(o.authorization_status,'unknown')||'|'||COALESCE(o.exposure_eligible,0)||'|'||COALESCE(o.updated_at,'')||'|'||COALESCE(o.reason,'')";

fn ownership_sha(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn ownership_algorithm_version() -> String {
    format!("{OWNERSHIP_GROUP_ALGORITHM};{OWNERSHIP_PSL_VERSION}")
}

fn ownership_scope_version() -> String {
    format!("scope-{}", &ownership_sha(OWNERSHIP_SCOPE_SQL)[..16])
}

fn ownership_now_epoch(connection: &rusqlite::Connection) -> Result<i64, String> {
    connection
        .query_row("SELECT CAST(strftime('%s','now') AS INTEGER)", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipAnalysisVersions {
    pub policy_version: i64,
    pub directory_version: i64,
    pub binding_version: i64,
    pub scope_version: String,
    pub algorithm_version: String,
    /// O3：项目学习规则（asset_ownership_rules 启用项）的指纹；规则变化与档案/目录变化一样使快照失效。
    pub rules_version: String,
}

/// O3：启用规则按 (类型|模式|动作) 排序后取指纹；没有规则时为固定值。
fn ownership_rules_version(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<String, String> {
    let mut statement = connection
        .prepare("SELECT rule_type||'|'||pattern||'|'||action||'|'||source FROM asset_ownership_rules WHERE project_id=?1 AND enabled=1 ORDER BY 1")
        .map_err(|error| error.to_string())?;
    let rules = statement
        .query_map([project_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(format!("rules-{}", &ownership_sha(&rules.join("\n"))[..16]))
}

fn ownership_current_versions(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<OwnershipAnalysisVersions, String> {
    let policy_version: i64 = connection
        .query_row(
            "SELECT policy_version FROM asset_ownership_profiles WHERE project_id=?1",
            [project_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or(0);
    let binding_version: i64 = connection
        .query_row(
            "SELECT binding_version FROM project_org_bindings WHERE project_id=?1",
            [project_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or(0);
    Ok(OwnershipAnalysisVersions {
        policy_version,
        directory_version: org_directory_version(connection)?,
        binding_version,
        scope_version: ownership_scope_version(),
        algorithm_version: ownership_algorithm_version(),
        rules_version: ownership_rules_version(connection, project_id)?,
    })
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipAnalysisRunView {
    pub id: i64,
    pub project_id: i64,
    pub status: String,
    pub versions: OwnershipAnalysisVersions,
    pub data_revision: String,
    pub data_revision_at_finish: String,
    pub total_assets: i64,
    pub processed_assets: i64,
    pub group_count: i64,
    pub member_count: i64,
    pub cancel_requested: bool,
    pub members_pruned: bool,
    pub error: String,
    pub created_at: String,
    pub started_at: String,
    pub finished_at: String,
    /// O3：full / incremental；增量任务沿用 base_run_id 快照中未变化行的分组结果。
    pub run_kind: String,
    pub base_run_id: Option<i64>,
    /// 选择全量的原因（无可用快照 / 版本变化列表），或增量的依据。
    pub mode_reason: String,
    /// 本任务实际重新分类的资产数与沿用基线快照的资产数（两者之和 = processed_assets）。
    pub reclassified_assets: i64,
    pub carried_assets: i64,
    /// 被重新认领续跑的次数（应用重启/工作线程丢失后自动接管）。
    pub resume_count: i64,
    pub watermark_at: String,
}

const OWNERSHIP_RUN_SELECT: &str = "SELECT id,project_id,status,policy_version,directory_version,binding_version,scope_version,algorithm_version,data_revision,data_revision_at_finish,total_assets,processed_assets,group_count,member_count,cancel_requested,members_pruned,error,created_at,started_at,finished_at,rules_version,run_kind,base_run_id,mode_reason,reclassified_assets,carried_assets,resume_count,watermark_at FROM ownership_analysis_runs";

fn ownership_run_from_row(row: &Row<'_>) -> rusqlite::Result<OwnershipAnalysisRunView> {
    Ok(OwnershipAnalysisRunView {
        id: row.get(0)?,
        project_id: row.get(1)?,
        status: row.get(2)?,
        versions: OwnershipAnalysisVersions {
            policy_version: row.get(3)?,
            directory_version: row.get(4)?,
            binding_version: row.get(5)?,
            scope_version: row.get(6)?,
            algorithm_version: row.get(7)?,
            rules_version: row.get(20)?,
        },
        data_revision: row.get(8)?,
        data_revision_at_finish: row.get(9)?,
        total_assets: row.get(10)?,
        processed_assets: row.get(11)?,
        group_count: row.get(12)?,
        member_count: row.get(13)?,
        cancel_requested: row.get::<_, i64>(14)? != 0,
        members_pruned: row.get::<_, i64>(15)? != 0,
        error: row.get(16)?,
        created_at: row.get(17)?,
        started_at: row.get(18)?,
        finished_at: row.get(19)?,
        run_kind: row.get(21)?,
        base_run_id: row.get(22)?,
        mode_reason: row.get(23)?,
        reclassified_assets: row.get(24)?,
        carried_assets: row.get(25)?,
        resume_count: row.get(26)?,
        watermark_at: row.get(27)?,
    })
}

fn ownership_read_run(
    connection: &rusqlite::Connection,
    run_id: i64,
) -> Result<Option<OwnershipAnalysisRunView>, String> {
    connection
        .query_row(
            &format!("{OWNERSHIP_RUN_SELECT} WHERE id=?1"),
            [run_id],
            ownership_run_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn ownership_project_run(
    connection: &rusqlite::Connection,
    project_id: i64,
    clause: &str,
) -> Result<Option<OwnershipAnalysisRunView>, String> {
    connection
        .query_row(
            &format!(
                "{OWNERSHIP_RUN_SELECT} WHERE project_id=?1 AND {clause} ORDER BY id DESC LIMIT 1"
            ),
            [project_id],
            ownership_run_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())
}

// ---------------------------------------------------------------------------------------------
// 分类：每个资产得到一个主组键（§6 第二层可操作组）。
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
struct OwnershipGroupKey {
    lane: &'static str,
    nav_bucket: String,
    entity_id: Option<i64>,
    candidate_label: String,
    candidate_source: &'static str,
    relation_kind: String,
    region_clue: String,
    boundary_kind: &'static str,
    boundary_value: String,
    support_kind: String,
    counter_kind: String,
    decision_status: String,
    decision_source: String,
    authorization_status: String,
    exposure_eligible: i64,
}

impl OwnershipGroupKey {
    fn text(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
struct OwnershipAssetInput {
    asset_id: i64,
    company: String,
    host: String,
    link: String,
    domain: String,
    title: String,
    ip: String,
    extra_json: String,
    decision_status: String,
    decision_source: String,
    authorization_status: String,
    exposure_eligible: i64,
    decision_state: String,
}

#[derive(Debug, Clone)]
struct OwnershipMemberFacts {
    asset_id: i64,
    host: String,
    registrable: String,
    suffix_section: String,
    company: String,
    evidence_hash: String,
    decision_state: String,
    key: OwnershipGroupKey,
}

/// 证据指纹：分组依据的原始字段。字段变化即视为证据版本变化，使旧预览失效。
fn ownership_evidence_hash(
    company: &str,
    host: &str,
    link: &str,
    domain: &str,
    title: &str,
    ip: &str,
    extra_json: &str,
) -> String {
    let org = serde_json::from_str::<JsonValue>(extra_json)
        .ok()
        .and_then(|value| {
            value
                .get("org")
                .and_then(JsonValue::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    let text = serde_json::to_string(&[company, host, link, domain, title, ip, org.as_str()])
        .unwrap_or_default();
    ownership_sha(&text)[..32].to_string()
}

struct OwnershipDirectoryEntity {
    legal_name: String,
    kind: String,
    region_code: String,
    in_scope: bool,
}

/// 一次分析任务固定的分类上下文（档案、规则、目录、绑定范围）。任务开始时加载，版本记录在任务上。
struct OwnershipClassifier {
    bound: bool,
    profile_name: String,
    profile_terms: Vec<String>,
    approved_domains: Vec<String>,
    /// O3：精确主机规则（rule_type='host'），只匹配完全相同的主机，不泛化到子域。
    approved_hosts: Vec<String>,
    excluded_hosts: Vec<String>,
    legacy_approved_rules: Vec<(String, String)>,
    shared_domains: Vec<String>,
    excluded_domains: Vec<String>,
    excluded_names: Vec<String>,
    entities: HashMap<i64, OwnershipDirectoryEntity>,
    official_hosts: HashMap<String, Vec<i64>>,
    shared_hosts: HashMap<String, Vec<i64>>,
    /// (规范化名称, 实体, 是否法定名称)
    names: Vec<(String, i64, bool)>,
}

fn ownership_classifier(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<OwnershipClassifier, String> {
    let profile = read_ownership_profile(connection, project_id)?;
    let saved = !profile.updated_at.is_empty();
    let mut approved_domains = if saved {
        profile.approved_domains.clone()
    } else {
        Vec::new()
    };
    let mut excluded_domains = if saved {
        profile.excluded_domains.clone()
    } else {
        Vec::new()
    };
    let mut excluded_names = if saved {
        profile.excluded_names.clone()
    } else {
        Vec::new()
    };
    let (mut approved_hosts, mut excluded_hosts) = (Vec::new(), Vec::new());
    let mut legacy_approved_rules = Vec::new();
    {
        let mut statement = connection
            .prepare("SELECT rule_type,pattern,action,source FROM asset_ownership_rules WHERE project_id=?1 AND enabled=1 ORDER BY id")
            .map_err(|error| error.to_string())?;
        let rules = statement
            .query_map([project_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        for (rule_type, pattern, action, source) in rules {
            if ownership_legacy_learned_rule(&rule_type, &action, &source) {
                legacy_approved_rules.push((rule_type, pattern));
                continue;
            }
            match (rule_type.as_str(), action.as_str()) {
                ("domain", "approve") => approved_domains.push(pattern),
                ("domain", "exclude") => excluded_domains.push(pattern),
                ("host", "approve") => approved_hosts.push(pattern),
                ("host", "exclude") => excluded_hosts.push(pattern),
                ("name", "exclude") => excluded_names.push(pattern),
                _ => {}
            }
        }
    }
    let binding = load_project_org_binding(connection, project_id)?;
    let all = load_org_entities(connection)?;
    let scope = org_binding_scope(&all, binding.as_ref());
    let mut classifier = OwnershipClassifier {
        bound: binding.is_some(),
        profile_name: if saved {
            profile.legal_name.clone()
        } else {
            String::new()
        },
        profile_terms: if saved {
            std::iter::once(profile.legal_name.clone())
                .chain(profile.aliases.iter().cloned())
                .filter(|term| term.trim().chars().count() >= 2)
                .collect()
        } else {
            Vec::new()
        },
        approved_domains,
        approved_hosts,
        excluded_hosts,
        legacy_approved_rules,
        shared_domains: if saved {
            profile.shared_domains.clone()
        } else {
            Vec::new()
        },
        excluded_domains,
        excluded_names,
        entities: HashMap::new(),
        official_hosts: HashMap::new(),
        shared_hosts: HashMap::new(),
        names: Vec::new(),
    };
    for entity in &all {
        classifier.entities.insert(
            entity.id,
            OwnershipDirectoryEntity {
                legal_name: entity.legal_name.clone(),
                kind: entity.entity_kind.clone(),
                region_code: entity.region_code.clone(),
                in_scope: scope.contains(&entity.id),
            },
        );
        classifier
            .names
            .push((entity.normalized_name.clone(), entity.id, true));
        for identifier in &entity.identifiers {
            match identifier.kind.as_str() {
                "official_domain" => classifier
                    .official_hosts
                    .entry(normalize_ownership_host(&identifier.value))
                    .or_default()
                    .push(entity.id),
                "shared_domain" => classifier
                    .shared_hosts
                    .entry(normalize_ownership_host(&identifier.value))
                    .or_default()
                    .push(entity.id),
                _ => {
                    let term = normalize_org_name(&identifier.value);
                    if term.chars().count() >= 2 {
                        classifier.names.push((term, entity.id, false));
                    }
                }
            }
        }
    }
    for ids in classifier.official_hosts.values_mut().chain(classifier.shared_hosts.values_mut()) {
        ids.sort_unstable();
        ids.dedup();
    }
    Ok(classifier)
}

fn ownership_nav_bucket(kind: &str) -> &'static str {
    match kind {
        "group" | "headquarters" => "headquarters_shared",
        "province_company" => "province",
        "city_branch" | "county_branch" => "city",
        "professional_subsidiary" => "subsidiary",
        "overseas_entity" => "overseas",
        "supplier" => "supplier",
        _ => "unknown",
    }
}

impl OwnershipClassifier {
    fn classify(&self, input: &OwnershipAssetInput) -> OwnershipMemberFacts {
        let host = ownership_endpoint_host(&input.link, &input.host);
        let is_ip = host.parse::<std::net::IpAddr>().is_ok()
            || (host.is_empty() && !input.ip.trim().is_empty());
        let parts = if is_ip {
            None
        } else {
            ownership_domain_parts(&host)
        };
        let registrable = parts
            .as_ref()
            .and_then(|parts| parts.registrable.clone())
            .unwrap_or_default();
        let section = parts
            .as_ref()
            .map(|parts| parts.section.as_str())
            .unwrap_or(if is_ip { "ip" } else { "none" });
        let company_norm = normalize_org_name(&input.company);
        let title_norm = normalize_org_name(&input.title);
        let org = serde_json::from_str::<JsonValue>(&input.extra_json)
            .ok()
            .and_then(|value| {
                value
                    .get("org")
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let combined = format!("{} {} {}", input.company, input.title, org);
        let mut counters: Vec<&'static str> = Vec::new();
        if ownership_source_domain_conflicts(&host, &input.domain) {
            counters.push("source_domain_mismatch");
        }
        if ownership_source_host_conflicts(&input.link, &input.host) {
            counters.push("source_host_mismatch");
        }
        if self.legacy_approved_rules.iter().any(|(kind, pattern)| ownership_legacy_rule_matches(&host, kind, pattern)) {
            counters.push("legacy_learned_rule_unverified");
        }

        // 边界：默认可注册域（PSL）；共享域名/共享主机与 private 后缀租户只按精确主机或租户，不泛化到根域。
        let profile_shared = !host.is_empty()
            && self
                .shared_domains
                .iter()
                .any(|rule| domain_matches(&host, rule));
        let directory_shared = self.shared_hosts.get(&host);
        let (boundary_kind, boundary_value): (&'static str, String) = if host.is_empty() && !is_ip {
            ("none", String::new())
        } else if is_ip {
            ("ip", String::new())
        } else if registrable.is_empty() {
            ("public_suffix_host", host.clone())
        } else if section == "private" {
            counters.push("hosting_platform");
            ("private_suffix_tenant", registrable.clone())
        } else if profile_shared || directory_shared.is_some() {
            counters.push("shared_domain");
            ("exact_host", host.clone())
        } else {
            ("registrable_domain", registrable.clone())
        };

        if !host.is_empty()
            && (self
                .excluded_domains
                .iter()
                .any(|rule| domain_matches(&host, rule))
                || self
                    .excluded_hosts
                    .iter()
                    .any(|rule| host_equals_rule(&host, rule)))
        {
            counters.push("profile_excluded_domain");
        }
        if self
            .excluded_names
            .iter()
            .any(|rule| text_matches_term(&combined, rule))
        {
            counters.push("profile_excluded_name");
        }
        let network_label = org_label_is_network_clue(&input.company);
        let domain_label = !network_label && org_label_is_domain_like(&input.company);
        if network_label {
            counters.push("network_label");
        }

        // 支持证据（强：目录官方精确主机 / 档案确认域名 / 目录法定名称完全一致；弱：别名包含 / 档案名称 / 标签候选）。
        let mut strong = false;
        let mut support = String::new();
        let mut relation = String::from("none");
        let mut entity_id: Option<i64> = None;
        let mut candidate_label = String::new();
        let mut candidate_source: &'static str = "none";
        let mut nav: String = "unknown".into();
        let mut region = String::new();

        let host_entities: Vec<i64> = self.official_hosts.get(&host).cloned().unwrap_or_default();
        let exact_name: Vec<i64> = if company_norm.is_empty() {
            Vec::new()
        } else {
            self.names
                .iter()
                .filter(|(name, _, legal)| *legal && *name == company_norm)
                .map(|(_, id, _)| *id)
                .collect()
        };
        // 最长包含词优先；同长度指向不同实体时为冲突（多实体）。
        let mut alias_best: Option<(usize, Vec<i64>)> = None;
        if exact_name.is_empty() {
            for (term, id, _) in &self.names {
                if company_norm.contains(term.as_str())
                    || (!title_norm.is_empty() && title_norm.contains(term.as_str()))
                {
                    let length = term.chars().count();
                    match alias_best.as_mut() {
                        Some((best, ids)) if *best == length => {
                            if !ids.contains(id) {
                                ids.push(*id);
                            }
                        }
                        Some((best, _)) if *best > length => {}
                        _ => alias_best = Some((length, vec![*id])),
                    }
                }
            }
        }
        let name_entities: Vec<i64> = if !exact_name.is_empty() {
            exact_name.clone()
        } else {
            alias_best
                .as_ref()
                .map(|(_, ids)| ids.clone())
                .unwrap_or_default()
        };
        if let Some(first) = host_entities.first() {
            strong = true;
            support = "directory_official_host".into();
            relation = "official_host".into();
            entity_id = Some(*first);
            if host_entities.len() > 1 {
                counters.push("multiple_entities");
            }
            if !name_entities.is_empty() && !name_entities.contains(first) {
                counters.push("entity_mismatch");
            }
        } else if !name_entities.is_empty() {
            entity_id = name_entities.first().copied();
            if !exact_name.is_empty() {
                strong = true;
                support = "directory_legal_name".into();
                relation = "exact_name".into();
            } else {
                support = "directory_alias".into();
                relation = "alias".into();
            }
            if name_entities.len() > 1 {
                counters.push("multiple_entities");
            }
        }
        if let Some(id) = entity_id {
            if let Some(entity) = self.entities.get(&id) {
                candidate_label = entity.legal_name.clone();
                candidate_source = if self.bound {
                    "directory"
                } else {
                    "directory_unbound"
                };
                nav = ownership_nav_bucket(&entity.kind).into();
                region = entity.region_code.clone();
                if entity.kind == "supplier" {
                    counters.push("supplier_entity");
                }
                if self.bound && !entity.in_scope {
                    counters.push("entity_outside_scope");
                }
                if !self.bound {
                    // 未绑定项目：目录命中只是候选，不能作为强证据批量确认。
                    strong = false;
                }
            }
        }
        if support.is_empty() || !strong {
            let profile_domain = !host.is_empty()
                && (self
                    .approved_domains
                    .iter()
                    .any(|rule| domain_matches(&host, rule))
                    || self
                        .approved_hosts
                        .iter()
                        .any(|rule| host_equals_rule(&host, rule)));
            if profile_domain && !self.profile_name.is_empty() {
                strong = true;
                support = "profile_approved_domain".into();
                relation = "profile_domain".into();
                if entity_id.is_none() {
                    candidate_label = self.profile_name.clone();
                    candidate_source = "profile";
                    nav = "headquarters_shared".into();
                }
            }
        }
        if support.is_empty() {
            if let Some(term) = self
                .profile_terms
                .iter()
                .find(|term| text_matches_term(&combined, term))
            {
                support = "profile_alias".into();
                relation = "profile_alias".into();
                candidate_label = if self.profile_name.is_empty() {
                    term.clone()
                } else {
                    self.profile_name.clone()
                };
                candidate_source = "profile";
                nav = "headquarters_shared".into();
            }
        }
        if support.is_empty() && !company_norm.is_empty() && !network_label && !domain_label {
            // 标签候选：只用于导航与弱支持，不生成法人、不作为批量确认依据。
            let guess = guess_org_label(&input.company);
            support = "company_label".into();
            relation = format!("label_guess:{}", guess.kind);
            candidate_label = company_norm.clone();
            candidate_source = "label_candidate";
            nav = ownership_nav_bucket(guess.kind).into();
            region = if guess.region_clue.is_empty() {
                guess.location_clue.clone()
            } else {
                guess.region_clue.clone()
            };
        }
        if network_label {
            // 网络线索按原标签分开导航，绝不生成法人或候选实体。
            relation = "network_label".into();
            if support.is_empty() {
                candidate_label = input.company.trim().chars().take(120).collect();
                candidate_source = "network_clue";
            }
        } else if domain_label && support.is_empty() {
            relation = "domain_label".into();
        }
        counters.sort_unstable();
        counters.dedup();
        let counter_kind = counters.join("+");
        let conflict = counters
            .iter()
            .any(|kind| matches!(*kind, "entity_mismatch" | "multiple_entities" | "source_domain_mismatch" | "source_host_mismatch"));
        let manual = input.decision_source == "manual";
        let lane: &'static str = if manual {
            "manual"
        } else if conflict
            || (!counters.is_empty() && !support.is_empty() && support != "company_label")
        {
            "conflict"
        } else if !counters.is_empty() {
            "counter_evidence"
        } else if strong {
            "consistent"
        } else if !support.is_empty() {
            "insufficient"
        } else {
            "unknown"
        };
        if lane == "conflict" {
            nav = "conflict".into();
        }
        let (decision_status, decision_source) = if manual {
            (input.decision_status.clone(), "manual".to_string())
        } else {
            // 自动判断（含未评估）不区分状态：同证据的资产不因是否跑过评估被拆开。
            ("auto".to_string(), "auto".to_string())
        };
        let key = OwnershipGroupKey {
            lane,
            nav_bucket: nav,
            entity_id,
            candidate_label,
            candidate_source,
            relation_kind: relation,
            region_clue: region,
            boundary_kind,
            boundary_value,
            support_kind: support,
            counter_kind,
            decision_status,
            decision_source,
            authorization_status: input.authorization_status.clone(),
            exposure_eligible: input.exposure_eligible,
        };
        OwnershipMemberFacts {
            asset_id: input.asset_id,
            host: if host.is_empty() {
                input.ip.trim().to_string()
            } else {
                host
            },
            registrable,
            suffix_section: section.to_string(),
            company: input.company.chars().take(240).collect(),
            evidence_hash: ownership_evidence_hash(
                &input.company,
                &input.host,
                &input.link,
                &input.domain,
                &input.title,
                &input.ip,
                &input.extra_json,
            ),
            decision_state: input.decision_state.clone(),
            key,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// 分析任务：持久化、分块、可取消、同项目单活跃任务、心跳丢失可续跑。
// ---------------------------------------------------------------------------------------------

fn ownership_project_exists(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
            [project_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

/// O3：工作令牌以本进程随机前缀开头。应用重启后旧进程留下的令牌前缀必然不同，
/// 启动恢复可以立即接管未完成任务，而不必等待心跳过期。
fn ownership_process_nonce() -> &'static str {
    static NONCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NONCE.get_or_init(|| Uuid::new_v4().to_string().replace('-', "")[..12].to_string())
}

fn ownership_token_prefix() -> String {
    format!("{}:", ownership_process_nonce())
}

fn ownership_worker_token() -> String {
    format!("{}{}", ownership_token_prefix(), Uuid::new_v4())
}

const OWNERSHIP_ANALYSIS_MODES: [&str; 3] = ["auto", "full", "incremental"];

/// O3：增量只在“最新已完成快照的全部版本（含规则）与当前一致”时成立；否则按全量重算并记录原因
/// ——档案/目录/绑定/规则/范围/算法变化就是这样传播失效的。
fn ownership_resolve_mode(
    connection: &rusqlite::Connection,
    project_id: i64,
    mode: &str,
    current: &OwnershipAnalysisVersions,
) -> Result<(&'static str, Option<i64>, String), String> {
    if mode == "full" {
        return Ok(("full", None, "requested_full".into()));
    }
    let Some(base) = ownership_latest_snapshot(connection, project_id)? else {
        return Ok(("full", None, "no_snapshot".into()));
    };
    let stale = ownership_stale_reasons(&base, current);
    if !stale.is_empty() {
        return Ok((
            "full",
            None,
            format!("versions_changed:{}", stale.join(",")),
        ));
    }
    if base.watermark_at.is_empty() {
        return Ok(("full", None, "base_without_watermark".into()));
    }
    Ok(("incremental", Some(base.id), format!("base_run:{}", base.id)))
}

/// 返回 (任务, 是否需要启动工作线程)。已有新鲜的活跃任务时直接复用，不重复启动全量分析。
#[cfg(test)]
fn start_ownership_analysis_for_connection(
    connection: &mut rusqlite::Connection,
    project_id: i64,
) -> Result<(OwnershipAnalysisRunView, bool), String> {
    start_ownership_analysis_with_mode(connection, project_id, "full")
}

fn start_ownership_analysis_with_mode(
    connection: &mut rusqlite::Connection,
    project_id: i64,
    mode: &str,
) -> Result<(OwnershipAnalysisRunView, bool), String> {
    if !OWNERSHIP_ANALYSIS_MODES.contains(&mode) {
        return Err(format!("{OWNERSHIP_REVIEW_INVALID}: 不支持的分析模式 {mode}"));
    }
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if !ownership_project_exists(&transaction, project_id)? {
        return Err(format!("{OWNERSHIP_REVIEW_INVALID}: 工作空间不存在"));
    }
    let now = ownership_now_epoch(&transaction)?;
    let active: Option<(i64, i64, String)> = transaction
        .query_row(
            "SELECT id,heartbeat_epoch,worker_token FROM ownership_analysis_runs WHERE project_id=?1 AND status IN ('queued','running') ORDER BY id DESC LIMIT 1",
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some((run_id, heartbeat, token)) = active {
        // 心跳过期，或令牌属于已退出的旧进程：需要本进程接管续跑。
        let orphaned = heartbeat < now - OWNERSHIP_ANALYSIS_STALE_SECS
            || (!token.is_empty() && !token.starts_with(&ownership_token_prefix()));
        transaction.commit().map_err(|error| error.to_string())?;
        let run = ownership_read_run(connection, run_id)?.ok_or("分析任务不存在")?;
        return Ok((run, orphaned));
    }
    let versions = ownership_current_versions(&transaction, project_id)?;
    let (kind, base, reason) = ownership_resolve_mode(&transaction, project_id, mode, &versions)?;
    let (_, data_revision) = project_data_revision(&transaction, project_id)?;
    transaction
        .execute(
            "INSERT INTO ownership_analysis_runs(project_id,status,scope_version,data_revision,policy_version,directory_version,binding_version,algorithm_version,heartbeat_epoch,rules_version,run_kind,base_run_id,mode_reason) VALUES(?1,'queued',?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![project_id, versions.scope_version, data_revision, versions.policy_version, versions.directory_version, versions.binding_version, versions.algorithm_version, now, versions.rules_version, kind, base, reason],
        )
        .map_err(|error| error.to_string())?;
    let run_id = transaction.last_insert_rowid();
    transaction.commit().map_err(|error| error.to_string())?;
    Ok((
        ownership_read_run(connection, run_id)?.ok_or("分析任务不存在")?,
        true,
    ))
}

/// 认领任务：queued，或心跳过期 / 令牌属于旧进程的 running（崩溃、掉线、重启后续跑）。
/// 返回工作令牌；被本进程其他工作线程持有时返回 None。续跑从 last_asset_id 继续，resume_count+1。
fn ownership_claim_run(
    connection: &mut rusqlite::Connection,
    run_id: i64,
) -> Result<Option<String>, String> {
    let token = ownership_worker_token();
    let prefix = ownership_token_prefix();
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let now = ownership_now_epoch(&transaction)?;
    let claimed = transaction
        .execute(
            "UPDATE ownership_analysis_runs SET resume_count=resume_count+CASE WHEN status='running' THEN 1 ELSE 0 END,status='running',worker_token=?2,heartbeat_epoch=?3,started_at=CASE WHEN started_at='' THEN datetime('now','localtime') ELSE started_at END,watermark_at=CASE WHEN last_asset_id=0 THEN datetime('now','localtime') ELSE watermark_at END WHERE id=?1 AND cancel_requested=0 AND (status='queued' OR (status='running' AND (heartbeat_epoch<?4 OR substr(worker_token,1,length(?5))<>?5)))",
            params![run_id, token, now, now - OWNERSHIP_ANALYSIS_STALE_SECS, prefix],
        )
        .map_err(|error| error.to_string())?;
    if claimed == 0 {
        return Ok(None);
    }
    let run = ownership_read_run(&transaction, run_id)?.ok_or("分析任务不存在")?;
    let current = ownership_current_versions(&transaction, run.project_id)?;
    let last_asset: i64 = transaction
        .query_row(
            "SELECT last_asset_id FROM ownership_analysis_runs WHERE id=?1",
            [run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if current != run.versions {
        if last_asset == 0 {
            // 尚未处理任何资产：以认领时的版本为准重新绑定（任务排队期间档案/目录/规则被保存）。
            // 增量任务的基线快照因此失效，改为全量并记录原因。
            let stale = ownership_stale_reasons(&run, &current);
            let (_, data_revision) = project_data_revision(&transaction, run.project_id)?;
            transaction
                .execute(
                    "UPDATE ownership_analysis_runs SET policy_version=?2,directory_version=?3,binding_version=?4,scope_version=?5,algorithm_version=?6,data_revision=?7,rules_version=?8,run_kind='full',base_run_id=NULL,mode_reason=CASE WHEN run_kind='incremental' THEN ?9 ELSE mode_reason END WHERE id=?1",
                    params![run_id, current.policy_version, current.directory_version, current.binding_version, current.scope_version, current.algorithm_version, data_revision, current.rules_version, format!("versions_changed_before_start:{}", stale.join(","))],
                )
                .map_err(|error| error.to_string())?;
        } else {
            // 中断续跑时版本已变：不能把两套规则的结果拼成一个快照。
            ownership_discard_staging(&transaction, run_id)?;
            transaction
                .execute(
                    "UPDATE ownership_analysis_runs SET status='failed',error='续跑时策略/目录/绑定/规则/算法版本已变化，已丢弃中间结果，请重新分析',finished_at=datetime('now','localtime'),worker_token='' WHERE id=?1",
                    [run_id],
                )
                .map_err(|error| error.to_string())?;
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(None);
        }
    }
    let total: i64 = transaction
        .query_row(
            &format!("SELECT COUNT(*) FROM project_assets pa JOIN assets a ON a.id=pa.asset_id WHERE pa.project_id=?1 AND {OWNERSHIP_SCOPE_SQL}"),
            [run.project_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    // 已处理数 + 剩余范围内资产数：续跑时如实反映进度（不把已处理部分重算一遍）。
    let remaining: i64 = transaction
        .query_row(
            &format!("SELECT COUNT(*) FROM project_assets pa JOIN assets a ON a.id=pa.asset_id WHERE pa.project_id=?1 AND pa.asset_id>?2 AND {OWNERSHIP_SCOPE_SQL}"),
            params![run.project_id, last_asset],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE ownership_analysis_runs SET total_assets=CASE WHEN last_asset_id=0 THEN ?2 ELSE processed_assets+?3 END WHERE id=?1",
            params![run_id, total, remaining],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(Some(token))
}

fn ownership_discard_staging(connection: &rusqlite::Connection, run_id: i64) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM ownership_review_group_members WHERE run_id=?1",
            [run_id],
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "DELETE FROM ownership_review_groups WHERE run_id=?1",
            [run_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnershipStep {
    Continued,
    Completed,
    Cancelled,
    /// 令牌已失效（被他人接管或任务已结束），本工作线程停止。
    Lost,
}

/// 组键的全部列（字符串形式）：新分类的键与增量沿用的基线组行都转换为它再写入。
#[derive(Debug, Clone)]
struct OwnershipGroupColumns {
    lane: String,
    nav_bucket: String,
    entity_id: Option<i64>,
    candidate_label: String,
    candidate_source: String,
    relation_kind: String,
    region_clue: String,
    boundary_kind: String,
    boundary_value: String,
    support_kind: String,
    counter_kind: String,
    decision_status: String,
    decision_source: String,
    authorization_status: String,
    exposure_eligible: i64,
}

impl From<&OwnershipGroupKey> for OwnershipGroupColumns {
    fn from(key: &OwnershipGroupKey) -> Self {
        Self {
            lane: key.lane.into(),
            nav_bucket: key.nav_bucket.clone(),
            entity_id: key.entity_id,
            candidate_label: key.candidate_label.clone(),
            candidate_source: key.candidate_source.into(),
            relation_kind: key.relation_kind.clone(),
            region_clue: key.region_clue.clone(),
            boundary_kind: key.boundary_kind.into(),
            boundary_value: key.boundary_value.clone(),
            support_kind: key.support_kind.clone(),
            counter_kind: key.counter_kind.clone(),
            decision_status: key.decision_status.clone(),
            decision_source: key.decision_source.clone(),
            authorization_status: key.authorization_status.clone(),
            exposure_eligible: key.exposure_eligible,
        }
    }
}

/// 一个待写入的成员：组键文本、组列与成员事实。
struct OwnershipStagedMember {
    asset_id: i64,
    key_text: String,
    columns: OwnershipGroupColumns,
    host: String,
    registrable: String,
    suffix_section: String,
    company: String,
    decision_state: String,
    evidence_hash: String,
}

struct OwnershipChunkRow {
    input: OwnershipAssetInput,
    /// 增量：基线快照中该资产的成员与组行，且归属结论未变、资产与关联低于数据水位线 → 直接沿用。
    carried: Option<OwnershipStagedMember>,
}

/// 按稳定主键读取一块范围内资产。base_run_id/watermark 为空（全量）时不会命中任何基线成员。
fn ownership_read_chunk(
    connection: &rusqlite::Connection,
    project_id: i64,
    after_asset_id: i64,
    limit: i64,
    base_run_id: i64,
    watermark: &str,
) -> Result<Vec<OwnershipChunkRow>, String> {
    let mut statement = connection
        .prepare(&format!("SELECT a.id,a.company,a.host,a.link,a.domain,a.title,a.ip,a.extra_json,COALESCE(o.status,'unreviewed'),COALESCE(o.source,''),COALESCE(o.authorization_status,'unknown'),COALESCE(o.exposure_eligible,0),{OWNERSHIP_DECISION_STATE_SQL},CASE WHEN bm.asset_id IS NOT NULL AND bg.id IS NOT NULL AND bm.decision_state={OWNERSHIP_DECISION_STATE_SQL} AND a.last_seen<?5 AND pa.last_seen<?5 THEN 1 ELSE 0 END,bm.host,bm.registrable_domain,bm.suffix_section,bm.company,bm.evidence_hash,bg.group_key,bg.lane,bg.nav_bucket,bg.candidate_entity_id,bg.candidate_label,bg.candidate_source,bg.relation_kind,bg.region_clue,bg.boundary_kind,bg.boundary_value,bg.support_kind,bg.counter_kind,bg.decision_status,bg.decision_source,bg.authorization_status,bg.exposure_eligible FROM project_assets pa JOIN assets a ON a.id=pa.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id LEFT JOIN ownership_review_group_members bm ON bm.run_id=?4 AND bm.asset_id=pa.asset_id LEFT JOIN ownership_review_groups bg ON bg.id=bm.group_id AND bg.run_id=?4 WHERE pa.project_id=?1 AND pa.asset_id>?2 AND {OWNERSHIP_SCOPE_SQL} ORDER BY pa.asset_id LIMIT ?3"))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![project_id, after_asset_id, limit, base_run_id, watermark],
            |row| {
                let input = OwnershipAssetInput {
                    asset_id: row.get(0)?,
                    company: row.get(1)?,
                    host: row.get(2)?,
                    link: row.get(3)?,
                    domain: row.get(4)?,
                    title: row.get(5)?,
                    ip: row.get(6)?,
                    extra_json: row.get(7)?,
                    decision_status: row.get(8)?,
                    decision_source: row.get(9)?,
                    authorization_status: row.get(10)?,
                    exposure_eligible: row.get(11)?,
                    decision_state: row.get(12)?,
                };
                let carried = if row.get::<_, i64>(13)? == 1 {
                    Some(OwnershipStagedMember {
                        asset_id: input.asset_id,
                        host: row.get(14)?,
                        registrable: row.get(15)?,
                        suffix_section: row.get(16)?,
                        company: row.get(17)?,
                        evidence_hash: row.get(18)?,
                        key_text: row.get(19)?,
                        columns: OwnershipGroupColumns {
                            lane: row.get(20)?,
                            nav_bucket: row.get(21)?,
                            entity_id: row.get(22)?,
                            candidate_label: row.get(23)?,
                            candidate_source: row.get(24)?,
                            relation_kind: row.get(25)?,
                            region_clue: row.get(26)?,
                            boundary_kind: row.get(27)?,
                            boundary_value: row.get(28)?,
                            support_kind: row.get(29)?,
                            counter_kind: row.get(30)?,
                            decision_status: row.get(31)?,
                            decision_source: row.get(32)?,
                            authorization_status: row.get(33)?,
                            exposure_eligible: row.get(34)?,
                        },
                        decision_state: input.decision_state.clone(),
                    })
                } else {
                    None
                };
                Ok(OwnershipChunkRow { input, carried })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

/// 处理一个分块：读稳定主键区间，短事务写入成员与组计数，并推进 last_asset_id / 心跳。
/// (project, status, worker_token, cancel, last_asset_id, base_run, base watermark)
type OwnershipStepState = (i64, String, String, i64, i64, Option<i64>, String);

/// 增量任务对未变化行沿用基线快照的组键与证据指纹，只对变化行重新分类；输出与全量分析逐资产一致。
fn ownership_analysis_step(
    connection: &mut rusqlite::Connection,
    run_id: i64,
    token: &str,
    classifier: &OwnershipClassifier,
    chunk: i64,
) -> Result<OwnershipStep, String> {
    let state: Option<OwnershipStepState> = connection
        .query_row(
            "SELECT r.project_id,r.status,r.worker_token,r.cancel_requested,r.last_asset_id,CASE WHEN r.run_kind='incremental' THEN r.base_run_id END,COALESCE(b.watermark_at,'') FROM ownership_analysis_runs r LEFT JOIN ownership_analysis_runs b ON b.id=r.base_run_id AND r.run_kind='incremental' AND b.status='completed' AND b.members_pruned=0 WHERE r.id=?1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((project_id, status, owner, cancel, last_asset_id, base_run, watermark)) = state else {
        return Ok(OwnershipStep::Lost);
    };
    if status != "running" || owner != token {
        return Ok(OwnershipStep::Lost);
    }
    if cancel != 0 {
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        ownership_discard_staging(&transaction, run_id)?;
        transaction
            .execute(
                "UPDATE ownership_analysis_runs SET status='cancelled',finished_at=datetime('now','localtime'),worker_token='' WHERE id=?1 AND worker_token=?2",
                params![run_id, token],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        return Ok(OwnershipStep::Cancelled);
    }
    let rows = ownership_read_chunk(
        connection,
        project_id,
        last_asset_id,
        chunk.max(1),
        base_run.unwrap_or(0),
        &watermark,
    )?;
    if rows.is_empty() {
        return ownership_finalize_run(connection, run_id, token, project_id);
    }
    let next_last = rows
        .last()
        .map(|row| row.input.asset_id)
        .unwrap_or(last_asset_id);
    let (mut carried_count, mut reclassified_count) = (0i64, 0i64);
    let members: Vec<OwnershipStagedMember> = rows
        .into_iter()
        .map(|row| match row.carried {
            Some(member) => {
                carried_count += 1;
                member
            }
            None => {
                reclassified_count += 1;
                let fact = classifier.classify(&row.input);
                OwnershipStagedMember {
                    asset_id: fact.asset_id,
                    key_text: fact.key.text(),
                    columns: OwnershipGroupColumns::from(&fact.key),
                    host: fact.host,
                    registrable: fact.registrable,
                    suffix_section: fact.suffix_section,
                    company: fact.company,
                    decision_state: fact.decision_state,
                    evidence_hash: fact.evidence_hash,
                }
            }
        })
        .collect();
    let mut by_key: HashMap<String, (OwnershipGroupColumns, i64)> = HashMap::new();
    for member in &members {
        by_key
            .entry(member.key_text.clone())
            .or_insert_with(|| (member.columns.clone(), 0))
            .1 += 1;
    }
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let now = ownership_now_epoch(&transaction)?;
    let advanced = transaction
        .execute(
            "UPDATE ownership_analysis_runs SET processed_assets=processed_assets+?3,last_asset_id=?4,heartbeat_epoch=?5,reclassified_assets=reclassified_assets+?7,carried_assets=carried_assets+?8 WHERE id=?1 AND worker_token=?2 AND status='running' AND cancel_requested=0 AND last_asset_id=?6",
            params![run_id, token, members.len() as i64, next_last, now, last_asset_id, reclassified_count, carried_count],
        )
        .map_err(|error| error.to_string())?;
    if advanced == 0 {
        // 期间被取消或被接管：放弃本块（事务回滚），下一步按最新状态处理。
        return Ok(OwnershipStep::Continued);
    }
    let mut group_ids: HashMap<String, i64> = HashMap::new();
    {
        let mut upsert = transaction
            .prepare("INSERT INTO ownership_review_groups(run_id,project_id,group_key,lane,nav_bucket,candidate_entity_id,candidate_label,candidate_source,relation_kind,region_clue,boundary_kind,boundary_value,support_kind,counter_kind,decision_status,decision_source,authorization_status,exposure_eligible,member_count) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19) ON CONFLICT(run_id,group_key) DO UPDATE SET member_count=member_count+excluded.member_count RETURNING id")
            .map_err(|error| error.to_string())?;
        for (text, (key, count)) in &by_key {
            let id: i64 = upsert
                .query_row(
                    params![
                        run_id,
                        project_id,
                        text,
                        key.lane,
                        key.nav_bucket,
                        key.entity_id,
                        key.candidate_label,
                        key.candidate_source,
                        key.relation_kind,
                        key.region_clue,
                        key.boundary_kind,
                        key.boundary_value,
                        key.support_kind,
                        key.counter_kind,
                        key.decision_status,
                        key.decision_source,
                        key.authorization_status,
                        key.exposure_eligible,
                        count
                    ],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            group_ids.insert(text.clone(), id);
        }
        let mut insert = transaction
            .prepare("INSERT INTO ownership_review_group_members(run_id,asset_id,project_id,group_id,host,registrable_domain,suffix_section,company,decision_state,evidence_hash) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)")
            .map_err(|error| error.to_string())?;
        for member in &members {
            let group_id = group_ids
                .get(&member.key_text)
                .copied()
                .ok_or("分组键丢失")?;
            insert
                .execute(params![
                    run_id,
                    member.asset_id,
                    project_id,
                    group_id,
                    member.host,
                    member.registrable,
                    member.suffix_section,
                    member.company,
                    member.decision_state,
                    member.evidence_hash
                ])
                .map_err(|error| error.to_string())?;
        }
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(OwnershipStep::Continued)
}

fn ownership_group_priority(lane: &str) -> i64 {
    match lane {
        "conflict" => 10,
        "counter_evidence" => 20,
        "consistent" => 30,
        "insufficient" => 40,
        "unknown" => 50,
        _ => 90,
    }
}

fn ownership_group_reason(
    lane: &str,
    support: &str,
    counter: &str,
    authorization: &str,
    eligible: i64,
) -> String {
    let support_text = match support {
        "directory_official_host" => "已确认目录实体的官方域名（精确主机）",
        "directory_legal_name" => "公司标签与已确认目录实体法定名称完全一致",
        "directory_alias" => "名称包含已确认目录实体的名称/别名（弱）",
        "profile_approved_domain" => "命中已保存档案的确认域名",
        "profile_alias" => "名称命中已保存档案的主体/别名（弱）",
        "company_label" => "仅公司标签候选（未经目录确认）",
        _ => "无支持证据",
    };
    let counter_text = if counter.is_empty() {
        "无".to_string()
    } else {
        counter
            .split('+')
            .map(|kind| match kind {
                "hosting_platform" => "公共后缀 private 区托管平台（租户边界）",
                "shared_domain" => "共享域名/共享主机",
                "profile_excluded_domain" => "命中排除域名",
                "profile_excluded_name" => "命中排除主体",
                "network_label" => "公司字段是 CIDR/IP/ASN 网络线索",
                "supplier_entity" => "目录实体为关联供应商",
                "entity_outside_scope" => "目录实体不在本项目绑定范围",
                "entity_mismatch" => "官方主机所属实体与名称实体不一致",
                "multiple_entities" => "同时命中多个目录实体",
                other => other,
            })
            .collect::<Vec<_>>()
            .join("；")
    };
    let lane_text = match lane {
        "consistent" => "规则一致",
        "insufficient" => "证据不足",
        "counter_evidence" => "存在反证",
        "conflict" => "证据冲突",
        "unknown" => "未知",
        _ => "已有人工结论",
    };
    format!("{lane_text}。依据：{support_text}；反证：{counter_text}；现有权限：授权 {authorization}，暴露面准入 {}", if eligible != 0 { "是" } else { "否" })
}

type OwnershipSampleAccumulator = (Vec<i64>, HashSet<String>, Vec<i64>, i64);

/// 全部分块写完后发布快照：组计数与成员对账、样本、排序；校验失败则任务失败，不发布部分结果。
fn ownership_finalize_run(
    connection: &mut rusqlite::Connection,
    run_id: i64,
    token: &str,
    project_id: i64,
) -> Result<OwnershipStep, String> {
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let owned: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM ownership_analysis_runs WHERE id=?1 AND worker_token=?2 AND status='running' AND cancel_requested=0)",
            params![run_id, token],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !owned {
        return Ok(OwnershipStep::Continued);
    }
    let (members, processed, split): (i64, i64, i64) = transaction
        .query_row(
            "SELECT (SELECT COUNT(*) FROM ownership_review_group_members WHERE run_id=?1),processed_assets,reclassified_assets+carried_assets FROM ownership_analysis_runs WHERE id=?1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| error.to_string())?;
    let group_sum: i64 = transaction
        .query_row(
            "SELECT COALESCE(SUM(member_count),0) FROM ownership_review_groups WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let indexed: i64 = transaction
        .query_row(
            "SELECT COALESCE(SUM(c),0) FROM (SELECT g.member_count=COUNT(m.asset_id) AS c FROM ownership_review_groups g LEFT JOIN ownership_review_group_members m ON m.run_id=g.run_id AND m.group_id=g.id WHERE g.run_id=?1 GROUP BY g.id)",
            [run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let groups: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM ownership_review_groups WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if members != processed || group_sum != members || indexed != groups || split != processed {
        ownership_discard_staging(&transaction, run_id)?;
        transaction
            .execute(
                "UPDATE ownership_analysis_runs SET status='failed',error=?2,finished_at=datetime('now','localtime'),worker_token='' WHERE id=?1",
                params![run_id, format!("对账失败：成员 {members}，已处理 {processed}（重算+沿用 {split}），组计数合计 {group_sum}；未发布快照")],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        return Ok(OwnershipStep::Completed);
    }
    // 组统计、代表样本（按不同精确主机各取 1 条，最多 5 条，不足按资产 ID 补齐）与说明。
    let group_rows: Vec<(i64, String, String, String, String, i64, i64)> = {
        let mut statement = transaction
            .prepare("SELECT id,lane,support_kind,counter_kind,authorization_status,exposure_eligible,member_count FROM ownership_review_groups WHERE run_id=?1")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([run_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    // group_id -> (按不同主机挑选的样本, 已见主机, 补位样本, 不同主机数)
    let mut samples: HashMap<i64, OwnershipSampleAccumulator> = HashMap::new();
    {
        let mut statement = transaction
            .prepare("SELECT group_id,asset_id,host FROM ownership_review_group_members WHERE run_id=?1 ORDER BY group_id,asset_id")
            .map_err(|error| error.to_string())?;
        let mut rows = statement
            .query([run_id])
            .map_err(|error| error.to_string())?;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let group_id: i64 = row.get(0).map_err(|error| error.to_string())?;
            let asset_id: i64 = row.get(1).map_err(|error| error.to_string())?;
            let host: String = row.get(2).map_err(|error| error.to_string())?;
            let entry = samples
                .entry(group_id)
                .or_insert_with(|| (Vec::new(), HashSet::new(), Vec::new(), 0));
            let new_host = entry.1.insert(host);
            if new_host {
                entry.3 += 1;
            }
            if new_host && entry.0.len() < OWNERSHIP_GROUP_SAMPLE_LIMIT {
                entry.0.push(asset_id);
            } else if entry.2.len() < OWNERSHIP_GROUP_SAMPLE_LIMIT {
                entry.2.push(asset_id);
            }
        }
    }
    let mut ranked: Vec<(i64, i64, i64)> = group_rows
        .iter()
        .map(|row| (ownership_group_priority(&row.1), -row.6, row.0))
        .collect();
    ranked.sort_unstable();
    let rank_of: HashMap<i64, i64> = ranked
        .iter()
        .enumerate()
        .map(|(index, row)| (row.2, index as i64 + 1))
        .collect();
    {
        let mut update = transaction
            .prepare("UPDATE ownership_review_groups SET priority=?2,sort_rank=?3,distinct_hosts=?4,sample_asset_ids_json=?5,sample_rule=?6,reason=?7 WHERE id=?1")
            .map_err(|error| error.to_string())?;
        for (id, lane, support, counter, authorization, eligible, _) in &group_rows {
            let (mut picked, _, filler, distinct) = samples.remove(id).unwrap_or_default();
            for asset_id in filler {
                if picked.len() >= OWNERSHIP_GROUP_SAMPLE_LIMIT {
                    break;
                }
                picked.push(asset_id);
            }
            picked.sort_unstable();
            update
                .execute(params![id, ownership_group_priority(lane), rank_of.get(id).copied().unwrap_or(0), distinct, serde_json::to_string(&picked).unwrap_or_else(|_| "[]".into()), "按不同精确主机各取 1 条（最多 5 条），不足时按资产 ID 补齐；样本不代表其余成员已核验", ownership_group_reason(lane, support, counter, authorization, *eligible)])
                .map_err(|error| error.to_string())?;
        }
    }
    let (_, data_revision) = project_data_revision(&transaction, project_id)?;
    transaction
        .execute(
            "UPDATE ownership_analysis_runs SET status='completed',group_count=?3,member_count=?4,data_revision_at_finish=?5,finished_at=datetime('now','localtime'),worker_token='' WHERE id=?1 AND worker_token=?2",
            params![run_id, token, groups, members, data_revision],
        )
        .map_err(|error| error.to_string())?;
    // 只保留最近两份已完成快照的成员索引，更早的快照仅保留任务元数据（members_pruned=1）。
    let old: Vec<i64> = {
        let mut statement = transaction
            .prepare("SELECT id FROM ownership_analysis_runs WHERE project_id=?1 AND status='completed' AND members_pruned=0 ORDER BY id DESC LIMIT -1 OFFSET ?2")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![project_id, OWNERSHIP_SNAPSHOTS_KEPT], |row| {
                row.get(0)
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    for old_run in old {
        ownership_discard_staging(&transaction, old_run)?;
        transaction
            .execute(
                "UPDATE ownership_analysis_runs SET members_pruned=1 WHERE id=?1",
                [old_run],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(OwnershipStep::Completed)
}

fn ownership_mark_run_failed(
    connection: &rusqlite::Connection,
    run_id: i64,
    token: &str,
    error: &str,
) {
    let _ = ownership_discard_staging(connection, run_id);
    let _ = connection.execute(
        "UPDATE ownership_analysis_runs SET status='failed',error=?3,finished_at=datetime('now','localtime'),worker_token='' WHERE id=?1 AND worker_token=?2",
        params![run_id, token, error.chars().take(500).collect::<String>()],
    );
}

/// 同步驱动一个已认领的任务直至结束（或 max_steps 步）；后台线程与测试共用。
fn ownership_drive_run(
    connection: &mut rusqlite::Connection,
    run_id: i64,
    token: &str,
    chunk: i64,
    max_steps: Option<usize>,
) -> Result<OwnershipStep, String> {
    let project_id: i64 = connection
        .query_row(
            "SELECT project_id FROM ownership_analysis_runs WHERE id=?1",
            [run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let classifier = match ownership_classifier(connection, project_id) {
        Ok(classifier) => classifier,
        Err(error) => {
            ownership_mark_run_failed(connection, run_id, token, &error);
            return Err(error);
        }
    };
    let mut steps = 0usize;
    loop {
        if max_steps.is_some_and(|limit| steps >= limit) {
            return Ok(OwnershipStep::Continued);
        }
        steps += 1;
        match ownership_analysis_step(connection, run_id, token, &classifier, chunk) {
            Ok(OwnershipStep::Continued) => continue,
            Ok(outcome) => return Ok(outcome),
            Err(error) => {
                ownership_mark_run_failed(connection, run_id, token, &error);
                return Err(error);
            }
        }
    }
}

fn ownership_analysis_worker(db_path: PathBuf, run_id: i64) {
    let Ok(mut connection) = db::open(&db_path) else {
        return;
    };
    if let Ok(Some(token)) = ownership_claim_run(&mut connection, run_id) {
        let _ = ownership_drive_run(
            &mut connection,
            run_id,
            &token,
            OWNERSHIP_ANALYSIS_CHUNK,
            None,
        );
    }
}

#[tauri::command]
pub fn start_ownership_analysis(
    state: State<AppState>,
    project_id: i64,
    mode: Option<String>,
) -> Result<OwnershipAnalysisRunView, String> {
    ownership_ensure_startup_recovery(&state.db_path);
    let (run, needs_worker) = start_ownership_analysis_with_mode(
        &mut db::open(&state.db_path)?,
        project_id,
        mode.as_deref().unwrap_or("auto"),
    )?;
    if needs_worker {
        let db_path = state.db_path.clone();
        let run_id = run.id;
        thread::spawn(move || ownership_analysis_worker(db_path, run_id));
    }
    Ok(run)
}

fn cancel_ownership_analysis_for_connection(
    connection: &mut rusqlite::Connection,
    project_id: i64,
    run_id: i64,
) -> Result<OwnershipAnalysisRunView, String> {
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let run = ownership_read_run(&transaction, run_id)?
        .filter(|run| run.project_id == project_id)
        .ok_or_else(|| format!("{OWNERSHIP_REVIEW_INVALID}: 分析任务不属于当前工作空间"))?;
    match run.status.as_str() {
        "queued" => {
            ownership_discard_staging(&transaction, run_id)?;
            transaction
                .execute(
                    "UPDATE ownership_analysis_runs SET status='cancelled',cancel_requested=1,finished_at=datetime('now','localtime'),worker_token='' WHERE id=?1",
                    [run_id],
                )
                .map_err(|error| error.to_string())?;
        }
        "running" => {
            // 工作线程在下一个分块边界看到取消标志后丢弃中间结果并结束；心跳已过期的任务直接结束。
            let now = ownership_now_epoch(&transaction)?;
            let stale: bool = transaction
                .query_row(
                    "SELECT heartbeat_epoch<?2 FROM ownership_analysis_runs WHERE id=?1",
                    params![run_id, now - OWNERSHIP_ANALYSIS_STALE_SECS],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            if stale {
                ownership_discard_staging(&transaction, run_id)?;
                transaction
                    .execute("UPDATE ownership_analysis_runs SET status='cancelled',cancel_requested=1,finished_at=datetime('now','localtime'),worker_token='' WHERE id=?1", [run_id])
                    .map_err(|error| error.to_string())?;
            } else {
                transaction
                    .execute(
                        "UPDATE ownership_analysis_runs SET cancel_requested=1 WHERE id=?1",
                        [run_id],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        _ => {}
    }
    transaction.commit().map_err(|error| error.to_string())?;
    ownership_read_run(connection, run_id)?.ok_or_else(|| "分析任务不存在".into())
}

#[tauri::command]
pub fn cancel_ownership_analysis(
    state: State<AppState>,
    project_id: i64,
    run_id: i64,
) -> Result<OwnershipAnalysisRunView, String> {
    cancel_ownership_analysis_for_connection(&mut db::open(&state.db_path)?, project_id, run_id)
}

// ---------------------------------------------------------------------------------------------
// 读取：任务概览（决策组队列顶部数字、版本、readiness）、组/成员 cursor 分页。
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipReviewQueueCounts {
    /// 未处理（非人工、未被组操作处理）组内资产数。
    pub pending_assets: i64,
    pub pending_groups: i64,
    pub conflict_groups: i64,
    pub counter_evidence_groups: i64,
    pub completed_groups: i64,
    pub manual_assets: i64,
    /// 快照之后新进入门禁范围的资产（不在任何主组中，不会被批量动作暗中纳入）。
    pub new_since_snapshot: i64,
    /// 快照成员中已删除/离开范围/归属结论变化的数量（证据字段变化在预览时逐条核对）。
    pub changed_since_snapshot: i64,
    pub member_total: i64,
    pub group_member_sum: i64,
    pub group_total: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipAnalysisOverview {
    pub project_id: i64,
    pub active_run: Option<OwnershipAnalysisRunView>,
    pub latest_run: Option<OwnershipAnalysisRunView>,
    pub snapshot: Option<OwnershipAnalysisRunView>,
    pub current: OwnershipAnalysisVersions,
    pub current_data_revision: String,
    pub stale_reasons: Vec<String>,
    pub queue: Option<OwnershipReviewQueueCounts>,
    pub readiness: Vec<String>,
}

fn ownership_stale_reasons(
    run: &OwnershipAnalysisRunView,
    current: &OwnershipAnalysisVersions,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if run.versions.policy_version != current.policy_version {
        reasons.push(format!(
            "policy:{}->{}",
            run.versions.policy_version, current.policy_version
        ));
    }
    if run.versions.directory_version != current.directory_version {
        reasons.push(format!(
            "directory:{}->{}",
            run.versions.directory_version, current.directory_version
        ));
    }
    if run.versions.binding_version != current.binding_version {
        reasons.push(format!(
            "binding:{}->{}",
            run.versions.binding_version, current.binding_version
        ));
    }
    if run.versions.scope_version != current.scope_version {
        reasons.push("scope".into());
    }
    if run.versions.algorithm_version != current.algorithm_version {
        reasons.push("algorithm".into());
    }
    if run.versions.rules_version != current.rules_version {
        reasons.push("rules".into());
    }
    reasons
}

fn ownership_latest_snapshot(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<Option<OwnershipAnalysisRunView>, String> {
    ownership_project_run(
        connection,
        project_id,
        "status='completed' AND members_pruned=0",
    )
}

fn read_ownership_analysis_for_connection(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<OwnershipAnalysisOverview, String> {
    if !ownership_project_exists(connection, project_id)? {
        return Err(format!("{OWNERSHIP_REVIEW_INVALID}: 工作空间不存在"));
    }
    let current = ownership_current_versions(connection, project_id)?;
    let (_, current_data_revision) = project_data_revision(connection, project_id)?;
    let active_run =
        ownership_project_run(connection, project_id, "status IN ('queued','running')")?;
    let latest_run = ownership_project_run(connection, project_id, "1=1")?;
    let snapshot = ownership_latest_snapshot(connection, project_id)?;
    let mut readiness = vec!["rules_only".to_string(), "model_not_ready".to_string()];
    if active_run.is_some() {
        readiness.push("analysis_running".into());
    }
    let mut stale_reasons = Vec::new();
    let queue = if let Some(run) = snapshot.as_ref() {
        stale_reasons = ownership_stale_reasons(run, &current);
        if run.data_revision_at_finish != current_data_revision {
            stale_reasons.push("data".into());
        }
        let counts = connection
            .query_row(
                "SELECT COALESCE(SUM(CASE WHEN lane<>'manual' AND applied_operations=0 THEN member_count ELSE 0 END),0),COALESCE(SUM(lane<>'manual' AND applied_operations=0),0),COALESCE(SUM(lane='conflict'),0),COALESCE(SUM(lane='counter_evidence'),0),COALESCE(SUM(lane='manual' OR applied_operations>0),0),COALESCE(SUM(CASE WHEN lane='manual' THEN member_count ELSE 0 END),0),COALESCE(SUM(member_count),0),COUNT(*) FROM ownership_review_groups WHERE run_id=?1",
                [run.id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?, row.get::<_, i64>(5)?, row.get::<_, i64>(6)?, row.get::<_, i64>(7)?)),
            )
            .map_err(|error| error.to_string())?;
        let member_total: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM ownership_review_group_members WHERE run_id=?1",
                [run.id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let new_since_snapshot: i64 = connection
            .query_row(
                &format!("SELECT COUNT(*) FROM project_assets pa JOIN assets a ON a.id=pa.asset_id WHERE pa.project_id=?1 AND {OWNERSHIP_SCOPE_SQL} AND NOT EXISTS(SELECT 1 FROM ownership_review_group_members m WHERE m.run_id=?2 AND m.asset_id=pa.asset_id)"),
                params![project_id, run.id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        // 已被本快照组操作处理过的成员不算“未处理变化”。
        let changed_since_snapshot: i64 = connection
            .query_row(
                &format!("SELECT COUNT(*) FROM ownership_review_group_members m LEFT JOIN project_assets pa ON pa.project_id=m.project_id AND pa.asset_id=m.asset_id LEFT JOIN assets a ON a.id=m.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=m.project_id AND o.asset_id=m.asset_id WHERE m.run_id=?1 AND NOT EXISTS(SELECT 1 FROM ownership_operation_items i JOIN ownership_operations op ON op.operation_id=i.operation_id WHERE op.run_id=m.run_id AND i.asset_id=m.asset_id AND i.outcome='applied') AND (pa.asset_id IS NULL OR a.id IS NULL OR NOT ({OWNERSHIP_SCOPE_SQL}) OR {OWNERSHIP_DECISION_STATE_SQL}<>m.decision_state)"),
                [run.id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        Some(OwnershipReviewQueueCounts {
            pending_assets: counts.0,
            pending_groups: counts.1,
            conflict_groups: counts.2,
            counter_evidence_groups: counts.3,
            completed_groups: counts.4,
            manual_assets: counts.5,
            new_since_snapshot,
            changed_since_snapshot,
            member_total,
            group_member_sum: counts.6,
            group_total: counts.7,
        })
    } else {
        readiness.push("no_snapshot".into());
        None
    };
    if !stale_reasons.is_empty() {
        readiness.push("stale_snapshot".into());
    }
    Ok(OwnershipAnalysisOverview {
        project_id,
        active_run,
        latest_run,
        snapshot,
        current,
        current_data_revision,
        stale_reasons,
        queue,
        readiness,
    })
}

#[tauri::command]
pub fn read_ownership_analysis(
    state: State<AppState>,
    project_id: i64,
) -> Result<OwnershipAnalysisOverview, String> {
    ownership_ensure_startup_recovery(&state.db_path);
    read_ownership_analysis_for_connection(&db::open(&state.db_path)?, project_id)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipMemberCurrent {
    pub linked: bool,
    pub in_scope: bool,
    pub status: String,
    pub source: String,
    pub authorization_status: String,
    pub exposure_eligible: bool,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipGroupMemberView {
    pub asset_id: i64,
    pub host: String,
    pub registrable_domain: String,
    pub suffix_section: String,
    pub company: String,
    pub link: String,
    pub title: String,
    pub current: OwnershipMemberCurrent,
    /// 快照后归属结论/证据/范围/删除状态是否已变化（变化成员不会被该快照的批量动作写入）。
    pub changed_since_snapshot: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipReviewGroupView {
    pub id: i64,
    pub run_id: i64,
    pub lane: String,
    pub nav_bucket: String,
    pub candidate_entity_id: Option<i64>,
    pub candidate_label: String,
    pub candidate_source: String,
    pub relation_kind: String,
    pub region_clue: String,
    pub boundary_kind: String,
    pub boundary_value: String,
    pub support_kind: String,
    pub counter_kind: String,
    pub decision_status: String,
    pub decision_source: String,
    pub authorization_status: String,
    pub exposure_eligible: bool,
    pub member_count: i64,
    pub distinct_hosts: i64,
    pub samples: Vec<OwnershipGroupMemberView>,
    pub sample_rule: String,
    pub reason: String,
    pub priority: i64,
    pub sort_rank: i64,
    pub group_version: i64,
    pub applied_operations: i64,
    pub allowed_actions: Vec<String>,
    /// O3：快照之后被撤销授权的成员数（显示索引；下游阻断在撤销事务内已先完成）。
    pub authorization_revoked: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipGroupPage {
    pub project_id: i64,
    pub snapshot: OwnershipAnalysisRunView,
    pub stale_reasons: Vec<String>,
    pub total: i64,
    pub items: Vec<OwnershipReviewGroupView>,
    pub next_cursor: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipMemberPage {
    pub project_id: i64,
    pub run_id: i64,
    pub group_id: i64,
    pub group_version: i64,
    pub total: i64,
    pub items: Vec<OwnershipGroupMemberView>,
    pub next_cursor: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OwnershipGroupPageInput {
    pub project_id: i64,
    pub run_id: i64,
    pub lane: String,
    pub nav_bucket: String,
    pub region: String,
    pub page_size: i64,
    pub cursor: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OwnershipMemberPageInput {
    pub project_id: i64,
    pub run_id: i64,
    pub group_id: i64,
    pub page_size: i64,
    pub cursor: String,
}

fn ownership_allowed_actions(lane: &str) -> Vec<String> {
    match lane {
        "manual" => Vec::new(),
        "consistent" => OWNERSHIP_GROUP_ACTIONS
            .iter()
            .map(|action| action.to_string())
            .collect(),
        _ => OWNERSHIP_GROUP_ACTIONS
            .iter()
            .filter(|action| **action != "confirm_ownership")
            .map(|action| action.to_string())
            .collect(),
    }
}

/// 快照必须属于当前项目、已完成且成员索引仍在。
fn ownership_snapshot_for(
    connection: &rusqlite::Connection,
    project_id: i64,
    run_id: i64,
) -> Result<OwnershipAnalysisRunView, String> {
    let run = ownership_read_run(connection, run_id)?
        .filter(|run| run.project_id == project_id)
        .ok_or_else(|| format!("{OWNERSHIP_REVIEW_INVALID}: 快照不属于当前工作空间"))?;
    if run.status != "completed" || run.members_pruned {
        return Err(format!(
            "{OWNERSHIP_SNAPSHOT_STALE}: 快照 #{run_id} 不可用（{}），请读取最新快照",
            if run.members_pruned {
                "成员索引已清理"
            } else {
                run.status.as_str()
            }
        ));
    }
    Ok(run)
}

fn ownership_parse_cursor(cursor: &str, prefix: &str) -> Result<i64, String> {
    if cursor.is_empty() {
        return Ok(0);
    }
    cursor
        .strip_prefix(prefix)
        .and_then(|rest| rest.parse::<i64>().ok())
        .filter(|value| *value >= 0)
        .ok_or_else(|| format!("{OWNERSHIP_REVIEW_INVALID}: cursor 与当前快照/组不匹配"))
}

const OWNERSHIP_MEMBER_VIEW_SQL: &str = "SELECT m.asset_id,m.host,m.registrable_domain,m.suffix_section,m.company,COALESCE(a.link,''),COALESCE(a.title,''),pa.asset_id IS NOT NULL AND a.id IS NOT NULL AND COALESCE(pa.is_deleted,1)=0,CASE WHEN pa.asset_id IS NOT NULL AND a.id IS NOT NULL AND {SCOPE} THEN 1 ELSE 0 END,COALESCE(o.status,'unreviewed'),COALESCE(o.source,''),COALESCE(o.authorization_status,'unknown'),COALESCE(o.exposure_eligible,0),COALESCE(o.reason,''),{STATE}=m.decision_state,COALESCE(a.company,''),COALESCE(a.host,''),COALESCE(a.domain,''),COALESCE(a.ip,''),COALESCE(a.extra_json,'{}'),m.evidence_hash FROM ownership_review_group_members m LEFT JOIN project_assets pa ON pa.project_id=m.project_id AND pa.asset_id=m.asset_id LEFT JOIN assets a ON a.id=m.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=m.project_id AND o.asset_id=m.asset_id";

fn ownership_member_view_sql() -> String {
    OWNERSHIP_MEMBER_VIEW_SQL
        .replace("{SCOPE}", OWNERSHIP_SCOPE_SQL)
        .replace("{STATE}", OWNERSHIP_DECISION_STATE_SQL)
}

fn ownership_member_from_row(row: &Row<'_>) -> rusqlite::Result<OwnershipGroupMemberView> {
    let linked: bool = row.get::<_, i64>(7)? != 0;
    let in_scope: bool = row.get::<_, i64>(8)? != 0;
    let same_state: bool = row.get::<_, Option<i64>>(14)?.unwrap_or(0) != 0;
    let link: String = row.get(5)?;
    let title: String = row.get(6)?;
    let evidence = ownership_evidence_hash(
        &row.get::<_, String>(15)?,
        &row.get::<_, String>(16)?,
        &link,
        &row.get::<_, String>(17)?,
        &title,
        &row.get::<_, String>(18)?,
        &row.get::<_, String>(19)?,
    );
    let same_evidence = evidence == row.get::<_, String>(20)?;
    Ok(OwnershipGroupMemberView {
        asset_id: row.get(0)?,
        host: row.get(1)?,
        registrable_domain: row.get(2)?,
        suffix_section: row.get(3)?,
        company: row.get(4)?,
        link,
        title,
        current: OwnershipMemberCurrent {
            linked,
            in_scope,
            status: row.get(9)?,
            source: row.get(10)?,
            authorization_status: row.get(11)?,
            exposure_eligible: row.get::<_, i64>(12)? != 0,
            reason: row.get(13)?,
        },
        changed_since_snapshot: !(linked && in_scope && same_state && same_evidence),
    })
}

fn page_ownership_groups_for_connection(
    connection: &rusqlite::Connection,
    input: OwnershipGroupPageInput,
) -> Result<OwnershipGroupPage, String> {
    let run = ownership_snapshot_for(connection, input.project_id, input.run_id)?;
    let page_size = input.page_size.clamp(1, OWNERSHIP_GROUP_PAGE_MAX);
    let after = ownership_parse_cursor(&input.cursor, &format!("g{}:", run.id))?;
    let mut filter = String::from("run_id=?1");
    let mut values: Vec<SqlValue> = vec![SqlValue::Integer(run.id)];
    for (column, value) in [
        ("lane", &input.lane),
        ("nav_bucket", &input.nav_bucket),
        ("region_clue", &input.region),
    ] {
        if !value.is_empty() {
            values.push(SqlValue::Text(value.clone()));
            filter.push_str(&format!(" AND {column}=?{}", values.len()));
        }
    }
    let total: i64 = connection
        .query_row(
            &format!("SELECT COUNT(*) FROM ownership_review_groups WHERE {filter}"),
            params_from_iter(values.iter()),
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    values.push(SqlValue::Integer(after));
    let after_index = values.len();
    values.push(SqlValue::Integer(page_size + 1));
    let limit_index = values.len();
    let mut items = {
        let mut statement = connection
            .prepare(&format!("SELECT id,run_id,lane,nav_bucket,candidate_entity_id,candidate_label,candidate_source,relation_kind,region_clue,boundary_kind,boundary_value,support_kind,counter_kind,decision_status,decision_source,authorization_status,exposure_eligible,member_count,distinct_hosts,sample_asset_ids_json,sample_rule,reason,priority,sort_rank,group_version,applied_operations,authorization_revoked FROM ownership_review_groups WHERE {filter} AND sort_rank>?{after_index} ORDER BY sort_rank LIMIT ?{limit_index}"))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params_from_iter(values.iter()), |row| {
                let lane: String = row.get(2)?;
                let samples_json: String = row.get(19)?;
                Ok((
                    OwnershipReviewGroupView {
                        id: row.get(0)?,
                        run_id: row.get(1)?,
                        allowed_actions: ownership_allowed_actions(&lane),
                        lane,
                        nav_bucket: row.get(3)?,
                        candidate_entity_id: row.get(4)?,
                        candidate_label: row.get(5)?,
                        candidate_source: row.get(6)?,
                        relation_kind: row.get(7)?,
                        region_clue: row.get(8)?,
                        boundary_kind: row.get(9)?,
                        boundary_value: row.get(10)?,
                        support_kind: row.get(11)?,
                        counter_kind: row.get(12)?,
                        decision_status: row.get(13)?,
                        decision_source: row.get(14)?,
                        authorization_status: row.get(15)?,
                        exposure_eligible: row.get::<_, i64>(16)? != 0,
                        member_count: row.get(17)?,
                        distinct_hosts: row.get(18)?,
                        samples: Vec::new(),
                        sample_rule: row.get(20)?,
                        reason: row.get(21)?,
                        priority: row.get(22)?,
                        sort_rank: row.get(23)?,
                        group_version: row.get(24)?,
                        applied_operations: row.get(25)?,
                        authorization_revoked: row.get(26)?,
                    },
                    serde_json::from_str::<Vec<i64>>(&samples_json).unwrap_or_default(),
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    let next_cursor = if items.len() as i64 > page_size {
        items.truncate(page_size as usize);
        format!(
            "g{}:{}",
            run.id,
            items.last().map(|item| item.0.sort_rank).unwrap_or(after)
        )
    } else {
        String::new()
    };
    let mut sample_statement = connection
        .prepare(&format!(
            "{} WHERE m.run_id=?1 AND m.group_id=?2 AND m.asset_id=?3",
            ownership_member_view_sql()
        ))
        .map_err(|error| error.to_string())?;
    let mut out = Vec::with_capacity(items.len());
    for (mut group, sample_ids) in items {
        for asset_id in sample_ids {
            if let Some(member) = sample_statement
                .query_row(
                    params![run.id, group.id, asset_id],
                    ownership_member_from_row,
                )
                .optional()
                .map_err(|error| error.to_string())?
            {
                group.samples.push(member);
            }
        }
        out.push(group);
    }
    let current = ownership_current_versions(connection, input.project_id)?;
    Ok(OwnershipGroupPage {
        project_id: input.project_id,
        stale_reasons: ownership_stale_reasons(&run, &current),
        snapshot: run,
        total,
        items: out,
        next_cursor,
    })
}

#[tauri::command]
pub fn page_ownership_groups(
    state: State<AppState>,
    input: OwnershipGroupPageInput,
) -> Result<OwnershipGroupPage, String> {
    page_ownership_groups_for_connection(&db::open(&state.db_path)?, input)
}

fn page_ownership_group_members_for_connection(
    connection: &rusqlite::Connection,
    input: OwnershipMemberPageInput,
) -> Result<OwnershipMemberPage, String> {
    let run = ownership_snapshot_for(connection, input.project_id, input.run_id)?;
    let group_version: i64 = connection
        .query_row(
            "SELECT group_version FROM ownership_review_groups WHERE id=?1 AND run_id=?2",
            params![input.group_id, run.id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("{OWNERSHIP_REVIEW_INVALID}: 复核组不属于该快照"))?;
    let page_size = input.page_size.clamp(1, OWNERSHIP_MEMBER_PAGE_MAX);
    let after = ownership_parse_cursor(&input.cursor, &format!("m{}:{}:", run.id, input.group_id))?;
    let total: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM ownership_review_group_members WHERE run_id=?1 AND group_id=?2",
            params![run.id, input.group_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let mut items = {
        let mut statement = connection
            .prepare(&format!("{} WHERE m.run_id=?1 AND m.group_id=?2 AND m.asset_id>?3 ORDER BY m.asset_id LIMIT ?4", ownership_member_view_sql()))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(
                params![run.id, input.group_id, after, page_size + 1],
                ownership_member_from_row,
            )
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    let next_cursor = if items.len() as i64 > page_size {
        items.truncate(page_size as usize);
        format!(
            "m{}:{}:{}",
            run.id,
            input.group_id,
            items.last().map(|item| item.asset_id).unwrap_or(after)
        )
    } else {
        String::new()
    };
    Ok(OwnershipMemberPage {
        project_id: input.project_id,
        run_id: run.id,
        group_id: input.group_id,
        group_version,
        total,
        items,
        next_cursor,
    })
}

#[tauri::command]
pub fn page_ownership_group_members(
    state: State<AppState>,
    input: OwnershipMemberPageInput,
) -> Result<OwnershipMemberPage, String> {
    page_ownership_group_members_for_connection(&db::open(&state.db_path)?, input)
}

// ---------------------------------------------------------------------------------------------
// 组批量预览 / 确认（§8）：提交 groupId；冻结快照成员；CAS；operationId 幂等；逐条审计。
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OwnershipGroupDecisionRequest {
    pub project_id: i64,
    pub run_id: i64,
    pub group_id: i64,
    pub action: String,
    pub note: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OwnershipGroupDecisionApply {
    pub operation_id: String,
    pub project_id: i64,
    pub run_id: i64,
    pub group_id: i64,
    pub group_version: i64,
    pub policy_version: i64,
    pub directory_version: i64,
    pub binding_version: i64,
    pub action: String,
    pub note: String,
    pub expected_action_hash: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipExcludedMember {
    pub asset_id: i64,
    pub outcome: String,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipProposedChange {
    pub status: String,
    pub source: String,
    /// unchanged：保留每条现有授权状态；denied：沿用既有“第三方/排除”收窄语义。
    pub authorization: String,
    /// unchanged：不改变暴露面准入；revoked：置为不准入（收窄）。
    pub exposure_eligible: String,
    pub grants_authorization: bool,
    pub sets_exposure_eligible: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipGroupDecisionPreview {
    pub project_id: i64,
    pub run_id: i64,
    pub group_id: i64,
    pub group_version: i64,
    pub policy_version: i64,
    pub directory_version: i64,
    pub binding_version: i64,
    pub action: String,
    pub note: String,
    pub expected_action_hash: String,
    pub member_count: i64,
    pub applicable: i64,
    pub skipped: i64,
    pub conflicted: i64,
    pub reasons: std::collections::BTreeMap<String, i64>,
    pub excluded_samples: Vec<OwnershipExcludedMember>,
    pub current_authorization: std::collections::BTreeMap<String, i64>,
    pub proposed: OwnershipProposedChange,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipGroupDecisionResult {
    pub operation_id: String,
    pub project_id: i64,
    pub run_id: i64,
    pub group_id: i64,
    pub action: String,
    pub applied: i64,
    pub skipped: i64,
    pub conflicted: i64,
    pub failed: i64,
    pub reasons: std::collections::BTreeMap<String, i64>,
    pub group_version: i64,
    pub replayed: bool,
    /// O3：applying（分块写入中，可续跑）/ completed。applying 时 applied 等计数是已提交分块的真实进度。
    #[serde(default = "ownership_completed_status")]
    pub status: String,
    #[serde(default)]
    pub total_items: i64,
    #[serde(default)]
    pub done_items: i64,
}

fn ownership_completed_status() -> String {
    "completed".into()
}

struct OwnershipMemberCheck {
    asset_id: i64,
    outcome: &'static str,
    reason: &'static str,
    current_state: String,
    evidence_hash: String,
    old: JsonValue,
    authorization: String,
    eligible: i64,
    evidence_json: String,
}

struct OwnershipGroupEvaluation {
    run: OwnershipAnalysisRunView,
    group_version: i64,
    lane: String,
    member_count: i64,
    checks: Vec<OwnershipMemberCheck>,
    hash: String,
}

fn ownership_normalize_note(action: &str, note: &str) -> String {
    let note: String = note.trim().chars().take(500).collect();
    if !note.is_empty() {
        return note;
    }
    match action {
        "confirm_ownership" => "组复核：确认归属（不含授权）",
        "related" => "组复核：仅集团/业务关联",
        "third_party" => "组复核：第三方服务",
        _ => "组复核：不属于当前主体",
    }
    .to_string()
}

/// 在同一连接/事务上重新计算组成员的可写入性与操作哈希。预览与确认共用，保证两者口径一致。
fn ownership_evaluate_group(
    connection: &rusqlite::Connection,
    request: &OwnershipGroupDecisionRequest,
) -> Result<OwnershipGroupEvaluation, String> {
    if !OWNERSHIP_GROUP_ACTIONS.contains(&request.action.as_str()) {
        return Err(format!(
            "{OWNERSHIP_REVIEW_INVALID}: 不支持的组动作 {}",
            request.action
        ));
    }
    let run = ownership_snapshot_for(connection, request.project_id, request.run_id)?;
    let latest = ownership_latest_snapshot(connection, request.project_id)?;
    if latest.as_ref().map(|item| item.id) != Some(run.id) {
        return Err(format!(
            "{OWNERSHIP_SNAPSHOT_STALE}: 已有更新的分析快照，请在最新快照上重新预览"
        ));
    }
    let current = ownership_current_versions(connection, request.project_id)?;
    let stale = ownership_stale_reasons(&run, &current);
    if !stale.is_empty() {
        return Err(format!("{OWNERSHIP_SNAPSHOT_STALE}: 快照之后策略/目录/绑定/算法已变化（{}），需要重新分析后再预览", stale.join(", ")));
    }
    let (group_version, lane, member_count): (i64, String, i64) = connection
        .query_row(
            "SELECT group_version,lane,member_count FROM ownership_review_groups WHERE id=?1 AND run_id=?2 AND project_id=?3",
            params![request.group_id, run.id, request.project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("{OWNERSHIP_REVIEW_INVALID}: 复核组不属于该快照或当前工作空间"))?;
    if !ownership_allowed_actions(&lane).contains(&request.action) {
        return Err(format!("{OWNERSHIP_GROUP_ACTION_DENIED}: {} 组不允许批量执行 {}（冲突/反证/证据不足/未知组不能整组确认归属；已有人工结论的组只能逐条修改）", lane, request.action));
    }
    let mut statement = connection
        .prepare(&format!("SELECT m.asset_id,m.decision_state,m.evidence_hash,pa.asset_id IS NOT NULL AND a.id IS NOT NULL AND COALESCE(pa.is_deleted,1)=0,CASE WHEN pa.asset_id IS NOT NULL AND a.id IS NOT NULL AND {OWNERSHIP_SCOPE_SQL} THEN 1 ELSE 0 END,{OWNERSHIP_DECISION_STATE_SQL},COALESCE(a.company,''),COALESCE(a.host,''),COALESCE(a.link,''),COALESCE(a.domain,''),COALESCE(a.title,''),COALESCE(a.ip,''),COALESCE(a.extra_json,'{{}}'),COALESCE(o.status,'unreviewed'),COALESCE(o.source,''),COALESCE(o.confidence,0),COALESCE(o.authorization_status,'unknown'),COALESCE(o.exposure_eligible,0),COALESCE(o.reason,''),COALESCE(o.evidence_json,'[]'),o.asset_id IS NOT NULL FROM ownership_review_group_members m LEFT JOIN project_assets pa ON pa.project_id=m.project_id AND pa.asset_id=m.asset_id LEFT JOIN assets a ON a.id=m.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=m.project_id AND o.asset_id=m.asset_id WHERE m.run_id=?1 AND m.group_id=?2 AND m.project_id=?3 ORDER BY m.asset_id"))
        .map_err(|error| error.to_string())?;
    let mut rows = statement
        .query(params![run.id, request.group_id, request.project_id])
        .map_err(|error| error.to_string())?;
    let mut checks = Vec::new();
    let mut digest = Sha256::new();
    digest.update(
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}\n",
            request.project_id,
            run.id,
            request.group_id,
            group_version,
            request.action,
            ownership_normalize_note(&request.action, &request.note),
            run.versions.policy_version,
            run.versions.directory_version,
            run.versions.binding_version
        )
        .as_bytes(),
    );
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let get_text = |index: usize| -> Result<String, String> {
            row.get::<_, String>(index)
                .map_err(|error| error.to_string())
        };
        let asset_id: i64 = row.get(0).map_err(|error| error.to_string())?;
        let snapshot_state = get_text(1)?;
        let snapshot_evidence = get_text(2)?;
        let active: bool = row.get::<_, i64>(3).map_err(|error| error.to_string())? != 0;
        let in_scope: bool = row.get::<_, i64>(4).map_err(|error| error.to_string())? != 0;
        let current_state = get_text(5)?;
        let evidence_hash = ownership_evidence_hash(
            &get_text(6)?,
            &get_text(7)?,
            &get_text(8)?,
            &get_text(9)?,
            &get_text(10)?,
            &get_text(11)?,
            &get_text(12)?,
        );
        let status = get_text(13)?;
        let source = get_text(14)?;
        let confidence: i64 = row.get(15).map_err(|error| error.to_string())?;
        let authorization = get_text(16)?;
        let eligible: i64 = row.get(17).map_err(|error| error.to_string())?;
        let reason_text = get_text(18)?;
        let evidence_json = get_text(19)?;
        let has_row: bool = row.get::<_, i64>(20).map_err(|error| error.to_string())? != 0;
        let (outcome, reason) = if !active {
            ("conflicted", "deleted_or_unlinked")
        } else if !in_scope {
            ("conflicted", "left_review_scope")
        } else if current_state != snapshot_state {
            ("conflicted", "decision_changed")
        } else if evidence_hash != snapshot_evidence {
            ("conflicted", "evidence_changed")
        } else if source == "manual" {
            ("skipped", "manual_decision_locked")
        } else {
            ("applicable", "")
        };
        digest.update(
            format!("{asset_id}|{outcome}|{reason}|{current_state}|{evidence_hash}\n").as_bytes(),
        );
        checks.push(OwnershipMemberCheck {
            asset_id,
            outcome,
            reason,
            current_state,
            evidence_hash,
            old: serde_json::json!({"exists": has_row, "status": status, "source": source, "confidence": confidence, "authorizationStatus": authorization, "exposureEligible": eligible, "reason": reason_text}),
            authorization,
            eligible,
            evidence_json,
        });
    }
    if checks.len() as i64 != member_count {
        return Err(format!(
            "{OWNERSHIP_SNAPSHOT_STALE}: 组成员索引与计数不一致（{} / {member_count}），请重新分析",
            checks.len()
        ));
    }
    let hash: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(OwnershipGroupEvaluation {
        run,
        group_version,
        lane,
        member_count,
        checks,
        hash,
    })
}

fn ownership_proposed_change(action: &str) -> OwnershipProposedChange {
    let narrowing = matches!(action, "third_party" | "excluded");
    OwnershipProposedChange {
        status: match action {
            "confirm_ownership" => "confirmed",
            other => other,
        }
        .to_string(),
        source: "manual".into(),
        authorization: if narrowing { "denied" } else { "unchanged" }.into(),
        exposure_eligible: if narrowing { "revoked" } else { "unchanged" }.into(),
        grants_authorization: false,
        sets_exposure_eligible: false,
    }
}

fn preview_ownership_group_decision_for_connection(
    connection: &rusqlite::Connection,
    request: OwnershipGroupDecisionRequest,
) -> Result<OwnershipGroupDecisionPreview, String> {
    let evaluation = ownership_evaluate_group(connection, &request)?;
    let mut reasons = std::collections::BTreeMap::new();
    let mut authorization = std::collections::BTreeMap::new();
    let (mut applicable, mut skipped, mut conflicted) = (0i64, 0i64, 0i64);
    let mut excluded_samples = Vec::new();
    for check in &evaluation.checks {
        match check.outcome {
            "applicable" => {
                applicable += 1;
                *authorization
                    .entry(format!(
                        "{}/{}",
                        check.authorization,
                        if check.eligible != 0 {
                            "eligible"
                        } else {
                            "not_eligible"
                        }
                    ))
                    .or_insert(0) += 1;
            }
            outcome => {
                if outcome == "skipped" {
                    skipped += 1;
                } else {
                    conflicted += 1;
                }
                *reasons.entry(check.reason.to_string()).or_insert(0) += 1;
                if excluded_samples.len() < OWNERSHIP_PREVIEW_EXCLUDED_SAMPLE {
                    excluded_samples.push(OwnershipExcludedMember {
                        asset_id: check.asset_id,
                        outcome: outcome.into(),
                        reason: check.reason.into(),
                    });
                }
            }
        }
    }
    let proposed = ownership_proposed_change(&request.action);
    let mut notes = vec![
        "只写入归属结论（人工锁定）；不创建授权、不设置暴露面准入，主动扫描等动作仍按既有授权合同单独批准".to_string(),
        "只包含快照内成员；快照之后新增的资产不会被纳入".to_string(),
    ];
    if proposed.authorization == "denied" {
        notes.push("第三方/排除沿用既有收窄语义：授权=拒绝、暴露面准入=否".into());
    }
    if evaluation.lane != "consistent" {
        notes.push(format!("该组为 {}，只允许收窄类动作", evaluation.lane));
    }
    Ok(OwnershipGroupDecisionPreview {
        project_id: request.project_id,
        run_id: evaluation.run.id,
        group_id: request.group_id,
        group_version: evaluation.group_version,
        policy_version: evaluation.run.versions.policy_version,
        directory_version: evaluation.run.versions.directory_version,
        binding_version: evaluation.run.versions.binding_version,
        note: ownership_normalize_note(&request.action, &request.note),
        action: request.action,
        expected_action_hash: evaluation.hash,
        member_count: evaluation.member_count,
        applicable,
        skipped,
        conflicted,
        reasons,
        excluded_samples,
        current_authorization: authorization,
        proposed,
        notes,
    })
}

#[tauri::command]
pub fn preview_ownership_group_decision(
    state: State<AppState>,
    input: OwnershipGroupDecisionRequest,
) -> Result<OwnershipGroupDecisionPreview, String> {
    preview_ownership_group_decision_for_connection(&db::open(&state.db_path)?, input)
}

fn ownership_operation_payload_hash(input: &OwnershipGroupDecisionApply) -> String {
    let canonical = serde_json::json!([
        input.project_id,
        input.run_id,
        input.group_id,
        input.group_version,
        input.policy_version,
        input.directory_version,
        input.binding_version,
        input.action,
        ownership_normalize_note(&input.action, &input.note),
        input.expected_action_hash,
    ]);
    ownership_sha(&canonical.to_string())
}

// O3：组批量确认改为分块暂存的操作状态机，见 ownership_operations.rs。

#[cfg(test)]
mod ownership_review_o2_tests {
    use super::*;

    const ZJ: &str = "中国移动通信集团浙江有限公司";
    const JS: &str = "中国移动通信集团江苏有限公司";

    pub(super) fn temp_db(tag: &str) -> (PathBuf, rusqlite::Connection) {
        let root =
            std::env::temp_dir().join(format!("oviraptor-ownership-o2-{tag}-{}", Uuid::new_v4()));
        let path = db::initialize(&root).unwrap();
        (root, db::open(&path).unwrap())
    }

    pub(super) fn count(connection: &rusqlite::Connection, sql: &str) -> i64 {
        connection.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    /// 目录直接写表（版本 1），项目 91 绑定为集团总账；联通集团是范围外实体。
    fn seed_directory(connection: &rusqlite::Connection) {
        let entities: [(i64, Option<i64>, &str, &str, &str); 6] = [
            (1, None, "group", "中国移动通信集团有限公司", ""),
            (2, Some(1), "province_company", ZJ, "浙江"),
            (3, Some(1), "province_company", JS, "江苏"),
            (
                4,
                Some(1),
                "professional_subsidiary",
                "中移（苏州）软件技术有限公司",
                "",
            ),
            (5, Some(1), "supplier", "杭州某供应商科技有限公司", ""),
            (6, None, "group", "中国联合网络通信集团有限公司", ""),
        ];
        connection
            .execute(
                "INSERT INTO org_directory_state(id,version) VALUES(1,1)",
                [],
            )
            .unwrap();
        for (id, group, kind, name, region) in entities {
            connection
                .execute(
                    "INSERT INTO org_entities(id,group_id,parent_id,entity_kind,legal_name,normalized_name,region_code,directory_version) VALUES(?1,?2,?2,?3,?4,?5,?6,1)",
                    params![id, group, kind, name, normalize_org_name(name), region],
                )
                .unwrap();
        }
        for (entity, kind, value) in [
            (2, "official_domain", "zj.10086.cn"),
            (3, "official_domain", "js.10086.cn"),
            (1, "shared_domain", "www.10086.cn"),
            (1, "short_name", "中国移动通信集团"),
            (2, "brand", "浙江移动"),
        ] {
            connection
                .execute(
                    "INSERT INTO org_entity_identifiers(entity_id,kind,value,normalized_value,directory_version) VALUES(?1,?2,?3,?4,1)",
                    params![entity, kind, value, if kind.ends_with("domain") { normalize_ownership_domain(value) } else { normalize_org_name(value) }],
                )
                .unwrap();
        }
        connection
            .execute("INSERT INTO project_org_bindings(project_id,project_kind,group_entity_id,binding_version) VALUES(91,'group_ledger',1,1)", [])
            .unwrap();
    }

    pub(super) fn fixture() -> (PathBuf, rusqlite::Connection) {
        let (root, connection) = temp_db("fixture");
        connection
            .execute_batch(
                "INSERT INTO projects(id,name) VALUES(91,'中国移动历史资产'),(92,'香港移动');",
            )
            .unwrap();
        seed_directory(&connection);
        let rows: [(i64, &str, &str, &str); 19] = [
            (1, ZJ, "zj.10086.cn", ""),
            (2, ZJ, "zj.10086.cn", ""),
            (3, JS, "js.10086.cn", ""),
            (4, ZJ, "a.zjgov.com.cn", ""),
            (5, ZJ, "b.other.com.cn", ""),
            (6, "中移（苏州）软件技术有限公司", "sz.example.cn", ""),
            (7, ZJ, "zj-portal.github.io", ""),
            (8, ZJ, "www.10086.cn", ""),
            (9, "161.81.120.0/22", "", "161.81.120.7"),
            (10, "", "random.example.org", ""),
            (11, "浙江移动", "js.10086.cn", ""),
            (12, "杭州某供应商科技有限公司", "vendor.example.com", ""),
            (13, "中国联合网络通信集团有限公司", "x.chinaunicom.cn", ""),
            (14, ZJ, "zj.10086.cn", ""),
            (15, ZJ, "zj.10086.cn", ""),
            (16, "某未知网络有限公司", "unknown-co.cn", ""),
            (17, ZJ, "zj.10086.cn", ""),
            (18, ZJ, "zj.10086.cn", ""),
            (19, ZJ, "zj.10086.cn", ""),
        ];
        for (id, company, domain, ip) in rows {
            connection
                .execute(
                    "INSERT INTO assets(id,asset_key,company,host,domain,link,ip,title) VALUES(?1,?2,?3,?4,?4,CASE WHEN ?4='' THEN '' ELSE 'https://'||?4||'/p'||?1 END,?5,'t')",
                    params![id, format!("o2-{id}"), company, domain, ip],
                )
                .unwrap();
            let project = if id == 18 { 92 } else { 91 };
            connection
                .execute(
                    "INSERT INTO project_assets(project_id,asset_id,is_deleted) VALUES(?1,?2,?3)",
                    params![project, id, i64::from(id == 17)],
                )
                .unwrap();
        }
        connection
            .execute_batch(
                "UPDATE assets SET probe_outcome='dead' WHERE id=19;
                 INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,authorization_status,exposure_eligible,source,reason) VALUES
                   (91,14,'confirmed',100,'allowed',1,'manual','人工确认'),
                   (91,15,'attributed',92,'allowed',1,'auto','历史遗留准入');",
            )
            .unwrap();
        (root, connection)
    }

    pub(super) fn run_analysis(connection: &mut rusqlite::Connection, project_id: i64, chunk: i64) -> i64 {
        let (run, needs_worker) =
            start_ownership_analysis_for_connection(connection, project_id).unwrap();
        assert!(needs_worker);
        let token = ownership_claim_run(connection, run.id)
            .unwrap()
            .expect("claim");
        assert_eq!(
            ownership_drive_run(connection, run.id, &token, chunk, None).unwrap(),
            OwnershipStep::Completed
        );
        let finished = ownership_read_run(connection, run.id).unwrap().unwrap();
        assert_eq!(finished.status, "completed", "{}", finished.error);
        run.id
    }

    #[derive(Debug, Clone)]
    pub(super) struct GroupRow {
        pub(super) id: i64,
        lane: String,
        nav: String,
        entity: Option<i64>,
        boundary_kind: String,
        boundary_value: String,
        counter: String,
        support: String,
    }

    pub(super) fn group_of(connection: &rusqlite::Connection, run_id: i64, asset_id: i64) -> GroupRow {
        connection
            .query_row(
                "SELECT g.id,g.lane,g.nav_bucket,g.candidate_entity_id,g.boundary_kind,g.boundary_value,g.counter_kind,g.support_kind FROM ownership_review_group_members m JOIN ownership_review_groups g ON g.id=m.group_id WHERE m.run_id=?1 AND m.asset_id=?2",
                params![run_id, asset_id],
                |row| Ok(GroupRow { id: row.get(0)?, lane: row.get(1)?, nav: row.get(2)?, entity: row.get(3)?, boundary_kind: row.get(4)?, boundary_value: row.get(5)?, counter: row.get(6)?, support: row.get(7)? }),
            )
            .unwrap_or_else(|error| panic!("asset {asset_id} has no group: {error}"))
    }

    pub(super) fn request(run_id: i64, group_id: i64, action: &str) -> OwnershipGroupDecisionRequest {
        OwnershipGroupDecisionRequest {
            project_id: 91,
            run_id,
            group_id,
            action: action.into(),
            note: String::new(),
        }
    }

    pub(super) fn apply_input(
        preview: &OwnershipGroupDecisionPreview,
        operation_id: &str,
    ) -> OwnershipGroupDecisionApply {
        OwnershipGroupDecisionApply {
            operation_id: operation_id.into(),
            project_id: preview.project_id,
            run_id: preview.run_id,
            group_id: preview.group_id,
            group_version: preview.group_version,
            policy_version: preview.policy_version,
            directory_version: preview.directory_version,
            binding_version: preview.binding_version,
            action: preview.action.clone(),
            note: preview.note.clone(),
            expected_action_hash: preview.expected_action_hash.clone(),
        }
    }

    pub(super) fn decision(
        connection: &rusqlite::Connection,
        asset_id: i64,
    ) -> Option<(String, String, String, i64)> {
        connection
            .query_row(
                "SELECT status,source,authorization_status,exposure_eligible FROM asset_ownership_decisions WHERE project_id=91 AND asset_id=?1",
                [asset_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .unwrap()
    }

    #[test]
    fn analysis_run_binds_versions_and_never_starts_a_duplicate() {
        let (root, mut connection) = fixture();
        let (run, needs_worker) =
            start_ownership_analysis_for_connection(&mut connection, 91).unwrap();
        assert!(needs_worker);
        assert_eq!(run.status, "queued");
        assert_eq!(
            (
                run.versions.policy_version,
                run.versions.directory_version,
                run.versions.binding_version
            ),
            (0, 1, 1)
        );
        assert!(
            run.versions
                .algorithm_version
                .contains("o2-review-groups-v1")
                && run.versions.algorithm_version.contains("psl-")
        );
        assert!(
            run.versions.scope_version.starts_with("scope-")
                && run.data_revision.starts_with("rows=16;"),
            "{}",
            run.data_revision
        );
        // 同项目已有活跃任务：复用，不再启动第二个工作线程。
        let (again, again_needs_worker) =
            start_ownership_analysis_for_connection(&mut connection, 91).unwrap();
        assert_eq!((again.id, again_needs_worker), (run.id, false));
        assert!(connection.execute("INSERT INTO ownership_analysis_runs(project_id,status,scope_version,data_revision,policy_version,directory_version,binding_version,algorithm_version) VALUES(91,'running','s','d',0,1,1,'a')", []).is_err(), "unique active index");
        let token = ownership_claim_run(&mut connection, run.id)
            .unwrap()
            .unwrap();
        assert!(
            ownership_claim_run(&mut connection, run.id)
                .unwrap()
                .is_none(),
            "a fresh running run cannot be claimed twice"
        );
        assert_eq!(
            ownership_drive_run(&mut connection, run.id, &token, 4, None).unwrap(),
            OwnershipStep::Completed
        );
        let done = ownership_read_run(&connection, run.id).unwrap().unwrap();
        assert_eq!(
            (
                done.status.as_str(),
                done.total_assets,
                done.processed_assets,
                done.member_count
            ),
            ("completed", 16, 16, 16)
        );
        assert!(!done.finished_at.is_empty() && !done.data_revision_at_finish.is_empty());
        let overview = read_ownership_analysis_for_connection(&connection, 91).unwrap();
        assert_eq!(overview.snapshot.as_ref().map(|item| item.id), Some(run.id));
        assert!(
            overview.readiness.contains(&"rules_only".to_string())
                && overview.readiness.contains(&"model_not_ready".to_string())
        );
        assert!(
            !overview.readiness.contains(&"stale_snapshot".to_string()),
            "{:?}",
            overview.stale_reasons
        );
        // 完成后可以再启动新的分析（新快照）。
        let (next, next_needs_worker) =
            start_ownership_analysis_for_connection(&mut connection, 91).unwrap();
        assert!(next.id > run.id && next_needs_worker);
        assert!(start_ownership_analysis_for_connection(&mut connection, 404).is_err());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn analysis_cancel_discards_staging_and_keeps_the_real_processed_count() {
        let (root, mut connection) = fixture();
        let (queued, _) = start_ownership_analysis_for_connection(&mut connection, 91).unwrap();
        let cancelled =
            cancel_ownership_analysis_for_connection(&mut connection, 91, queued.id).unwrap();
        assert_eq!(cancelled.status, "cancelled");
        assert!(
            ownership_claim_run(&mut connection, queued.id)
                .unwrap()
                .is_none(),
            "cancelled run is never claimed"
        );
        assert!(
            cancel_ownership_analysis_for_connection(&mut connection, 92, queued.id).is_err(),
            "other project cannot cancel"
        );

        let (run, _) = start_ownership_analysis_for_connection(&mut connection, 91).unwrap();
        let token = ownership_claim_run(&mut connection, run.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            ownership_drive_run(&mut connection, run.id, &token, 5, Some(1)).unwrap(),
            OwnershipStep::Continued
        );
        assert_eq!(
            count(
                &connection,
                &format!(
                    "SELECT COUNT(*) FROM ownership_review_group_members WHERE run_id={}",
                    run.id
                )
            ),
            5
        );
        let requested =
            cancel_ownership_analysis_for_connection(&mut connection, 91, run.id).unwrap();
        assert_eq!(
            (requested.status.as_str(), requested.cancel_requested),
            ("running", true)
        );
        // 有界步数：取消检查失效时测试变红而不是挂起。
        assert_eq!(
            ownership_drive_run(&mut connection, run.id, &token, 5, Some(10)).unwrap(),
            OwnershipStep::Cancelled
        );
        let after = ownership_read_run(&connection, run.id).unwrap().unwrap();
        assert_eq!(
            (
                after.status.as_str(),
                after.processed_assets,
                after.group_count
            ),
            ("cancelled", 5, 0)
        );
        assert_eq!(
            count(
                &connection,
                &format!(
                    "SELECT COUNT(*) FROM ownership_review_group_members WHERE run_id={}",
                    run.id
                )
            ),
            0
        );
        assert_eq!(
            count(
                &connection,
                &format!(
                    "SELECT COUNT(*) FROM ownership_review_groups WHERE run_id={}",
                    run.id
                )
            ),
            0
        );
        let overview = read_ownership_analysis_for_connection(&connection, 91).unwrap();
        assert!(
            overview.snapshot.is_none()
                && overview.queue.is_none()
                && overview.readiness.contains(&"no_snapshot".to_string()),
            "no snapshot must not read as an empty queue"
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn analysis_resumes_after_a_lost_worker_without_duplicating_members() {
        let (root, mut connection) = fixture();
        let (run, _) = start_ownership_analysis_for_connection(&mut connection, 91).unwrap();
        let first = ownership_claim_run(&mut connection, run.id)
            .unwrap()
            .unwrap();
        ownership_drive_run(&mut connection, run.id, &first, 6, Some(1)).unwrap();
        connection
            .execute(
                "UPDATE ownership_analysis_runs SET heartbeat_epoch=0 WHERE id=?1",
                [run.id],
            )
            .unwrap();
        let (same, needs_worker) =
            start_ownership_analysis_for_connection(&mut connection, 91).unwrap();
        assert_eq!(
            (same.id, needs_worker),
            (run.id, true),
            "stale worker must be resumable"
        );
        let second = ownership_claim_run(&mut connection, run.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            ownership_drive_run(&mut connection, run.id, &first, 6, None).unwrap(),
            OwnershipStep::Lost,
            "old worker loses ownership"
        );
        assert_eq!(
            ownership_drive_run(&mut connection, run.id, &second, 6, None).unwrap(),
            OwnershipStep::Completed
        );
        let done = ownership_read_run(&connection, run.id).unwrap().unwrap();
        assert_eq!(
            (
                done.status.as_str(),
                done.processed_assets,
                done.member_count
            ),
            ("completed", 16, 16)
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn grouping_splits_by_entity_boundary_evidence_decision_and_authorization() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 3);
        let g = |asset| group_of(&connection, run, asset);
        // 同实体同可注册域、同证据：同组；不同省单位即使同根域也拆开。
        assert_eq!(g(1).id, g(2).id);
        assert_eq!(
            (
                g(1).lane.as_str(),
                g(1).support.as_str(),
                g(1).entity,
                g(1).nav.as_str()
            ),
            ("consistent", "directory_official_host", Some(2), "province")
        );
        assert_eq!(
            (g(1).boundary_kind.as_str(), g(1).boundary_value.as_str()),
            ("registrable_domain", "10086.cn")
        );
        assert_ne!(g(1).id, g(3).id);
        assert_eq!(g(3).entity, Some(3));
        // com.cn 不按最后两段合并。
        assert_eq!(g(4).boundary_value, "zjgov.com.cn");
        assert_eq!(g(5).boundary_value, "other.com.cn");
        assert_ne!(g(4).id, g(5).id);
        // 专业子公司按名称匹配到自身，导航为专业子公司，不归省公司。
        assert_eq!((g(6).entity, g(6).nav.as_str()), (Some(4), "subsidiary"));
        // 托管平台租户、共享域名：反证 + 支持 → 冲突组，边界不泛化到平台根域。
        assert_eq!(
            (
                g(7).lane.as_str(),
                g(7).boundary_kind.as_str(),
                g(7).boundary_value.as_str()
            ),
            ("conflict", "private_suffix_tenant", "zj-portal.github.io")
        );
        assert!(g(7).counter.contains("hosting_platform"));
        assert_eq!(
            (
                g(8).lane.as_str(),
                g(8).boundary_kind.as_str(),
                g(8).boundary_value.as_str()
            ),
            ("conflict", "exact_host", "10086.cn")
        );
        // CIDR 公司字段：网络线索，反证组，不当法人。
        assert_eq!(
            (g(9).lane.as_str(), g(9).counter.as_str(), g(9).entity),
            ("counter_evidence", "network_label", None)
        );
        assert_eq!(
            (g(10).lane.as_str(), g(10).nav.as_str()),
            ("unknown", "unknown")
        );
        // 官方主机属于江苏，名称指向浙江：实体不一致 → 冲突。
        assert!(
            g(11).counter.contains("entity_mismatch")
                && g(11).lane == "conflict"
                && g(11).nav == "conflict"
        );
        assert!(g(12).counter.contains("supplier_entity") && g(12).lane == "conflict");
        assert!(g(13).counter.contains("entity_outside_scope") && g(13).lane == "conflict");
        // 人工结论与授权状态不同：各自成组。
        assert_eq!(g(14).lane, "manual");
        assert_ne!(g(14).id, g(1).id);
        assert_eq!(g(15).lane, "consistent");
        assert_ne!(
            g(15).id,
            g(1).id,
            "different authorization state must split the group"
        );
        assert_eq!(
            (g(16).lane.as_str(), g(16).support.as_str()),
            ("insufficient", "company_label")
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn each_asset_has_exactly_one_primary_group_and_counts_reconcile() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 2);
        let members = count(
            &connection,
            &format!("SELECT COUNT(*) FROM ownership_review_group_members WHERE run_id={run}"),
        );
        let distinct = count(&connection, &format!("SELECT COUNT(DISTINCT asset_id) FROM ownership_review_group_members WHERE run_id={run}"));
        let sum = count(
            &connection,
            &format!("SELECT SUM(member_count) FROM ownership_review_groups WHERE run_id={run}"),
        );
        let scope = count(&connection, &format!("SELECT COUNT(*) FROM project_assets pa JOIN assets a ON a.id=pa.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id=91 AND {OWNERSHIP_SCOPE_SQL}"));
        assert_eq!((members, distinct, sum, scope), (16, 16, 16, 16));
        for (asset, member) in [(17, false), (18, false), (19, false), (1, true)] {
            assert_eq!(count(&connection, &format!("SELECT COUNT(*) FROM ownership_review_group_members WHERE run_id={run} AND asset_id={asset}")) == 1, member, "asset {asset}");
        }
        let any_group = group_of(&connection, run, 1).id;
        let other_group = group_of(&connection, run, 3).id;
        assert!(connection.execute("INSERT INTO ownership_review_group_members(run_id,asset_id,project_id,group_id,decision_state,evidence_hash) VALUES(?1,1,91,?2,'x','y')", params![run, other_group]).is_err(), "an asset cannot join a second primary group");
        let overview = read_ownership_analysis_for_connection(&connection, 91).unwrap();
        let queue = overview.queue.unwrap();
        assert_eq!((queue.member_total, queue.group_member_sum), (16, 16));
        assert_eq!(queue.manual_assets, 1);
        assert_eq!(queue.pending_assets + queue.manual_assets, 16);
        assert!(queue.conflict_groups >= 5 && queue.counter_evidence_groups >= 1);
        assert_ne!(any_group, 0);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn group_and_member_pages_use_cursors_with_real_totals() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 7);
        let snapshot = ownership_read_run(&connection, run).unwrap().unwrap();
        let mut cursor = String::new();
        let mut seen = Vec::new();
        let mut members = 0;
        loop {
            let page = page_ownership_groups_for_connection(
                &connection,
                OwnershipGroupPageInput {
                    project_id: 91,
                    run_id: run,
                    page_size: 2,
                    cursor: cursor.clone(),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(page.total, snapshot.group_count);
            assert_eq!(page.snapshot.id, run);
            assert!(page.items.len() <= 2);
            for item in &page.items {
                assert!(
                    item.samples.len() <= 5
                        && !item.samples.is_empty()
                        && item.samples.len() as i64 <= item.member_count
                );
                seen.push(item.id);
                members += item.member_count;
            }
            if page.next_cursor.is_empty() {
                break;
            }
            cursor = page.next_cursor;
        }
        let unique: HashSet<i64> = seen.iter().copied().collect();
        assert_eq!(
            (seen.len() as i64, unique.len() as i64, members),
            (snapshot.group_count, snapshot.group_count, 16)
        );
        // 冲突优先排序。
        let first = page_ownership_groups_for_connection(
            &connection,
            OwnershipGroupPageInput {
                project_id: 91,
                run_id: run,
                page_size: 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(first.items[0].lane, "conflict");
        let conflicts = page_ownership_groups_for_connection(
            &connection,
            OwnershipGroupPageInput {
                project_id: 91,
                run_id: run,
                lane: "conflict".into(),
                page_size: 200,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(conflicts.total, count(&connection, &format!("SELECT COUNT(*) FROM ownership_review_groups WHERE run_id={run} AND lane='conflict'")));
        assert!(conflicts.items.iter().all(|item| item.lane == "conflict"
            && !item
                .allowed_actions
                .contains(&"confirm_ownership".to_string())));
        let province = page_ownership_groups_for_connection(
            &connection,
            OwnershipGroupPageInput {
                project_id: 91,
                run_id: run,
                nav_bucket: "province".into(),
                region: "浙江".into(),
                page_size: 200,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            province.total >= 2 && province.items.iter().all(|item| item.region_clue == "浙江")
        );
        // 成员分页。
        let group = group_of(&connection, run, 1).id;
        let one = page_ownership_group_members_for_connection(
            &connection,
            OwnershipMemberPageInput {
                project_id: 91,
                run_id: run,
                group_id: group,
                page_size: 1,
                cursor: String::new(),
            },
        )
        .unwrap();
        assert_eq!(
            (one.total, one.items.len(), one.items[0].asset_id),
            (2, 1, 1)
        );
        let two = page_ownership_group_members_for_connection(
            &connection,
            OwnershipMemberPageInput {
                project_id: 91,
                run_id: run,
                group_id: group,
                page_size: 1,
                cursor: one.next_cursor.clone(),
            },
        )
        .unwrap();
        assert_eq!((two.items[0].asset_id, two.next_cursor.as_str()), (2, ""));
        // 越权与错配游标。
        assert!(page_ownership_groups_for_connection(
            &connection,
            OwnershipGroupPageInput {
                project_id: 92,
                run_id: run,
                page_size: 10,
                ..Default::default()
            }
        )
        .is_err());
        assert!(page_ownership_groups_for_connection(
            &connection,
            OwnershipGroupPageInput {
                project_id: 91,
                run_id: run,
                page_size: 10,
                cursor: "g999:1".into(),
                ..Default::default()
            }
        )
        .is_err());
        assert!(page_ownership_group_members_for_connection(
            &connection,
            OwnershipMemberPageInput {
                project_id: 91,
                run_id: run,
                group_id: group,
                page_size: 1,
                cursor: format!("m{run}:{}:1", group + 1)
            }
        )
        .is_err());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn group_confirmation_writes_ownership_only_and_audits_every_row() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 50);
        let group = group_of(&connection, run, 1).id;
        let preview = preview_ownership_group_decision_for_connection(
            &connection,
            request(run, group, "confirm_ownership"),
        )
        .unwrap();
        assert_eq!(
            (
                preview.member_count,
                preview.applicable,
                preview.skipped,
                preview.conflicted
            ),
            (2, 2, 0, 0)
        );
        assert!(
            !preview.proposed.grants_authorization
                && !preview.proposed.sets_exposure_eligible
                && preview.proposed.authorization == "unchanged"
        );
        let result = apply_ownership_group_decision_for_connection(
            &mut connection,
            apply_input(&preview, "op-confirm-0001"),
        )
        .unwrap();
        assert_eq!(
            (
                result.applied,
                result.skipped,
                result.conflicted,
                result.failed,
                result.replayed
            ),
            (2, 0, 0, 0, false)
        );
        for asset in [1, 2] {
            assert_eq!(
                decision(&connection, asset),
                Some(("confirmed".into(), "manual".into(), "unknown".into(), 0)),
                "ownership confirmation must not authorize or admit"
            );
        }
        assert_eq!(decision(&connection, 3), None, "other groups untouched");
        assert_eq!(
            decision(&connection, 15),
            Some(("attributed".into(), "auto".into(), "allowed".into(), 1))
        );
        let (old_json, new_json, evidence): (String, String, String) = connection
            .query_row("SELECT old_json,new_json,evidence_ref FROM ownership_operation_items WHERE operation_id='op-confirm-0001' AND asset_id=1", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap();
        let old: JsonValue = serde_json::from_str(&old_json).unwrap();
        let new: JsonValue = serde_json::from_str(&new_json).unwrap();
        assert_eq!(
            (old["status"].as_str(), old["exists"].as_bool()),
            (Some("unreviewed"), Some(false))
        );
        assert_eq!(
            (
                new["status"].as_str(),
                new["authorizationStatus"].as_str(),
                new["exposureEligible"].as_i64()
            ),
            (Some("confirmed"), Some("unknown"), Some(0))
        );
        assert!(evidence.starts_with(&format!("run:{run}/group:{group}/evidence:")));
        assert_eq!(
            count(
                &connection,
                "SELECT COUNT(*) FROM asset_events WHERE project_id=91 AND event_type='ownership'"
            ),
            2
        );
        // 排除类动作沿用既有收窄语义。
        let excluded_group = group_of(&connection, run, 10).id;
        let preview = preview_ownership_group_decision_for_connection(
            &connection,
            request(run, excluded_group, "excluded"),
        )
        .unwrap();
        assert_eq!(
            (
                preview.proposed.authorization.as_str(),
                preview.proposed.exposure_eligible.as_str()
            ),
            ("denied", "revoked")
        );
        apply_ownership_group_decision_for_connection(
            &mut connection,
            apply_input(&preview, "op-exclude-0001"),
        )
        .unwrap();
        assert_eq!(
            decision(&connection, 10),
            Some(("excluded".into(), "manual".into(), "denied".into(), 0))
        );
        let overview = read_ownership_analysis_for_connection(&connection, 91).unwrap();
        assert!(
            overview.queue.unwrap().completed_groups >= 3,
            "two operated groups + manual group"
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn group_operation_id_is_idempotent_and_rejects_a_different_payload() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 50);
        let group = group_of(&connection, run, 1).id;
        let preview = preview_ownership_group_decision_for_connection(
            &connection,
            request(run, group, "confirm_ownership"),
        )
        .unwrap();
        let first = apply_ownership_group_decision_for_connection(
            &mut connection,
            apply_input(&preview, "op-idem-0001"),
        )
        .unwrap();
        let replay = apply_ownership_group_decision_for_connection(
            &mut connection,
            apply_input(&preview, "op-idem-0001"),
        )
        .unwrap();
        assert!(replay.replayed && !first.replayed);
        assert_eq!(
            (replay.applied, replay.group_version),
            (first.applied, first.group_version)
        );
        assert_eq!(
            count(
                &connection,
                "SELECT COUNT(*) FROM ownership_operation_items WHERE operation_id='op-idem-0001'"
            ),
            2
        );
        assert_eq!(
            count(
                &connection,
                "SELECT COUNT(*) FROM asset_events WHERE event_type='ownership'"
            ),
            2,
            "replay must not write twice"
        );
        let mut other = apply_input(&preview, "op-idem-0001");
        other.note = "另一份说明".into();
        let error =
            apply_ownership_group_decision_for_connection(&mut connection, other).unwrap_err();
        assert!(error.starts_with(OWNERSHIP_OPERATION_MISMATCH), "{error}");
        let mut other_action = apply_input(&preview, "op-idem-0001");
        other_action.action = "related".into();
        assert!(
            apply_ownership_group_decision_for_connection(&mut connection, other_action)
                .unwrap_err()
                .starts_with(OWNERSHIP_OPERATION_MISMATCH)
        );
        // 新 operationId 携带旧预览：组版本已前进，拒绝。
        let error = apply_ownership_group_decision_for_connection(
            &mut connection,
            apply_input(&preview, "op-idem-0002"),
        )
        .unwrap_err();
        assert!(error.starts_with(OWNERSHIP_PREVIEW_STALE), "{error}");
        assert!(apply_ownership_group_decision_for_connection(
            &mut connection,
            apply_input(&preview, "x")
        )
        .unwrap_err()
        .starts_with(OWNERSHIP_REVIEW_INVALID));
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn apply_rejects_a_preview_whose_group_or_policy_versions_do_not_match() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 50);
        let group = group_of(&connection, run, 1).id;
        let preview = preview_ownership_group_decision_for_connection(
            &connection,
            request(run, group, "confirm_ownership"),
        )
        .unwrap();
        // 哈希相同、只篡改版本号：组版本 / 策略 / 目录 / 绑定版本任一不一致都必须拒绝且不写入。
        type Tamper = (&'static str, fn(&mut OwnershipGroupDecisionApply));
        let tamper: [Tamper; 4] = [
            ("group_version", |input| input.group_version += 1),
            ("policy_version", |input| input.policy_version += 1),
            ("directory_version", |input| input.directory_version += 1),
            ("binding_version", |input| input.binding_version += 1),
        ];
        for (index, (field, change)) in tamper.iter().enumerate() {
            let mut input = apply_input(&preview, &format!("op-version-000{index}"));
            change(&mut input);
            let error =
                apply_ownership_group_decision_for_connection(&mut connection, input).unwrap_err();
            assert!(
                error.starts_with(OWNERSHIP_PREVIEW_STALE),
                "{field}: {error}"
            );
        }
        assert_eq!(
            count(&connection, "SELECT COUNT(*) FROM ownership_operations"),
            0
        );
        assert_eq!(
            count(
                &connection,
                "SELECT COUNT(*) FROM asset_events WHERE event_type='ownership'"
            ),
            0
        );
        apply_ownership_group_decision_for_connection(
            &mut connection,
            apply_input(&preview, "op-version-ok"),
        )
        .unwrap();
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_preview_is_rejected_after_delete_revoke_evidence_or_manual_change() {
        let mutations: [(&str, &str); 5] = [
            ("deleted_or_unlinked", "UPDATE project_assets SET is_deleted=1 WHERE project_id=91 AND asset_id=2"),
            ("decision_changed", "INSERT INTO asset_ownership_decisions(project_id,asset_id,status,authorization_status,source) VALUES(91,2,'unreviewed','revoked','auto')"),
            ("evidence_changed", "UPDATE assets SET title='页面内容已变化' WHERE id=2"),
            ("left_review_scope", "UPDATE assets SET probe_outcome='dead' WHERE id=2"),
            ("decision_changed", "MANUAL"),
        ];
        for (expected_reason, mutation) in mutations {
            let (root, mut connection) = fixture();
            let run = run_analysis(&mut connection, 91, 50);
            let group = group_of(&connection, run, 1).id;
            let preview = preview_ownership_group_decision_for_connection(
                &connection,
                request(run, group, "confirm_ownership"),
            )
            .unwrap();
            if mutation == "MANUAL" {
                // 人工逐条结论走 O0 既有路径。
                let input = AssetOwnershipDecisionInput {
                    selections: vec![AssetSelection {
                        project_id: 91,
                        asset_id: 2,
                    }],
                    status: "related".into(),
                    note: "人工".into(),
                    learn_domain_rule: false,
                };
                let seen = AssetOwnershipDecisionExpectation {
                    asset_id: 2,
                    status: "unreviewed".into(),
                    source: String::new(),
                    confidence: 0,
                    authorization_status: "unknown".into(),
                    exposure_eligible: false,
                    reason: String::new(),
                };
                apply_asset_ownership_decisions(&mut connection, input, 91, vec![seen]).unwrap();
            } else {
                connection.execute_batch(mutation).unwrap();
            }
            let before = decision(&connection, 1);
            let error = apply_ownership_group_decision_for_connection(
                &mut connection,
                apply_input(&preview, "op-stale-0001"),
            )
            .unwrap_err();
            assert!(
                error.starts_with(OWNERSHIP_PREVIEW_STALE),
                "{mutation}: {error}"
            );
            assert_eq!(
                decision(&connection, 1),
                before,
                "{mutation}: nothing written"
            );
            assert_eq!(
                count(&connection, "SELECT COUNT(*) FROM ownership_operations"),
                0
            );
            // 重新预览：变化成员单列为冲突且不写入，其余成员照常。
            let again = preview_ownership_group_decision_for_connection(
                &connection,
                request(run, group, "confirm_ownership"),
            )
            .unwrap();
            assert_eq!((again.applicable, again.conflicted), (1, 1), "{mutation}");
            assert_eq!(
                again.reasons.get(expected_reason),
                Some(&1),
                "{mutation}: {:?}",
                again.reasons
            );
            let result = apply_ownership_group_decision_for_connection(
                &mut connection,
                apply_input(&again, "op-stale-0002"),
            )
            .unwrap();
            assert_eq!(
                (result.applied, result.conflicted, result.failed),
                (1, 1, 0)
            );
            assert_eq!(decision(&connection, 1).unwrap().0, "confirmed");
            assert_ne!(
                decision(&connection, 2).map(|row| row.0),
                Some("confirmed".to_string()),
                "{mutation}: changed member never confirmed"
            );
            let outcome: String = connection.query_row("SELECT outcome FROM ownership_operation_items WHERE operation_id='op-stale-0002' AND asset_id=2", [], |row| row.get(0)).unwrap();
            assert_eq!(outcome, "conflicted");
            drop(connection);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn members_added_after_the_snapshot_are_never_included() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 50);
        connection
            .execute_batch(&format!("INSERT INTO assets(id,asset_key,company,host,domain,link,title) VALUES(30,'o2-30','{ZJ}','zj.10086.cn','zj.10086.cn','https://zj.10086.cn/p30','t'); INSERT INTO project_assets(project_id,asset_id) VALUES(91,30);"))
            .unwrap();
        let group = group_of(&connection, run, 1).id;
        let preview = preview_ownership_group_decision_for_connection(
            &connection,
            request(run, group, "confirm_ownership"),
        )
        .unwrap();
        assert_eq!((preview.member_count, preview.applicable), (2, 2));
        let result = apply_ownership_group_decision_for_connection(
            &mut connection,
            apply_input(&preview, "op-new-member-1"),
        )
        .unwrap();
        assert_eq!(result.applied, 2);
        assert_eq!(
            decision(&connection, 30),
            None,
            "asset added after the snapshot is not approved"
        );
        let queue = read_ownership_analysis_for_connection(&connection, 91)
            .unwrap()
            .queue
            .unwrap();
        assert_eq!(queue.new_since_snapshot, 1);
        assert_eq!(
            queue.changed_since_snapshot, 0,
            "members handled by a group operation are not pending changes"
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn conflict_counter_unknown_insufficient_and_manual_groups_cannot_be_confirmed() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 50);
        for asset in [7, 8, 9, 10, 11, 12, 13, 14, 16] {
            let group = group_of(&connection, run, asset);
            let error = preview_ownership_group_decision_for_connection(
                &connection,
                request(run, group.id, "confirm_ownership"),
            )
            .unwrap_err();
            assert!(
                error.starts_with(OWNERSHIP_GROUP_ACTION_DENIED),
                "asset {asset} lane {}: {error}",
                group.lane
            );
        }
        // 人工结论组不允许任何组动作；收窄动作在非人工组可用。
        assert!(preview_ownership_group_decision_for_connection(
            &connection,
            request(run, group_of(&connection, run, 14).id, "excluded")
        )
        .unwrap_err()
        .starts_with(OWNERSHIP_GROUP_ACTION_DENIED));
        assert!(preview_ownership_group_decision_for_connection(
            &connection,
            request(run, group_of(&connection, run, 9).id, "excluded")
        )
        .is_ok());
        // 手工构造的 apply 也走同一检查。
        let consistent = preview_ownership_group_decision_for_connection(
            &connection,
            request(run, group_of(&connection, run, 1).id, "confirm_ownership"),
        )
        .unwrap();
        let mut forged = apply_input(&consistent, "op-forged-0001");
        forged.group_id = group_of(&connection, run, 7).id;
        assert!(
            apply_ownership_group_decision_for_connection(&mut connection, forged)
                .unwrap_err()
                .starts_with(OWNERSHIP_GROUP_ACTION_DENIED)
        );
        assert!(preview_ownership_group_decision_for_connection(
            &connection,
            OwnershipGroupDecisionRequest {
                project_id: 92,
                ..request(run, group_of(&connection, run, 1).id, "confirm_ownership")
            }
        )
        .is_err());
        assert_eq!(
            count(
                &connection,
                "SELECT COUNT(*) FROM asset_ownership_decisions WHERE source='manual'"
            ),
            1
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn snapshot_is_stale_after_policy_directory_or_newer_snapshot() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 50);
        let group = group_of(&connection, run, 1).id;
        let preview = preview_ownership_group_decision_for_connection(
            &connection,
            request(run, group, "confirm_ownership"),
        )
        .unwrap();
        save_ownership_profile_for_connection(
            &mut connection,
            AssetOwnershipProfile {
                project_id: 91,
                legal_name: "中国移动通信集团有限公司".into(),
                ..Default::default()
            },
            0,
        )
        .unwrap();
        let error = apply_ownership_group_decision_for_connection(
            &mut connection,
            apply_input(&preview, "op-policy-0001"),
        )
        .unwrap_err();
        assert!(
            error.starts_with(OWNERSHIP_SNAPSHOT_STALE) && error.contains("policy:0->1"),
            "{error}"
        );
        assert!(preview_ownership_group_decision_for_connection(
            &connection,
            request(run, group, "confirm_ownership")
        )
        .unwrap_err()
        .starts_with(OWNERSHIP_SNAPSHOT_STALE));
        let overview = read_ownership_analysis_for_connection(&connection, 91).unwrap();
        assert!(overview.readiness.contains(&"stale_snapshot".to_string()));
        let fresh = run_analysis(&mut connection, 91, 50);
        let fresh_group = group_of(&connection, fresh, 1).id;
        assert!(
            preview_ownership_group_decision_for_connection(
                &connection,
                request(run, group, "confirm_ownership")
            )
            .unwrap_err()
            .starts_with(OWNERSHIP_SNAPSHOT_STALE),
            "older snapshot is rejected"
        );
        let preview = preview_ownership_group_decision_for_connection(
            &connection,
            request(fresh, fresh_group, "confirm_ownership"),
        )
        .unwrap();
        connection
            .execute("UPDATE org_directory_state SET version=version+1", [])
            .unwrap();
        let error = apply_ownership_group_decision_for_connection(
            &mut connection,
            apply_input(&preview, "op-dir-0001"),
        )
        .unwrap_err();
        assert!(
            error.starts_with(OWNERSHIP_SNAPSHOT_STALE) && error.contains("directory:1->2"),
            "{error}"
        );
        assert_eq!(decision(&connection, 1), None);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn page_queries_use_the_snapshot_indexes() {
        let (root, mut connection) = fixture();
        let run = run_analysis(&mut connection, 91, 50);
        let plan = |sql: &str| -> String {
            let mut statement = connection
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .unwrap();
            let rows = statement
                .query_map([], |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.join(" | ")
        };
        let groups = plan(&format!("SELECT id FROM ownership_review_groups WHERE run_id={run} AND sort_rank>0 ORDER BY sort_rank LIMIT 51"));
        assert!(
            groups.contains("idx_ownership_groups_rank") && !groups.contains("TEMP B-TREE"),
            "{groups}"
        );
        let lane = plan(&format!("SELECT id FROM ownership_review_groups WHERE run_id={run} AND lane='conflict' AND sort_rank>0 ORDER BY sort_rank LIMIT 51"));
        assert!(
            lane.contains("idx_ownership_groups_lane") && !lane.contains("TEMP B-TREE"),
            "{lane}"
        );
        let members = plan(&format!("SELECT asset_id FROM ownership_review_group_members m WHERE m.run_id={run} AND m.group_id=1 AND m.asset_id>0 ORDER BY m.asset_id LIMIT 101"));
        assert!(
            members.contains("idx_ownership_members_group") && !members.contains("TEMP B-TREE"),
            "{members}"
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    pub(super) fn percentile(values: &mut [f64], fraction: f64) -> f64 {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        values[((values.len() as f64 - 1.0) * fraction).round() as usize]
    }

    /// 4000 行级验收：省公司/专业子公司/多租户/共享域名/CIDR/重复 URL/弱标签/人工结论/删除/范围外混合。
    #[test]
    fn acceptance_4000_rows_reconciles_pages_and_applies_one_group() {
        let (root, mut connection) = fixture();
        let provinces = [("浙江", "zj"), ("江苏", "js")];
        let started = Instant::now();
        {
            let transaction = connection.transaction().unwrap();
            for index in 0..4000i64 {
                let id = 1000 + index;
                let (province, code) = provinces[(index % 2) as usize];
                let (company, domain, ip) = match index % 10 {
                    0..=2 => (
                        format!("中国移动通信集团{province}有限公司"),
                        format!("{code}.10086.cn"),
                        String::new(),
                    ),
                    3 => (
                        format!("中国移动通信集团{province}有限公司"),
                        format!("svc{}.{code}gov.com.cn", index % 7),
                        String::new(),
                    ),
                    4 => (
                        "中移（苏州）软件技术有限公司".into(),
                        format!("app{}.sz{}.com.cn", index % 3, (index / 10) % 40),
                        String::new(),
                    ),
                    5 => (
                        format!("161.81.{}.0/22", index % 200),
                        String::new(),
                        format!("161.81.{}.7", index % 200),
                    ),
                    6 => (
                        format!("{province}移动"),
                        format!("t{}.github.io", (index / 10) % 25),
                        String::new(),
                    ),
                    7 => (
                        String::new(),
                        format!("h{}.example{}.org", index, (index / 10) % 30),
                        String::new(),
                    ),
                    8 => (
                        format!("中国移动通信集团{province}有限公司"),
                        "www.10086.cn".into(),
                        String::new(),
                    ),
                    _ => (
                        format!("某合作单位{}有限公司", (index / 10) % 50),
                        format!("p{}.partner{}.cn", index, (index / 10) % 50),
                        String::new(),
                    ),
                };
                transaction
                    .execute(
                        "INSERT INTO assets(id,asset_key,company,host,domain,link,ip,title) VALUES(?1,?2,?3,?4,?4,CASE WHEN ?4='' THEN '' ELSE 'https://'||?4||'/x?utm='||?1 END,?5,'门户')",
                        params![id, format!("o2-accept-{id}"), company, domain, ip],
                    )
                    .unwrap();
                transaction
                    .execute("INSERT INTO project_assets(project_id,asset_id,is_deleted) VALUES(91,?1,?2)", params![id, i64::from(index % 97 == 0)])
                    .unwrap();
                if index % 113 == 0 {
                    transaction
                        .execute("INSERT INTO asset_ownership_decisions(project_id,asset_id,status,confidence,authorization_status,exposure_eligible,source,reason) VALUES(91,?1,'related',70,'unknown',0,'manual','人工')", [id])
                        .unwrap();
                }
            }
            transaction.commit().unwrap();
        }
        let seeded = started.elapsed();
        let scope = count(&connection, &format!("SELECT COUNT(*) FROM project_assets pa JOIN assets a ON a.id=pa.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id=91 AND {OWNERSHIP_SCOPE_SQL}"));
        assert!(scope > 3900, "{scope}");
        let analysis_started = Instant::now();
        let run = run_analysis(&mut connection, 91, OWNERSHIP_ANALYSIS_CHUNK);
        let analysis = analysis_started.elapsed();
        let snapshot = ownership_read_run(&connection, run).unwrap().unwrap();
        assert_eq!(
            (
                snapshot.member_count,
                snapshot.processed_assets,
                snapshot.total_assets
            ),
            (scope, scope, scope)
        );
        let mut cursor = String::new();
        let mut page_ms = Vec::new();
        let (mut groups_seen, mut member_sum) = (HashSet::new(), 0i64);
        let mut biggest_consistent: Option<(i64, i64)> = None;
        let mut lanes: std::collections::BTreeMap<String, (i64, i64)> = Default::default();
        loop {
            let page_started = Instant::now();
            let page = page_ownership_groups_for_connection(
                &connection,
                OwnershipGroupPageInput {
                    project_id: 91,
                    run_id: run,
                    page_size: 50,
                    cursor: cursor.clone(),
                    ..Default::default()
                },
            )
            .unwrap();
            page_ms.push(page_started.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(page.total, snapshot.group_count);
            for item in &page.items {
                assert!(groups_seen.insert(item.id));
                member_sum += item.member_count;
                let lane = lanes.entry(item.lane.clone()).or_default();
                lane.0 += 1;
                lane.1 += item.member_count;
                if item.lane == "consistent"
                    && biggest_consistent.is_none_or(|(_, size)| item.member_count > size)
                {
                    biggest_consistent = Some((item.id, item.member_count));
                }
            }
            if page.next_cursor.is_empty() {
                break;
            }
            cursor = page.next_cursor;
        }
        assert_eq!(
            (groups_seen.len() as i64, member_sum),
            (snapshot.group_count, scope),
            "every asset counted once"
        );
        // 公共后缀：*.com.cn 不同注册域不合并；多租户平台各租户独立。
        let distinct_com_cn = count(&connection, &format!("SELECT COUNT(DISTINCT boundary_value) FROM ownership_review_groups WHERE run_id={run} AND boundary_value LIKE '%.com.cn'"));
        assert!(distinct_com_cn >= 40, "{distinct_com_cn}");
        assert_eq!(count(&connection, &format!("SELECT COUNT(*) FROM ownership_review_groups WHERE run_id={run} AND boundary_value IN ('com.cn','github.io')")), 0);
        let tenants = count(&connection, &format!("SELECT COUNT(DISTINCT boundary_value) FROM ownership_review_groups WHERE run_id={run} AND boundary_kind='private_suffix_tenant'"));
        assert_eq!(
            tenants, 26,
            "25 synthetic tenants + the fixture's zj-portal.github.io"
        );
        let (group, size) = biggest_consistent.expect("a consistent group");
        let mut member_ms = Vec::new();
        let mut cursor = String::new();
        let mut members = 0;
        loop {
            let member_started = Instant::now();
            let page = page_ownership_group_members_for_connection(
                &connection,
                OwnershipMemberPageInput {
                    project_id: 91,
                    run_id: run,
                    group_id: group,
                    page_size: 100,
                    cursor: cursor.clone(),
                },
            )
            .unwrap();
            member_ms.push(member_started.elapsed().as_secs_f64() * 1000.0);
            members += page.items.len() as i64;
            assert_eq!(page.total, size);
            if page.next_cursor.is_empty() {
                break;
            }
            cursor = page.next_cursor;
        }
        assert_eq!(members, size);
        let preview_started = Instant::now();
        let preview = preview_ownership_group_decision_for_connection(
            &connection,
            request(run, group, "confirm_ownership"),
        )
        .unwrap();
        let preview_time = preview_started.elapsed();
        let before_other = count(
            &connection,
            "SELECT COUNT(*) FROM asset_ownership_decisions WHERE project_id=91",
        );
        let before_eligible = count(&connection, "SELECT COUNT(*) FROM asset_ownership_decisions WHERE project_id=91 AND exposure_eligible=1");
        let apply_started = Instant::now();
        let result = apply_ownership_group_decision_for_connection(
            &mut connection,
            apply_input(&preview, "op-accept-0001"),
        )
        .unwrap();
        let apply_time = apply_started.elapsed();
        assert_eq!((result.applied, result.failed), (preview.applicable, 0));
        assert_eq!(result.applied + result.skipped + result.conflicted, size);
        assert_eq!(
            count(
                &connection,
                "SELECT COUNT(*) FROM asset_ownership_decisions WHERE project_id=91"
            ) - before_other,
            result.applied,
            "only this group's members gained a decision row"
        );
        assert_eq!(count(&connection, "SELECT COUNT(*) FROM asset_ownership_decisions WHERE project_id=91 AND exposure_eligible=1"), before_eligible, "group confirmation adds no exposure eligibility");
        assert_eq!(count(&connection, "SELECT COUNT(*) FROM asset_ownership_decisions WHERE project_id=91 AND authorization_status='allowed'"), 2, "only the two fixture rows carry an authorization");
        let queue = read_ownership_analysis_for_connection(&connection, 91)
            .unwrap()
            .queue
            .unwrap();
        assert_eq!((queue.member_total, queue.group_member_sum), (scope, scope));
        eprintln!(
            "O2_ACCEPTANCE rows={} scope={scope} groups={} lanes={lanes:?} seed_ms={} analysis_ms={} group_page_p50_ms={:.2} group_page_p95_ms={:.2} pages={} member_page_p50_ms={:.2} member_page_p95_ms={:.2} biggest_consistent={size} preview_ms={} apply_ms={} applied={} compression={:.3}",
            4019,
            snapshot.group_count,
            seeded.as_millis(),
            analysis.as_millis(),
            percentile(&mut page_ms.clone(), 0.5),
            percentile(&mut page_ms.clone(), 0.95),
            page_ms.len(),
            percentile(&mut member_ms.clone(), 0.5),
            percentile(&mut member_ms.clone(), 0.95),
            preview_time.as_millis(),
            apply_time.as_millis(),
            result.applied,
            snapshot.group_count as f64 / scope as f64,
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    /// 真实规模只读副本验收（不在常规测试中运行）：OVI_O2_SOURCE_DB 指向从应用库以 mode=ro 抽取的单项目副本，
    /// OVI_O2_PROJECT 为项目 id。所有写入都只发生在本测试新建的临时库中。
    #[test]
    #[ignore]
    fn acceptance_on_extracted_project_copy() {
        let source = std::env::var("OVI_O2_SOURCE_DB").expect("OVI_O2_SOURCE_DB");
        let project_id: i64 = std::env::var("OVI_O2_PROJECT")
            .expect("OVI_O2_PROJECT")
            .parse()
            .unwrap();
        let (root, mut connection) = temp_db("copy");
        connection
            .execute("ATTACH DATABASE ?1 AS src", [&source])
            .unwrap();
        let columns = |table: &str, source_table: &str| -> String {
            let main: Vec<String> = connection
                .prepare(&format!("PRAGMA main.table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let other: HashSet<String> = connection
                .prepare(&format!("PRAGMA src.table_info({source_table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            main.into_iter()
                .filter(|column| other.contains(column))
                .collect::<Vec<_>>()
                .join(",")
        };
        let copy = |connection: &rusqlite::Connection,
                    table: &str,
                    source_table: &str,
                    columns: &str| {
            connection.execute(&format!("INSERT INTO main.{table}({columns}) SELECT {columns} FROM src.{source_table}"), []).unwrap();
        };
        let (projects, assets, links, decisions) = (
            columns("projects", "x_projects"),
            columns("assets", "x_assets"),
            columns("project_assets", "x_project_assets"),
            columns("asset_ownership_decisions", "x_decisions"),
        );
        copy(&connection, "projects", "x_projects", &projects);
        copy(&connection, "assets", "x_assets", &assets);
        // last_run_id 指向未复制的 runs，置空（与归属分析无关）。
        let links = links
            .split(',')
            .filter(|column| *column != "last_run_id")
            .collect::<Vec<_>>()
            .join(",");
        connection.execute(&format!("INSERT INTO main.project_assets({links}) SELECT {links} FROM src.x_project_assets"), []).unwrap();
        copy(
            &connection,
            "asset_ownership_decisions",
            "x_decisions",
            &decisions,
        );
        connection.execute("DETACH DATABASE src", []).unwrap();
        let scope = count(&connection, &format!("SELECT COUNT(*) FROM project_assets pa JOIN assets a ON a.id=pa.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=pa.project_id AND o.asset_id=pa.asset_id WHERE pa.project_id={project_id} AND {OWNERSHIP_SCOPE_SQL}"));
        let analysis_started = Instant::now();
        let run = run_analysis(&mut connection, project_id, OWNERSHIP_ANALYSIS_CHUNK);
        let analysis = analysis_started.elapsed();
        let snapshot = ownership_read_run(&connection, run).unwrap().unwrap();
        assert_eq!(snapshot.member_count, scope);
        let mut cursor = String::new();
        let mut page_ms = Vec::new();
        let mut member_sum = 0;
        let mut lanes: std::collections::BTreeMap<String, (i64, i64)> = Default::default();
        let mut largest: Option<(i64, i64, String)> = None;
        loop {
            let started = Instant::now();
            let page = page_ownership_groups_for_connection(
                &connection,
                OwnershipGroupPageInput {
                    project_id,
                    run_id: run,
                    page_size: 50,
                    cursor: cursor.clone(),
                    ..Default::default()
                },
            )
            .unwrap();
            page_ms.push(started.elapsed().as_secs_f64() * 1000.0);
            for item in &page.items {
                member_sum += item.member_count;
                let lane = lanes.entry(item.lane.clone()).or_default();
                lane.0 += 1;
                lane.1 += item.member_count;
                if item.lane != "manual"
                    && largest
                        .as_ref()
                        .is_none_or(|(_, size, _)| item.member_count > *size)
                {
                    largest = Some((item.id, item.member_count, item.lane.clone()));
                }
            }
            if page.next_cursor.is_empty() {
                break;
            }
            cursor = page.next_cursor;
        }
        assert_eq!(member_sum, scope);
        let (group, size, lane) = largest.expect("a non-manual group");
        let action = if lane == "consistent" {
            "confirm_ownership"
        } else {
            "related"
        };
        let preview = preview_ownership_group_decision_for_connection(
            &connection,
            OwnershipGroupDecisionRequest {
                project_id,
                run_id: run,
                group_id: group,
                action: action.into(),
                note: String::new(),
            },
        )
        .unwrap();
        let before_eligible = count(&connection, &format!("SELECT COUNT(*) FROM asset_ownership_decisions WHERE project_id={project_id} AND exposure_eligible=1"));
        let started = Instant::now();
        let result = apply_ownership_group_decision_for_connection(
            &mut connection,
            apply_input(&preview, "op-copy-acceptance-1"),
        )
        .unwrap();
        let apply_ms = started.elapsed().as_millis();
        assert_eq!(
            result.applied + result.skipped + result.conflicted + result.failed,
            size
        );
        assert_eq!(count(&connection, &format!("SELECT COUNT(*) FROM asset_ownership_decisions WHERE project_id={project_id} AND exposure_eligible=1")), before_eligible);
        let replay = apply_ownership_group_decision_for_connection(
            &mut connection,
            apply_input(&preview, "op-copy-acceptance-1"),
        )
        .unwrap();
        assert!(replay.replayed);
        let top_counters: Vec<(String, i64)> = connection.prepare(&format!("SELECT counter_kind,SUM(member_count) FROM ownership_review_groups WHERE run_id={run} AND counter_kind<>'' GROUP BY counter_kind ORDER BY 2 DESC LIMIT 8")).unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap().collect::<Result<_, _>>().unwrap();
        let boundaries: Vec<(String, i64)> = connection.prepare(&format!("SELECT boundary_kind,COUNT(*) FROM ownership_review_groups WHERE run_id={run} GROUP BY boundary_kind ORDER BY 2 DESC")).unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap().collect::<Result<_, _>>().unwrap();
        let singletons = count(&connection, &format!("SELECT COUNT(*) FROM ownership_review_groups WHERE run_id={run} AND member_count=1"));
        let suffixes: Vec<(String, i64)> = connection.prepare(&format!("SELECT suffix_section||':'||substr(registrable_domain,instr(registrable_domain,'.')+1),COUNT(*) FROM ownership_review_group_members WHERE run_id={run} GROUP BY 1 ORDER BY 2 DESC LIMIT 12")).unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap().collect::<Result<_, _>>().unwrap();
        eprintln!("O2_COPY_SUFFIXES {suffixes:?}");
        eprintln!(
            "O2_COPY_ACCEPTANCE project={project_id} scope={scope} groups={} singletons={singletons} lanes={lanes:?} boundaries={boundaries:?} counters={top_counters:?} analysis_ms={} group_page_p50_ms={:.2} group_page_p95_ms={:.2} pages={} largest_group={size} lane={lane} action={action} applied={} skipped={} conflicted={} failed={} apply_ms={apply_ms}",
            snapshot.group_count,
            analysis.as_millis(),
            percentile(&mut page_ms.clone(), 0.5),
            percentile(&mut page_ms.clone(), 0.95),
            page_ms.len(),
            result.applied,
            result.skipped,
            result.conflicted,
            result.failed,
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
