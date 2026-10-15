#[path = "native_collection_resume.rs"]
mod native_collection_resume;
#[path = "native_collection_resume_transport.rs"]
mod native_collection_resume_transport;

const FOFA_BASE_FIELDS: &[&str] = &[
    "host",
    "link",
    "ip",
    "port",
    "protocol",
    "domain",
    "title",
    "icp",
    "cert.subject.org",
    "cert.subject.cn",
    "cert.domain",
    "cert.sn",
    "org",
    "server",
    "status_code",
    "jarm",
];
const FOFA_PRO_FIELDS: &[&str] = &[
    "header_hash",
    "banner_hash",
    "banner_fid",
    "cname",
    "lastupdatetime",
    "product",
    "product_category",
];
const CANDIDATE_FIELDS: &[&str] = &[
    "company",
    "decision",
    "reviewer",
    "review_note",
    "score",
    "confidence",
    "evidence",
    "phases",
    "asset_key",
    "host",
    "link",
    "ip",
    "port",
    "protocol",
    "domain",
    "title",
    "icp",
    "cert.subject.org",
    "cert.subject.cn",
    "cert.domain",
    "cert.sn",
    "org",
    "server",
    "status_code",
    "jarm",
    "header_hash",
    "banner_hash",
    "banner_fid",
    "cname",
    "lastupdatetime",
    "product",
    "product_category",
    "product.version",
    "icon_hash",
    "cname_domain",
    "cert.is_valid",
    "cert.is_match",
    "cert.is_equal",
    "fid",
    "matched_queries",
    "discovery_seed_kind",
    "discovery_sources_json",
    "discovery_source_count",
];

#[derive(Clone)]
struct EvidenceQuery {
    company: String,
    seed_kind: &'static str,
    phase: String,
    category: String,
    label: String,
    score: i64,
    query: String,
}

fn split_seed(value: &str) -> Vec<String> {
    value
        .split(['|', ';', '；', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}
fn fofa_quote(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{}\"", value.replace('"', "\\\"")))
}
fn normalize_host(value: &str) -> String {
    let raw = value.trim().trim_start_matches("*.");
    reqwest::Url::parse(raw)
        .or_else(|_| reqwest::Url::parse(&format!("http://{raw}")))
        .ok()
        .and_then(|url| {
            url.host_str().map(|host| {
                host.trim_start_matches("*.")
                    .trim_end_matches('.')
                    .to_ascii_lowercase()
            })
        })
        .unwrap_or_default()
}

