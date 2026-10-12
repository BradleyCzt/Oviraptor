//! G1-3 信息分类与公开招采排除（规格 §6/§7）。
//!
//! 每条线索在被观察到的那一刻按固定策略版本分类：信息类别、来源渠道、处置。处置只有四种：
//! 风险候选、待分类、公开招采排除、公开材料排除。招采判定看的是页面本身（标题字样 + 结构字段，
//! 或公认招采平台），不看文件名；混合页面（招采公告里同时出现凭据候选或业务源码图）一律留给人工。
//! 排除是粘性的：后续观察不会自动撤销排除，也不会覆盖人工已确认的结论，冲突只记标记，等人工改判
//! （改判通道见 IR-07）。

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::observations::FindingDraft;
use super::rules;

// v2（FIX2-5/T08）：加入广告、新闻、公开领导信息、招聘、客服联系方式、公开产品文档的正常公开排除。
pub const CLASSIFY_POLICY_VERSION: &str = "oviraptor.exposure.classify.v2";

pub const DISPOSITION_RISK: &str = "risk_candidate";
pub const DISPOSITION_PENDING: &str = "classification_pending";
pub const DISPOSITION_EXCLUDED_PROCUREMENT: &str = "excluded_public_procurement";
pub const DISPOSITION_EXCLUDED_MATERIAL: &str = "excluded_public_material";
pub const DISPOSITIONS: &[&str] = &[
    DISPOSITION_RISK,
    DISPOSITION_PENDING,
    DISPOSITION_EXCLUDED_PROCUREMENT,
    DISPOSITION_EXCLUDED_MATERIAL,
];

/// 线索列表的处置筛选。`unclassified` 是没有分类行的旧线索（G1 之前），`not_excluded` 隐藏两类排除。
pub const DISPOSITION_FILTERS: &[&str] = &[
    "risk_candidate",
    "classification_pending",
    "excluded",
    "excluded_public_procurement",
    "excluded_public_material",
    "not_excluded",
    "unclassified",
];

pub fn disposition_filter(name: &str) -> Option<&'static str> {
    DISPOSITION_FILTERS.iter().copied().find(|known| *known == name)
}

/// 筛选名 → SQL 里允许的处置值（固定字面量，不拼接用户输入）。
pub fn disposition_filter_values(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "risk_candidate" => Some(&[DISPOSITION_RISK]),
        "classification_pending" => Some(&[DISPOSITION_PENDING]),
        "excluded" => Some(&[DISPOSITION_EXCLUDED_PROCUREMENT, DISPOSITION_EXCLUDED_MATERIAL]),
        "excluded_public_procurement" => Some(&[DISPOSITION_EXCLUDED_PROCUREMENT]),
        "excluded_public_material" => Some(&[DISPOSITION_EXCLUDED_MATERIAL]),
        "not_excluded" => Some(&[DISPOSITION_RISK, DISPOSITION_PENDING, "unclassified"]),
        "unclassified" => Some(&["unclassified"]),
        _ => None,
    }
}

/// 规格 §6 的信息类别。`sold_intel` 目前没有任何来源会产出，保留类别是为了让页面能说“没有该来源”。
pub const INFO_CATEGORIES: &[&str] = &[
    "internal_business",
    "technical_network",
    "source_code",
    "credential",
    "employee_data",
    "customer_data",
    "storage_misexposure",
    "sold_intel",
    "unclassified",
];

/// 规格 §6 的来源渠道。G2 起 `code_platform` 由 GitHub 两个来源产出（元数据匿名，代码搜索需凭据）；
/// G3 起 `document_link`（指定公开文档 URL）、`document_import`（文件导入）、`public_intel_import`（情报导入）分列。没有映射的来源键记为 `unrecognized_source` 并进入待分类。
pub const SOURCE_CHANNELS: &[&str] = &[
    "first_party_web",
    "certificate_index",
    "archive_index",
    "code_platform",
    "document_link",
    "document_import",
    "public_intel_import",
    "unrecognized_source",
];

const PROCUREMENT_TITLE_MARKERS: &[&str] = &[
    "招标公告",
    "采购公告",
    "中标公告",
    "中标结果",
    "中标候选人",
    "成交公告",
    "成交结果",
    "竞争性磋商",
    "竞争性谈判",
    "询价公告",
    "单一来源",
    "招标文件",
    "比选公告",
    "资格预审",
];

