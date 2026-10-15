use super::*;

fn options(allow_private: bool) -> ProbeOptions {
    ProbeOptions {
        workers: 1,
        timeout: Duration::from_secs(2),
        total_timeout: Duration::from_secs(2),
        retries: 0,
        max_body_bytes: 4096,
        include_other: false,
        include_weak: false,
        allow_private,
        strict_tls: true,
        scheme_fallback: false,
        content_threshold: 12,
        replace_default_content_rules: false,
        gambling_keywords: vec![],
        porn_keywords: vec![],
        negative_keywords: vec![],
        custom_keywords: vec![],
        priority_rate: 1000.0,
        other_rate: 1000.0,
        per_host_interval: Duration::ZERO,
    }
}

#[test]
fn actual_probe_blocks_bracketed_private_ipv6_and_mapped_ipv4_before_network() {
    let listener = std::net::TcpListener::bind("[::1]:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let options = options(false);
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap();
    let content = native_content::ContentRules::from_options(&options);
    let pacing = native_probe_control::ProbePacing::new();
    for host in [
        "::1",
        "::",
        "fd12:3456::1",
        "fe80::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
    ] {
        let url = format!("http://[{host}]:{port}/private-test");
        let row = [("link".into(), url.clone())]
            .into_iter()
            .collect::<Record>();
        let result = probe_one(
            &row,
            &options,
            &client,
            &content,
            &pacing,
            1000.0,
            &AtomicBool::new(false),
        );
        assert_eq!(text(&result, &["probe_outcome"]), "skipped", "{url}");
        assert_eq!(text(&result, &["probe_error"]), "non_public_ip", "{url}");
        assert_eq!(text(&result, &["probe_attempts"]), "0", "{url}");
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn actual_probe_explicit_private_permission_allows_ipv6_loopback_http() {
    let listener = std::net::TcpListener::bind("[::1]:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "actual private probe never contacted fixture"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let mut buffer = [0; 1024];
            let length = stream.read(&mut buffer).unwrap();
            assert!(length > 0 && request.len() < 16_384);
            request.extend_from_slice(&buffer[..length]);
        }
        let body = "<html><title>Private IPv6 portal</title><p>Local fixture</p></html>";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let options = options(true);
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap();
    let content = native_content::ContentRules::from_options(&options);
    let pacing = native_probe_control::ProbePacing::new();
    let row = [("link".into(), format!("http://{address}/private-test"))]
        .into_iter()
        .collect::<Record>();
    let result = probe_one(
        &row,
        &options,
        &client,
        &content,
        &pacing,
        1000.0,
        &AtomicBool::new(false),
    );
    server.join().unwrap();
    assert_eq!(text(&result, &["probe_outcome"]), "web_alive");
    assert_eq!(text(&result, &["probe_attempts"]), "1");
    assert_eq!(text(&result, &["probe_body_complete"]), "true");
}

#[test]
fn private_literal_policy_preserves_public_ipv6_mapped_ipv4_and_domain_paths() {
    for url in [
        "https://[2606:4700:4700::1111]/",
        "http://[::ffff:8.8.8.8]/",
        "http://8.8.8.8/",
        "https://public.example.test/A?x=1",
    ] {
        assert!(
            !non_public_literal(url),
            "public/domain route changed: {url}"
        );
    }
    for url in [
        "http://127.0.0.1/",
        "http://10.0.0.1/",
        "http://localhost/",
        "http://service.internal/",
    ] {
        assert!(
            non_public_literal(url),
            "existing private gate changed: {url}"
        );
    }
}
