// G2 — GitHub, the first third-party code platform. Only the official REST API is used, never
// page scraping. Repository metadata search runs anonymously; code search and the controlled
// blob read need a credential that only a secret-reference port may hand over (IR-04). Until
// that port exists the production credential lookup is `integration_pending`, code search is
// reported `unconfigured`, and not a single request is sent for it.
//
// What a hit means: a repository name or a code fragment that mentions a group name is a lead
// for a human. It never makes GitHub, the repository or its owner a group asset, and it never
// authorizes fetching a URL the API returned.

use super::super::observations::{FindingDraft, NewObservation};
use super::super::orchestrator::ItemOutcome;
use super::super::redaction::{content_hash, credential_sightings, evidence_excerpt, CredentialGrade};
use super::super::rules::CREDENTIAL_PLACEHOLDER_CONFIDENCE;
use super::super::state::{SourceStatus, WorkItemStatus};
use super::super::work_items::WorkItem;
use super::http::{FetchBlock, PassiveClient, PassiveFetch, ProviderAuthorization, RequestMeter};
use serde_json::{json, Value as JsonValue};
use std::collections::BTreeMap;

pub const ADAPTER_VERSION: &str = "oviraptor.exposure.github.v1";
/// The only API origin production talks to. Tests reach a localhost fixture through the
/// `#[cfg(test)]` endpoint override in `commands/exposure.rs`; there is no runtime setting.
pub const API_ORIGIN: &str = "https://api.github.com";
pub const WEB_ORIGIN: &str = "https://github.com";
pub const API_VERSION: &str = "2022-11-28";
pub const REPOSITORY_SEARCH_PATH: &str = "/search/repositories";
pub const CODE_SEARCH_PATH: &str = "/search/code";
pub const KIND_REPOSITORIES: &str = "github_repositories";
pub const KIND_CODE: &str = "github_code";

/// Plan bounds. They are frozen per adapter version and recorded on every page observation;
/// the tail beyond them is reported as unread, never as "searched everything".
pub const REPOSITORY_PER_PAGE: i64 = 30;
pub const REPOSITORY_MAX_PAGES: i64 = 2;
pub const CODE_PER_PAGE: i64 = 20;
pub const CODE_MAX_PAGES: i64 = 1;
pub const BLOB_READS_PER_QUERY: usize = 3;
pub const BLOB_MAX_BYTES: i64 = 256 * 1024;
/// GitHub search serves at most this many results per query, whatever `total_count` says.
pub const SEARCH_RESULT_WINDOW: i64 = 1_000;
/// No automatic retry in G2: a refused or failed page is reported with its own status and the
/// next round asks again. Rotating credentials to get around a limit is never done.
pub const AUTOMATIC_RETRIES: i64 = 0;

pub const TITLE_REPOSITORY: &str = "GitHub 仓库元数据命中";
pub const TITLE_OFFICIAL_REPOSITORY: &str = "GitHub 正式开源账号下的仓库";
pub const TITLE_CODE_HIT: &str = "GitHub 代码搜索命中";
pub const TITLE_CODE_CREDENTIAL: &str = "GitHub 代码命中含凭据形态值";

pub const CAPABILITY_REPOSITORY_METADATA: &str = "repository_metadata";
pub const CAPABILITY_CODE_SEARCH: &str = "default_branch_code_search";
pub const CAPABILITY_BLOB_READ: &str = "controlled_blob_read";

const ACCEPT_JSON: &str = "application/vnd.github+json";
const ACCEPT_TEXT_MATCH: &str = "application/vnd.github.text-match+json";

// ───────────────────────────── credential port ─────────────────────────────

/// A GitHub token. No `Display`, no `Serialize`; `Debug` prints nothing of it. It only ever
/// leaves this type as an origin-bound `ProviderAuthorization`.
pub struct GithubToken(String);

impl GithubToken {
    pub fn new(raw: &str) -> Option<Self> {
        let token = raw.trim();
        let shaped = (8..=255).contains(&token.len())
            && token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
        shaped.then(|| Self(token.to_string()))
    }

    pub fn authorization_for(&self, api_origin: &str) -> ProviderAuthorization {
        ProviderAuthorization::bearer(api_origin, &self.0)
    }
}

impl std::fmt::Debug for GithubToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("GithubToken(<redacted>)")
    }
}

/// What the credential port answered for one capability.
#[derive(Debug)]
pub enum CredentialLookup {
    Unconfigured { code: &'static str, reason: String },
    Available(GithubToken),
}

const KEYCHAIN_SERVICE: &str = "oviraptor.exposure.github";

pub fn credential_ref_valid(value: &str) -> bool {
    (1..=80).contains(&value.len())
        && value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

/// Reads one token from the system keychain. The value is returned only to the caller and is
/// never written to a log, a database, or a settings document. A missing item is `Ok(None)`.
fn read_keychain(account: &str) -> Result<Option<String>, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = account;
        return Ok(None);
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("security")
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", account, "-w"])
            .output()
            .map_err(|_| "无法启动 security".to_string())?;
        if !output.status.success() {
            return Ok(None);
        }
        let token = String::from_utf8(output.stdout).map_err(|_| "钥匙串返回不是 UTF-8".to_string())?;
        let token = token.trim().to_string();
        if token.is_empty() {
            return Ok(None);
        }
        Ok(Some(token))
    }
}

impl CredentialLookup {
    /// Production with no project reference: nothing is read from the keychain.
    pub fn production() -> Self {
        Self::from_ref("")
    }

    /// IR-04. `reference` is a keychain account name, never a token. Empty means unconfigured
    /// and does not touch the keychain.
    pub fn from_ref(reference: &str) -> Self {
        Self::from_ref_with(reference, read_keychain)
    }

