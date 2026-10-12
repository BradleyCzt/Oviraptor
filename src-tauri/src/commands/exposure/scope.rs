// §Q2 — the authorization contract for the passive exposure surface. Ownership and
// exposure authorization are two different questions, and a grant is frozen per run:
// origin-and-method exact, no guessed sibling subdomains, no redirect following into a
// different host, no private destination unless someone configured that on purpose.

use serde_json::{json, Value as JsonValue};
use std::net::{IpAddr, Ipv6Addr};

use super::state::utc_now;

/// Every denial carries a stable code: the run records why it stopped rather than
/// quietly producing fewer results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExposureDenial {
    pub code: &'static str,
    pub message: String,
}

impl ExposureDenial {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn as_json(&self) -> JsonValue {
        json!({"code": self.code, "reason": self.message})
    }
}

/// One frozen target grant. `origin` is scheme + host + effective port; a subdomain of a
/// granted host is a *different* origin and needs its own row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExposureGrant {
    pub origin: String,
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub asset_id: Option<i64>,
    pub label: String,
}

impl ExposureGrant {
    pub fn from_url(url: &reqwest::Url) -> Option<Self> {
        let scheme = match url.scheme() {
            "http" | "https" => url.scheme().to_string(),
            _ => return None,
        };
        let host = url.host_str()?.to_ascii_lowercase();
        // Only the scheme's default port is normalized away. `port_or_known_default()`
        // echoes an explicit port, so comparing the two would drop every real port and
        // silently widen a grant to all ports of the host.
        let port = match (url.scheme(), url.port()) {
            ("http", Some(80)) | ("https", Some(443)) | (_, None) => None,
            (_, Some(value)) => Some(value),
        };
        Some(Self {
            origin: match port {
                Some(value) => format!("{scheme}://{host}:{value}"),
                None => format!("{scheme}://{host}"),
            },
            scheme,
            host,
            port,
            asset_id: None,
            label: String::new(),
        })
    }

    fn matches(&self, other: &Self) -> bool {
        self.origin.eq_ignore_ascii_case(&other.origin)
    }
}

#[derive(Clone, Debug)]
pub struct ExposureScope {
    pub project_id: i64,
    /// The ownership policy version this scope was derived from. A change mid-run means
    /// the in-flight assumptions are no longer the ones a human approved.
    pub scope_version: i64,
    pub issued_at: String,
    pub valid_until: Option<String>,
    /// True when the grants came from the pre-fine-grained coarse `confirmed` rows.
    pub coarse_legacy: bool,
    pub grants: Vec<ExposureGrant>,
    pub sources: Vec<String>,
    pub max_requests: i64,
    pub max_response_bytes: i64,
    pub max_total_bytes: i64,
    pub allow_private_destinations: bool,
    /// Assets that were excluded, with the reason, so "nothing to do" is explainable.
    pub excluded: Vec<Exclusion>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exclusion {
    pub asset_id: i64,
    pub reason: &'static str,
}

impl ExposureScope {
    /// Reads the current authorization state. Nothing here guesses: an asset that is not
    /// an active, non-removed project member or whose decision is not eligible simply is
    /// not part of the scope, and it is recorded as an exclusion instead.
    pub fn load(
        connection: &rusqlite::Connection,
        project_id: i64,
        settings: &JsonValue,
    ) -> Result<Self, ExposureDenial> {
        let active: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM projects WHERE id=?1 AND status='active'",
                [project_id],
                |row| row.get(0),
            )
            .map_err(|error| ExposureDenial::new("scope_read_failed", error.to_string()))?;
        if active == 0 {
            return Err(ExposureDenial::new(
                "project_inactive",
                format!("工作空间 {project_id} 不存在或已归档，被动采集没有授权范围"),
            ));
        }
        let scope_version: i64 = connection
            .query_row(
                "SELECT COALESCE(MAX(policy_version),1) FROM asset_ownership_profiles WHERE project_id=?1",
                [project_id],
                |row| row.get(0),
            )
            .unwrap_or(1);
        let rows: Vec<(i64, String, String, i64, String, i64)> = {
            let mut statement = connection
                .prepare(
                    "SELECT a.id, COALESCE(NULLIF(a.link,''), CASE WHEN a.host<>'' THEN 'https://'||a.host ELSE '' END),
                            COALESCE(o.status,'unreviewed'), COALESCE(o.exposure_eligible,0),
                            COALESCE(o.authorization_status,'unknown'), COALESCE(pa.is_deleted,1)
                     FROM assets a
                     LEFT JOIN asset_ownership_decisions o ON o.asset_id=a.id AND o.project_id=?1
                     LEFT JOIN project_assets pa ON pa.asset_id=a.id AND pa.project_id=?1
                     ORDER BY a.id",
                )
                .map_err(|error| ExposureDenial::new("scope_read_failed", error.to_string()))?;
            let collected = statement
                .query_map([project_id], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                })
                .map_err(|error| ExposureDenial::new("scope_read_failed", error.to_string()))?;
            collected.filter_map(|row| row.ok()).collect()
        };

