// §Q2/Q5 — one redaction authority for the exposure surface. Every entry point that can
// carry a secret (evidence excerpt, log line, error text, CSV cell, preview) goes through
// here, so "don't paste a key" is not the only thing standing between a leak and a report.

use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// Bump when the rules below change: the version is stored with the evidence so a later
/// reviewer can tell which redaction pass produced what they are reading.
pub const REDACTION_VERSION: &str = "oviraptor.exposure.redaction.v2";

const VALUE_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "access_key",
    "accesskey",
    "secret_key",
    "secretkey",
    "client_secret",
    "client_secret_basic",
    "private_token",
    "secret",
    "token",
    "password",
    "passwd",
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "session",
    "bearer",
    "x-amz-credential",
    "x-amz-security-token",
    "security_token",
];

/// Values that are documentation or environment lookups, not exposed material. They are
/// reported as placeholders rather than candidates so a template never out-ranks a real
/// assignment.
const PLACEHOLDER_TERMS: &[&str] = &[
    "example",
    "placeholder",
    "your_",
    "your-",
    "yoursite",
    "xxxx",
    "****",
    "changeme",
    "change_me",
    "test_key",
    "testkey",
    "dummy",
    "sample",
    "process.env",
    "import.meta.env",
    "${",
    "<",
    "{{",
    "todo",
    "redacted",
];

fn secret_key_pattern() -> &'static regex::Regex {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        // The regex crate has no verbose mode and no backreferences, so the optional
        // quotes are matched positionally instead of being captured and re-checked.
        let keys = VALUE_KEYS.join("|");
        regex::Regex::new(&format!(
            r#"(?i)(?P<key>{keys})["']?\s*[:=]\s*["']?(?P<value>[^\s,"';&<>]{{6,}})"#
        ))
        .expect("脱敏正则必须可编译")
    })
}

fn header_pattern() -> &'static regex::Regex {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        regex::Regex::new(
            r"(?i)(?P<scheme>bearer|basic|token)\s+(?P<value>[A-Za-z0-9._~+/=\-]{8,})",
        )
        .expect("认证头正则必须可编译")
    })
}

fn url_credential_pattern() -> &'static regex::Regex {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        regex::Regex::new(r"(?i)\b(?P<scheme>https?)://(?P<user>[^/\s:@]+):(?P<secret>[^/\s@]+)@")
            .expect("URL 凭据正则必须可编译")
    })
}

/// FIX4 P1／R8 URL 查询串/片段里“键名像凭据”的参数（如 access_token=…）。
fn url_query_secret_pattern() -> &'static regex::Regex {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        let keys = VALUE_KEYS.iter().map(|key| regex::escape(key)).collect::<Vec<_>>().join("|");
        regex::Regex::new(&format!(r"(?i)(?P<sep>[?&#;])(?P<key>[^=&#;\s]*(?:{keys})[^=&#;\s]*)=(?P<value>[^&#;\s]+)"))
            .expect("URL 查询凭据正则必须可编译")
    })
}

/// FIX4 P1／R8 落库前的 URL 脱敏（导入声明 URL 使用）：用户信息里的口令、键名像凭据的查询/片段参数值按 mask_value 打码，
/// 其余原样保留（仍可读、可聚类）。已打码的值（含 ***）不再处理，结果幂等。
pub fn redact_url(url: &str) -> String {
    let out = url_credential_pattern().replace_all(url, |caps: &regex::Captures| {
        let secret = grab(caps, "secret");
        let masked = if secret.contains("***") { secret.to_string() } else { mask_value(secret) };
        format!("{}://{}:{}@", grab(caps, "scheme").to_lowercase(), grab(caps, "user"), masked)
    });
    url_query_secret_pattern()
        .replace_all(&out, |caps: &regex::Captures| {
            let value = grab(caps, "value");
            let masked = if value.contains("***") { value.to_string() } else { mask_value(value) };
            format!("{}{}={}", grab(caps, "sep"), grab(caps, "key"), masked)
        })
        .to_string()
}

/// Keep enough to correlate a value without storing it: three characters at the front,
/// two at the back, and the length. A value shorter than eight characters is masked whole.
pub fn mask_value(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() < 8 {
        return "***".to_string();
    }
    let head: String = chars.iter().take(3).collect();
    let tail: String = chars
        .iter()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}***{tail}(len={})", chars.len())
}

fn is_placeholder(value: &str) -> bool {
    let lowered = value.to_lowercase();
    PLACEHOLDER_TERMS.iter().any(|term| lowered.contains(term))
}

/// Removes credential material from arbitrary text. The result is safe to store, log and
/// export; the original length is preserved only as a coarse hash so equality checks
/// still work without keeping the bytes.
fn grab<'a>(caps: &'a regex::Captures, name: &str) -> &'a str {
    caps.name(name).map(|row| row.as_str()).unwrap_or_default()
}