// 互不包含的结构字段，避免一个词被数成两个。
const PROCUREMENT_STRUCTURE_MARKERS: &[&str] = &[
    "项目编号",
    "采购人",
    "招标人",
    "代理机构",
    "投标截止",
    "开标时间",
    "预算金额",
    "最高限价",
    "采购需求",
    "中标人",
    "中标供应商",
    "成交供应商",
    "成交金额",
    "中标金额",
    "采购方式",
];

const CANONICAL_PROCUREMENT_HOSTS: &[&str] = &[
    "ccgp.gov.cn",
    "ggzy.gov.cn",
    "cebpubservice.com",
    "chinabidding.com.cn",
    "b2b.10086.cn",
];

const FILENAME_PROCUREMENT_HINTS: &[&str] = &[
    "招标", "采购", "中标", "投标", "tender", "bidding", "zbgg", "cggg", "zbwj",
];

// T08 正常公开页面：标题（<title>/<h1>）有对应字样，且正文至少两个该类结构字段，才算这一类页面。
// 每类的结构字段互不包含，避免一个词被数成两个。
const PUBLIC_PAGE_RULES: &[(&str, &[&str], &[&str])] = &[
    (
        "公开领导介绍",
        &["领导班子", "领导简介", "领导介绍", "公司领导", "管理团队", "领导成员", "高管介绍"],
        &["现任", "分管", "职务", "出生", "学历", "毕业于", "工作经历"],
    ),
    (
        "公开招聘信息",
        &["人才招聘", "招聘公告", "招聘信息", "社会招聘", "校园招聘", "诚聘", "招贤纳士"],
        &["岗位职责", "任职要求", "工作地点", "招聘人数", "薪资待遇", "简历投递", "报名方式"],
    ),
    (
        "公开客服联系方式",
        &["联系我们", "客服中心", "客户服务", "服务热线"],
        &["客服热线", "客服电话", "服务时间", "在线客服", "客服邮箱", "投诉电话"],
    ),
    (
        "公开新闻",
        &["新闻中心", "新闻动态", "公司新闻", "新闻发布", "媒体报道"],
        &["发布时间", "发布日期", "来源：", "责任编辑", "浏览次数"],
    ),
    (
        "公开产品文档",
        &["产品手册", "产品说明", "用户手册", "使用说明", "帮助中心", "开发者文档", "产品文档"],
        &["产品介绍", "功能说明", "操作步骤", "常见问题", "快速入门", "使用指南"],
    ),
    (
        "公开广告",
        &["优惠活动", "活动专区", "产品推广", "营销活动"],
        &["活动时间", "活动规则", "活动对象", "立即办理", "参与方式"],
    ),
];

// 出现在“公开页面”里就说明内容混合（可能夹带非公开信息），不自动排除。
const PRIVATE_MARKERS: &[&str] = &["通讯录", "工号", "身份证", "花名册", "内部资料", "内部使用", "仅限内部", "请勿外传", "密级", "机密", "薪酬明细"];

const SCAN_LIMIT: usize = 256 * 1024;

/// 页面本身说明了什么。只由 URL、正文和内容类型决定，不读任何外部状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageContext {
    pub title_markers: Vec<String>,
    pub structural_markers: Vec<String>,
    pub canonical_host: Option<String>,
    pub filename_hint: Option<String>,
    /// T08：确认的正常公开页面类别（理由），与命中的标题字样/结构字段。
    pub public_page: Option<String>,
    pub public_markers: Vec<String>,
    /// 公开页面里出现的非公开字样（只在确认为公开页面时才收集）。
    pub private_markers: Vec<String>,
}

impl PageContext {
    /// 公认招采平台且页面有任一招采字样，或标题字样 + 至少两个结构字段。
    pub fn procurement_confirmed(&self) -> bool {
        (self.canonical_host.is_some()
            && (!self.title_markers.is_empty() || !self.structural_markers.is_empty()))
            || (!self.title_markers.is_empty() && self.structural_markers.len() >= 2)
    }

    pub fn procurement_signal(&self) -> bool {
        self.procurement_confirmed() || !self.title_markers.is_empty()
    }