    pub fn from_ref_with<F>(reference: &str, read: F) -> Self
    where
        F: FnOnce(&str) -> Result<Option<String>, String>,
    {
        let reference = reference.trim();
        if reference.is_empty() {
            return Self::Unconfigured {
                code: "unconfigured",
                reason: "未填写凭据引用（钥匙串账户名）。token 不进配置、日志或数据库；未读取钥匙串，未发送请求".to_string(),
            };
        }
        if !credential_ref_valid(reference) {
            return Self::Unconfigured {
                code: "unconfigured",
                reason: "凭据引用名不合规（只允许字母、数字、. _ : -，最长 80）。未读取钥匙串，未发送请求".to_string(),
            };
        }
        match read(reference) {
            Ok(None) => Self::Unconfigured {
                code: "unconfigured",
                reason: format!("钥匙串没有引用 {reference} 的项（服务 {KEYCHAIN_SERVICE}）。未发送请求"),
            },
            Ok(Some(secret)) => match GithubToken::new(&secret) {
                Some(token) => Self::Available(token),
                None => Self::Unconfigured {
                    code: "unconfigured",
                    reason: "钥匙串中的值不是合法 token 形态，已丢弃，不写入日志。未发送请求".to_string(),
                },
            },
            Err(error) => Self::Unconfigured {
                code: "unconfigured",
                reason: format!("钥匙串不可用（{error}）。未发送请求"),
            },
        }
    }

    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }
}

/// Where the adapter sends requests. Production is `API_ORIGIN` with private destinations off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubEndpoint {
    pub api_origin: String,
    pub allow_private: bool,
}

impl GithubEndpoint {
    pub fn production() -> Self {
        Self {
            api_origin: API_ORIGIN.to_string(),
            allow_private: false,
        }
    }
}

/// Provider quota seen during one round. Once GitHub says a resource is exhausted, later items
/// of the same capability are closed as rate-limited without sending anything.
#[derive(Debug, Default)]
pub struct GithubSession {
    pub exhausted: BTreeMap<&'static str, String>,
    pub last_rate: Option<RateLimit>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RateLimit {
    pub limit: Option<i64>,
    pub remaining: Option<i64>,
    pub reset: Option<i64>,
    pub resource: String,
}

impl RateLimit {
    pub fn of(fetch: &PassiveFetch) -> Self {
        let number = |name: &str| fetch.provider_header(name).and_then(|v| v.trim().parse::<i64>().ok());
        Self {
            limit: number("x-ratelimit-limit"),
            remaining: number("x-ratelimit-remaining"),
            reset: number("x-ratelimit-reset"),
            resource: fetch.provider_header("x-ratelimit-resource").unwrap_or_default().to_string(),
        }
    }

    pub fn as_json(&self) -> JsonValue {
        json!({"limit": self.limit, "remaining": self.remaining, "reset": self.reset, "resource": self.resource})
    }

    fn reset_text(&self) -> String {
        self.reset
            .and_then(|epoch| chrono::DateTime::from_timestamp(epoch, 0))
            .map(|at| at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "未知".to_string())
    }
}

// ───────────────────────────── queries and pages ─────────────────────────────

/// Repository search for one confirmed name (root domain or legal name). Quotes are stripped so
/// a name cannot inject qualifiers.
pub fn repository_query(entity: &str) -> String {
    format!("\"{}\" in:name,description,readme", entity.replace('"', " ").trim())
}

pub fn code_query(entity: &str) -> String {
    format!("\"{}\"", entity.replace('"', " ").trim())
}

pub fn search_url(api_origin: &str, path: &str, query: &str, per_page: i64, page: i64) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(&format!("{}{path}", api_origin.trim_end_matches('/')))
        .map_err(|error| format!("GitHub 地址无效：{error}"))?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("per_page", &per_page.to_string())
        .append_pair("page", &page.to_string());
    Ok(url)
}

/// The `rel="next"` target of a Link header, if any.
pub fn link_next(header: &str) -> Option<String> {
    header.split(',').find_map(|part| {
        let mut pieces = part.split(';');
        let target = pieces.next()?.trim();
        let is_next = pieces.any(|p| p.trim().eq_ignore_ascii_case("rel=\"next\""));
        (is_next && target.starts_with('<') && target.ends_with('>'))
            .then(|| target[1..target.len() - 1].to_string())
    })
}

/// A Link target is consumed only if it is the same API origin, the same search path, the same
/// query and exactly the next page. Anything else is a provider URL we were not asked to visit.
pub fn accept_next(next: &str, api_origin: &str, path: &str, query: &str, expected_page: i64) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(next).map_err(|_| "Link 下一页地址无法解析".to_string())?;
    if !url.origin().ascii_serialization().eq_ignore_ascii_case(api_origin.trim_end_matches('/')) {
        return Err(format!("Link 下一页指向 {}，不是批准的 API 端点", url.origin().ascii_serialization()));
    }
    if url.path() != path || !url.username().is_empty() || url.password().is_some() {
        return Err("Link 下一页的路径不是批准的搜索接口".to_string());
    }
    let pairs: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
    if pairs.get("q").map(String::as_str) != Some(query) {
        return Err("Link 下一页换了查询词".to_string());
    }
    if pairs.get("page").and_then(|p| p.parse::<i64>().ok()) != Some(expected_page) {
        return Err(format!("Link 下一页不是第 {expected_page} 页"));
    }
    Ok(url)
}

/// One refused or failed provider answer, already split into the status the page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderFailure {
    pub source_status: SourceStatus,
    pub item_status: WorkItemStatus,
    pub code: &'static str,
    pub message: String,
}

fn provider_message(fetch: &PassiveFetch) -> String {
    serde_json::from_slice::<JsonValue>(&fetch.body)
        .ok()
        .and_then(|value| value.get("message").and_then(JsonValue::as_str).map(str::to_string))
        .map(|text| evidence_excerpt(&text, 160))
        .unwrap_or_default()
}