fn build_fofa_queries(seeds: &Path, enable_cidr24: bool) -> Result<Vec<EvidenceQuery>, String> {
    let (_, rows) = read_csv(seeds)?;
    let mut queries = Vec::new();
    for row in rows {
        let names = split_seed(&text(&row, &["names", "full_names", "企业全称", "名称"]));
        let domains = split_seed(&text(
            &row,
            &["domains", "domain", "域名", "urls", "资产域名"],
        ));
        let icps = split_seed(&text(&row, &["icps", "icp", "备案号"]));
        let ips = split_seed(&text(&row, &["ip_ranges", "ips", "ip", "IP段", "IP"]));
        let orgs = split_seed(&text(&row, &["asn_orgs", "asn_org", "ASN组织", "网络组织"]));
        let keywords = split_seed(&text(
            &row,
            &["keywords", "unique_keywords", "独有关键词", "系统关键词"],
        ));
        let company = {
            let value = text(&row, &["company", "企业", "企业名称", "name"]);
            if value.is_empty() {
                names
                    .first()
                    .or(domains.first())
                    .cloned()
                    .unwrap_or_else(|| "未命名种子".into())
            } else {
                value
            }
        };
        let seed_kind = if !names.is_empty() {
            "company"
        } else if !domains.is_empty() {
            "domain"
        } else if !ips.is_empty() {
            "ip"
        } else if !icps.is_empty() {
            "icp"
        } else {
            "other"
        };
        let mut push = |category: &str, label: String, score: i64, query: String| {
            queries.push(EvidenceQuery {
                company: company.clone(),
                seed_kind,
                phase: "seed".into(),
                category: category.into(),
                label,
                score,
                query,
            })
        };
        for name in names {
            let quoted = fofa_quote(&name);
            push(
                "name_page",
                format!("标题全称:{name}"),
                45,
                format!("title={quoted}"),
            );
            push(
                "name_page",
                format!("正文全称:{name}"),
                30,
                format!("body={quoted}"),
            );
            push(
                "cert_org",
                format!("证书组织:{name}"),
                70,
                format!("cert.subject.org={quoted}"),
            );
        }
        for domain in domains {
            let host = normalize_host(&domain);
            if host.is_empty() {
                continue;
            }
            let quoted = fofa_quote(&host);
            push(
                "seed_domain",
                format!("精确主机:{host}"),
                100,
                format!("host={quoted}"),
            );
            push(
                "seed_domain",
                format!("注册域:{host}"),
                95,
                format!("domain={quoted}"),
            );
            push(
                "seed_domain",
                format!("证书域:{host}"),
                90,
                format!("cert.domain={quoted}"),
            );
        }
        for icp in icps {
            push(
                "icp",
                format!("备案号:{icp}"),
                95,
                format!("icp={}", fofa_quote(&icp)),
            );
        }
        for ip in ips {
            push(
                "seed_ip",
                format!("明确IP:{ip}"),
                100,
                format!("ip={}", fofa_quote(&ip)),
            );
            if enable_cidr24 {
                if let Ok(address) = ip.parse::<std::net::Ipv4Addr>() {
                    let octets = address.octets();
                    let cidr = format!("{}.{}.{}.0/24", octets[0], octets[1], octets[2]);
                    push(
                        "cidr24",
                        format!("种子C段:{cidr}"),
                        12,
                        format!("ip={}", fofa_quote(&cidr)),
                    );
                }
            }
        }
        for org in orgs {
            push(
                "asn_org",
                format!("ASN组织:{org}"),
                25,
                format!("org={}", fofa_quote(&org)),
            );
        }
        for keyword in keywords {
            let quoted = fofa_quote(&keyword);
            push(
                "unique_keyword",
                format!("独有标题:{keyword}"),
                35,
                format!("title={quoted}"),
            );
            push(
                "unique_keyword",
                format!("独有正文:{keyword}"),
                25,
                format!("body={quoted}"),
            );
        }
    }
    let mut unique = HashMap::new();
    for query in queries {
        let key = (
            query.company.clone(),
            query.category.clone(),
            query.query.clone(),
        );
        if unique
            .get(&key)
            .map(|old: &EvidenceQuery| old.score < query.score)
            .unwrap_or(true)
        {
            unique.insert(key, query);
        }
    }
    let mut queries = unique.into_values().collect::<Vec<_>>();
    queries.sort_by(|left, right| {
        (&left.company, &left.category, &left.query).cmp(&(
            &right.company,
            &right.category,
            &right.query,
        ))
    });
    Ok(queries)
}

#[derive(Default)]
struct Candidate {
    row: Record,
    scores: HashMap<String, i64>,
    evidence: BTreeSet<String>,
    phases: BTreeSet<String>,
    queries: BTreeSet<String>,
}

fn flatten_fofa(values: &serde_json::Value, fields: &[String]) -> Vec<Record> {
    values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            if let Some(object) = item.as_object() {
                Some(
                    fields
                        .iter()
                        .map(|field| {
                            (
                                field.clone(),
                                object
                                    .get(field)
                                    .map(|value| {
                                        value
                                            .as_str()
                                            .map(str::to_string)
                                            .unwrap_or_else(|| value.to_string())
                                    })
                                    .unwrap_or_default(),
                            )
                        })
                        .collect(),
                )
            } else {
                item.as_array().map(|array| {
                    fields
                        .iter()
                        .enumerate()
                        .map(|(index, field)| {
                            (
                                field.clone(),
                                array
                                    .get(index)
                                    .map(|value| {
                                        value
                                            .as_str()
                                            .map(str::to_string)
                                            .unwrap_or_else(|| value.to_string())
                                    })
                                    .unwrap_or_default(),
                            )
                        })
                        .collect()
                })
            }
        })
        .collect()
}

pub struct CollectOptions {
    pub key: String,
    pub proxy: String,
    pub page_size: usize,
    pub max_pages: usize,
    pub timeout: Duration,
    pub interval: Duration,
    pub full: bool,
    pub enable_cidr24: bool,
    pub profile: String,
    pub cache: bool,
}