    pub fn markers(&self) -> Vec<String> {
        let mut rows = Vec::new();
        rows.extend(self.title_markers.iter().map(|m| format!("title:{m}")));
        rows.extend(self.structural_markers.iter().map(|m| format!("field:{m}")));
        rows.extend(self.canonical_host.iter().map(|h| format!("host:{h}")));
        rows.extend(self.filename_hint.iter().map(|h| format!("path:{h}")));
        rows.extend(self.public_markers.iter().map(|m| format!("public:{m}")));
        rows.extend(self.private_markers.iter().map(|m| format!("private:{m}")));
        rows
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let pair = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if let Ok(value) = u8::from_str_radix(pair, 16) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn between_ci(haystack: &str, open: &str, close: &str) -> Option<String> {
    let lowered = haystack.to_ascii_lowercase();
    let start = lowered.find(open)?;
    let after = start + lowered[start..].find('>')? + 1;
    let end = after + lowered[after..].find(close)?;
    haystack.get(after..end).map(|s| s.trim().to_string())
}

fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|h| h.trim_end_matches('.').to_ascii_lowercase()))
}

fn textual(content_type: &str) -> bool {
    let lowered = content_type.to_ascii_lowercase();
    lowered.is_empty()
        || lowered.contains("html")
        || lowered.contains("text")
        || lowered.contains("json")
        || lowered.contains("xml")
}

pub fn page_context(url: &str, body: &[u8], content_type: &str) -> PageContext {
    let mut context = PageContext::default();
    if let Some(host) = host_of(url) {
        context.canonical_host = CANONICAL_PROCUREMENT_HOSTS
            .iter()
            .find(|known| host == **known || host.ends_with(&format!(".{known}")))
            .map(|known| known.to_string());
    }
    if textual(content_type) && !body.is_empty() {
        let slice = &body[..body.len().min(SCAN_LIMIT)];
        let text = String::from_utf8_lossy(slice);
        let heading = match (between_ci(&text, "<title", "</title>"), between_ci(&text, "<h1", "</h1>")) {
            (None, None) => text.chars().take(300).collect::<String>(),
            (title, h1) => format!("{} {}", title.unwrap_or_default(), h1.unwrap_or_default()),
        };
        context.title_markers = PROCUREMENT_TITLE_MARKERS
            .iter()
            .filter(|marker| heading.contains(**marker))
            .map(|marker| marker.to_string())
            .collect();
        context.structural_markers = PROCUREMENT_STRUCTURE_MARKERS
            .iter()
            .filter(|marker| text.contains(**marker))
            .map(|marker| marker.to_string())
            .collect();
        for (label, headings, fields) in PUBLIC_PAGE_RULES {
            let heads: Vec<&str> = headings.iter().copied().filter(|marker| heading.contains(*marker)).collect();
            if heads.is_empty() {
                continue;
            }
            let found: Vec<&str> = fields.iter().copied().filter(|marker| text.contains(*marker)).collect();
            if found.len() >= 2 {
                context.public_page = Some(label.to_string());
                context.public_markers = heads.iter().chain(found.iter()).map(|marker| marker.to_string()).collect();
                context.private_markers = PRIVATE_MARKERS
                    .iter()
                    .filter(|marker| text.contains(**marker))
                    .map(|marker| marker.to_string())
                    .collect();
                break;
            }
        }
    }
    let path = reqwest::Url::parse(url)
        .map(|parsed| parsed.path().to_string())
        .unwrap_or_else(|_| url.to_string());
    let decoded = percent_decode(&path).to_lowercase();
    context.filename_hint = FILENAME_PROCUREMENT_HINTS
        .iter()
        .find(|hint| decoded.contains(**hint))
        .map(|hint| hint.to_string());
    context
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classification {
    pub info_category: &'static str,
    pub source_channel: &'static str,
    pub disposition: &'static str,
    pub reason: String,
    pub markers: Vec<String>,
}

pub fn source_channel(source_key: &str) -> &'static str {
    match source_key {
        "passive_web" => "first_party_web",
        "certificate_transparency" => "certificate_index",
        "web_archive" => "archive_index",
        // G2: an external code platform. Its results are leads about the group, never assets.
        "github_repositories" | "github_code" => "code_platform",
        // G3: an explicit document URL read over HTTP, a file a person imported, and an
        // imported intel report are three different kinds of evidence and stay apart.
        "document_link" => "document_link",
        "document_import" => "document_import",
        "public_intel_import" => "public_intel_import",
        _ => "unrecognized_source",
    }
}

