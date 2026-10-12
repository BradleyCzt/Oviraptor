// §Q5 — the deterministic classifiers. Everything here is a statement about a pattern in
// bytes that were actually fetched: a source map is parsed, not guessed from a file suffix,
// and a credential sighting stays a candidate because the surface never tests a key.

use super::observations::FindingDraft;
use super::redaction::{credential_sightings, redact_text};
use serde_json::Value as JsonValue;

pub const KIND_RESOURCE: &str = "resource";
pub const KIND_SOURCE_MAP: &str = "source_map";
pub const KIND_CREDENTIAL: &str = "credential";
pub const KIND_REPOSITORY: &str = "repository";
pub const KIND_STORAGE: &str = "storage";
pub const KIND_DOCUMENT: &str = "document";
pub const KIND_TRANSPORT: &str = "transport";

// G1-3 classification keys off these exact titles, so they are named once and shared.
pub const TITLE_MAP_BUSINESS: &str = "公开 Source Map 还原业务源码";
pub const TITLE_MAP_VENDOR: &str = "Source Map 仅含第三方包路径";
pub const TITLE_MAP_EMPTY: &str = "Source Map 为空图";
pub const TITLE_MAP_NOT_A_MAP: &str = "同名 .map 资源不是可解析的 Source Map";
/// A placeholder credential (`password=changeme`) is graded with exactly this confidence.
pub const CREDENTIAL_PLACEHOLDER_CONFIDENCE: i64 = 25;

/// What a `.map` payload actually is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceMapKind {
    /// Parses as a v3 map and lists first-party sources.
    Business,
    /// Parses, but every source is a published package or runtime helper.
    ThirdPartyOnly,
    /// Parses with no source list, or all sources empty.
    Empty,
    /// Bytes are not a source map at all (an error page served as `.map` is common).
    NotAMap,
}

#[derive(Clone, Debug)]
pub struct SourceMapVerdict {
    pub kind: SourceMapKind,
    pub sources: usize,
    pub first_party: usize,
    pub inline_content: usize,
    pub sample: String,
    pub note: String,
}

