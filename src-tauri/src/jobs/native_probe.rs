const PROBE_OUTCOMES: &[&str] = &[
    "web_alive",
    "web_restricted",
    "browser_render_required",
    "virtual_host_required",
    "web_abnormal",
    "tcp_alive_non_http",
    "blocked_content",
    "unreachable",
    "skipped",
];
const PROBE_FIELDS: &[&str] = &[
    "probe_checked_at",
    "probe_input_url",
    "probe_effective_url",
    "probe_outcome",
    "probe_entry_state",
    "probe_status_code",
    "probe_title",
    "probe_content_type",
    "probe_server",
    "probe_latency_ms",
    "probe_tls_verified",
    "probe_attempts",
    "probe_body_truncated",
    "probe_body_complete",
    "probe_body_limit",
    "probe_content_encoding",
    "probe_content_range",
    "probe_response_fingerprint",
    "probe_network_elapsed_ms",
    "probe_total_timeout_ms",
    "probe_error",
    "content_category",
    "content_risk_score",
    "content_matches",
    "body_sha256",
    "probe_body_bytes",
    "probe_virtual_host",
    "probe_redirect_scope",
    "has_login",
    "has_register",
];

#[derive(Clone, Debug)]
pub struct ProbeOptions {
    pub workers: usize,
    pub timeout: Duration,
    pub total_timeout: Duration,
    pub retries: usize,
    pub max_body_bytes: usize,
    pub include_other: bool,
    pub include_weak: bool,
    pub allow_private: bool,
    pub strict_tls: bool,
    pub scheme_fallback: bool,
    pub content_threshold: i64,
    pub replace_default_content_rules: bool,
    pub gambling_keywords: Vec<String>,
    pub porn_keywords: Vec<String>,
    pub negative_keywords: Vec<String>,
    pub custom_keywords: Vec<String>,
    pub priority_rate: f64,
    pub other_rate: f64,
    pub per_host_interval: Duration,
}

fn build_probe_url(row: &Record) -> Option<String> {
    super::asset_identity::endpoint_url(
        &text(row, &["link"]),
        &text(row, &["host"]),
        &text(row, &["protocol"]),
        &text(row, &["ip"]),
        &text(row, &["port"]),
    )
}

fn non_public_literal(url: &str) -> bool {
    let normalized = url
        .replace("tcp+web://", "http://")
        .replace("tcp+tls://", "https://")
        .replace("tcp://", "http://");
    let Ok(parsed) = reqwest::Url::parse(&normalized) else {
        return true;
    };
    let Some(host) = parsed.host_str() else {
        return true;
    };
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".lan")
    {
        return true;
    }
    // URL host_str retains IPv6 brackets, unlike IpAddr::from_str. Treat
    // mapped IPv4 with the same private-address policy as its IPv4 endpoint.
    host.trim_matches(['[', ']'])
        .parse::<IpAddr>()
        .map(|ip| match ip {
            IpAddr::V4(ip) => {
                ip.is_private()
                    || ip.is_loopback()
                    || ip.is_link_local()
                    || ip.is_unspecified()
                    || ip.is_broadcast()
            }
            IpAddr::V6(ip) => {
                ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
                    || ip.to_ipv4_mapped().is_some_and(|ip| {
                        ip.is_private()
                            || ip.is_loopback()
                            || ip.is_link_local()
                            || ip.is_unspecified()
                            || ip.is_broadcast()
                    })
            }
        })
        .unwrap_or(false)
}

fn empty_probe(url: &str, outcome: &str, error: &str) -> Record {
    let mut result = PROBE_FIELDS
        .iter()
        .map(|field| ((*field).to_string(), String::new()))
        .collect::<Record>();
    result.insert(
        "probe_checked_at".into(),
        chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
    );
    result.insert("probe_input_url".into(), url.into());
    result.insert("probe_effective_url".into(), url.into());
    result.insert("probe_outcome".into(), outcome.into());
    result.insert("probe_error".into(), error.into());
    result.insert("content_category".into(), "not_checked".into());
    result.insert("content_risk_score".into(), "0".into());
    result.insert("probe_attempts".into(), "0".into());
    result
}

