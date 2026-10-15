//! Identity is the requested endpoint, never the discovery seed or resolved IP.
//! Keep schemes, non-default ports, subdomains, paths and queries distinct.

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
        let scheme = if protocol.contains("https") {
            "https"
        } else if protocol.contains("http") {
            "http"
        } else if port.is_some_and(|v| [443, 4443, 6443, 7443, 8443, 9443, 10443].contains(&v)) {
            "https"
        } else if port.is_some() {
            "tcp+web"
        } else {
            return None;
        };
        let authority = if raw.parse::<std::net::Ipv6Addr>().is_ok() {
            format!("[{raw}]")
        } else {
            raw.to_string()
        };
        let mut url = reqwest::Url::parse(&format!("{scheme}://{authority}")).ok()?;
        if url.port().is_none() {
            if let Some(port) = port {
                url.set_port(Some(port)).ok()?;
            }
        }
        url
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
}