pub fn info_category(draft: &FindingDraft) -> &'static str {
    match draft.category.as_str() {
        "credential" => "credential",
        "source_code" | "repository" => "source_code",
        "storage" => "storage_misexposure",
        "document" => {
            let decoded = percent_decode(&draft.url).to_lowercase();
            if ["员工", "employee", "人事"].iter().any(|w| decoded.contains(w)) {
                "employee_data"
            } else if ["客户", "customer", "用户信息"].iter().any(|w| decoded.contains(w)) {
                "customer_data"
            } else {
                "internal_business"
            }
        }
        "metadata" | "transport" | "resource" => "technical_network",
        "intel_claim" => "sold_intel",
        _ => "unclassified",
    }
}

fn high_risk_candidate(draft: &FindingDraft) -> Option<&'static str> {
    if draft.category == "credential" && draft.confidence > rules::CREDENTIAL_PLACEHOLDER_CONFIDENCE {
        Some("凭据候选")
    } else if draft.category == "source_code" && draft.title == rules::TITLE_MAP_BUSINESS {
        Some("业务源码图")
    } else if draft.category == "storage" {
        Some("存储地址")
    } else {
        None
    }
}

fn public_material(draft: &FindingDraft) -> Option<&'static str> {
    if draft.title == rules::TITLE_MAP_VENDOR {
        Some("Source Map 只含公开第三方包")
    } else if draft.title == rules::TITLE_MAP_EMPTY {
        Some("Source Map 为空图")
    } else if draft.title == rules::TITLE_MAP_NOT_A_MAP {
        Some("同名 .map 不是 Source Map")
    } else if draft.category == "credential" && draft.confidence <= rules::CREDENTIAL_PLACEHOLDER_CONFIDENCE {
        Some("凭据字段只有占位值")
    } else if draft.title == super::adapters::github::TITLE_OFFICIAL_REPOSITORY {
        // The owner list is maintained by a human in the settings; a name that merely looks
        // like the group never lands here.
        Some("运营人员维护的正式开源账号下的仓库")
    } else if draft.title == super::adapters::documents::TITLE_DOCUMENT_DECLARED_PUBLIC {
        // A human declaration with its reason (in the evidence); it never covers a credential
        // lead from the same document, which carries its own title.
        Some("人工声明为公开材料的文档（理由见证据）")
    } else {
        None
    }
}