        let mut grants: Vec<ExposureGrant> = Vec::new();
        let mut excluded = Vec::new();
        for (asset_id, raw, status, eligible, authorization, in_project) in &rows {
            let reason = if *in_project == 1 {
                Some("asset_removed_from_project")
            } else if *eligible != 1 {
                if authorization.eq_ignore_ascii_case("revoked") {
                    Some("authorization_revoked")
                } else {
                    Some("not_exposure_eligible")
                }
            } else if status.eq_ignore_ascii_case("rejected") {
                Some("ownership_rejected")
            } else {
                None
            };
            if let Some(code) = reason {
                excluded.push(Exclusion {
                    asset_id: *asset_id,
                    reason: code,
                });
                continue;
            }
            let Ok(parsed) = reqwest::Url::parse(raw) else {
                excluded.push(Exclusion {
                    asset_id: *asset_id,
                    reason: "unparseable_origin",
                });
                continue;
            };
            let Some(mut grant) = ExposureGrant::from_url(&parsed) else {
                excluded.push(Exclusion {
                    asset_id: *asset_id,
                    reason: "non_http_origin",
                });
                continue;
            };
            grant.asset_id = Some(*asset_id);
            grant.label = if parsed.host_str().unwrap_or_default().is_empty() {
                raw.clone()
            } else {
                parsed.host_str().unwrap_or_default().to_string()
            };
            if !grants.iter().any(|existing| existing.matches(&grant)) {
                grants.push(grant);
            }
        }

        let days = settings
            .get("exposureAuthorizationDays")
            .and_then(JsonValue::as_i64)
            .filter(|value| *value > 0);
        let issued_at = utc_now();
        Ok(Self {
            project_id,
            scope_version,
            issued_at: issued_at.clone(),
            valid_until: days.map(|value| shift_days(&issued_at, value)),
            // Nothing in the current schema can express a per-origin, time-boxed grant, so
            // every row derived from `confirmed` is labelled a legacy coarse scope.
            coarse_legacy: !settings
                .get("exposureFineGrainedScope")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            grants,
            sources: string_list(
                settings.get("exposureSources"),
                &["passive_web", "certificate_transparency", "web_archive"],
            ),
            max_requests: positive(settings, "exposureMaxRequests", 2_000),
            max_response_bytes: positive(settings, "exposureMaxResponseBytes", 1_000_000),
            max_total_bytes: positive(settings, "exposureMaxTotalBytes", 200_000_000),
            allow_private_destinations: settings
                .get("exposureAllowPrivateDestinations")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            excluded,
        })
    }

    pub fn grant_for(&self, url: &reqwest::Url) -> Option<&ExposureGrant> {
        let candidate = ExposureGrant::from_url(url)?;
        self.grants.iter().find(|grant| grant.matches(&candidate))
    }

    /// The check made before every single outbound request, not once per run.
    pub fn authorize(
        &self,
        url: &reqwest::Url,
        now: &str,
    ) -> Result<ExposureGrant, ExposureDenial> {
        if self.is_expired_at(now) {
            return Err(ExposureDenial::new(
                "scope_expired",
                format!(
                    "授权有效期至 {}，已过期，被动采集停止在该目标上继续发请求",
                    self.valid_until.clone().unwrap_or_default()
                ),
            ));
        }
        if url.username() != "" || url.password().is_some() {
            return Err(ExposureDenial::new(
                "userinfo_denied",
                "URL 含 userinfo（user:pass@host），凭据不允许出现在采集入口".to_string(),
            ));
        }
        if !matches!(url.scheme(), "http" | "https") {
            return Err(ExposureDenial::new(
                "scheme_denied",
                format!("只允许 http(s)，收到 {} 协议", url.scheme()),
            ));
        }
        let Some(found) = self.grant_for(url) else {
            return Err(ExposureDenial::new(
                "host_not_granted",
                format!(
                    "{} 不在本项目已授权的 origin 内；批准某个 host 推不出它的兄弟子域、其他端口或其他 host",
                    url.origin().ascii_serialization()
                ),
            ));
        };
        let grant = found.clone();
        let addresses = destination_addresses(url)?;
        self.check_addresses(&addresses, &grant.host)?;
        Ok(grant)
    }

    /// Auto-redirect is off; each hop is a fresh authorization. A hop into another host,
    /// scheme, port or a private address stops the walk and the observation is kept.
    pub fn authorize_redirect(
        &self,
        from: &reqwest::Url,
        to: &reqwest::Url,
        now: &str,
    ) -> Result<ExposureGrant, ExposureDenial> {
        let left = ExposureGrant::from_url(from);
        let right = ExposureGrant::from_url(to);
        let (Some(left), Some(right)) = (left, right) else {
            return Err(ExposureDenial::new(
                "redirect_unparseable",
                "重定向目标无法解析为 http(s) origin，已停止跟随".to_string(),
            ));
        };
        if left.origin != right.origin {
            return Err(ExposureDenial::new(
                "redirect_cross_origin",
                format!(
                    "重定向跳出已授权 origin：{} → {}",
                    left.origin, right.origin
                ),
            ));
        }
        self.authorize(to, now)
    }

    pub fn check_addresses(&self, addresses: &[IpAddr], host: &str) -> Result<(), ExposureDenial> {
        if addresses.is_empty() {
            return Err(ExposureDenial::new(
                "destination_unresolved",
                format!("{host} 解析不到地址，不能确定出口策略"),
            ));
        }
        if self.allow_private_destinations {
            return Ok(());
        }
        if let Some(ip) = addresses.iter().find(|address| is_private_address(address)) {
            return Err(ExposureDenial::new(
                "private_destination_denied",
                format!("{host} 解析到内网/回环/链路本地地址 {ip}，暴露面通道默认不得访问；需要内部资产时必须显式配置并批准"),
            ));
        }
        Ok(())
    }

    pub fn is_expired_at(&self, now: &str) -> bool {
        match (&self.valid_until, now) {
            (Some(until), value) => !until.is_empty() && value > until.as_str(),
            _ => false,
        }
    }

    pub fn as_json(&self) -> JsonValue {
        json!({
            "projectId": self.project_id,
            "scopeVersion": self.scope_version,
            "issuedAt": self.issued_at,
            "validUntil": self.valid_until,
            "coarseLegacy": self.coarse_legacy,
            "origins": self.grants.iter().map(|grant| json!({
                "origin": grant.origin, "assetId": grant.asset_id, "label": grant.label
            })).collect::<Vec<_>>(),
            "sources": self.sources,
            "budget": {
                "maxRequests": self.max_requests,
                "maxResponseBytes": self.max_response_bytes,
                "maxTotalBytes": self.max_total_bytes,
                "allowPrivateDestinations": self.allow_private_destinations
            },
            "excluded": self.excluded.iter().map(|row| json!({
                "assetId": row.asset_id, "reason": row.reason
            })).collect::<Vec<_>>(),
        })
    }
}

