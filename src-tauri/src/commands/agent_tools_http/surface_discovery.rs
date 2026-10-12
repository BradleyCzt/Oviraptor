fn agent_extract_script_srcs(body: &str) -> Vec<String> {
    let lower = body.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut search_from = 0;
    while let Some(rel) = lower[search_from..].find("<script") {
        let start = search_from + rel;
        let Some(tag_end_rel) = lower[start..].find('>') else {
            break;
        };
        let tag_end = start + tag_end_rel;
        let tag = &body[start..=tag_end];
        let tag_l = tag.to_ascii_lowercase();
        if let Some(src_rel) = tag_l.find("src=") {
            let after = &tag[src_rel + 4..];
            let after_trim = after.trim_start();
            let (quote, rest) = match after_trim.chars().next() {
                Some('\'') => ('\'', &after_trim[1..]),
                Some('"') => ('"', &after_trim[1..]),
                _ => {
                    search_from = tag_end + 1;
                    continue;
                }
            };
            if let Some(end) = rest.find(quote) {
                let src = rest[..end].trim();
                if !src.is_empty() {
                    out.push(src.to_string());
                }
            }
        }
        search_from = tag_end + 1;
    }
    out
}

fn agent_resolve_against(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() || href.starts_with("data:") || href.starts_with("javascript:") {
        return None;
    }
    if let Ok(abs) = reqwest::Url::parse(href) {
        match abs.scheme() {
            "http" | "https" => return Some(abs.to_string()),
            // U82: inventory realtime endpoints as same-origin https/http paths.
            "ws" | "wss" => {
                let mut rewritten = abs;
                let httpish = if rewritten.scheme() == "ws" {
                    "http"
                } else {
                    "https"
                };
                rewritten.set_scheme(httpish).ok()?;
                return Some(rewritten.to_string());
            }
            _ => {}
        }
    }
    let base = reqwest::Url::parse(base).ok()?;
    base.join(href).ok().map(|u| u.to_string())
}

fn agent_looks_like_account_check(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let path = reqwest::Url::parse(&lower)
        .ok()
        .map(|u| u.path().to_string())
        .unwrap_or_else(|| lower.clone());
    [
        "checkaccount",
        "checkuser",
        "userexist",
        "accountexist",
        "verifyuser",
        "accountavailable",
        "isuserexist",
    ]
    .iter()
    .any(|needle| path.contains(needle))
}

fn agent_membership_token(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.len() > 200 {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    // {"R":"OK"} / {"R":"NO"} and close variants
    for (needle, token) in [
        ("\"r\":\"ok\"", "OK"),
        ("\"r\": \"ok\"", "OK"),
        ("\"r\":\"no\"", "NO"),
        ("\"r\": \"no\"", "NO"),
        ("\"exists\":true", "EXISTS"),
        ("\"exists\": false", "MISSING"),
        ("\"exists\":false", "MISSING"),
        ("\"exists\": true", "EXISTS"),
    ] {
        if lower.contains(needle) {
            return Some(token.to_string());
        }
    }
    None
}

fn agent_account_probe_url(url: &str) -> Option<String> {
    let mut parsed = reqwest::Url::parse(url).ok()?;
    let marker = "oviraptor_no_such_user_7f2a";
    let mut pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let account_keys = ["Account", "account", "UserName", "username", "user", "User", "loginName", "LoginName"];
    let mut replaced = false;
    for (k, v) in pairs.iter_mut() {
        if account_keys.iter().any(|n| k.eq_ignore_ascii_case(n)) {
            *v = marker.to_string();
            replaced = true;
        }
    }
    if !replaced {
        pairs.push(("Account".to_string(), marker.to_string()));
    }
    parsed.set_query(None);
    let query = pairs
        .into_iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");
    parsed.set_query(Some(&query));
    Some(parsed.to_string())
}



fn agent_is_static_surface_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let path_only = lower.split(['?', '#']).next().unwrap_or("");
    [
        ".css", ".js", ".map", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".webp",
        ".woff", ".woff2", ".ttf", ".eot", ".mp3", ".mp4", ".pdf", ".avif", ".bmp",
    ]
    .iter()
    .any(|ext| path_only.ends_with(ext))
}

/// Pull same-origin app paths out of HTML/JS so classic sites (onclick / $.ajax)
/// still feed the agent queue when the browser only saw one XHR.
/// U96: public-surface seeding mode. String harvest is live; CDP click/BFS
/// explore is not wired yet — do not advertise click-explore coverage.
const NATIVE_SURFACE_EXPLORE_MODE: &str = "string_only";

fn native_surface_explore_mode() -> &'static str {
    NATIVE_SURFACE_EXPLORE_MODE
}

fn native_surface_click_explore_ready() -> bool {
    native_surface_explore_mode() == "click_explore"
}