fn fofa_fields(profile: &str) -> Vec<String> {
    let mut fields = FOFA_BASE_FIELDS
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>();
    if profile != "basic" {
        fields.extend(
            FOFA_PRO_FIELDS
                .iter()
                .take(if profile == "personal" {
                    3
                } else {
                    FOFA_PRO_FIELDS.len()
                })
                .map(|v| v.to_string()),
        );
    }
    if matches!(profile, "business" | "corporate" | "auto") {
        fields.extend(
            [
                "product.version",
                "icon_hash",
                "cname_domain",
                "cert.is_valid",
                "cert.is_match",
                "cert.is_equal",
            ]
            .iter()
            .map(|v| v.to_string()),
        );
    }
    if profile == "corporate" {
        fields.push("fid".into());
    }
    fields
}

fn host_matches_scope(candidate: &str, expected: &str) -> bool {
    let candidate = candidate.trim().trim_end_matches('.').to_ascii_lowercase();
    let expected = expected.trim().trim_end_matches('.').to_ascii_lowercase();
    !expected.is_empty() && (candidate == expected || candidate.ends_with(&format!(".{expected}")))
}

fn fofa_endpoint(
    api_url: &str,
    options: &CollectOptions,
    query: &EvidenceQuery,
    page: usize,
    size: usize,
    fields: &[String],
) -> Result<reqwest::Url, String> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(query.query.as_bytes());
    let mut endpoint = reqwest::Url::parse(api_url).map_err(|_| "FOFA API URL 无效")?;
    endpoint
        .query_pairs_mut()
        .append_pair("key", &options.key)
        .append_pair("qbase64", &encoded)
        .append_pair("fields", &fields.join(","))
        .append_pair("page", &page.to_string())
        .append_pair("size", &size.to_string())
        .append_pair("full", &options.full.to_string())
        .append_pair("r_type", "json");
    Ok(endpoint)
}

fn cached_collection_page(
    checkpoint: &native_collection_resume::CollectionCheckpoint,
    legacy_directory: &Path,
    endpoint: &reqwest::Url,
    fields: &[String],
) -> Option<(serde_json::Value, bool)> {
    let signature = format!("{:x}", Sha256::digest(endpoint.as_str().as_bytes()));
    if let Some(payload) = checkpoint.read_page(&signature, fields) {
        return Some((payload, false));
    }
    // 1.1.62 wrote exactly SHA256(the complete authenticated request URL).
    // Recompute it; a legacy page from another key/query/options cannot match.
    let payload = fs::read(legacy_directory.join(format!("{signature}.json")))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .filter(native_collection_resume::valid_page)?;
    Some((payload, true))
}

fn validate_legacy_collection_credentials(
    output: &Path,
    api_url: &str,
    options: &CollectOptions,
    queries: &[EvidenceQuery],
    fields: &[String],
) -> Result<(), String> {
    let legacy = output.join("native-cache");
    if output
        .join("collection-checkpoint-v1/binding.json")
        .exists()
        || !legacy.exists()
    {
        return Ok(());
    }
    let safe = [
        "host",
        "link",
        "ip",
        "port",
        "protocol",
        "domain",
        "title",
        "icp",
        "org",
        "server",
        "status_code",
    ]
    .iter()
    .map(|value| value.to_string())
    .collect::<Vec<_>>();
    for query in queries {
        let size = if query.query.contains("body=") {
            options.page_size.clamp(1, 500)
        } else {
            options.page_size.max(1)
        };
        for schema in [fields, safe.as_slice()] {
            let endpoint = fofa_endpoint(api_url, options, query, 1, size, schema)?;
            let signature = format!("{:x}", Sha256::digest(endpoint.as_str().as_bytes()));
            if fs::read(legacy.join(format!("{signature}.json")))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .is_some_and(|payload| native_collection_resume::valid_page(&payload))
            {
                return Ok(());
            }
        }
    }
    for entry in fs::read_dir(&legacy).map_err(|error| format!("无法盘点旧采集缓存：{error}"))?
    {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") || !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path)
            .map_err(|error| format!("无法读取旧采集缓存，未重新发起查询：{error}"))?;
        if serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .is_some_and(|payload| native_collection_resume::valid_page(&payload))
        {
            return Err("旧采集存在成功页面，但当前 FOFA Key 与原请求选项均无法匹配；请恢复原 Key 和任务快照，或创建新任务。未静默重发整轮查询".into());
        }
    }
    Ok(())
}

