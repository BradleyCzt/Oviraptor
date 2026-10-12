// §Q5 — identity of an observation and of the finding it produces. The old surface folded
// the whole URL to lowercase, which merges paths that a server can treat as different
// objects; this layer normalizes only what is provably equivalent and keeps the legacy
// fingerprint as a read alias so history is not re-keyed.

use sha2::{Digest, Sha256};

/// Lowercases scheme and host, drops a port that the scheme already implies, and keeps the
/// path and query bytes intact.
pub fn canonical_url(raw: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(raw) else {
        return raw.trim().to_string();
    };
    let scheme = parsed.scheme().to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https") {
        return raw.trim().to_string();
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let port = match (scheme.as_str(), parsed.port()) {
        ("http", Some(80)) | ("https", Some(443)) | (_, None) => None,
        (_, Some(value)) => Some(value),
    };
    let authority = match port {
        Some(value) => format!("{host}:{value}"),
        None => host,
    };
    let path = parsed.path();
    let query = parsed
        .query()
        .map(|value| format!("?{value}"))
        .unwrap_or_default();
    let fragment = parsed
        .fragment()
        .map(|value| format!("#{value}"))
        .unwrap_or_default();
    format!("{scheme}://{authority}{path}{query}{fragment}")
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// The current key: category + canonical origin (lowercased) + case-sensitive path/query +
/// case-insensitive title.
pub fn finding_fingerprint(category: &str, url: &str, title: &str) -> String {
    let canonical = canonical_url(url);
    let (origin, rest) = match canonical.find("://").map(|index| index + 3) {
        Some(index) => match canonical[index..].find(['/', '?', '#']) {
            Some(offset) => (
                canonical[..index + offset].to_ascii_lowercase(),
                canonical[index + offset..].to_string(),
            ),
            None => (canonical.to_ascii_lowercase(), String::new()),
        },
        None => (canonical.to_ascii_lowercase(), String::new()),
    };
    digest(&format!(
        "{category}|{origin}|{}|{}",
        rest,
        title.trim().to_lowercase()
    ))
}

/// The pre-v2 algorithm, kept only so an existing row is found and updated instead of being
/// duplicated. Nothing writes a v1 fingerprint any more.
pub fn legacy_fingerprint(category: &str, url: &str, title: &str) -> String {
    digest(&format!(
        "{category}|{}|{}",
        url.trim().to_lowercase(),
        title.trim().to_lowercase()
    ))
}

/// A fingerprint of the frozen inputs of a work item, so a resumed round can prove it is
/// still the same plan instead of silently re-pointing at new settings.
pub fn input_hash(parts: &[&str]) -> String {
    digest(&parts.join("\u{1}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_keeps_path_case_but_folds_only_the_origin() {
        let upper =
            finding_fingerprint("source_code", "https://Example.COM/Config.JSON?a=1", "泄漏");
        let same = finding_fingerprint(
            "source_code",
            "https://example.com/Config.JSON?a=1",
            " 泄漏 ",
        );
        let other =
            finding_fingerprint("source_code", "https://example.com/config.JSON?a=1", "泄漏");
        assert_eq!(upper, same, "scheme/host 大小写与默认端口必须归一");
        assert_ne!(upper, other, "path 大小写可能是有语义的不同对象");
    }

    #[test]
    fn default_ports_and_non_http_are_normalized() {
        assert_eq!(
            canonical_url("http://Example.COM:80/a"),
            "http://example.com/a"
        );
        assert_eq!(
            canonical_url("https://example.com:443"),
            "https://example.com/"
        );
        assert_eq!(
            canonical_url("ftp://example.com/pub"),
            "ftp://example.com/pub"
        );
        assert_eq!(canonical_url("not a url"), "not a url");
    }

    #[test]
    fn legacy_alias_is_different_from_the_current_key() {
        let current = finding_fingerprint("document", "https://x.invalid/A.pdf", "T");
        let legacy = legacy_fingerprint("document", "https://x.invalid/A.pdf", "T");
        assert_ne!(
            current, legacy,
            "v2 必须能区分大小写路径，别名只用于读取历史行"
        );
    }
}