fn html_title(body: &str) -> String {
    let lower = body.to_ascii_lowercase();
    let Some(start) = lower.find("<title") else {
        return String::new();
    };
    let Some(open) = lower[start..].find('>').map(|value| start + value + 1) else {
        return String::new();
    };
    let Some(end) = lower[open..].find("</title").map(|value| open + value) else {
        return String::new();
    };
    body[open..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect()
}

fn portal_flags(url: &str, title: &str, body: &str) -> (&'static str, &'static str) {
    let mut login = false;
    let mut register = false;
    note_portal_url(&mut login, &mut register, url);
    note_portal_text(&mut login, &mut register, title, true);
    scan_portal_markup(body, &mut login, &mut register);
    (
        if login { "1" } else { "0" },
        if register { "1" } else { "0" },
    )
}

fn cached_probe_is_current(row: &Record) -> bool {
    let outcome = text(row, &["probe_outcome"]);
    let needs_portal = matches!(
        outcome.as_str(),
        "web_alive"
            | "web_restricted"
            | "browser_render_required"
            | "virtual_host_required"
            | "blocked_content"
            | "web_abnormal"
    );
    if !needs_portal {
        return true;
    }
    let login = text(row, &["has_login"]);
    let register = text(row, &["has_register"]);
    matches!(login.as_str(), "0" | "1")
        && matches!(register.as_str(), "0" | "1")
        && matches!(
            text(row, &["probe_body_complete"]).as_str(),
            "true" | "false"
        )
}

fn note_portal_url(login: &mut bool, register: &mut bool, raw: &str) {
    let path = url_path(raw);
    if path_has_login(&path) {
        *login = true;
    }
    if path_has_register(&path) {
        *register = true;
    }
}

fn note_portal_text(login: &mut bool, register: &mut bool, raw: &str, allow_admin_label: bool) {
    let value = collapse_ws(&raw.to_ascii_lowercase());
    if value.is_empty() {
        return;
    }
    if value.contains("登录")
        || value.contains("登入")
        || value.contains("登陆")
        || value.contains("sign in")
        || value.contains("signin")
        || value.contains("log in")
        || value.contains("login")
        || (allow_admin_label && value.chars().count() <= 24 && value.contains("后台"))
    {
        *login = true;
    }
    if value.contains("注册")
        || value.contains("註冊")
        || value.contains("sign up")
        || value.contains("signup")
        || value.contains("register")
        || value.contains("create account")
    {
        *register = true;
    }
}

fn collapse_ws(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn url_path(raw: &str) -> String {
    let trimmed = raw.trim();
    let no_query = trimmed.split(['#', '?']).next().unwrap_or(trimmed);
    if let Some((_, rest)) = no_query.split_once("://") {
        return rest
            .find('/')
            .map(|index| rest[index..].to_ascii_lowercase())
            .unwrap_or_else(|| "/".into());
    }
    if let Some(rest) = no_query.strip_prefix("//") {
        return rest
            .find('/')
            .map(|index| rest[index..].to_ascii_lowercase())
            .unwrap_or_else(|| "/".into());
    }
    no_query.to_ascii_lowercase()
}

fn path_has_login(path: &str) -> bool {
    const NEEDLES: &[&str] = &[
        "/login", "login.", "signin", "sign-in", "sign_in", "logon", "log-in", "log_in",
    ];
    NEEDLES.iter().any(|needle| path.contains(needle))
        || path_segments(path).any(|segment| {
            matches!(
                segment,
                "login" | "signin" | "logon" | "admin" | "auth" | "authentication"
            )
        })
}

fn path_has_register(path: &str) -> bool {
    const NEEDLES: &[&str] = &[
        "/register",
        "register.",
        "signup",
        "sign-up",
        "sign_up",
        "/regist",
    ];
    NEEDLES.iter().any(|needle| path.contains(needle))
        || path_segments(path)
            .any(|segment| matches!(segment, "register" | "signup" | "regist" | "registration"))
}

fn path_segments(path: &str) -> impl Iterator<Item = &str> {
    path.split(['/', '.', '-', '_', ';'])
        .filter(|segment| !segment.is_empty())
}

fn scan_portal_markup(body: &str, login: &mut bool, register: &mut bool) {
    let lower = body.to_ascii_lowercase();
    let mut index = 0usize;
    while index < lower.len() {
        let rest = &lower[index..];
        if starts_tag(rest, "script") {
            index += skip_until(rest, "</script>");
            continue;
        }
        if starts_tag(rest, "style") {
            index += skip_until(rest, "</style>");
            continue;
        }
        if let Some(name) = control_tag_name(rest) {
            let Some(close) = rest.find('>') else {
                // A truncated page can end inside a tag. len-1 is not a char boundary
                // when that ending is Chinese, and slicing there panics the probe thread.
                apply_control(name, rest, login, register);
                index += rest.len();
                continue;
            };
            apply_control(name, &rest[..close], login, register);
            if matches!(name, "a" | "button" | "label") {
                let inner = &rest[close + 1..];
                let end = inner.find('<').unwrap_or(inner.len());
                note_portal_text(login, register, prefix_bytes(inner, end.min(80)), true);
            }
            index += close + 1;
            continue;
        }
        index += rest
            .chars()
            .next()
            .map(|character| character.len_utf8())
            .unwrap_or(1);
    }
}

fn starts_tag(rest: &str, name: &str) -> bool {
    let marker = format!("<{name}");
    rest.starts_with(&marker) && tag_boundary(rest, marker.len())
}

fn tag_boundary(rest: &str, at: usize) -> bool {
    match rest[at..].chars().next() {
        None => true,
        Some(character) => {
            !character.is_ascii_alphanumeric() && character != '-' && character != '_'
        }
    }
}

fn control_tag_name(rest: &str) -> Option<&'static str> {
    ["button", "input", "form", "label", "a"]
        .into_iter()
        .find(|name| starts_tag(rest, name))
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_string())
        })
        .unwrap_or_else(|| "未提供异常说明".into())
}