/// 纯函数：同一草稿、同一来源、同一页面上下文永远得到同一结论。
pub fn classify(draft: &FindingDraft, source_key: &str, context: &PageContext) -> Classification {
    let info_category = info_category(draft);
    let source_channel = source_channel(source_key);
    let markers = context.markers();
    let build = |disposition: &'static str, reason: String| Classification {
        info_category,
        source_channel,
        disposition,
        reason,
        markers: markers.clone(),
    };
    let titles = context.title_markers.join("、");
    let fields = context.structural_markers.join("、");
    if source_channel == "unrecognized_source" {
        return build(
            DISPOSITION_PENDING,
            format!("来源 {source_key} 不在已知渠道里，不自动处置"),
        );
    }
    // G3: an intel report is a claim by a third party. Whatever it contains, it is never
    // turned into a risk or an exclusion automatically; the grade travels in the evidence.
    if source_channel == "public_intel_import" {
        return build(
            DISPOSITION_PENDING,
            "公开情报导入：声称不等于已验证事件，等级见证据，留人工复核".to_string(),
        );
    }
    if let Some(risk) = high_risk_candidate(draft) {
        if context.procurement_signal() {
            return build(
                DISPOSITION_PENDING,
                format!("招采页面（{titles}）里同时出现{risk}，混合页面不自动排除，留人工复核"),
            );
        }
        if let Some(page) = &context.public_page {
            return build(
                DISPOSITION_PENDING,
                format!("{page}页面里同时出现{risk}，混合页面不自动排除，留人工复核"),
            );
        }
    }
    if context.procurement_confirmed() {
        let host = context
            .canonical_host
            .as_ref()
            .map(|h| format!("，公认招采平台 {h}"))
            .unwrap_or_default();
        return build(
            DISPOSITION_EXCLUDED_PROCUREMENT,
            format!("公开招采公告：标题字样「{titles}」，结构字段「{fields}」{host}"),
        );
    }
    if !context.title_markers.is_empty() {
        return build(
            DISPOSITION_PENDING,
            format!("只有招采标题字样「{titles}」，结构字段不足 2 个（{}），不自动排除", context.structural_markers.len()),
        );
    }
    if let Some(hint) = &context.filename_hint {
        return build(
            DISPOSITION_PENDING,
            format!("只有地址或文件名含招采字样「{hint}」，正文未见公告结构，不自动排除"),
        );
    }
    if let Some(page) = &context.public_page {
        if !context.private_markers.is_empty() {
            return build(
                DISPOSITION_PENDING,
                format!("看似{page}，但出现非公开字样「{}」，内容混合，留人工复核", context.private_markers.join("、")),
            );
        }
        return build(
            DISPOSITION_EXCLUDED_MATERIAL,
            format!("正常公开：{page}（字样「{}」）", context.public_markers.join("、")),
        );
    }
    if let Some(material) = public_material(draft) {
        return build(DISPOSITION_EXCLUDED_MATERIAL, format!("公开材料：{material}"));
    }
    // G2: a name in a repository or a code fragment proves neither first-party ownership nor a
    // leak. Only credential-shaped evidence (the high-risk branch above) becomes a risk lead.
    if source_channel == "code_platform" && high_risk_candidate(draft).is_none() {
        return build(
            DISPOSITION_PENDING,
            "代码平台命中只说明出现了集团名称，第一方归属未证实，留人工复核".to_string(),
        );
    }
    // G3: a document that mentions the group proves neither that it is non-public nor that it
    // belongs to the group. Only credential-shaped evidence becomes a risk lead.
    if matches!(source_channel, "document_link" | "document_import") && high_risk_candidate(draft).is_none() {
        return build(
            DISPOSITION_PENDING,
            "文档提及集团名称只说明出现，未证实内容非公开或属于本集团，留人工复核".to_string(),
        );
    }
    if info_category == "unclassified" {
        return build(
            DISPOSITION_PENDING,
            format!("规则类别 {} 没有对应的信息类别", draft.category),
        );
    }
    build(DISPOSITION_RISK, format!("{info_category} 类候选，需人工复核"))
}

fn storage(error: impl std::fmt::Display) -> String {
    format!("exposure_classification_storage：{error}")
}

pub fn is_excluded(disposition: &str) -> bool {
    disposition == DISPOSITION_EXCLUDED_PROCUREMENT || disposition == DISPOSITION_EXCLUDED_MATERIAL
}

