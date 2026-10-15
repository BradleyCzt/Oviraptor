//! Identity is the requested endpoint, never the discovery seed or resolved IP.
//! Keep schemes, non-default ports, subdomains, paths and queries distinct.

fn with_scheme(raw: &str, scheme: &str, port: Option<u16>) -> Option<reqwest::Url> {
    let authority = if raw.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{raw}]")
    } else {
        raw.to_string()
    };
    let mut url = reqwest::Url::parse(&format!("{scheme}://{authority}")).ok()?;
    // Non-special schemes do not normalize DNS case themselves.
    let host = url.host_str()?.to_ascii_lowercase();
    url.set_host(Some(&host)).ok()?;
    if url.port().is_none() {
        if let Some(port) = port {
            url.set_port(Some(port)).ok()?;
        }
    }
    url.set_fragment(None);
    Some(url)
}

fn network_authority(raw: &str) -> Option<reqwest::Url> {
    let raw = raw.trim();
    if raw.is_empty() || raw.chars().any(char::is_whitespace) || raw.chars().any(char::is_control) {
        return None;
    }
    let url = if raw.contains("://") {
        reqwest::Url::parse(raw).ok()?
    } else {
        with_scheme(raw, "http", None)?
    };
    if !url.username().is_empty()
        || url.password().is_some()
        || matches!(
            url.scheme(),
            "file" | "data" | "javascript" | "blob" | "about"
        )
        || url.host_str().is_none_or(str::is_empty)
        || url.port() == Some(0)
    {
        return None;
    }
    Some(url)
}

/// Admission checks network identifiers; an opaque fallback key alone is not
/// proof that a row describes a target. Legacy hosts without a port remain valid.
pub(super) fn valid_network_identity(
    link: &str,
    host: &str,
    protocol: &str,
    ip: &str,
    port: &str,
) -> bool {
    let port = port.trim();
    if !port.is_empty()
        && port
            .parse::<u16>()
            .ok()
            .filter(|value| *value > 0)
            .is_none()
    {
        return false;
    }
    if !ip.trim().is_empty()
        && ip
            .trim()
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_err()
    {
        return false;
    }
    if protocol.chars().any(char::is_control) {
        return false;
    }
    let identifiers: Vec<_> = [link, host]
        .into_iter()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect();
    if identifiers.is_empty() {
        return !ip.trim().is_empty();
    }
    identifiers
        .into_iter()
        .all(|raw| network_authority(raw).is_some())
}

pub(super) fn endpoint_url(
    link: &str,
    host: &str,
    protocol: &str,
    ip: &str,
    port: &str,
) -> Option<String> {
    let raw = [link, host, ip]
        .into_iter()
        .map(str::trim)
        .find(|v| !v.is_empty())?;
    let port = port.trim().parse::<u16>().ok().filter(|v| *v > 0);
    let protocol = protocol.trim().to_ascii_lowercase();
    let mut url = if raw.contains("://") {
        reqwest::Url::parse(raw).ok()?
    } else {
        let scheme = if protocol == "https" {
            "https"
        } else if protocol == "http" {
            "http"
        } else if port.is_some_and(|v| [443, 4443, 6443, 7443, 8443, 9443, 10443].contains(&v)) {
            "https"
        } else if port.is_some() {
            "tcp+web"
        } else {
            return None;
        };
        with_scheme(raw, scheme, port)?
    };
    url.host_str()?;
    url.set_fragment(None);
    Some(url.to_string())
}