fn skip_until(rest: &str, closer: &str) -> usize {
    rest.find(closer)
        .map(|position| position + closer.len())
        .unwrap_or(rest.len())
        .max(1)
}

fn apply_control(name: &str, tag: &str, login: &mut bool, register: &mut bool) {
    if name == "input" && attr_is(tag, "type", "password") {
        *login = true;
    }
    for key in ["href", "action", "formaction", "onclick", "onsubmit"] {
        if let Some(value) = attr_value(tag, key) {
            note_portal_url(login, register, value);
        }
    }
    for key in ["value", "placeholder", "aria-label", "title", "alt"] {
        if let Some(value) = attr_value(tag, key) {
            note_portal_text(login, register, value, true);
        }
    }
}

fn attr_is(tag: &str, name: &str, expected: &str) -> bool {
    attr_value(tag, name).is_some_and(|value| value == expected)
}

fn attr_value<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=");
    let mut search = tag;
    while let Some(position) = search.find(&needle) {
        let boundary = position == 0
            || match search[..position].chars().next_back() {
                None => true,
                Some(character) => {
                    !character.is_ascii_alphanumeric() && character != '-' && character != '_'
                }
            };
        if boundary {
            return Some(read_attr(&search[position + needle.len()..]));
        }
        search = &search[position + needle.len()..];
    }
    None
}