fn write_collection_csv(path: &Path, fields: &[String], rows: &[Record]) -> Result<(), String> {
    let parent = path.parent().ok_or("采集结果路径没有父目录")?;
    let staged = parent.join(format!(".collection-csv-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        write_csv(&staged, fields, rows)?;
        fs::OpenOptions::new()
            .write(true)
            .open(&staged)
            .and_then(|file| file.sync_all())
            .map_err(|error| error.to_string())?;
        fs::rename(&staged, path).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| error.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(staged);
    }
    result
}

pub fn collect_fofa(
    seeds: &Path,
    output: &Path,
    options: CollectOptions,
    cancel: &AtomicBool,
    progress: impl FnMut(usize, usize, &str),
) -> Result<serde_json::Value, String> {
    collect_fofa_at(
        seeds,
        output,
        options,
        cancel,
        "https://fofa.info/api/v1/search/all",
        progress,
    )
}

fn collect_fofa_at(
    seeds: &Path,
    output: &Path,
    options: CollectOptions,
    cancel: &AtomicBool,
    api_url: &str,
    mut progress: impl FnMut(usize, usize, &str),
) -> Result<serde_json::Value, String> {
    if options.key.trim().is_empty() {
        return Err("未配置 FOFA Key".into());
    }
    fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let queries = build_fofa_queries(seeds, options.enable_cidr24)?;
    if queries.is_empty() {
        return Err("没有生成任何 FOFA 查询".into());
    }
    let requested_fields = fofa_fields(&options.profile);
    let mut fields = requested_fields.clone();
    validate_legacy_collection_credentials(output, api_url, &options, &queries, &requested_fields)?;
    let plan = serde_json::Value::Array(
        queries
            .iter()
            .map(|query| {
                json!({
                    "company": query.company, "seed_kind": query.seed_kind,
                    "phase": query.phase, "category": query.category,
                    "label": query.label, "score": query.score, "query": query.query,
                })
            })
            .collect(),
    );
    let binding = native_collection_resume::CollectionBinding::new(
        &fs::read(seeds).map_err(|error| format!("无法读取原采集种子：{error}"))?,
        &plan,
        api_url,
        &options.key,
        fields.clone(),
        options.page_size,
        options.max_pages,
        options.full,
        options.enable_cidr24,
        &options.profile,
    )?;
    let checkpoint = native_collection_resume::CollectionCheckpoint::open(output, binding)?;
    checkpoint.write_state(
        output,
        &json!({"status":"running", "complete":false,
        "queries":queries.len(), "completed_queries":0, "failed_queries":0}),
    )?;
    let safe_fields = [
        "host",
        "link",
        "ip",
        "port",
        "protocol",
        "domain",
        "title",
        "icp",
        "org",
        "server",
        "status_code",
    ]
    .iter()
    .map(|value| value.to_string())
    .collect::<Vec<_>>();
    let cache_directory = output.join("native-cache");
    if options.cache {
        fs::create_dir_all(&cache_directory).map_err(|error| error.to_string())?;
    }
    let mut transport = native_collection_resume_transport::Transport::new(
        &options.key,
        &options.proxy,
        options.timeout,
        options.interval,
    )?;
    let mut candidates: HashMap<(String, String), Candidate> = HashMap::new();
    let mut logs = Vec::new();
    let mut halted: Option<(&'static str, String)> = None;
    let mut reused_pages = 0usize;
    let mut migrated_pages = 0usize;
    let mut request_attempts = 0usize;
    for (index, query) in queries.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            halted = Some(("cancelled", "任务已取消；保留已完成页面供恢复".into()));
        }
        progress(index, queries.len(), &query.label);
        let effective_size = if query.query.contains("body=") {
            options.page_size.clamp(1, 500)
        } else {
            options.page_size.max(1)
        };
        let mut page = 1usize;
        let mut fetched = 0usize;
        let mut total_matches = 0usize;
        let mut status = "ok".to_string();
        let mut error = String::new();
        let mut error_category = String::new();
        let mut query_attempts = 0usize;
        let mut query_reused = 0usize;
        let mut limited = false;
        loop {
            if cancel.load(Ordering::Relaxed) {
                halted = Some(("cancelled", "任务已取消；保留已完成页面供恢复".into()));
            }
            let mut endpoint =
                fofa_endpoint(api_url, &options, query, page, effective_size, &fields)?;
            let mut cached =
                cached_collection_page(&checkpoint, &cache_directory, &endpoint, &fields);
            if cached.is_none() && fields != requested_fields {
                let original = fofa_endpoint(
                    api_url,
                    &options,
                    query,
                    page,
                    effective_size,
                    &requested_fields,
                )?;
                if let Some(payload) = cached_collection_page(
                    &checkpoint,
                    &cache_directory,
                    &original,
                    &requested_fields,
                ) {
                    fields = requested_fields.clone();
                    endpoint = original;
                    cached = Some(payload);
                }
            }
            if cached.is_none() && fields != safe_fields {
                let fallback =
                    fofa_endpoint(api_url, &options, query, page, effective_size, &safe_fields)?;
                if let Some(payload) =
                    cached_collection_page(&checkpoint, &cache_directory, &fallback, &safe_fields)
                {
                    // A prior 1.1.62 account field downgrade is itself a matching
                    // request page. Reuse it with its actual column schema.
                    fields = safe_fields.clone();
                    endpoint = fallback;
                    cached = Some(payload);
                }
            }
            let signature = format!("{:x}", Sha256::digest(endpoint.as_str().as_bytes()));
            let fetched_payload =
                (|| -> Result<serde_json::Value, native_collection_resume_transport::Failure> {
                    let (payload, save_checkpoint) = if let Some((payload, legacy)) = cached {
                        reused_pages += 1;
                        query_reused += 1;
                        migrated_pages += usize::from(legacy);
                        (payload, legacy)
                    } else {
                        if let Some((category, detail)) = &halted {
                            return Err(native_collection_resume_transport::Failure {
                                category,
                                detail: detail.clone(),
                                retryable: false,
                                stop_collection: true,
                                attempts: 0,
                            });
                        }
                        match transport.request(&endpoint, cancel, |detail| {
                            progress(
                                index,
                                queries.len(),
                                &format!("FOFA 自动重试：{}；第 {page} 页；{detail}", query.query),
                            );
                        }) {
                            Ok(receipt) => {
                                request_attempts += receipt.attempts;
                                query_attempts += receipt.attempts;
                                (receipt.payload, true)
                            }
                            Err(failure) => {
                                request_attempts += failure.attempts;
                                query_attempts += failure.attempts;
                                return Err(failure);
                            }
                        }
                    };
                    if native_collection_resume::valid_page(&payload) && save_checkpoint {
                        checkpoint
                            .save_page(&signature, &fields, &payload)
                            .map_err(native_collection_resume_transport::Failure::checkpoint)?;
                        if options.cache {
                            native_collection_resume::atomic_write(
                                &cache_directory.join(format!("{signature}.json")),
                                &serde_json::to_vec(&payload).map_err(|error| {
                                    native_collection_resume_transport::Failure::checkpoint(
                                        error.to_string(),
                                    )
                                })?,
                            )
                            .map_err(native_collection_resume_transport::Failure::checkpoint)?;
                        }
                    }
                    Ok(payload)
                })();
            let payload = match fetched_payload {
                Ok(payload) => payload,
                Err(failure) => {
                    status = "error".into();
                    error_category = failure.category.into();
                    error = failure.detail;
                    if failure.category != "cancelled" && (query_attempts > 0 || halted.is_none()) {
                        progress(
                            index,
                            queries.len(),
                            &format!(
                                "FOFA 查询失败：{}；第 {page} 页；[{}] {}",
                                query.query, error_category, error
                            ),
                        );
                    }
                    if failure.stop_collection {
                        halted = Some((failure.category, error.clone()));
                    }
                    break;
                }
            };
            if payload
                .get("error")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                status = "error".into();
                let failure = native_collection_resume_transport::provider_failure(
                    &payload,
                    &options.key,
                    &options.proxy,
                );
                error = failure.detail;
                error_category = failure.category.into();
                if native_collection_resume_transport::field_restricted(&error)
                    && fields != safe_fields
                {
                    fields = safe_fields.clone();
                    status = "ok".into();
                    error.clear();
                    error_category.clear();
                    progress(
                        index,
                        queries.len(),
                        "当前字段权限不足，回退基础资产字段后重试",
                    );
                    continue;
                }
                if failure.stop_collection {
                    halted = Some((failure.category, error.clone()));
                }
                progress(
                    index,
                    queries.len(),
                    &format!(
                        "FOFA 查询失败：{}；第 {page} 页；[{}] {}",
                        query.query, error_category, error
                    ),
                );
                break;
            }
            let records = flatten_fofa(
                payload.get("results").unwrap_or(&serde_json::Value::Null),
                &fields,
            );
            total_matches = payload
                .get("size")
                .and_then(|value| value.as_u64())
                .unwrap_or(records.len() as u64) as usize;
            for record in &records {
                if query.category == "seed_domain" {
                    let expected =
                        normalize_host(query.label.rsplit(':').next().unwrap_or_default());
                    let candidate = normalize_host(&text(record, &["host", "link"]));
                    if !expected.is_empty()
                        && !candidate.is_empty()
                        && !host_matches_scope(&candidate, &expected)
                    {
                        continue;
                    }
                }
                let asset_key = discovery_identity(record);
                let item = candidates
                    .entry((query.company.clone(), asset_key.clone()))
                    .or_default();
                item.row
                    .entry("company".into())
                    .or_insert_with(|| query.company.clone());
                item.row
                    .entry("discovery_seed_kind".into())
                    .or_insert_with(|| query.seed_kind.into());
                item.row.entry("asset_key".into()).or_insert(asset_key);
                for (field, value) in record {
                    if !value.is_empty()
                        && item
                            .row
                            .get(field)
                            .map(|old| old.is_empty())
                            .unwrap_or(true)
                    {
                        item.row.insert(field.clone(), value.clone());
                    }
                }
                item.scores
                    .entry(query.category.clone())
                    .and_modify(|score| *score = (*score).max(query.score))
                    .or_insert(query.score);
                item.evidence.insert(query.label.clone());
                item.phases.insert(query.phase.clone());
                item.queries.insert(query.query.clone());
            }
            fetched += records.len();
            checkpoint.write_state(
                output,
                &json!({"status":"running", "complete":false,
                "queries":queries.len(), "processed_queries":index,
                "last_query":query.query, "last_page":page,
                "reused_pages":reused_pages, "migrated_legacy_pages":migrated_pages,
                "request_attempts":request_attempts}),
            )?;
            limited = options.max_pages > 0
                && page >= options.max_pages
                && records.len() >= effective_size
                && fetched < total_matches;
            if records.len() < effective_size
                || fetched >= total_matches
                || (options.max_pages > 0 && page >= options.max_pages)
            {
                break;
            }
            page += 1;
        }
        let mut log = Record::new();
        for (key, value) in [
            ("company", query.company.clone()),
            ("phase", query.phase.clone()),
            ("category", query.category.clone()),
            ("label", query.label.clone()),
            ("score", query.score.to_string()),
            ("query", query.query.clone()),
            ("total_matches", total_matches.to_string()),
            ("fetched", fetched.to_string()),
            ("status", status),
            ("error", error),
            ("error_category", error_category),
            ("request_attempts", query_attempts.to_string()),
            ("reused_pages", query_reused.to_string()),
            ("last_page", page.to_string()),
            (
                "partial",
                (fetched > 0 && fetched < total_matches).to_string(),
            ),
            ("limited", limited.to_string()),
        ] {
            log.insert(key.into(), value);
        }
        logs.push(log);
    }
    let plan_fields = ["company", "phase", "category", "label", "score", "query"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let plans = queries
        .iter()
        .map(|query| {
            [
                ("company", query.company.clone()),
                ("phase", query.phase.clone()),
                ("category", query.category.clone()),
                ("label", query.label.clone()),
                ("score", query.score.to_string()),
                ("query", query.query.clone()),
            ]
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect()
        })
        .collect::<Vec<Record>>();
    write_collection_csv(&output.join("query_plan.csv"), &plan_fields, &plans)?;
    write_collection_csv(
        &output.join("query_log.csv"),
        &[
            "company",
            "phase",
            "category",
            "label",
            "score",
            "query",
            "total_matches",
            "fetched",
            "status",
            "error",
            "error_category",
            "request_attempts",
            "reused_pages",
            "last_page",
            "partial",
            "limited",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>(),
        &logs,
    )?;
    let mut rows = Vec::new();
    for (_, mut candidate) in candidates {
        let score = candidate.scores.values().sum::<i64>().min(100);
        candidate.row.insert("score".into(), score.to_string());
        candidate.row.insert(
            "confidence".into(),
            if score >= 85 {
                "high"
            } else if score >= 55 {
                "medium"
            } else {
                "low"
            }
            .into(),
        );
        candidate.row.insert(
            "evidence".into(),
            candidate
                .evidence
                .into_iter()
                .collect::<Vec<_>>()
                .join(" | "),
        );
        candidate.row.insert(
            "phases".into(),
            candidate.phases.into_iter().collect::<Vec<_>>().join(" | "),
        );
        candidate.row.insert(
            "matched_queries".into(),
            candidate
                .queries
                .into_iter()
                .collect::<Vec<_>>()
                .join(" | "),
        );
        rows.push(candidate.row);
    }
    let source_candidates = rows.len();
    let mut rows = merge_discovery_rows(rows);
    rows.sort_by(|left, right| {
        text(left, &["company"])
            .cmp(&text(right, &["company"]))
            .then(number(right, &["score"]).cmp(&number(left, &["score"])))
            .then(discovery_identity(left).cmp(&discovery_identity(right)))
    });
    let candidate_fields = CANDIDATE_FIELDS
        .iter()
        .map(|field| (*field).to_string())
        .collect::<Vec<_>>();
    write_collection_csv(&output.join("candidates.csv"), &candidate_fields, &rows)?;
    write_collection_csv(
        &output.join("review_candidates.csv"),
        &candidate_fields,
        &rows
            .iter()
            .filter(|row| number(row, &["score"]) >= 30)
            .cloned()
            .collect::<Vec<_>>(),
    )?;
    let failed_queries = logs
        .iter()
        .filter(|row| text(row, &["status"]) == "error")
        .count();
    let truncated_queries = logs
        .iter()
        .filter(|row| text(row, &["limited"]) == "true")
        .count();
    let cancelled = cancel.load(Ordering::Relaxed)
        || halted
            .as_ref()
            .is_some_and(|(category, _)| *category == "cancelled");
    let complete = failed_queries == 0 && !cancelled;
    checkpoint.write_state(output, &json!({
        "status": if cancelled { "cancelled" } else if complete { "completed" } else { "incomplete" },
        "complete":complete, "coverage_complete":complete && truncated_queries == 0,
        "queries":queries.len(), "completed_queries":queries.len() - failed_queries,
        "failed_queries":failed_queries, "truncated_queries":truncated_queries,
        "source_candidates":source_candidates, "candidates":rows.len(),
        "reused_pages":reused_pages, "migrated_legacy_pages":migrated_pages,
        "request_attempts":request_attempts,
    }))?;
    if cancelled {
        return Err("__CANCELLED__".into());
    }
    if failed_queries > 0 {
        let first = logs
            .iter()
            .find(|row| text(row, &["status"]) == "error")
            .map(|row| {
                format!(
                    "{} [{}] {}",
                    text(row, &["query"]),
                    text(row, &["error_category"]),
                    text(row, &["error"])
                )
            })
            .unwrap_or_default();
        return Err(format!("FOFA {failed_queries}/{} 条查询未完成；已保留 {} 条候选、页面检查点和 query_log.csv，可原地恢复。首条失败：{first}", queries.len(), rows.len()));
    }
    progress(queries.len(), queries.len(), "采集完成");
    Ok(
        json!({"queries": queries.len(), "source_candidates": source_candidates,
        "candidates": rows.len(), "merged_duplicates": source_candidates - rows.len(), "native": true,
        "complete":true, "coverage_complete":truncated_queries == 0, "truncated_queries":truncated_queries,
        "reused_pages":reused_pages, "migrated_legacy_pages":migrated_pages, "request_attempts":request_attempts}),
    )
}

#[cfg(test)]
#[path = "native_collection_resume_tests.rs"]
mod native_collection_resume_tests;