fn positive(settings: &JsonValue, key: &str, fallback: i64) -> i64 {
    settings
        .get(key)
        .and_then(JsonValue::as_i64)
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

fn string_list(value: Option<&JsonValue>, fallback: &[&str]) -> Vec<String> {
    value
        .and_then(JsonValue::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(JsonValue::as_str)
                .map(|row| row.trim().to_ascii_lowercase())
                .filter(|row| !row.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|rows| !rows.is_empty())
        .unwrap_or_else(|| fallback.iter().map(|row| row.to_string()).collect())
}

/// `yyyy-mm-dd hh:mm:ss` + N days, lexicographically comparable.
fn shift_days(from: &str, days: i64) -> String {
    let Ok(date) = chrono::NaiveDateTime::parse_from_str(from, "%Y-%m-%d %H:%M:%S") else {
        return String::new();
    };
    (date + chrono::Duration::days(days))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

pub fn destination_addresses(url: &reqwest::Url) -> Result<Vec<IpAddr>, ExposureDenial> {
    let host = url
        .host_str()
        .ok_or_else(|| ExposureDenial::new("host_denied", "URL 没有 host".to_string()))?;
    if let Ok(literal) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        return Ok(vec![literal]);
    }
    let port = url.port_or_known_default().unwrap_or(80);
    let resolved = std::net::ToSocketAddrs::to_socket_addrs(&(host, port))
        .map_err(|error| {
            ExposureDenial::new(
                "destination_unresolved",
                format!("{host} 解析失败：{error}"),
            )
        })?
        .map(|socket| socket.ip())
        .collect::<Vec<_>>();
    if resolved.is_empty() {
        return Err(ExposureDenial::new(
            "destination_unresolved",
            format!("{host} 没有可用地址"),
        ));
    }
    Ok(resolved)
}

pub fn is_private_address(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(value) => {
            value.is_loopback()
                || value.is_private()
                || value.is_link_local()
                || value.is_unspecified()
                || value.is_multicast()
                // 100.64.0.0/10 (carrier-grade NAT) and 192.0.0.0/24 are not public here.
                || (value.octets()[0] == 100 && value.octets()[1] >= 64 && value.octets()[1] <= 127)
                || value.octets()[0] == 192 && value.octets()[1] == 0 && value.octets()[2] == 0
        }
        IpAddr::V6(value) => {
            value.is_loopback()
                || value.is_unspecified()
                || value.is_multicast()
                || is_ula_or_link_local(*value)
        }
    }
}

fn is_ula_or_link_local(value: Ipv6Addr) -> bool {
    let first = value.segments()[0];
    // fe80::/10 link-local and fc00::/7 unique-local.
    (first & 0xffc0) == 0xfe80 || (first & 0xfe00) == 0xfc00
}