fn prefix_bytes(value: &str, max_bytes: usize) -> &str {
    if max_bytes >= value.len() {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn read_attr(raw: &str) -> &str {
    let Some(first) = raw.chars().next() else {
        return "";
    };
    if first == '"' || first == '\'' {
        let rest = &raw[first.len_utf8()..];
        let end = rest.find(first).unwrap_or(rest.len());
        return &rest[..end];
    }
    let end = raw
        .find(|character: char| character.is_whitespace() || character == '>')
        .unwrap_or(raw.len());
    &raw[..end]
}

fn javascript_shell(body: &str, content_type: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    if !content_type.to_ascii_lowercase().contains("html") && !lower.contains("<html") {
        return false;
    }
    let scripts = lower.matches("<script").count();
    let root = [
        "id=\"app\"",
        "id='app'",
        "id=\"root\"",
        "id='root'",
        "id=\"__next\"",
        "id=\"__nuxt\"",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    let mut visible = String::new();
    let mut in_tag = false;
    for character in body.chars() {
        if character == '<' {
            in_tag = true;
        } else if character == '>' {
            in_tag = false;
        } else if !in_tag && !character.is_whitespace() {
            visible.push(character);
        }
    }
    visible.chars().count() < 40 && scripts > 0 && (root || scripts >= 2)
}

fn tcp_alive(url: &str, source_ip: &str, timeout: Duration, allow_private: bool) -> bool {
    let Ok(parsed) = reqwest::Url::parse(
        url.replace("tcp+web://", "http://")
            .replace("tcp+tls://", "https://")
            .replace("tcp://", "http://")
            .as_str(),
    ) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let Some(port) = parsed.port_or_known_default() else {
        return false;
    };
    // Never perform another blocking system DNS lookup after HTTP failed.
    // reqwest's own request deadline already covers its resolution; this
    // fallback uses only an observed FOFA IP or an explicitly literal URL IP.
    let ip = host
        .trim_matches(['[', ']'])
        .parse::<IpAddr>()
        .ok()
        .or_else(|| source_ip.parse::<IpAddr>().ok());
    ip.is_some_and(|ip| {
        let address = std::net::SocketAddr::new(ip, port);
        (allow_private || !non_public_literal(&format!("http://{address}")))
            && TcpStream::connect_timeout(&address, timeout).is_ok()
    })
}

fn deadline_probe(input_url: &str, spent: Duration, budget: Duration, attempts: usize) -> Record {
    let mut result = empty_probe(
        input_url,
        "unreachable",
        "单个入口的总网络时间预算已耗尽，已停止重试和协议回退",
    );
    result.insert("probe_entry_state".into(), "deadline_exceeded".into());
    result.insert(
        "probe_network_elapsed_ms".into(),
        spent.as_millis().to_string(),
    );
    result.insert(
        "probe_total_timeout_ms".into(),
        budget.as_millis().to_string(),
    );
    result.insert("probe_attempts".into(), attempts.to_string());
    result
}

fn probe_one(
    row: &Record,
    options: &ProbeOptions,
    client: &reqwest::blocking::Client,
    content_rules: &native_content::ContentRules,
    pacing: &native_probe_control::ProbePacing,
    rate: f64,
    cancel: &AtomicBool,
) -> Record {
    let Some(input_url) = build_probe_url(row) else {
        return empty_probe("", "skipped", "missing_or_non_web_url");
    };
    if !options.allow_private && non_public_literal(&input_url) {
        return empty_probe(&input_url, "skipped", "non_public_ip");
    }
    let tcp_mode = input_url.starts_with("tcp+") || input_url.starts_with("tcp://");
    let normalized = input_url
        .replace("tcp+web://", "http://")
        .replace("tcp+tls://", "https://");
    let mut candidates = vec![normalized.clone()];
    if tcp_mode || options.scheme_fallback {
        // One scheme alternation, used by both the tcp+ form and the fallback.
        candidates.push(if normalized.starts_with("https://") {
            normalized.replacen("https://", "http://", 1)
        } else {
            normalized.replacen("http://", "https://", 1)
        });
    }
    let mut attempts = 0usize;
    // Deliberate rate/per-host queue waits do not count as slow target response.
    // All actual network work, including body streaming and fallback, shares
    // this one budget rather than granting a fresh timeout to every retry.
    let mut network_spent = Duration::ZERO;
    let mut last_error = String::new();
    let mut abnormal = None;
    for candidate in candidates {
        for _ in 0..=options.retries {
            let host = reqwest::Url::parse(&candidate)
                .ok()
                .and_then(|url| url.host_str().map(str::to_string))
                .unwrap_or_default();
            if !pacing.wait(&host, rate, options.per_host_interval, cancel) {
                return empty_probe(&input_url, "skipped", "__CANCELLED__");
            }
            let remaining = options.total_timeout.saturating_sub(network_spent);
            if remaining.is_zero() {
                return deadline_probe(&input_url, network_spent, options.total_timeout, attempts);
            }
            attempts += 1;
            let started = Instant::now();
            match client
                .get(&candidate)
                .timeout(options.timeout.min(remaining))
                .header(
                    "User-Agent",
                    "Mozilla/5.0 (compatible; AuthorizedAssetVerifier/2.0)",
                )
                .header("Accept-Encoding", "identity")
                .send()
            {
                Ok(mut response) => {
                    let status = response.status().as_u16();
                    let final_url = response.url().to_string();
                    let content_type = response
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    let server = response
                        .headers()
                        .get(reqwest::header::SERVER)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    let content_encoding = response
                        .headers()
                        .get(reqwest::header::CONTENT_ENCODING)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    let content_range = response
                        .headers()
                        .get(reqwest::header::CONTENT_RANGE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    let mut bytes = Vec::new();
                    let read_result = std::io::Read::by_ref(&mut response)
                        .take(options.max_body_bytes.saturating_add(1) as u64)
                        .read_to_end(&mut bytes);
                    network_spent = network_spent.saturating_add(started.elapsed());
                    if let Err(error) = read_result {
                        if network_spent >= options.total_timeout {
                            return deadline_probe(
                                &input_url,
                                network_spent,
                                options.total_timeout,
                                attempts,
                            );
                        }
                        last_error = error.to_string();
                        continue;
                    }
                    let truncated = bytes.len() > options.max_body_bytes;
                    bytes.truncate(options.max_body_bytes);
                    let body_hash = format!("{:x}", Sha256::digest(&bytes));
                    // A limit-sized response is complete only after the extra
                    // byte read reaches EOF. A shared prefix is never equality.
                    let response_fingerprint = if !truncated && !bytes.is_empty() {
                        full_response_fingerprint(status, bytes.len(), &body_hash, &content_type)
                    } else {
                        String::new()
                    };
                    let body = String::from_utf8_lossy(&bytes);
                    let title = html_title(&body);
                    let (has_login, has_register) = portal_flags(&final_url, &title, &body);
                    let (content_category, content_score, content_matches) = content_rules
                        .classify(&final_url, &title, &body, options.content_threshold);
                    let state = if (200..400).contains(&status) || [401, 403].contains(&status) {
                        "usable_or_restricted"
                    } else if [404, 410].contains(&status) {
                        "reachable_but_path_missing"
                    } else if status >= 500 {
                        "reachable_server_error"
                    } else {
                        "reachable_client_error"
                    };
                    let outcome = if matches!(content_category.as_str(), "gambling" | "porn") {
                        "blocked_content"
                    } else if [401, 403, 407, 429].contains(&status) {
                        "web_restricted"
                    } else if (200..300).contains(&status) && bytes.is_empty() {
                        "web_abnormal"
                    } else if (200..300).contains(&status) && javascript_shell(&body, &content_type)
                    {
                        "browser_render_required"
                    } else if (200..400).contains(&status) {
                        "web_alive"
                    } else {
                        "web_abnormal"
                    };
                    let mut result = empty_probe(&input_url, outcome, "");
                    for (key, value) in [
                        ("probe_effective_url", final_url),
                        ("probe_entry_state", state.into()),
                        ("probe_status_code", status.to_string()),
                        ("probe_title", title),
                        ("probe_content_type", content_type),
                        ("probe_server", server),
                        (
                            "probe_latency_ms",
                            started.elapsed().as_millis().to_string(),
                        ),
                        (
                            "probe_tls_verified",
                            if candidate.starts_with("https://") {
                                options.strict_tls.to_string()
                            } else {
                                "not_applicable".into()
                            },
                        ),
                        ("probe_attempts", attempts.to_string()),
                        ("probe_body_truncated", truncated.to_string()),
                        ("probe_body_complete", (!truncated).to_string()),
                        ("probe_body_limit", options.max_body_bytes.to_string()),
                        ("probe_content_encoding", content_encoding),
                        ("probe_content_range", content_range),
                        ("probe_response_fingerprint", response_fingerprint),
                        (
                            "probe_network_elapsed_ms",
                            network_spent.as_millis().to_string(),
                        ),
                        (
                            "probe_total_timeout_ms",
                            options.total_timeout.as_millis().to_string(),
                        ),
                        ("content_category", content_category),
                        ("content_risk_score", content_score.to_string()),
                        ("content_matches", content_matches),
                        ("body_sha256", body_hash),
                        ("probe_body_bytes", bytes.len().to_string()),
                        ("has_login", has_login.into()),
                        ("has_register", has_register.into()),
                    ] {
                        result.insert(key.into(), value);
                    }
                    if outcome != "web_abnormal" {
                        return result;
                    }
                    abnormal = Some(result);
                }
                Err(error) => {
                    network_spent = network_spent.saturating_add(started.elapsed());
                    if network_spent >= options.total_timeout {
                        return deadline_probe(
                            &input_url,
                            network_spent,
                            options.total_timeout,
                            attempts,
                        );
                    }
                    last_error = error.to_string();
                }
            }
        }
    }
    if let Some(mut result) = abnormal {
        result.insert("probe_attempts".into(), attempts.to_string());
        return result;
    }
    if cancel.load(Ordering::Relaxed) {
        return empty_probe(&input_url, "skipped", "__CANCELLED__");
    }
    let remaining = options.total_timeout.saturating_sub(network_spent);
    if remaining.is_zero() {
        return deadline_probe(&input_url, network_spent, options.total_timeout, attempts);
    }
    let tcp_started = Instant::now();
    let tcp_success = tcp_alive(
        &input_url,
        &text(row, &["ip"]),
        options.timeout.min(remaining),
        options.allow_private,
    );
    network_spent = network_spent.saturating_add(tcp_started.elapsed());
    if tcp_success {
        let mut result = empty_probe(
            &input_url,
            "tcp_alive_non_http",
            &format!("Web协议未识别，仅确认TCP存活：{last_error}"),
        );
        result.insert("probe_entry_state".into(), "tcp_alive_non_http".into());
        result.insert("probe_attempts".into(), (attempts + 1).to_string());
        result.insert("content_category".into(), "not_applicable".into());
        result.insert(
            "probe_network_elapsed_ms".into(),
            network_spent.as_millis().to_string(),
        );
        result.insert(
            "probe_total_timeout_ms".into(),
            options.total_timeout.as_millis().to_string(),
        );
        return result;
    }
    if network_spent >= options.total_timeout {
        return deadline_probe(
            &input_url,
            network_spent,
            options.total_timeout,
            attempts + 1,
        );
    }
    let mut result = empty_probe(&input_url, "unreachable", &last_error);
    result.insert("probe_attempts".into(), (attempts + 1).to_string());
    result.insert(
        "probe_network_elapsed_ms".into(),
        network_spent.as_millis().to_string(),
    );
    result.insert(
        "probe_total_timeout_ms".into(),
        options.total_timeout.as_millis().to_string(),
    );
    result
}

fn row_identity(row: &Record) -> String {
    format!(
        "{}\u{1f}{}",
        text(row, &["company"]),
        if !text(row, &["asset_key"]).is_empty() {
            text(row, &["asset_key"])
        } else {
            format!(
                "{}\u{1f}{}\u{1f}{}\u{1f}{}",
                text(row, &["link"]),
                text(row, &["host"]),
                text(row, &["ip"]),
                text(row, &["port"])
            )
        }
    )
}
fn other_bucket(row: &Record) -> &'static str {
    if !text(row, &["company"]).is_empty()
        && text(row, &["company"]) == text(row, &["cert.subject.org"])
    {
        "Q2"
    } else if ["标题全称:", "正文全称:", "独有标题:", "独有正文:"]
        .iter()
        .any(|marker| text(row, &["evidence"]).contains(marker))
    {
        "Q3"
    } else {
        "Q1"
    }
}

pub fn probe_assets(
    refined_dir: &Path,
    other_input: &Path,
    output: &Path,
    options: ProbeOptions,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize),
) -> Result<serde_json::Value, String> {
    fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let sources = [
        ("P1", refined_dir.join("P1_active_strong.csv")),
        ("P2", refined_dir.join("P2_strong_needs_validation.csv")),
        ("P3", refined_dir.join("P3_name_candidates.csv")),
    ];
    let mut stages: Vec<(String, Vec<String>, Vec<Record>)> = Vec::new();
    let mut priority_ids = HashSet::new();
    for (stage, path) in sources {
        let (fields, rows) = read_csv(&path)?;
        priority_ids.extend(rows.iter().map(row_identity));
        stages.push((stage.into(), fields, rows));
    }
    if options.include_other || options.include_weak {
        let (fields, rows) = read_csv(other_input)?;
        for stage in ["Q2", "Q3", "Q1"] {
            if (stage == "Q1" && !options.include_weak) || (stage != "Q1" && !options.include_other)
            {
                continue;
            }
            stages.push((
                stage.into(),
                fields.clone(),
                rows.iter()
                    .filter(|row| {
                        !priority_ids.contains(&row_identity(row)) && other_bucket(row) == stage
                    })
                    .cloned()
                    .collect(),
            ));
        }
    }
    let merged_duplicates = merge_probe_stages(&mut stages);
    let total = stages.iter().map(|(_, _, rows)| rows.len()).sum::<usize>();
    // Cache identity includes the full selected input and all effective probe
    // options. Changing a rule, URL, retry policy or TLS policy starts a new
    // journal instead of reusing incompatible historical outcomes.
    let mut digest = Sha256::new();
    digest.update(format!("native-probe-v7-full-response-proof|{options:?}"));
    for (stage, fields, rows) in &stages {
        digest.update(stage.as_bytes());
        digest.update(serde_json::to_vec(fields).map_err(|error| error.to_string())?);
        for row in rows {
            let canonical = row.iter().collect::<BTreeMap<_, _>>();
            digest.update(serde_json::to_vec(&canonical).map_err(|error| error.to_string())?);
        }
    }
    let fingerprint = format!("{:x}", digest.finalize());
    let journal_path = output.join(format!("probe-{fingerprint}.jsonl"));
    let mut cached = HashMap::<(String, String), Record>::new();
    if let Ok(file) = fs::File::open(&journal_path) {
        for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                if let (Some(stage), Some(row)) = (
                    value.get("stage").and_then(|v| v.as_str()),
                    value.get("row"),
                ) {
                    if let Ok(row) = serde_json::from_value::<Record>(row.clone()) {
                        cached.insert((stage.to_string(), row_identity(&row)), row);
                    }
                }
            }
        }
    }
    let mut journal = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&journal_path)
        .map_err(|error| error.to_string())?;
    // Separate a potentially truncated last record after an interrupted write.
    journal
        .write_all(b"\n")
        .map_err(|error| error.to_string())?;
    let mut completed = 0usize;
    let mut reused = 0usize;
    let mut stage_summary = serde_json::Map::new();
    let content_rules = Arc::new(native_content::ContentRules::from_options(&options));
    let pacing = Arc::new(native_probe_control::ProbePacing::new());
    let client = Arc::new(
        reqwest::blocking::Client::builder()
            .timeout(options.timeout)
            .danger_accept_invalid_certs(!options.strict_tls)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|error| error.to_string())?,
    );
    for (stage, source_fields, rows) in stages {
        let mut output_fields = source_fields;
        for field in PROBE_FIELDS {
            if !output_fields.iter().any(|existing| existing == field) {
                output_fields.push((*field).into());
            }
        }
        let mut buckets: HashMap<String, Vec<Record>> = PROBE_OUTCOMES
            .iter()
            .map(|outcome| ((*outcome).into(), Vec::new()))
            .collect();
        let mut pending = Vec::new();
        for row in &rows {
            if let Some(previous) = cached.remove(&(stage.clone(), row_identity(row))) {
                let outcome = text(&previous, &["probe_outcome"]);
                if PROBE_OUTCOMES.contains(&outcome.as_str()) && cached_probe_is_current(&previous)
                {
                    buckets.entry(outcome).or_default().push(previous);
                    completed += 1;
                    reused += 1;
                    continue;
                }
            }
            pending.push(row.clone());
        }
        progress(completed, total);
        let rate = if stage.starts_with('P') {
            options.priority_rate
        } else {
            options.other_rate
        };
        if !pending.is_empty() {
            if cancel.load(Ordering::Relaxed) {
                return Err("__CANCELLED__".into());
            }
            let completed_before_pending = completed;
            let next = std::sync::atomic::AtomicUsize::new(0);
            std::thread::scope(|scope| -> Result<(), String> {
                let worker_count = options.workers.clamp(1, 256).min(pending.len());
                let (sender, receiver) = std::sync::mpsc::sync_channel(worker_count * 2);
                let mut handles = Vec::new();
                for _ in 0..worker_count {
                    let sender = sender.clone();
                    let client = &client;
                    let options = &options;
                    let content_rules = &content_rules;
                    let pacing = &pacing;
                    let pending = &pending;
                    let next = &next;
                    handles.push(scope.spawn(move || {
                        // Refill each available worker immediately. A single
                        // slow URL no longer stalls the next batch of fast URLs.
                        while !cancel.load(Ordering::Relaxed) {
                            let index = next.fetch_add(1, Ordering::Relaxed);
                            let Some(row) = pending.get(index) else { break };
                            let result = probe_one(
                                row,
                                options,
                                client,
                                content_rules,
                                pacing,
                                rate,
                                cancel,
                            );
                            if sender.send((row.clone(), result)).is_err() {
                                break;
                            }
                        }
                    }));
                }
                drop(sender);
                // Receive by completion order, not launch order: a slow first
                // host must not hide all other completed requests from the UI.
                for (mut row, result) in receiver {
                    if text(&result, &["probe_error"]) == "__CANCELLED__" {
                        continue;
                    }
                    let outcome = text(&result, &["probe_outcome"]);
                    row.extend(result);
                    let mut line = serde_json::to_vec(&json!({"stage": stage, "row": row}))
                        .map_err(|error| error.to_string())?;
                    line.push(b'\n');
                    journal
                        .write_all(&line)
                        .and_then(|_| journal.flush())
                        .map_err(|error| error.to_string())?;
                    buckets
                        .entry(if PROBE_OUTCOMES.contains(&outcome.as_str()) {
                            outcome
                        } else {
                            "unreachable".into()
                        })
                        .or_default()
                        .push(row);
                    completed += 1;
                    progress(completed, total);
                }
                for handle in handles {
                    handle.join().map_err(|payload| {
                        format!("Rust 探测工作线程异常退出：{}", panic_message(payload))
                    })?;
                }
                Ok(())
            })?;
            if cancel.load(Ordering::Relaxed) {
                return Err("__CANCELLED__".into());
            }
            if completed - completed_before_pending != pending.len() {
                return Err("Rust 探测结果不完整，已保留检查点；请续跑补齐缺失入口".into());
            }
        }
        let mut counts = serde_json::Map::new();
        for outcome in PROBE_OUTCOMES {
            let bucket = buckets.remove(*outcome).unwrap_or_default();
            counts.insert((*outcome).into(), json!(bucket.len()));
            write_csv(
                &output.join(format!("{stage}_{outcome}.csv")),
                &output_fields,
                &bucket,
            )?;
        }
        stage_summary.insert(
            stage,
            json!({"complete": true, "selected_rows": rows.len(), "counts": counts}),
        );
    }
    let checkpoint = json!({"version": 2, "probe_version": 7, "native": true, "fingerprint":fingerprint,"reused_results":reused,"merged_duplicates":merged_duplicates,"stage_order": stage_summary.keys().collect::<Vec<_>>(), "stages": stage_summary});
    fs::write(
        output.join("checkpoint.json"),
        serde_json::to_vec_pretty(&checkpoint).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        output.join("summary.json"),
        serde_json::to_vec_pretty(&checkpoint).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(checkpoint)
}