pub(super) fn canonical_identity(
    link: &str,
    host: &str,
    protocol: &str,
    ip: &str,
    port: &str,
    fallback: &str,
) -> String {
    let raw = [link, host, ip]
        .into_iter()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .unwrap_or("");
    let service = protocol.trim().to_ascii_lowercase();
    // A guessed Web access URL is useful to the probe, but does not establish
    // equality between known non-Web services, even on a common TLS port.
    if !raw.contains("://") && !matches!(service.as_str(), "http" | "https") {
        let port = port.trim().parse::<u16>().ok().filter(|value| *value > 0);
        if let Some(url) = with_scheme(raw, "tcp+web", port) {
            let service = if service.is_empty() {
                "unknown"
            } else {
                &service
            };
            return format!("service:{service}|{url}");
        }
    }
    endpoint_url(link, host, protocol, ip, port).unwrap_or_else(|| {
        // Invalid/non-Web records still need stable identities. Do not merge
        // different services or lowercase a case-sensitive path.
        format!(
            "raw:{}|{}|{}|{}|{}|{}",
            protocol.trim().to_ascii_lowercase(),
            link.trim(),
            host.trim(),
            ip.trim(),
            port.trim(),
            fallback.trim()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_normalizes_only_equivalent_urls() {
        let key = |url: &str| canonical_identity(url, "", "http", "", "", "");
        assert_eq!(
            key("HTTP://EXAMPLE.COM:80"),
            key("http://example.com/#section")
        );
        for other in [
            "https://example.com/",
            "http://www.example.com/",
            "http://example.com:8080/",
            "http://example.com/A",
            "http://example.com/a",
            "http://example.com/a/",
            "http://example.com/?x=1",
        ] {
            assert_ne!(key("http://example.com/"), key(other));
        }
        assert_ne!(key("http://example.com/A"), key("http://example.com/a"));
        assert_ne!(key("http://example.com/a"), key("http://example.com/a/"));
    }

    #[test]
    fn identity_preserves_existing_ports_and_ipv6() {
        assert_eq!(
            endpoint_url("", "example.com:8080", "http", "", "8080").unwrap(),
            "http://example.com:8080/"
        );
        assert_ne!(
            endpoint_url("", "example.com", "http", "", "8443"),
            endpoint_url("", "example.com", "https", "", "8443")
        );
        assert_eq!(
            endpoint_url("", "", "http", "::1", "8080").unwrap(),
            "http://[::1]:8080/"
        );
        assert_eq!(
            endpoint_url("https://example.com/", "", "http", "1.2.3.4", "80").unwrap(),
            "https://example.com/"
        );
    }
    #[test]
    fn known_non_web_services_keep_protocol_and_ports_outside_probe_guesses() {
        let key = |protocol, port| {
            canonical_identity("", "EXAMPLE.com", protocol, "192.0.2.1", port, "seed")
        };
        for port in ["22", "80", "443", "8443"] {
            assert_ne!(key("ssh", port), key("redis", port));
            assert_ne!(key("ssh", port), key("http", port));
            assert_ne!(key("ssh", port), key("https", port));
            assert_eq!(key("SSH", port), key("ssh", port));
            assert!(key("ssh", port).contains(&format!(":{port}")));
        }
        for service in ["non-http", "https-alt", "http-proxy", "http/1.1"] {
            assert!(key(service, "443").starts_with(&format!("service:{service}|")));
            assert_ne!(key(service, "443"), key("https", "443"));
        }
        assert!(key("", "443").starts_with("service:unknown|"));
        assert_ne!(key("", "443"), key("https", "443"));
        assert_ne!(key("ssh", "80"), key("ssh", ""));
        assert_eq!(
            canonical_identity("", "example.com", "ssh", "192.0.2.2", "22", "another-seed"),
            key("ssh", "22")
        );
        assert_eq!(
            endpoint_url("", "example.com", "ssh", "", "443").unwrap(),
            "https://example.com/"
        );
        assert_eq!(
            endpoint_url("", "example.com", "ssh", "", "22").unwrap(),
            "tcp+web://example.com:22"
        );
    }

    #[test]
    fn network_identity_admission_requires_an_authority_without_credentials() {
        assert!(valid_network_identity("", "legacy.example", "", "", ""));
        assert!(valid_network_identity(
            "https://www.example/A?q=1",
            "different.example",
            "http",
            "192.0.2.1",
            "80"
        ));
        assert!(valid_network_identity("", "", "ssh", "2001:db8::1", "22"));
        for (link, host, ip, port) in [
            ("", "", "", ""),
            ("https://user:password@example.test/", "", "", "443"),
            ("https://approved.example@evil.example/", "", "", "443"),
            ("https://[invalid/", "example.test", "", "443"),
            ("", "not a hostname", "", ""),
            ("file://example.test/local", "", "", ""),
            ("https://example.test:0/", "", "", ""),
            ("", "example.test", "", "65536"),
            ("", "example.test", "", "bad"),
            ("", "example.test", "invalid-ip", "80"),
        ] {
            assert!(
                !valid_network_identity(link, host, "http", ip, port),
                "{link:?} {host:?} {ip:?} {port:?}"
            );
        }
    }
}