/// 写入一次分类并更新线索的当前处置，返回生效的处置。排除粘性、人工确认优先。
pub fn record(
    connection: &Connection,
    project_id: i64,
    finding_id: i64,
    observation_id: Option<i64>,
    run_id: i64,
    classification: &Classification,
) -> Result<&'static str, String> {
    let markers = serde_json::to_string(&classification.markers).map_err(storage)?;
    let now = super::state::utc_now();
    connection
        .execute(
            "INSERT INTO exposure_classifications(project_id,finding_id,observation_id,run_id,policy_version,info_category,source_channel,disposition,reason,markers_json,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(finding_id,observation_id,policy_version) DO UPDATE SET
               run_id=excluded.run_id,info_category=excluded.info_category,source_channel=excluded.source_channel,
               disposition=excluded.disposition,reason=excluded.reason,markers_json=excluded.markers_json",
            params![
                project_id,
                finding_id,
                observation_id,
                run_id,
                CLASSIFY_POLICY_VERSION,
                classification.info_category,
                classification.source_channel,
                classification.disposition,
                classification.reason,
                markers,
                now
            ],
        )
        .map_err(storage)?;
    let classification_id: i64 = connection
        .query_row(
            "SELECT id FROM exposure_classifications WHERE finding_id=?1 AND observation_id IS ?2 AND policy_version=?3",
            params![finding_id, observation_id, CLASSIFY_POLICY_VERSION],
            |row| row.get(0),
        )
        .map_err(storage)?;
    let status: String = connection
        .query_row(
            "SELECT status FROM exposure_findings WHERE id=?1 AND project_id=?2",
            params![finding_id, project_id],
            |row| row.get(0),
        )
        .map_err(storage)?;
    let current: Option<String> = connection
        .query_row(
            "SELECT disposition FROM exposure_finding_dispositions WHERE finding_id=?1",
            [finding_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?;
    let proposed = classification.disposition;
    let human_confirmed = status == "confirmed";
    let (effective, conflict): (&'static str, Option<String>) = match current.as_deref() {
        Some(existing) if is_excluded(existing) && !is_excluded(proposed) => (
            static_disposition(existing)?,
            Some(format!(
                "此前已判为 {existing}；观察 {} 判为 {proposed}（{}）。排除不自动撤销，需人工改判",
                observation_id.unwrap_or(0),
                classification.reason
            )),
        ),
        _ if human_confirmed && is_excluded(proposed) => (
            match current.as_deref() {
                Some(existing) => static_disposition(existing)?,
                None => DISPOSITION_PENDING,
            },
            Some(format!(
                "人工已确认为暴露，自动规则判为 {proposed}（{}），不覆盖人工结论",
                classification.reason
            )),
        ),
        _ => (proposed, None),
    };
    let reason = if effective == proposed {
        classification.reason.clone()
    } else {
        connection
            .query_row(
                "SELECT reason FROM exposure_finding_dispositions WHERE finding_id=?1",
                [finding_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage)?
            .unwrap_or_else(|| "人工结论与自动排除冲突，待人工改判".to_string())
    };
    let class_id = if effective == proposed {
        Some(classification_id)
    } else {
        None
    };
    connection
        .execute(
            "INSERT INTO exposure_finding_dispositions(finding_id,project_id,disposition,info_category,source_channel,reason,policy_version,classification_id,conflict,conflict_note,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,COALESCE(?8,?12),?9,?10,?11)
             ON CONFLICT(finding_id) DO UPDATE SET
               disposition=excluded.disposition,
               info_category=CASE WHEN ?8 IS NULL THEN exposure_finding_dispositions.info_category ELSE excluded.info_category END,
               source_channel=CASE WHEN ?8 IS NULL THEN exposure_finding_dispositions.source_channel ELSE excluded.source_channel END,
               reason=excluded.reason,policy_version=excluded.policy_version,
               classification_id=COALESCE(?8,exposure_finding_dispositions.classification_id),
               conflict=MAX(exposure_finding_dispositions.conflict,excluded.conflict),
               conflict_note=CASE WHEN excluded.conflict=1 THEN excluded.conflict_note ELSE exposure_finding_dispositions.conflict_note END,
               updated_at=excluded.updated_at",
            params![
                finding_id,
                project_id,
                effective,
                classification.info_category,
                classification.source_channel,
                reason,
                CLASSIFY_POLICY_VERSION,
                class_id,
                i64::from(conflict.is_some()),
                conflict.clone().unwrap_or_default(),
                now,
                classification_id
            ],
        )
        .map_err(storage)?;
    Ok(effective)
}


/// IR-07: a person overrides the automatic disposition. The expected classification id is a
/// compare-and-swap on the current row. The operation id makes a retry return the first result.
/// Nothing here contacts a third party. A mismatch or a foreign project writes nothing.
pub fn reclassify(connection: &Connection, project_id: i64, raw: &serde_json::Value) -> Result<serde_json::Value, String> {
    let request = raw.as_object().ok_or_else(|| "[exposure_reclassify_invalid] 请求必须是对象".to_string())?;
    let text = |key: &str| request.get(key).and_then(|value| value.as_str()).unwrap_or("").trim().to_string();
    let number = |key: &str| request.get(key).and_then(|value| value.as_i64());
    let operation_id = text("operationId");
    let disposition = text("disposition");
    let info_category = text("infoCategory");
    let actor = text("actor");
    // P1-5 复核理由里贴进来的口令/令牌只以脱敏形态参与幂等哈希与落库。
    let raw_reason = text("reason");
    let reason = super::redaction::redact_text(&raw_reason);
    let finding_id = number("findingId").ok_or_else(|| "[exposure_reclassify_invalid] findingId 必须是整数".to_string())?;
    let expected = number("expectedClassificationId").ok_or_else(|| "[exposure_reclassify_invalid] expectedClassificationId 必须是整数".to_string())?;
    if !super::imports::operation_id_valid(&operation_id) {
        return Err("[exposure_reclassify_invalid] operationId 必须是 8~80 位字母、数字、- 或 _".to_string());
    }
    if !DISPOSITIONS.contains(&disposition.as_str()) {
        return Err(format!("[exposure_reclassify_invalid] 处置只能是 {}", DISPOSITIONS.join("/")));
    }
    if !INFO_CATEGORIES.contains(&info_category.as_str()) {
        return Err("[exposure_reclassify_invalid] 信息类别不在已知集合里".to_string());
    }
    if !(1..=80).contains(&actor.chars().count()) {
        return Err("[exposure_reclassify_invalid] 操作人必填（不超过 80 字）".to_string());
    }
    if !(4..=500).contains(&reason.chars().count()) {
        return Err("[exposure_reclassify_invalid] 改判必须写明理由（4~500 字）".to_string());
    }
    let payload_hash = super::redaction::idempotency_hash(
        &serde_json::json!([finding_id, expected, disposition, info_category, actor, reason]).to_string(),
        &[&raw_reason],
    );
    let transaction = super::transaction::ImmediateTx::begin(connection)?;
    let tx = transaction.connection();
    let replay: Option<(String, String)> = tx
        .query_row(
            "SELECT payload_hash,result_json FROM exposure_reclassifications WHERE project_id=?1 AND operation_id=?2",
            params![project_id, operation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    if let Some((hash, result_json)) = replay {
        if hash != payload_hash {
            return Err(format!("[exposure_reclassify_operation_conflict] operationId {operation_id} 已用于另一份改判"));
        }
        let mut result: serde_json::Value = serde_json::from_str(&result_json).map_err(storage)?;
        if let Some(object) = result.as_object_mut() {
            object.insert("replayed".to_string(), serde_json::Value::Bool(true));
        }
        return Ok(result);
    }
    let finding_here: Option<i64> = tx
        .query_row(
            "SELECT id FROM exposure_findings WHERE id=?1 AND project_id=?2",
            params![finding_id, project_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?;
    if finding_here.is_none() {
        return Err(format!("[exposure_reclassify_not_found] 项目 {project_id} 没有线索 {finding_id}"));
    }
    let current: Option<i64> = tx
        .query_row(
            "SELECT classification_id FROM exposure_finding_dispositions WHERE finding_id=?1 AND project_id=?2",
            params![finding_id, project_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?;
    let Some(current) = current else {
        return Err("[exposure_reclassify_missing] 这条线索还没有自动分类，不能改判".to_string());
    };
    if current != expected {
        return Err(format!("[exposure_reclassify_conflict] 当前分类编号是 {current}，不是 {expected}；请刷新后再改"));
    }
    let policy = format!("manual:{actor}");
    let now = super::state::utc_now();
    tx.execute(
        "INSERT INTO exposure_classifications(project_id,finding_id,observation_id,run_id,policy_version,info_category,source_channel,disposition,reason,markers_json,created_at)
         VALUES(?1,?2,NULL,0,?3,?4,'manual',?5,?6,'[\"manual\"]',?7)",
        params![project_id, finding_id, policy, info_category, disposition, reason, now],
    )
    .map_err(storage)?;
    let classification_id = tx.last_insert_rowid();
    let updated = tx
        .execute(
            "UPDATE exposure_finding_dispositions
             SET disposition=?1,info_category=?2,source_channel='manual',reason=?3,policy_version=?4,classification_id=?5,conflict=0,conflict_note='',updated_at=?6
             WHERE finding_id=?7 AND project_id=?8 AND classification_id=?9",
            params![disposition, info_category, reason, policy, classification_id, now, finding_id, project_id, expected],
        )
        .map_err(storage)?;
    if updated != 1 {
        return Err("[exposure_reclassify_conflict] 分类在写入前变了，未写入任何结论".to_string());
    }
    let result = serde_json::json!({
        "findingId": finding_id,
        "classificationId": classification_id,
        "previousClassificationId": expected,
        "disposition": disposition,
        "infoCategory": info_category,
        "policyVersion": policy,
        "actor": actor,
        "replayed": false,
    });
    tx.execute(
        "INSERT INTO exposure_reclassifications(project_id,operation_id,payload_hash,finding_id,classification_id,actor,reason,disposition,info_category,result_json,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![project_id, operation_id, payload_hash, finding_id, classification_id, actor, reason, disposition, info_category, result.to_string(), now],
    )
    .map_err(storage)?;
    transaction.commit()?;
    Ok(result)
}

fn static_disposition(value: &str) -> Result<&'static str, String> {
    DISPOSITIONS
        .iter()
        .copied()
        .find(|known| *known == value)
        .ok_or_else(|| format!("exposure_classification_corrupt：处置 {value} 不在已知集合里"))
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DispositionView {
    pub disposition: String,
    pub info_category: String,
    pub source_channel: String,
    pub reason: String,
    pub policy_version: String,
    pub conflict: bool,
    pub conflict_note: String,
    pub classification_id: i64,
}

fn tables_present(connection: &Connection) -> Result<bool, String> {
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('exposure_classifications','exposure_finding_dispositions')",
            [],
            |row| row.get(0),
        )
        .map_err(storage)?;
    Ok(count == 2)
}

/// 线索 id → 当前处置。没有行的线索是 G1 之前的旧线索，页面显示“未分类（旧线索）”。
pub fn dispositions_for(
    connection: &Connection,
    project_id: i64,
    finding_ids: &[i64],
) -> Result<BTreeMap<i64, DispositionView>, String> {
    let mut map = BTreeMap::new();
    if finding_ids.is_empty() || !tables_present(connection)? {
        return Ok(map);
    }
    let mut statement = connection
        .prepare(
            "SELECT disposition,info_category,source_channel,reason,policy_version,conflict,conflict_note,classification_id
             FROM exposure_finding_dispositions WHERE finding_id=?1 AND project_id=?2",
        )
        .map_err(storage)?;
    for id in finding_ids {
        let row = statement
            .query_row(params![id, project_id], |row| {
                Ok(DispositionView {
                    disposition: row.get(0)?,
                    info_category: row.get(1)?,
                    source_channel: row.get(2)?,
                    reason: row.get(3)?,
                    policy_version: row.get(4)?,
                    conflict: row.get::<_, i64>(5)? != 0,
                    conflict_note: row.get(6)?,
                    classification_id: row.get(7)?,
                })
            })
            .optional()
            .map_err(storage)?;
        if let Some(view) = row {
            map.insert(*id, view);
        }
    }
    Ok(map)
}

pub fn disposition_of(
    connection: &Connection,
    project_id: i64,
    finding_id: i64,
) -> Result<Option<DispositionView>, String> {
    Ok(dispositions_for(connection, project_id, &[finding_id])?.remove(&finding_id))
}

/// 证据页用：这条线索每次观察的分类记录（新在前）。
pub fn classifications_of(
    connection: &Connection,
    project_id: i64,
    finding_id: i64,
) -> Result<Vec<serde_json::Value>, String> {
    if !tables_present(connection)? {
        return Ok(Vec::new());
    }
    let mut statement = connection
        .prepare(
            "SELECT observation_id,run_id,policy_version,info_category,source_channel,disposition,reason,markers_json,created_at
             FROM exposure_classifications WHERE finding_id=?1 AND project_id=?2 ORDER BY id DESC LIMIT 20",
        )
        .map_err(storage)?;
    let rows = statement
        .query_map(params![finding_id, project_id], |row| {
            Ok(serde_json::json!({
                "observationId": row.get::<_, Option<i64>>(0)?,
                "runId": row.get::<_, i64>(1)?,
                "policyVersion": row.get::<_, String>(2)?,
                "infoCategory": row.get::<_, String>(3)?,
                "sourceChannel": row.get::<_, String>(4)?,
                "disposition": row.get::<_, String>(5)?,
                "reason": row.get::<_, String>(6)?,
                "markers": crate::commands::json(row.get::<_, String>(7)?),
                "createdAt": row.get::<_, String>(8)?,
            }))
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(rows)
}