/// U97: CDP/DOM click-BFS entry. Not wired — callers must not invent candidates.
fn native_surface_click_explore_candidates(
    _dom_html: &str,
    _base_url: &str,
) -> Result<Vec<(String, String)>, String> {
    if !native_surface_click_explore_ready() {
        return Err("surface_click_explore_not_ready".into());
    }
    // Future: walk onclick/nav controls via CDP and return method/path pairs.
    Ok(Vec::new())
}

fn agent_harvest_surface_paths(body: &str, base_url: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push = |method: &str, raw: &str| {
        let raw = raw.trim().trim_matches(|c| c == '\'' || c == '"' || c == '`');
        if raw.is_empty()
            || raw.starts_with('#')
            || raw.starts_with("javascript:")
            || raw.contains('{')
            || raw.contains('}')
            || raw.contains("@{")
        {
            return;
        }
        let Some(abs) = agent_resolve_against(base_url, raw) else {
            return;
        };
        let Ok(parsed) = reqwest::Url::parse(&abs) else {
            return;
        };
        let Ok(base) = reqwest::Url::parse(base_url) else {
            return;
        };
        if parsed.host_str() != base.host_str() {
            return;
        }
        let path = normalized_investigation_path(parsed.as_str());
        if path.is_empty() || path == "/" || agent_is_static_surface_path(&path) {
            return;
        }
        // Skip obvious non-routes from greedy regex.
        if path.split('/').filter(|s| !s.is_empty()).count() < 1 {
            return;
        }
        let method = method.to_ascii_uppercase();
        if !out.iter().any(|(m, p)| m == &method && p == &path) {
            out.push((method, path));
        }
    };

    // Explicit ajax / navigation patterns — prefer POST when ajax-like.
    let patterns = [
        (r#"(?i)url\s*:\s*["']([^"']+)["']"#, "POST"),
        (r#"(?i)\burl\s*=\s*["'](/[^"']+)["']"#, "GET"),
        (r#"(?i)\baction\s*=\s*["'](/[^"']+)["']"#, "POST"),
        (r#"(?i)\bhref\s*=\s*["'](/[^"']+)["']"#, "GET"),
        (r#"(?i)setAttribute\(\s*["'](?:href|action|src)["']\s*,\s*["'](/[^"']+)["']"#, "GET"),
        (r#"(?i)\b(?:routerLink|ng-href)\s*=\s*["'](/[^"']+)["']"#, "GET"),
        (r#"(?i)xlink:href\s*=\s*["'](/[^"']+)["']"#, "GET"),
        (r#"(?i)<use\b[^>]*\bhref\s*=\s*["'](/[^"']+)["']"#, "GET"),
        (r#"(?i)\b(?:poster|cite)\s*=\s*["'](/[^"']+)["']"#, "GET"),
        (r#"(?i)\bsrcset\s*=\s*["']\s*(/[^\s,"']+)"#, "GET"),
        (r#"(?i)\bimagesrcset\s*=\s*["']\s*(/[^\s,"']+)"#, "GET"),
        (r#"(?i)<meta\b[^>]*(?:property|name)\s*=\s*["'](?:og:url|og:image|twitter:url|twitter:image)["'][^>]*content\s*=\s*["']([^"']+)["']"#, "GET"),
        (r#"(?i)\bbackground\s*=\s*["'](/[^"']+)["']"#, "GET"),
        (r#"(?i)url\(\s*["']?(/[^"')\s]+)["']?\s*\)"#, "GET"),
        (r#"(?i)\b(?:longdesc|usemap)\s*=\s*["'](/[^"']+)["']"#, "GET"),
        (r#"(?i)<base\b[^>]*\bhref\s*=\s*["'](/[^"']+)["']"#, "GET"),
        (r#"(?i)<meta\b[^>]*content\s*=\s*["']([^"']+)["'][^>]*(?:property|name)\s*=\s*["'](?:og:url|og:image|twitter:url|twitter:image)["']"#, "GET"),
        (r#"(?i)\.ajax\(\s*\{[^}]{0,200}?url\s*:\s*["']([^"']+)["']"#, "POST"),
        // jQuery / axios / http client shorthand — classic sites often skip .ajax({url:}).
        (r#"(?i)\$\.(?:get|post|getJSON)\(\s*["']([^"']+)["']"#, "POST"),
        (r#"(?i)\$\.(?:load|getScript)\(\s*["']([^"']+)["']"#, "GET"),
        (r#"(?i)\baxios\.(?:get|post|put|delete|patch)\(\s*["']([^"']+)["']"#, "POST"),
        (r#"(?i)\bhttps?\.(?:get|post|put|delete|patch|request)\(\s*["']([^"']+)["']"#, "POST"),
        (r#"(?i)\bfetch\(\s*["']([^"']+)["']"#, "GET"),
        // SPA routers + meta refresh — common public surfaces without classic href.
        (r#"(?i)\b(?:router\.(?:push|replace)|navigate)\(\s*["']([^"']+)["']"#, "GET"),
        (r#"(?i)\bpath\s*:\s*["'](/[^"']+)["']"#, "GET"),
        (r#"(?i)http-equiv\s*=\s*["']refresh["'][^>]*content\s*=\s*["'][^"']*url=([^"'\s>]+)"#, "GET"),
        (r#"(?i)content\s*=\s*["'][^"']*url=([^"'\s>]+)["'][^>]*http-equiv\s*=\s*["']refresh["']"#, "GET"),
        (r#"(?i)(?:location\.href|window\.location|location)\s*=\s*["']([^"']+)["']"#, "GET"),
        (r#"(?i)location\.(?:assign|replace)\(\s*["']([^"']+)["']"#, "GET"),
        (r#"(?i)history\.(?:pushState|replaceState)\(\s*[^,]+,\s*[^,]+,\s*["']([^"']+)["']"#, "GET"),
        (r#"(?i)(?:window\.)?open\(\s*["']([^"']+)["']"#, "GET"),
        (r#"(?i)tourl{1,2}\(\s*["']([^"']+)["']"#, "GET"),
        // Any onclick handler whose first string arg looks like a same-site path
        // (covers tourl/location/open and site-specific navigators).
        (r#"(?i)onclick\s*=\s*["'][^"']*?\(\s*['"]([^'"]+)['"]"#, "GET"),
        (r#"(?i)(?:href|action)\s*=\s*["']([^"']+)["']"#, "GET"),
        // HTML5 per-control form overrides.
        (r#"(?i)formaction\s*=\s*["']([^"']+)["']"#, "POST"),
        // data-* navigation hooks used by many admin skins.
        (r#"(?i)data-(?:href|url|path|link|action|route|api|endpoint|target)\s*=\s*["']([^"']+)["']"#, "GET"),

        // Classic XHR / realtime endpoints often miss href/ajax({url}).
        (r#"(?i)\.open\(\s*["'](?:GET|POST|PUT|DELETE|PATCH)["']\s*,\s*["']([^"']+)["']"#, "POST"),
        (r#"(?i)new\s+WebSocket\(\s*["']([^"']+)["']"#, "GET"),
        (r#"(?i)new\s+EventSource\(\s*["']([^"']+)["']"#, "GET"),

        (r#"(?i)navigator\.sendBeacon\(\s*["']([^"']+)["']"#, "POST"),
        (r#"(?i)\bimport\(\s*["']([^"']+)["']"#, "GET"),

        (r#"(?i)new\s+Worker\(\s*["']([^"']+)["']"#, "GET"),
        (r#"(?i)navigator\.serviceWorker\.register\(\s*["']([^"']+)["']"#, "GET"),

        (r#"(?i)\bping\s*=\s*["']([^"']+)["']"#, "POST"),

        // HTMX declarative fetches on classic and hybrid pages.
        (r#"(?i)\bhx-(?:get|post|put|patch|delete)\s*=\s*["']([^"']+)["']"#, "POST"),

        // Vue/Alpine click handlers with a first string path arg (mirrors onclick).
        (r#"(?i)(?:@click|v-on:click|x-on:click)\s*=\s*["'][^"']*?\(\s*['"]([^'"]+)['"]"#, "GET"),
        // U148: <track src="/..."> text-track endpoints (classic media gap).
        (r#"(?i)<track\b[^>]*\bsrc\s*=\s*["'](/[^"']+)["']"#, "GET"),
        // U149: <video|audio|source src="/..."> media endpoints (often stream routes).
        (r#"(?i)<(?:video|audio|source)\b[^>]*\bsrc\s*=\s*["'](/[^"']+)["']"#, "GET"),
        // U150: data-src lazy-load hook (not in the data-* nav alternation).
        (r#"(?i)\bdata-src\s*=\s*["'](/[^"']+)["']"#, "GET"),
        // U154: fetch(new Request("/...")) wrapper — plain fetch( misses the Request() wrap.
        (r#"(?i)new\s+Request\(\s*["'](/[^"']+)["']"#, "GET"),
        // U155: JSON/config endpoint keys ("api"/"endpoint"/...): "/..." — quoted key skips url: regex.
        (r#"(?i)"(?:api|apiUrl|apiBase|endpoint|baseURL|basePath)"\s*:\s*["'](/[^"']+)["']"#, "GET"),
        // U156: `.src = "/..."` dynamic property assignment (Image()/script/media); tag src= has a space not a dot.
        (r#"(?i)\.src\s*=\s*["'](/[^"']+)["']"#, "GET"),
        // U157: bare CSS `@import "/..."` (string form) — url() form already covered, bare string was not.
        (r#"(?i)@import\s+["'](/[^"']+)["']"#, "GET"),
        // U158: Worker `importScripts("/...")` — \bimport( needs "import(", here it's "importScripts(".
        (r#"(?i)importScripts\(\s*["'](/[^"']+)["']"#, "GET"),
        // U164: <img|input|frame src="/..."> element sources (classic/media/legacy frameset).
        (r#"(?i)<(?:img|input|frame)\b[^>]*\bsrc\s*=\s*["'](/[^"']+)["']"#, "GET"),
        // U165: fetch(`/...`) backtick template literal — plain fetch( needs a quote.
        (r#"(?i)\bfetch\(\s*`(/[^`]+)`"#, "GET"),
    ];
    for (pat, method) in patterns {
        if let Ok(re) = regex::Regex::new(pat) {
            for cap in re.captures_iter(body).take(80) {
                if let Some(m) = cap.get(1) {
                    push(method, m.as_str());
                }
            }
        }
    }

    // Script src — fetch later for deeper harvest (GET).
    for src in agent_extract_script_srcs(body) {
        push("GET", &src);
    }

    // Same-site iframe app shells (classic portals embed admin frames).
    if let Ok(re) = regex::Regex::new(r#"(?i)<iframe\b[^>]*\bsrc\s*=\s*["']([^"']+)["']"#) {
        for cap in re.captures_iter(body).take(40) {
            if let Some(m) = cap.get(1) {
                push("GET", m.as_str());
            }
        }
    }

    // Module / prefetch hints often point at same-site app bundles that hold routes.
    if let Ok(re) = regex::Regex::new(
        r#"(?i)<link\b[^>]*\brel\s*=\s*["'](?:modulepreload|prefetch|preload)["'][^>]*\bhref\s*=\s*["']([^"']+)["']"#,
    ) {
        for cap in re.captures_iter(body).take(40) {
            if let Some(m) = cap.get(1) {
                push("GET", m.as_str());
            }
        }
    }
    if let Ok(re) = regex::Regex::new(
        r#"(?i)<link\b[^>]*\bhref\s*=\s*["']([^"']+)["'][^>]*\brel\s*=\s*["'](?:modulepreload|prefetch|preload)["']"#,
    ) {
        for cap in re.captures_iter(body).take(40) {
            if let Some(m) = cap.get(1) {
                push("GET", m.as_str());
            }
        }
    }

    // Embedded plugin shells sometimes host same-site app routes.
    if let Ok(re) = regex::Regex::new(r#"(?i)<(?:embed|object)\b[^>]*(?:src|data)\s*=\s*["']([^"']+)["']"#) {
        for cap in re.captures_iter(body).take(40) {
            if let Some(m) = cap.get(1) {
                push("GET", m.as_str());
            }
        }
    }

    // Generic /Controller/Action style paths in JS/HTML text.
    if let Ok(re) = regex::Regex::new(r###"["'`](/[A-Za-z][A-Za-z0-9_-]{1,40}/[A-Za-z][A-Za-z0-9_-]{1,60})(?:[?"'`]|$)"###) {
        for cap in re.captures_iter(body).take(120) {
            if let Some(m) = cap.get(1) {
                push("GET", m.as_str());
            }
        }
    }

    out
}

fn persist_agent_surface_api(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    method: &str,
    path: &str,
    source: &str,
) -> Result<bool, String> {
    let normalized_path = path.trim();
    if !normalized_path.starts_with('/') || runtime.discovered_api_keys.contains(&format!(
        "{}|{normalized_path}", method.to_ascii_uppercase()
    )) {
        return Ok(false);
    }
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let (path_only, parameters) = match path.split_once('?') {
        Some((path_only, query)) => (
            path_only.to_string(),
            query
                .split('&')
                .filter_map(|pair| pair.split('=').next())
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>(),
        ),
        None => (path.to_string(), Vec::new()),
    };
    let absolute = reqwest::Url::parse(&context.target_url)
        .ok()
        .and_then(|base| base.join(&path_only).ok())
        .map(|value| value.to_string())
        .unwrap_or_else(|| path_only.clone());
    let record = serde_json::json!({
        "method": method,
        "path": path_only,
        "url": absolute,
        "parameters": parameters,
        "queryKeys": parameters,
        "confidence": "medium",
        "extractionEngine": "agent-html-js-harvest",
        "source": source,
        "replayable": true,
        "updatedAt": now,
    });
    stage_agent_finding(
        context,
        "frontend-recon",
        "api",
        &format!("harvest:{method}:{path}"),
        path,
        "info",
        &record,
    )?;
    // A failed insert must not poison the in-memory dedup set or enqueue a
    // phantom API. The durable inventory is the prerequisite for scheduling.
    runtime.note_discovered_api(method, path);
    Ok(true)
}