fn vendor_marker(source: &str) -> bool {
    let lowered = source.to_ascii_lowercase();
    [
        "node_modules/",
        "webpack:///node_modules",
        "core-js",
        "regenerator-runtime",
        "react-dom",
        "react.production",
        "vue/dist",
        "vue.runtime",
        "angular",
        "moment/min",
        "lodash",
        "jquery",
        "polyfill",
        "caniuse-lite",
        "@babel/",
        "tslib",
        "zone.js",
        "echarts",
        "antd/dist",
        "element-ui/lib",
        "bootstrap/dist",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

/// Parses the map instead of trusting the path. `sourcesContent` is only counted, never
/// copied, because its contents are the most sensitive part of a real leak.
pub fn parse_source_map(bytes: &[u8]) -> SourceMapVerdict {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim_start();
    if !trimmed.starts_with('{') {
        return SourceMapVerdict {
            kind: SourceMapKind::NotAMap,
            sources: 0,
            first_party: 0,
            inline_content: 0,
            sample: String::new(),
            note: format!(
                "响应开头不是 JSON 对象：{}",
                redact_text(&trimmed.chars().take(60).collect::<String>())
            ),
        };
    }
    let Ok(value) = serde_json::from_str::<JsonValue>(text.as_ref()) else {
        return SourceMapVerdict {
            kind: SourceMapKind::NotAMap,
            sources: 0,
            first_party: 0,
            inline_content: 0,
            sample: String::new(),
            note: "JSON 解析失败，可能只是同名文件".to_string(),
        };
    };
    let version = value
        .get("version")
        .and_then(|row| {
            row.as_i64()
                .or_else(|| row.as_str().and_then(|text| text.parse().ok()))
        })
        .unwrap_or(0);
    let Some(sources) = value.get("sources").and_then(JsonValue::as_array) else {
        return SourceMapVerdict {
            kind: SourceMapKind::NotAMap,
            sources: 0,
            first_party: 0,
            inline_content: 0,
            sample: String::new(),
            note: format!("缺少 sources 字段（version={version}）"),
        };
    };
    let names: Vec<String> = sources
        .iter()
        .filter_map(|row| row.as_str().map(str::to_string))
        .collect();
    let first_party = names.iter().filter(|name| !vendor_marker(name)).count();
    let inline_content = value
        .get("sourcesContent")
        .and_then(JsonValue::as_array)
        .map(|rows| {
            rows.iter()
                .filter(|row| row.as_str().is_some_and(|text| !text.trim().is_empty()))
                .count()
        })
        .unwrap_or(0);
    let mappings_present = value
        .get("mappings")
        .and_then(JsonValue::as_str)
        .is_some_and(|text| !text.trim().is_empty());
    let kind = if names.is_empty() || first_party == 0 && !mappings_present {
        SourceMapKind::Empty
    } else if first_party == 0 {
        SourceMapKind::ThirdPartyOnly
    } else {
        SourceMapKind::Business
    };
    let sample = names
        .iter()
        .find(|name| !vendor_marker(name))
        .cloned()
        .or_else(|| names.first().cloned())
        .unwrap_or_default();
    SourceMapVerdict {
        kind,
        sources: names.len(),
        first_party,
        inline_content,
        sample: redact_text(&sample.chars().take(160).collect::<String>()),
        note: format!(
            "version={version}, mappings={}",
            if mappings_present { "有" } else { "无" }
        ),
    }
}

fn has_query(url: &str) -> bool {
    url.contains('?')
}

/// Resolves `href`/`src`/`sourceMappingURL` references. These are natural links a browser
/// would follow, not brute-forced paths.
pub fn natural_links(text: &str, base: &reqwest::Url) -> Vec<String> {
    let mut values = Vec::new();
    for marker in ["href=\"", "src=\"", "href='", "src='", "sourceMappingURL="] {
        let mut rest = text;
        while let Some(index) = rest.find(marker) {
            rest = &rest[index + marker.len()..];
            let end = if marker.ends_with('=') {
                rest.find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                    .unwrap_or(rest.len())
            } else {
                rest.find(if marker.ends_with('"') { '"' } else { '\'' })
                    .unwrap_or(rest.len())
            };
            let value = rest[..end].trim();
            rest = &rest[end.min(rest.len())..];
            if value.is_empty()
                || value.starts_with("data:")
                || value.starts_with("javascript:")
                || value.len() > 2000
            {
                continue;
            }
            let Ok(joined) = base.join(value) else {
                continue;
            };
            if !matches!(joined.scheme(), "http" | "https") {
                continue;
            }
            let url = joined.to_string();
            if !values.contains(&url) {
                values.push(url);
            }
            if values.len() >= 200 {
                return values;
            }
        }
    }
    values
}

pub fn resource_kind(url: &str, content_type: &str) -> &'static str {
    let lowered = url.to_ascii_lowercase();
    let ctype = content_type.to_ascii_lowercase();
    if lowered.ends_with(".map") || ctype.contains("application/json") && lowered.contains(".map") {
        return KIND_SOURCE_MAP;
    }
    if [".pdf", ".docx", ".doc", ".xlsx", ".xls", ".pptx", ".zip"]
        .iter()
        .any(|suffix| lowered.ends_with(suffix))
    {
        return KIND_DOCUMENT;
    }
    if repository_host(&lowered) {
        return KIND_REPOSITORY;
    }
    if storage_host(&lowered) {
        return KIND_STORAGE;
    }
    KIND_RESOURCE
}

pub fn repository_host(url: &str) -> bool {
    [
        "github.com",
        "gitlab.com",
        "gitlab.",
        "bitbucket.org",
        "gitee.com",
        "coding.net",
        "/.git/",
        "svn.",
    ]
    .iter()
    .any(|marker| url.to_ascii_lowercase().contains(marker))
}

pub fn storage_host(url: &str) -> bool {
    let lowered = url.to_ascii_lowercase();
    [
        ".s3.amazonaws.com",
        "s3.",
        ".oss-",
        ".myqcloud.com",
        "aliyuncs.com",
        "storage.googleapis.com",
        ".bos.",
        ".obs.",
        "pan.",
        "drive.",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
        && !lowered.contains("/api/")
}

/// A public path that merely *looks* internal is a lead, never a verdict.
pub fn document_suspect(url: &str) -> bool {
    let lowered = url.to_ascii_lowercase();
    let doc = [".pdf", ".docx", ".xlsx", ".pptx", ".zip"]
        .iter()
        .any(|suffix| lowered.ends_with(suffix));
    let sensitive = [
        "internal",
        "confidential",
        "private",
        "backup",
        "内部",
        "运维",
        "架构",
        "员工",
        "客户",
    ]
    .iter()
    .any(|word| lowered.contains(word));
    doc && sensitive
}

// One candidate row, assembled from the resource it was seen on. The eight inputs are the
// fields of the candidate itself; splitting them across calls would only repeat the mapping.
#[allow(clippy::too_many_arguments)]
fn draft(
    category: &str,
    title: &str,
    url: &str,
    source_type: &str,
    evidence: String,
    severity: &str,
    confidence: i64,
    asset_id: Option<i64>,
) -> FindingDraft {
    FindingDraft {
        category: category.to_string(),
        title: title.to_string(),
        url: url.to_string(),
        source_type: source_type.to_string(),
        evidence,
        severity: severity.to_string(),
        confidence,
        asset_id,
        content_hash: String::new(),
    }
}

/// Turns one fetched resource into candidates. The severity of a source map depends on what
/// the map actually contains, so an error page served as `.map` cannot out-rank a real one.
pub fn findings_for(
    url: &str,
    content_type: &str,
    body: &[u8],
    asset_id: Option<i64>,
) -> Vec<FindingDraft> {
    let mut rows = Vec::new();
    let lowered = url.to_ascii_lowercase();
    if lowered.ends_with(".map") {
        let verdict = parse_source_map(body);
        match verdict.kind {
            SourceMapKind::Business => rows.push(draft(
                "source_code",
                TITLE_MAP_BUSINESS,
                url,
                "passive_web",
                format!(
                    "解析到 {} 个源文件（第一方 {}/{}, 内联源码 {} 段），示例路径 {}；{}",
                    verdict.sources,
                    verdict.first_party,
                    verdict.sources,
                    verdict.inline_content,
                    verdict.sample,
                    verdict.note
                ),
                "high",
                if verdict.inline_content > 0 { 82 } else { 68 },
                asset_id,
            )),
            SourceMapKind::ThirdPartyOnly => rows.push(draft(
                "source_code",
                TITLE_MAP_VENDOR,
                url,
                "passive_web",
                format!(
                    "{} 个源文件全部命中公开包标记，示例 {}；{}",
                    verdict.sources, verdict.sample, verdict.note
                ),
                "low",
                35,
                asset_id,
            )),
            SourceMapKind::Empty => rows.push(draft(
                "source_code",
                TITLE_MAP_EMPTY,
                url,
                "passive_web",
                format!("可解析但没有第一方源文件；{}", verdict.note),
                "low",
                30,
                asset_id,
            )),
            SourceMapKind::NotAMap => rows.push(draft(
                "metadata",
                TITLE_MAP_NOT_A_MAP,
                url,
                "passive_web",
                format!("按内容判定为无效图；{}", verdict.note),
                "low",
                20,
                asset_id,
            )),
        }
    }
    if content_type.contains("html")
        || content_type.is_empty()
        || lowered.contains("config")
        || lowered.ends_with(".json")
        || lowered.ends_with(".map")
        || lowered.contains("backup")
    {
        let text = String::from_utf8_lossy(body).to_string();
        for sighting in credential_sightings(&text, 4000) {
            let (severity, confidence) = match sighting.grade {
                super::redaction::CredentialGrade::Candidate => ("high", 62),
                super::redaction::CredentialGrade::Placeholder => ("low", CREDENTIAL_PLACEHOLDER_CONFIDENCE),
                super::redaction::CredentialGrade::None => continue,
            };
            rows.push(draft(
                "credential",
                "公开资源出现凭据字段",
                url,
                "passive_web",
                format!(
                    "第 {} 行字段 {}，值已脱敏 {}；未做任何有效性验证",
                    sighting.line, sighting.key, sighting.masked
                ),
                severity,
                confidence,
                asset_id,
            ));
        }
    }
    let lower_all = url.to_ascii_lowercase();
    if repository_host(&lower_all) {
        rows.push(draft(
            "repository",
            "公开页面引用代码仓库地址",
            url,
            "passive_web",
            "仅记录外链引用，未拉取仓库内容，也未确认归属".to_string(),
            "medium",
            45,
            asset_id,
        ));
    }
    if storage_host(&lower_all) {
        rows.push(draft(
            "storage",
            "公开页面引用对象存储或网盘地址",
            url,
            "passive_web",
            "仅记录外链引用，未枚举目录内容".to_string(),
            "medium",
            45,
            asset_id,
        ));
    }
    if document_suspect(&lower_all) {
        rows.push(draft(
            "document",
            "公开可访问的疑似内部文档",
            url,
            "passive_web",
            format!(
                "文件名命中内部语义且为文档/归档类型{}",
                if has_query(url) {
                    "（带查询参数）"
                } else {
                    ""
                }
            ),
            "medium",
            55,
            asset_id,
        ));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(json: &str) -> Vec<u8> {
        json.as_bytes().to_vec()
    }

    #[test]
    fn source_map_kind_is_decided_by_the_parsed_body() {
        let business = parse_source_map(&map(
            r#"{"version":3,"sources":["src/api/payments.ts","src/main.ts"],"sourcesContent":["export const x=1;",""],"mappings":"AAAA"}"#,
        ));
        assert_eq!(business.kind, SourceMapKind::Business);
        assert_eq!(business.first_party, 2);
        assert_eq!(business.inline_content, 1);

        let vendor = parse_source_map(&map(
            r#"{"version":3,"sources":["node_modules/core-js/index.js"],"mappings":"AAAA"}"#,
        ));
        assert_eq!(vendor.kind, SourceMapKind::ThirdPartyOnly);

        let empty = parse_source_map(&map(r#"{"version":3,"sources":[],"mappings":""}"#));
        assert_eq!(empty.kind, SourceMapKind::Empty);

        let error_page = parse_source_map(&map("<html><title>404</title></html>"));
        assert_eq!(error_page.kind, SourceMapKind::NotAMap);

        let json_error = parse_source_map(&map(r#"{"error":"not found"}"#));
        assert_eq!(json_error.kind, SourceMapKind::NotAMap);
    }

    #[test]
    fn a_map_shaped_error_page_cannot_be_reported_as_a_leak() {
        let rows = findings_for(
            "https://x.invalid/app.js.map",
            "text/html",
            &map("<html>Not Found</html>"),
            None,
        );
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].category, "metadata");
        assert_eq!(rows[0].severity, "low");
        assert_eq!(rows[0].confidence, 20);
    }

    #[test]
    fn real_map_and_credentials_produce_separate_candidates() {
        let rows = findings_for(
            "https://x.invalid/main.a1b2.js.map",
            "application/json",
            &map(
                r#"{"version":3,"sources":["src/services/billing.ts"],"sourcesContent":["const key='zzzzzzzzzzzz'"],"mappings":"AAAA"}"#,
            ),
            Some(9),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].severity, "high");
        assert_eq!(rows[0].asset_id, Some(9));

        let html = "<p>api_key = ZZZZZZZZZZZZZZ9</p><p>api_key = YOUR_KEY_HERE_1</p>";
        let rows = findings_for(
            "https://x.invalid/status",
            "text/html",
            html.as_bytes(),
            None,
        );
        let kinds: Vec<&str> = rows.iter().map(|row| row.severity.as_str()).collect();
        assert_eq!(kinds, vec!["high", "low"], "模板值不能和候选同级：{rows:?}");
        assert!(
            !rows
                .iter()
                .any(|row| row.evidence.contains("ZZZZZZZZZZZZZZ9")),
            "证据必须已脱敏"
        );
    }

    #[test]
    fn natural_links_stay_resolvable_and_skip_data_urls() {
        let base = reqwest::Url::parse("https://example.com/app/index.html").unwrap();
        let text = r#"<a href="/docs/内部手册.pdf">x</a><img src='assets/a.png'><script src="https://other.invalid/x.js"></script><link href="data:image/png;base64,AA"><sourceMappingURL=app.js.map"#;
        let links = natural_links(text, &base);
        assert!(
            links.iter().any(
                |row| row.starts_with("https://example.com/docs/") && row.contains("%E5%86%85")
            ),
            "非 ASCII 路径必须被保留而不是丢弃：{links:?}"
        );
        assert!(links.contains(&"https://example.com/app/assets/a.png".to_string()));
        assert!(links.contains(&"https://other.invalid/x.js".to_string()));
        assert!(links.contains(&"https://example.com/app/app.js.map".to_string()));
        assert!(!links.iter().any(|row| row.starts_with("data:")));
    }

    #[test]
    fn reference_hosts_are_labelled_as_references_only() {
        assert!(repository_host("https://github.com/acme/payments"));
        assert!(storage_host("https://bucket.s3.amazonaws.com/back.zip"));
        assert!(!storage_host("https://api.example.com/v1/s3/status"));
        assert!(document_suspect("https://x.invalid/运维手册.pdf"));
        assert!(!document_suspect("https://x.invalid/pricing.pdf"));
        let rows = findings_for("https://github.com/acme/x", "text/html", b"", None);
        assert_eq!(
            rows[0].evidence,
            "仅记录外链引用，未拉取仓库内容，也未确认归属"
        );
    }
}