#[cfg(test)]
mod portal_flag_tests {
    use super::portal_flags;

    #[test]
    fn portal_flags_read_login_and_register_controls() {
        let (login, register) = portal_flags(
            "http://127.0.0.1/app",
            "业务系统",
            r#"<html><a href="/user/login">进入</a><button>注册账号</button><script>var login = "/admin";</script><p>正文里的登录和注册不算入口</p></html>"#,
        );
        assert_eq!((login, register), ("1", "1"));

        let (login, register) =
            portal_flags("http://127.0.0.1/login.jsp", "首页", "<html>hello</html>");
        assert_eq!((login, register), ("1", "0"));

        let (login, register) = portal_flags(
            "http://127.0.0.1/news",
            "行业新闻",
            "<html><p>如何登录并注册账号</p><script>location='/login'</script></html>",
        );
        assert_eq!((login, register), ("0", "0"));

        let (login, register) = portal_flags(
            "http://127.0.0.1/",
            "欢迎",
            r#"<form><input type="password" name="pwd"></form>"#,
        );
        assert_eq!((login, register), ("1", "0"));

        let (login, register) = portal_flags("http://127.0.0.1/", "系统登录", "<html></html>");
        assert_eq!((login, register), ("1", "0"));

        let label = "业务办理入口".repeat(20);
        let long_page = format!(
            "<html><a href=\"/portal\">{label}</a><button>{label}</button><label>{label}</label><a title=\"{label}\""
        );
        let (login, register) = portal_flags("http://127.0.0.1/app", "首页", &long_page);
        assert_eq!((login, register), ("0", "0"));

        let open_tag = format!("<a title=\"{}\"", "系统登录");
        let (login, register) = portal_flags("http://127.0.0.1/app", "首页", &open_tag);
        assert_eq!((login, register), ("1", "0"));
    }
}

#[cfg(test)]
#[path = "native_probe_private_tests.rs"]
mod private_address_tests;