/// 401/403/422/429/503 and redirects each get their own code; empty results are not here.
pub fn provider_failure(fetch: &PassiveFetch) -> Option<ProviderFailure> {
    let status = fetch.status;
    let rate = RateLimit::of(fetch);
    let said = provider_message(fetch);
    let said = if said.is_empty() { String::new() } else { format!("（GitHub：{said}）") };
    let build = |source_status, item_status, code, message: String| {
        Some(ProviderFailure { source_status, item_status, code, message })
    };
    match status {
        200 => None,
        300..=399 => {
            let target = fetch
                .provider_header("location")
                .and_then(|raw| reqwest::Url::parse(raw).ok())
                .map(|url| url.origin().ascii_serialization())
                .unwrap_or_else(|| "未知地址".to_string());
            build(
                SourceStatus::Failed,
                WorkItemStatus::Failed,
                "github_redirect_not_followed",
                format!("GitHub 返回 {status} 跳转到 {target}；接口跳转一律不跟随，凭据不会随跳转发出，记为缺口"),
            )
        }
        401 => build(
            SourceStatus::AuthFailed,
            WorkItemStatus::Failed,
            "github_auth_failed_401",
            format!("GitHub 401：身份无效或已过期{said}"),
        ),
        403 => {
            let retry_after = fetch.provider_header("retry-after").map(str::to_string);
            let lowered = String::from_utf8_lossy(&fetch.body).to_ascii_lowercase();
            if rate.remaining == Some(0) || retry_after.is_some() || lowered.contains("rate limit") {
                let when = match retry_after {
                    Some(seconds) => format!("Retry-After {seconds} 秒（二级限流）"),
                    None => format!("额度重置于 {}", rate.reset_text()),
                };
                build(
                    SourceStatus::RateLimited,
                    WorkItemStatus::BlockedQuota,
                    "github_rate_limited_403",
                    format!("GitHub 403：额度已用尽，{when}{said}"),
                )
            } else {
                build(
                    SourceStatus::AuthFailed,
                    WorkItemStatus::Failed,
                    "github_forbidden_403",
                    format!("GitHub 403：无权访问该资源{said}"),
                )
            }
        }
        422 => build(
            SourceStatus::Failed,
            WorkItemStatus::Failed,
            "github_query_rejected_422",
            format!("GitHub 422：查询被拒绝（语法、长度或范围不被接受）{said}"),
        ),
        429 => build(
            SourceStatus::RateLimited,
            WorkItemStatus::BlockedQuota,
            "github_rate_limited_429",
            format!(
                "GitHub 429：请求过多，{}{said}",
                fetch
                    .provider_header("retry-after")
                    .map(|s| format!("Retry-After {s} 秒"))
                    .unwrap_or_else(|| format!("额度重置于 {}", rate.reset_text()))
            ),
        ),
        503 => build(
            SourceStatus::Failed,
            WorkItemStatus::Failed,
            "github_unavailable_503",
            format!("GitHub 503：服务暂不可用，本轮不自动重试{said}"),
        ),
        other => build(
            SourceStatus::Failed,
            WorkItemStatus::Failed,
            "github_http_error",
            format!("GitHub 返回 HTTP {other}{said}"),
        ),
    }
}

// ───────────────────────────── parsing ─────────────────────────────