pub fn redact_text(text: &str) -> String {
    let out = url_credential_pattern()
        .replace_all(text, |caps: &regex::Captures| {
            format!(
                "{}://{}:{}@",
                grab(caps, "scheme").to_lowercase(),
                grab(caps, "user"),
                mask_value(grab(caps, "secret"))
            )
        })
        .to_string();
    let out = header_pattern()
        .replace_all(&out, |caps: &regex::Captures| {
            format!(
                "{} {}",
                grab(caps, "scheme"),
                mask_value(grab(caps, "value"))
            )
        })
        .to_string();
    secret_key_pattern()
        .replace_all(&out, |caps: &regex::Captures| {
            format!("{}:{}", grab(caps, "key"), mask_value(grab(caps, "value")))
        })
        .to_string()
}

/// What an assignment looks like after inspection. `Placeholder` and `Candidate` are
/// statements about a *pattern*, never proof that a key works: the plan forbids using a
/// discovered value against the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialGrade {
    None,
    Placeholder,
    Candidate,
}

impl CredentialGrade {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Placeholder => "placeholder",
            Self::Candidate => "candidate",
        }
    }
}

#[derive(Debug)]
pub struct CredentialSighting {
    pub grade: CredentialGrade,
    pub key: String,
    pub masked: String,
    pub line: usize,
}

/// Scans at most `max_lines` lines and returns every sighting, each already reduced to a
/// masked fragment: the raw value never leaves this function.
pub fn credential_sightings(text: &str, max_lines: usize) -> Vec<CredentialSighting> {
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if index >= max_lines {
            break;
        }
        for captures in secret_key_pattern().captures_iter(line) {
            let key = captures
                .name("key")
                .map(|row| row.as_str().trim_matches(['"', '\'']).to_string())
                .unwrap_or_default();
            let value = captures
                .name("value")
                .map(|row| row.as_str().trim_matches(['"', '\'']).to_string())
                .unwrap_or_default();
            if value.len() < 8 || value.contains(' ') {
                continue;
            }
            rows.push(CredentialSighting {
                grade: if is_placeholder(&value) {
                    CredentialGrade::Placeholder
                } else {
                    CredentialGrade::Candidate
                },
                key,
                masked: mask_value(&value),
                line: index + 1,
            });
        }
    }
    rows
}

/// A stable fingerprint of the raw bytes, so "the same secret was seen again" is
/// detectable without storing it.
pub fn content_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// FIX3-R6：幂等哈希。人工文本先脱敏再落库，但两段原文不同、脱敏后相同（同前缀/后缀/长度）
/// 的输入必须是不同载荷，不能互相重放。只有脱敏真的改写了文本时才把原文的不可逆摘要并入哈希；
/// 未改写时结果与旧哈希完全一致，已有 operationId 的重放不受影响。原文本身不保存。
pub fn idempotency_hash(redacted_payload: &str, raw_texts: &[&str]) -> String {
    if raw_texts.iter().all(|text| redact_text(text) == *text) {
        return content_hash(redacted_payload.as_bytes());
    }
    let raw = content_hash(raw_texts.join("\u{1f}").as_bytes());
    content_hash(format!("{redacted_payload}\u{1f}{raw}").as_bytes())
}

/// Leading `=`, `+`, `-`, `@`, tab and CR are evaluated by spreadsheet software. Every
/// exported cell goes through here, not through a call-site memory.
pub fn escape_csv_cell(value: &str) -> String {
    if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{value}")
    } else {
        value.to_string()
    }
}

/// Bounded, redacted and always valid UTF-8: what the evidence table stores.
pub fn evidence_excerpt(text: &str, limit: usize) -> String {
    let redacted = redact_text(text);
    let cleaned = redacted
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect::<String>();
    let mut out: String = cleaned.chars().take(limit).collect();
    if cleaned.chars().count() > limit {
        out.push_str("…[已截断]");
    }
    out
}

/// Every string inside a JSON document, redacted. Used for request/response metadata that
/// is stored as an object rather than a blob.
pub fn redact_json(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::String(text) => Value::String(redact_text(text)),
        Value::Array(rows) => Value::Array(rows.iter().map(redact_json).collect()),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, item) in map {
                let key_matches = VALUE_KEYS
                    .iter()
                    .any(|secret| key.to_lowercase().contains(secret));
                let masked = if key_matches {
                    match item.as_str() {
                        Some(text) => Value::String(mask_value(text)),
                        None => Value::String("***".to_string()),
                    }
                } else {
                    redact_json(item)
                };
                out.insert(key.clone(), masked);
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}