fn repo_name_valid(name: &str) -> bool {
    !name.is_empty() && name.len() <= 100 && name != "." && name != ".."
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// `owner/repo`, both parts checked; anything else is not used to build a URL.
pub fn valid_full_name(full_name: &str) -> Option<(String, String)> {
    let (owner, repo) = full_name.split_once('/')?;
    (super::super::settings::github_login_valid(&owner.to_ascii_lowercase()) && repo_name_valid(repo))
        .then(|| (owner.to_string(), repo.to_string()))
}

#[derive(Clone, Debug, PartialEq)]
pub struct RepositoryHit {
    pub id: i64,
    pub full_name: String,
    pub owner: String,
    pub canonical_url: String,
    pub provider_url_mismatch: bool,
    pub description: String,
    pub topics: Vec<String>,
    pub fork: bool,
    pub archived: bool,
    pub stars: i64,
    pub pushed_at: String,
    pub default_branch: String,
    pub language: String,
}

#[derive(Debug, Default)]
pub struct SearchPage {
    pub total_count: i64,
    pub incomplete_results: bool,
    pub items: Vec<JsonValue>,
}

pub fn search_page(body: &[u8]) -> Option<SearchPage> {
    let value: JsonValue = serde_json::from_slice(body).ok()?;
    let items = value.get("items")?.as_array()?.clone();
    Some(SearchPage {
        total_count: value.get("total_count").and_then(JsonValue::as_i64).unwrap_or(items.len() as i64),
        incomplete_results: value.get("incomplete_results").and_then(JsonValue::as_bool).unwrap_or(false),
        items,
    })
}

/// `None` for a malformed row or a private repository: only public resources are taken, even if
/// a credential could see more.
pub fn repository_hit(row: &JsonValue) -> Option<RepositoryHit> {
    if row.get("private").and_then(JsonValue::as_bool).unwrap_or(false)
        || row.get("visibility").and_then(JsonValue::as_str).is_some_and(|v| v != "public")
    {
        return None;
    }
    let id = row.get("id")?.as_i64()?;
    let full_name = row.get("full_name")?.as_str()?.to_string();
    let (owner, _) = valid_full_name(&full_name)?;
    let canonical_url = format!("{WEB_ORIGIN}/{full_name}");
    let html_url = row.get("html_url").and_then(JsonValue::as_str).unwrap_or_default();
    let text = |key: &str| row.get(key).and_then(JsonValue::as_str).unwrap_or_default().to_string();
    Some(RepositoryHit {
        id,
        owner: owner.to_ascii_lowercase(),
        provider_url_mismatch: !html_url.eq_ignore_ascii_case(&canonical_url),
        canonical_url,
        full_name,
        description: text("description"),
        topics: row
            .get("topics")
            .and_then(JsonValue::as_array)
            .map(|rows| rows.iter().filter_map(JsonValue::as_str).take(20).map(str::to_string).collect())
            .unwrap_or_default(),
        fork: row.get("fork").and_then(JsonValue::as_bool).unwrap_or(false),
        archived: row.get("archived").and_then(JsonValue::as_bool).unwrap_or(false),
        stars: row.get("stargazers_count").and_then(JsonValue::as_i64).unwrap_or(0),
        pushed_at: text("pushed_at"),
        default_branch: text("default_branch"),
        language: text("language"),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodeHit {
    pub repository_id: i64,
    pub full_name: String,
    pub path: String,
    pub blob_sha: String,
    pub canonical_url: String,
    pub provider_url_mismatch: bool,
    pub fragments: Vec<String>,
}

fn sha_valid(sha: &str) -> bool {
    sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn code_hit(row: &JsonValue) -> Option<CodeHit> {
    let repository = row.get("repository")?;
    if repository.get("private").and_then(JsonValue::as_bool).unwrap_or(false) {
        return None;
    }
    let full_name = repository.get("full_name")?.as_str()?.to_string();
    valid_full_name(&full_name)?;
    let path = row.get("path")?.as_str()?.to_string();
    if path.is_empty() || path.len() > 1024 || path.split('/').any(|p| p == ".." || p.is_empty()) {
        return None;
    }
    let blob_sha = row.get("sha")?.as_str()?.to_ascii_lowercase();
    if !sha_valid(&blob_sha) {
        return None;
    }
    let html_url = row.get("html_url").and_then(JsonValue::as_str).unwrap_or_default();
    let prefix = format!("{WEB_ORIGIN}/{full_name}/blob/");
    let trusted = html_url.starts_with(&prefix) && html_url.ends_with(&format!("/{path}"));
    let canonical_url = if trusted {
        html_url.to_string()
    } else {
        format!("{WEB_ORIGIN}/{full_name}/blob/HEAD/{path}")
    };
    Some(CodeHit {
        repository_id: repository.get("id").and_then(JsonValue::as_i64).unwrap_or(0),
        full_name,
        path,
        blob_sha,
        canonical_url,
        provider_url_mismatch: !trusted,
        fragments: row
            .get("text_matches")
            .and_then(JsonValue::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|m| m.get("fragment").and_then(JsonValue::as_str))
                    .take(5)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

// ───────────────────────────── shared page loop ─────────────────────────────

struct Walk<'a> {
    client: &'a PassiveClient,
    item: &'a WorkItem,
    meter: &'a mut RequestMeter,
    endpoint: &'a GithubEndpoint,
    session: &'a mut GithubSession,
    authorization: Option<&'a ProviderAuthorization>,
    cancelled: &'a dyn Fn() -> bool,
    capability: &'static str,
}

/// What the page loop ended with.
#[derive(Debug, Default)]
struct WalkResult {
    rows: Vec<(i64, JsonValue)>,
    pages_read: i64,
    total_count: i64,
    incomplete: bool,
    unread_tail: i64,
    stop: Option<(SourceStatus, WorkItemStatus, String)>,
}

fn scrub(authorization: Option<&ProviderAuthorization>, text: &str) -> String {
    match authorization {
        Some(secret) => secret.scrub(text),
        None => text.to_string(),
    }
}

fn block_stop(block: &FetchBlock, authorization: Option<&ProviderAuthorization>) -> (SourceStatus, WorkItemStatus, String) {
    let reason = scrub(authorization, &format!("{}:{}", block.code(), block.message()));
    match block {
        FetchBlock::Cancelled => (SourceStatus::Cancelled, WorkItemStatus::Cancelled, "用户取消".to_string()),
        _ if matches!(block.code(), "request_budget_exhausted" | "byte_budget_exhausted") => {
            (SourceStatus::Partial, WorkItemStatus::BlockedQuota, reason)
        }
        _ => (SourceStatus::Failed, WorkItemStatus::Failed, reason),
    }
}

fn page_observation(walk: &Walk, url: &str, fetch: &PassiveFetch, query: &str, page: i64, detail: JsonValue) -> NewObservation {
    let rate = RateLimit::of(fetch);
    NewObservation {
        status_code: i64::from(fetch.status),
        content_type: fetch.content_type.clone(),
        body_bytes: fetch.body.len() as i64,
        truncated: fetch.truncated,
        boundary: fetch.boundary.clone(),
        final_url: url.to_string(),
        extra: json!({
            "adapterVersion": ADAPTER_VERSION,
            "capability": walk.capability,
            "query": query,
            "entity": walk.item.entity,
            "page": page,
            "rateLimit": rate.as_json(),
            "credentialUsed": walk.authorization.is_some(),
            "detail": detail,
        }),
        ..NewObservation::new(&walk.item.source_key, "github_search_page", url)
    }
}

fn walk_search(walk: &mut Walk, outcome: &mut ItemOutcome, path: &str, query: &str, per_page: i64, max_pages: i64, accept: &str) -> WalkResult {
    let mut result = WalkResult::default();
    let mut next = match search_url(&walk.endpoint.api_origin, path, query, per_page, 1) {
        Ok(url) => Some(url),
        Err(error) => {
            result.stop = Some((SourceStatus::Failed, WorkItemStatus::Failed, error));
            return result;
        }
    };
    let origins = vec![walk.endpoint.api_origin.clone()];
    let headers = [("Accept", accept), ("X-GitHub-Api-Version", API_VERSION)];
    let mut page = 1i64;
    while let Some(url) = next.take() {
        if let Some(reason) = walk.session.exhausted.get(walk.capability) {
            result.stop = Some((
                SourceStatus::RateLimited,
                WorkItemStatus::BlockedQuota,
                format!("GitHub 额度已在本轮用尽，未再发送请求：{reason}"),
            ));
            break;
        }
        let fetch = match walk.client.fetch_provider_api(
            &url,
            &origins,
            &headers,
            walk.authorization,
            walk.meter,
            walk.endpoint.allow_private,
            walk.cancelled,
        ) {
            Ok(fetch) => fetch,
            Err(block) => {
                result.stop = Some(block_stop(&block, walk.authorization));
                break;
            }
        };
        let rate = RateLimit::of(&fetch);
        walk.session.last_rate = Some(rate.clone());
        let requested = url.to_string();
        if let Some(failure) = provider_failure(&fetch) {
            let message = scrub(walk.authorization, &failure.message);
            outcome.observations.push(page_observation(
                walk,
                &requested,
                &fetch,
                query,
                page,
                json!({"code": failure.code, "message": message}),
            ));
            if failure.source_status == SourceStatus::RateLimited {
                walk.session.exhausted.insert(walk.capability, message.clone());
            }
            // FIX4 P2：末页之前被拒（403/429 等）时，已读页保留，并写明未读尾部（截断）。
            if result.pages_read > 0 {
                result.unread_tail = (result.total_count.min(SEARCH_RESULT_WINDOW) - result.rows.len() as i64).max(0);
            }
            let tail = if result.unread_tail > 0 { format!("；未读尾部 {} 条（截断）", result.unread_tail) } else { String::new() };
            result.stop = Some((failure.source_status, failure.item_status, format!("{}:{message}{tail}", failure.code)));
            break;
        }
        let Some(parsed) = search_page(&fetch.body) else {
            outcome.observations.push(page_observation(
                walk,
                &requested,
                &fetch,
                query,
                page,
                json!({"code": "github_parse_error", "truncated": fetch.truncated}),
            ));
            result.stop = Some((
                SourceStatus::ParseError,
                WorkItemStatus::Failed,
                if fetch.truncated {
                    "github_parse_error:响应被单响应上限截断，无法解析".to_string()
                } else {
                    "github_parse_error:响应不是搜索结果 JSON".to_string()
                },
            ));
            break;
        };
        result.pages_read += 1;
        result.total_count = parsed.total_count.max(0);
        result.incomplete |= parsed.incomplete_results;
        let on_page = parsed.items.len() as i64;
        let link = fetch.provider_header("link").and_then(link_next);
        outcome.observations.push(page_observation(
            walk,
            &requested,
            &fetch,
            query,
            page,
            json!({
                "itemsOnPage": on_page,
                "totalCount": parsed.total_count,
                "incompleteResults": parsed.incomplete_results,
                "nextLink": link.is_some(),
                "perPage": per_page,
                "maxPages": max_pages,
            }),
        ));
        result.rows.extend(parsed.items.into_iter().map(|row| (page, row)));
        let window = result.total_count.min(SEARCH_RESULT_WINDOW);
        let seen = result.rows.len() as i64;
        // FIX4 P2：任何成功页报 remaining=0，都记下本能力额度已尽（带重置时间），之后零请求；
        // 本页结果照常保留，这一页若恰是末页，本项仍按完整结束，不标截断。
        if rate.remaining == Some(0) {
            walk.session.exhausted.insert(walk.capability, format!("额度剩余 0，重置于 {}", rate.reset_text()));
        }
        if on_page < per_page || seen >= window {
            break;
        }
        if page >= max_pages {
            result.unread_tail = (window - seen).max(0);
            break;
        }
        if rate.remaining == Some(0) {
            let reason = format!("额度剩余 0，重置于 {}", rate.reset_text());
            walk.session.exhausted.insert(walk.capability, reason.clone());
            result.unread_tail = (window - seen).max(0);
            result.stop = Some((
                SourceStatus::RateLimited,
                WorkItemStatus::BlockedQuota,
                format!("github_rate_limited_remaining_0:{reason}；已读 {} 页，未读尾部 {} 条", result.pages_read, result.unread_tail),
            ));
            break;
        }
        let Some(target) = link else {
            // GitHub says there is more but gives no next page: the tail is unknown, not empty.
            result.unread_tail = (window - seen).max(0);
            break;
        };
        match accept_next(&target, &walk.endpoint.api_origin, path, query, page + 1) {
            Ok(url) => {
                next = Some(url);
                page += 1;
            }
            Err(reason) => {
                result.unread_tail = (window - seen).max(0);
                result.stop = Some((
                    SourceStatus::Partial,
                    WorkItemStatus::Completed,
                    format!("github_link_rejected:{reason}，未访问；未读尾部 {} 条", result.unread_tail),
                ));
            }
        }
    }
    result
}

/// Closes the item from the page loop: failure first, then empty, then partial, then complete.
fn conclude(outcome: &mut ItemOutcome, result: &WalkResult, collected: usize, noun: &str) {
    let window = result.total_count.min(SEARCH_RESULT_WINDOW);
    let read = format!("已读 {} 页，记录 {collected} 个{noun}", result.pages_read);
    if let Some((source, item, reason)) = &result.stop {
        let kept = if result.pages_read > 0 { format!("；{read}（已保存）") } else { String::new() };
        outcome.status = Some(*item);
        outcome.reason = format!("{reason}{kept}");
        outcome.source_status = Some(*source);
        outcome.source_reason = outcome.reason.clone();
        return;
    }
    let mut gaps = Vec::new();
    if result.incomplete {
        gaps.push("GitHub 标记 incomplete_results（搜索超时），结果不完整".to_string());
    }
    if result.total_count > SEARCH_RESULT_WINDOW {
        gaps.push(format!(
            "GitHub 报告 {} 条，超出接口结果窗口 {SEARCH_RESULT_WINDOW} 条，不能宣称全平台检索完毕",
            result.total_count
        ));
    }
    if result.unread_tail > 0 {
        gaps.push(format!("按计划只读 {} 页，未读尾部 {} 条", result.pages_read, result.unread_tail));
    }
    if result.total_count == 0 && gaps.is_empty() {
        outcome.status = Some(WorkItemStatus::Completed);
        outcome.reason = "查询已执行，当前有界结果未命中（不代表没有泄漏）".to_string();
        outcome.source_status = Some(SourceStatus::CompletedNoMatches);
        outcome.source_reason = outcome.reason.clone();
    } else if gaps.is_empty() {
        outcome.status = Some(WorkItemStatus::Completed);
        outcome.reason = format!("共 {} 条（窗口 {window}），{read}；只代表 GitHub 搜索接口的有界结果", result.total_count);
        outcome.source_status = Some(SourceStatus::Completed);
        outcome.source_reason = String::new();
    } else {
        outcome.status = Some(if result.unread_tail > 0 { WorkItemStatus::BlockedQuota } else { WorkItemStatus::Completed });
        outcome.reason = format!("{}；{read}", gaps.join("；"));
        outcome.source_status = Some(SourceStatus::Partial);
        outcome.source_reason = outcome.reason.clone();
    }
}

fn unconfigured(outcome: &mut ItemOutcome, reason: String) -> ItemOutcome {
    outcome.status = Some(WorkItemStatus::SkippedUnconfigured);
    outcome.reason = reason.clone();
    outcome.source_status = Some(SourceStatus::Unconfigured);
    outcome.source_reason = reason;
    std::mem::take(outcome)
}

// ───────────────────────────── repository metadata ─────────────────────────────

/// `repository_metadata`: anonymous repository search for one confirmed name.
#[allow(clippy::too_many_arguments)]
pub fn execute_repositories(
    client: &PassiveClient,
    item: &WorkItem,
    meter: &mut RequestMeter,
    endpoint: &GithubEndpoint,
    session: &mut GithubSession,
    official_owners: &[String],
    cancelled: &dyn Fn() -> bool,
) -> ItemOutcome {
    let mut outcome = ItemOutcome::default();
    if item.entity.trim().is_empty() {
        return unconfigured(&mut outcome, "没有已确认的根域名或实体全称，GitHub 仓库搜索未配置".to_string());
    }
    let query = repository_query(&item.entity);
    let mut walk = Walk {
        client,
        item,
        meter,
        endpoint,
        session,
        authorization: None,
        cancelled,
        capability: CAPABILITY_REPOSITORY_METADATA,
    };
    let result = walk_search(&mut walk, &mut outcome, REPOSITORY_SEARCH_PATH, &query, REPOSITORY_PER_PAGE, REPOSITORY_MAX_PAGES, ACCEPT_JSON);
    let mut seen_ids: Vec<i64> = Vec::new();
    let mut skipped_private = 0usize;
    let mut malformed: Vec<String> = Vec::new();
    for (page, row) in &result.rows {
        let Some(hit) = repository_hit(row) else {
            if row.get("private").and_then(JsonValue::as_bool).unwrap_or(false)
                || row.get("visibility").and_then(JsonValue::as_str).is_some_and(|v| v != "public")
            {
                skipped_private += 1;
            } else {
                malformed.push(evidence_excerpt(
                    row.get("full_name").and_then(JsonValue::as_str).unwrap_or("<无 full_name>"),
                    120,
                ));
            }
            continue;
        };
        if seen_ids.contains(&hit.id) {
            continue;
        }
        seen_ids.push(hit.id);
        let official = official_owners.iter().any(|owner| owner == &hit.owner);
        let description = evidence_excerpt(&hit.description, 300);
        let flags = [(hit.fork, "fork"), (hit.archived, "已归档")]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, label)| *label)
            .collect::<Vec<_>>()
            .join("、");
        let evidence = format!(
            "仓库 {}{}；描述：{}；查询 {query}；只有仓库元数据，未读取代码，归属未证实",
            hit.full_name,
            if flags.is_empty() { String::new() } else { format!("（{flags}）") },
            if description.is_empty() { "（无）".to_string() } else { description.clone() },
        );
        // The page context of a repository is its own words: name, description and topics.
        let text = format!("{}\n{}\n{}", hit.full_name, hit.description, hit.topics.join(" "));
        outcome.observations.push(NewObservation {
            status_code: 200,
            content_type: "text/plain".to_string(),
            body_bytes: text.len() as i64,
            body: text.into_bytes(),
            indexed_at: hit.pushed_at.clone(),
            extra: json!({
                "adapterVersion": ADAPTER_VERSION,
                "capability": CAPABILITY_REPOSITORY_METADATA,
                "collectionMode": "repository_metadata",
                "providerObjectId": hit.id,
                "fullName": hit.full_name,
                "owner": hit.owner,
                "description": description,
                "topics": hit.topics,
                "fork": hit.fork,
                "archived": hit.archived,
                "stars": hit.stars,
                "pushedAt": hit.pushed_at,
                "defaultBranch": hit.default_branch,
                "language": hit.language,
                "query": query,
                "entities": [item.entity],
                "page": page,
                "providerUrlMismatch": hit.provider_url_mismatch,
                "officialOwner": official,
                "bodyRead": false,
            }),
            ..NewObservation::new(&item.source_key, "github_repository", &hit.canonical_url)
        });
        outcome.findings.push(FindingDraft {
            category: "repository".to_string(),
            title: if official { TITLE_OFFICIAL_REPOSITORY } else { TITLE_REPOSITORY }.to_string(),
            url: hit.canonical_url.clone(),
            source_type: item.source_key.clone(),
            evidence,
            severity: "low".to_string(),
            confidence: 30,
            asset_id: None,
            content_hash: content_hash(format!("{}|{}|{}", hit.id, hit.pushed_at, hit.description).as_bytes()),
        });
        outcome.observed += 1;
    }
    conclude(&mut outcome, &result, seen_ids.len(), "仓库");
    if skipped_private > 0 {
        outcome.reason = format!("{}；跳过 {skipped_private} 条非公开仓库", outcome.reason);
    }
    if !malformed.is_empty() {
        // Named, so a validation rule that is too strict shows up as a visible gap.
        outcome.reason = format!(
            "{}；跳过 {} 条名称不合规的结果（{}）",
            outcome.reason,
            malformed.len(),
            malformed.join("、")
        );
    }
    outcome
}

// ───────────────────────────── code search + controlled read ─────────────────────────────

pub struct BlobRead {
    pub status: u16,
    pub verified: bool,
    pub content: Option<String>,
    pub note: String,
    pub bytes: i64,
}

/// Reads one blob by the identifiers the search returned, rebuilt from validated parts. The
/// answer is used only if its `sha` is the one requested; an oversized or truncated blob is
/// recorded as "not read in full", never analysed as if complete.
fn read_blob(walk: &mut Walk, hit: &CodeHit, outcome: &mut ItemOutcome) -> Result<BlobRead, (SourceStatus, WorkItemStatus, String)> {
    let url = reqwest::Url::parse(&format!(
        "{}/repos/{}/git/blobs/{}",
        walk.endpoint.api_origin.trim_end_matches('/'),
        hit.full_name,
        hit.blob_sha
    ))
    .map_err(|e| (SourceStatus::Failed, WorkItemStatus::Failed, format!("blob 地址无效：{e}")))?;
    let origins = vec![walk.endpoint.api_origin.clone()];
    let headers = [("Accept", ACCEPT_JSON), ("X-GitHub-Api-Version", API_VERSION)];
    let fetch = walk
        .client
        .fetch_provider_api(&url, &origins, &headers, walk.authorization, walk.meter, walk.endpoint.allow_private, walk.cancelled)
        .map_err(|block| block_stop(&block, walk.authorization))?;
    let mut read = BlobRead { status: fetch.status, verified: false, content: None, note: String::new(), bytes: fetch.body.len() as i64 };
    if let Some(failure) = provider_failure(&fetch) {
        read.note = format!("{}:{}", failure.code, scrub(walk.authorization, &failure.message));
    } else if fetch.truncated {
        read.note = "blob 响应被单响应上限截断，未作为全文分析".to_string();
    } else if let Ok(value) = serde_json::from_slice::<JsonValue>(&fetch.body) {
        let sha = value.get("sha").and_then(JsonValue::as_str).unwrap_or_default().to_ascii_lowercase();
        let size = value.get("size").and_then(JsonValue::as_i64).unwrap_or(-1);
        let encoding = value.get("encoding").and_then(JsonValue::as_str).unwrap_or_default();
        if sha != hit.blob_sha {
            read.note = format!("github_blob_mismatch:返回的 sha {} 与请求的 {} 不一致，正文丢弃", evidence_excerpt(&sha, 48), hit.blob_sha);
        } else if !(0..=BLOB_MAX_BYTES).contains(&size) {
            read.note = format!("blob 大小 {size} 超过上限 {BLOB_MAX_BYTES}，未作为全文分析");
        } else if encoding != "base64" {
            read.note = format!("blob 编码 {encoding} 不支持");
        } else {
            use base64::Engine;
            let packed: String = value
                .get("content")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            match base64::engine::general_purpose::STANDARD.decode(packed) {
                Ok(bytes) if bytes.len() as i64 == size => {
                    read.verified = true;
                    read.content = Some(String::from_utf8_lossy(&bytes).to_string());
                }
                Ok(bytes) => read.note = format!("blob 解码后 {} 字节与声明的 {size} 不一致", bytes.len()),
                Err(_) => read.note = "blob 内容不是有效 base64".to_string(),
            }
        }
    } else {
        read.note = "blob 响应不是 JSON".to_string();
    }
    if fetch.status == 403 || fetch.status == 429 {
        if let Some(failure) = provider_failure(&fetch).filter(|f| f.source_status == SourceStatus::RateLimited) {
            walk.session.exhausted.insert(CAPABILITY_BLOB_READ, failure.message);
        }
    }
    outcome.observations.push(NewObservation {
        status_code: i64::from(fetch.status),
        content_type: fetch.content_type.clone(),
        body_bytes: read.bytes,
        truncated: fetch.truncated,
        final_url: url.to_string(),
        extra: json!({
            "adapterVersion": ADAPTER_VERSION,
            "capability": CAPABILITY_BLOB_READ,
            "collectionMode": "blob_read",
            "fullName": hit.full_name,
            "path": hit.path,
            "blobSha": hit.blob_sha,
            "verified": read.verified,
            "note": read.note,
            "rateLimit": RateLimit::of(&fetch).as_json(),
        }),
        ..NewObservation::new(&walk.item.source_key, "github_blob_read", url.as_str())
    });
    Ok(read)
}

/// `default_branch_code_search` (+ `controlled_blob_read` for the first few hits). Without a
/// credential nothing is sent and the item is `unconfigured` with the port's own reason.
#[allow(clippy::too_many_arguments)]
pub fn execute_code(
    client: &PassiveClient,
    item: &WorkItem,
    meter: &mut RequestMeter,
    endpoint: &GithubEndpoint,
    session: &mut GithubSession,
    credential: &CredentialLookup,
    cancelled: &dyn Fn() -> bool,
) -> ItemOutcome {
    let mut outcome = ItemOutcome::default();
    let token = match credential {
        CredentialLookup::Unconfigured { code, reason } => {
            return unconfigured(&mut outcome, format!("{code}:{reason}；未发送任何请求"));
        }
        CredentialLookup::Available(token) => token,
    };
    if item.entity.trim().is_empty() {
        return unconfigured(&mut outcome, "没有已确认的根域名，GitHub 代码搜索未配置".to_string());
    }
    let authorization = token.authorization_for(&endpoint.api_origin);
    let query = code_query(&item.entity);
    let mut walk = Walk {
        client,
        item,
        meter,
        endpoint,
        session,
        authorization: Some(&authorization),
        cancelled,
        capability: CAPABILITY_CODE_SEARCH,
    };
    let result = walk_search(&mut walk, &mut outcome, CODE_SEARCH_PATH, &query, CODE_PER_PAGE, CODE_MAX_PAGES, ACCEPT_TEXT_MATCH);
    let mut recorded: Vec<String> = Vec::new();
    let mut reads = 0usize;
    let mut read_stop: Option<(SourceStatus, WorkItemStatus, String)> = None;
    let mut read_notes: Vec<String> = Vec::new();
    for (page, row) in &result.rows {
        let Some(hit) = code_hit(row) else { continue };
        let identity = format!("{}:{}:{}", hit.repository_id, hit.blob_sha, hit.path);
        if recorded.contains(&identity) {
            continue;
        }
        recorded.push(identity.clone());
        let snippet = scrub(Some(&authorization), &hit.fragments.join("\n…\n"));
        let mut body_text = snippet.clone();
        let mut mode = "snippet";
        if reads < BLOB_READS_PER_QUERY && read_stop.is_none() && !walk.session.exhausted.contains_key(CAPABILITY_BLOB_READ) {
            reads += 1;
            match read_blob(&mut walk, &hit, &mut outcome) {
                Ok(read) => match read.content {
                    Some(content) => {
                        body_text = scrub(Some(&authorization), &content);
                        mode = "blob_read";
                    }
                    None => read_notes.push(format!("{}：{}", hit.path, read.note)),
                },
                Err(stop) => read_stop = Some(stop),
            }
        }
        let sightings = credential_sightings(&body_text, 2_000);
        let candidate = sightings.iter().find(|s| s.grade == CredentialGrade::Candidate);
        let placeholder = sightings.iter().find(|s| s.grade == CredentialGrade::Placeholder);
        let excerpt = evidence_excerpt(&snippet, 400);
        let (category, title, confidence, said) = match (candidate, placeholder) {
            (Some(s), _) => ("credential", TITLE_CODE_CREDENTIAL, CREDENTIAL_PLACEHOLDER_CONFIDENCE + 40, format!("字段 {} = {}（第 {} 行）", s.key, s.masked, s.line)),
            (None, Some(s)) => ("credential", TITLE_CODE_CREDENTIAL, CREDENTIAL_PLACEHOLDER_CONFIDENCE, format!("字段 {} 只有占位值 {}", s.key, s.masked)),
            (None, None) => ("source_code", TITLE_CODE_HIT, 30, "未见凭据形态值".to_string()),
        };
        outcome.observations.push(NewObservation {
            status_code: 200,
            content_type: "text/plain".to_string(),
            body_bytes: body_text.len() as i64,
            body: body_text.into_bytes(),
            extra: json!({
                "adapterVersion": ADAPTER_VERSION,
                "capability": CAPABILITY_CODE_SEARCH,
                "collectionMode": mode,
                "providerObjectId": identity,
                "repositoryId": hit.repository_id,
                "fullName": hit.full_name,
                "path": hit.path,
                "blobSha": hit.blob_sha,
                "excerpt": excerpt,
                "query": query,
                "entities": [item.entity],
                "page": page,
                "providerUrlMismatch": hit.provider_url_mismatch,
                "bodyRead": mode == "blob_read",
            }),
            ..NewObservation::new(&item.source_key, "github_code_hit", &hit.canonical_url)
        });
        outcome.findings.push(FindingDraft {
            category: category.to_string(),
            title: title.to_string(),
            url: hit.canonical_url.clone(),
            source_type: item.source_key.clone(),
            evidence: format!(
                "{} 的 {}：{said}；{}；查询 {query}",
                hit.full_name,
                hit.path,
                if mode == "blob_read" { "已按 blob sha 读取并核对正文" } else { "只有搜索摘要，未读取正文" }
            ),
            severity: if candidate.is_some() { "high" } else { "low" }.to_string(),
            confidence,
            asset_id: None,
            content_hash: content_hash(identity.as_bytes()),
        });
        outcome.observed += 1;
    }
    conclude(&mut outcome, &result, recorded.len(), "代码命中");
    if let Some((source, item_status, reason)) = read_stop {
        if source.severity_rank() >= outcome.source_status.map(SourceStatus::severity_rank).unwrap_or(-1) {
            outcome.source_status = Some(source);
            outcome.status = Some(item_status);
        }
        outcome.reason = format!("{}；正文读取中止：{reason}", outcome.reason);
        outcome.source_reason = outcome.reason.clone();
    }
    if !read_notes.is_empty() {
        outcome.reason = format!("{}；未读全文：{}", outcome.reason, read_notes.join("；"));
        if outcome.source_status == Some(SourceStatus::Completed) {
            outcome.source_status = Some(SourceStatus::Partial);
        }
        outcome.source_reason = outcome.reason.clone();
    }
    outcome
}

/// The capability matrix the settings page shows. Each capability says what it needs and
/// whether it can run now; metadata success never marks code search as done.
pub fn capabilities(credential: &CredentialLookup) -> JsonValue {
    let (state, reason) = match credential {
        CredentialLookup::Available(_) => ("configured", String::new()),
        CredentialLookup::Unconfigured { code, reason } => (*code, reason.clone()),
    };
    json!([
        {
            "sourceKey": KIND_REPOSITORIES,
            "capability": CAPABILITY_REPOSITORY_METADATA,
            "credential": "none",
            "state": "configured",
            "reason": "匿名公开元数据；默认不启用，需在启用来源里勾选",
            "bounds": {"perPage": REPOSITORY_PER_PAGE, "maxPages": REPOSITORY_MAX_PAGES, "resultWindow": SEARCH_RESULT_WINDOW, "automaticRetries": AUTOMATIC_RETRIES},
        },
        {
            "sourceKey": KIND_CODE,
            "capability": CAPABILITY_CODE_SEARCH,
            "credential": "required",
            "state": if state == "configured" { "configured" } else { "unconfigured" },
            "integration": state,
            "reason": reason,
            "bounds": {"perPage": CODE_PER_PAGE, "maxPages": CODE_MAX_PAGES, "resultWindow": SEARCH_RESULT_WINDOW, "automaticRetries": AUTOMATIC_RETRIES},
        },
        {
            "sourceKey": KIND_CODE,
            "capability": CAPABILITY_BLOB_READ,
            "credential": "required",
            "state": if state == "configured" { "configured" } else { "unconfigured" },
            "integration": state,
            "reason": if state == "configured" { String::new() } else { "依赖代码搜索命中与凭据；未配置时不读取任何正文".to_string() },
            "bounds": {"readsPerQuery": BLOB_READS_PER_QUERY, "maxBytes": BLOB_MAX_BYTES},
        },
    ])
}
