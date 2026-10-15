#[path = "native_collection_resume.rs"]
mod native_collection_resume;
#[path = "native_collection_resume_transport.rs"]
mod native_collection_resume_transport;
include!("native_collection_resume_summary.rs");

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
    "discovery_sources_truncated",
    "collection_received_at",
    "collection_binding_sha256",
    "collection_provider_observed_at",
    "collection_time_status",
    "collection_actual_fields_json",
    "collection_payload_sha256",
    "collection_request_sha256",
    "collection_page",
    "collection_rule_version",
    "collection_payload_redacted",
    "collection_payload_hash_semantics",
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
                                    .filter(|value| !value.is_null())
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
                                    .filter(|value| !value.is_null())
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
) -> Result<Option<native_collection_resume::PageEvidence>, String> {
    let signature = format!("{:x}", Sha256::digest(endpoint.as_str().as_bytes()));
    if let Some(page) = checkpoint.read_page(&signature, fields)? {
        return Ok(Some(page));
    }
    // 1.1.62 wrote exactly SHA256(the complete authenticated request URL).
    // Recompute it; a legacy page from another key/query/options cannot match.
    let path = legacy_directory.join(format!("{signature}.json"));
    let payload = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .filter(native_collection_resume::valid_page);
    if let Some(payload) = payload {
        // Legacy raw caches contain no trustworthy receive time. Preserve a
        // compatibility receipt, then fetch again under the original binding.
        checkpoint.note_legacy(&signature, fields, &payload)?;
        checkpoint.sanitize_legacy(&path, &payload)?;
    }
    Ok(None)
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

fn write_collection_csv(
    path: &Path,
    fields: &[String],
    rows: &[Record],
    storage: Option<&crate::jobs::collection_retention::CollectionStorage>,
) -> Result<(), String> {
    let mut writer = WriterBuilder::new().from_writer(Vec::new());
    writer.write_record(fields).map_err(|e| e.to_string())?;
    for row in rows {
        writer
            .write_record(
                fields
                    .iter()
                    .map(|field| row.get(field).map(String::as_str).unwrap_or("")),
            )
            .map_err(|e| e.to_string())?;
    }
    let bytes = writer.into_inner().map_err(|e| e.to_string())?;
    if let Some(storage) = storage {
        storage.write(
            Path::new(path.file_name().ok_or("采集CSV路径缺少文件名")?),
            &bytes,
            crate::jobs::collection_retention::ArtifactKind::Csv,
            None,
            chrono::Utc::now().timestamp(),
        )?;
        Ok(())
    } else {
        native_collection_resume::atomic_write(path, &bytes)
    }
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

pub fn collect_fofa_with_storage(
    seeds: &Path,
    output: &Path,
    options: CollectOptions,
    cancel: &AtomicBool,
    storage: &crate::jobs::collection_retention::CollectionStorage,
    progress: impl FnMut(usize, usize, &str),
) -> Result<serde_json::Value, String> {
    collect_fofa_at_impl(
        seeds,
        output,
        options,
        cancel,
        "https://fofa.info/api/v1/search/all",
        Some(storage),
        progress,
    )
}

fn collect_fofa_at(
    seeds: &Path,
    output: &Path,
    options: CollectOptions,
    cancel: &AtomicBool,
    api_url: &str,
    progress: impl FnMut(usize, usize, &str),
) -> Result<serde_json::Value, String> {
    collect_fofa_at_impl(seeds, output, options, cancel, api_url, None, progress)
}

fn collect_fofa_at_impl(
    seeds: &Path,
    output: &Path,
    options: CollectOptions,
    cancel: &AtomicBool,
    api_url: &str,
    storage: Option<&crate::jobs::collection_retention::CollectionStorage>,
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
    let checkpoint = native_collection_resume::CollectionCheckpoint::open_with_storage(
        output,
        binding,
        &options.key,
        &options.proxy,
        storage,
    )?;
    let mut summary = CollectionSummary::pending(
        &uuid::Uuid::new_v4().to_string(),
        queries.len(),
        &requested_fields,
    );
    summary.binding_sha256 = checkpoint.binding_sha256().into();
    summary.query_summaries = queries
        .iter()
        .map(|query| QueryCollectionSummary {
            company: query.company.clone(),
            category: query.category.clone(),
            query: query.query.clone(),
            status: "pending".into(),
            requested_fields: requested_fields.clone(),
            field_coverage_complete: true,
            ..QueryCollectionSummary::default()
        })
        .collect();
    summary.refresh(&checkpoint)?;
    checkpoint.write_state(output, &summary.state()?)?;
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
    // New pages have one checkpoint copy. The old native-cache namespace is
    // read only for compatibility, apart from removing echoed credentials.
    let mut transport = match native_collection_resume_transport::Transport::new(
        &options.key,
        &options.proxy,
        options.timeout,
        options.interval,
    ) {
        Ok(transport) => transport,
        Err(detail) => {
            for query in &mut summary.query_summaries {
                query.status = "error".into();
                query.error_category = Some("transport_config".into());
                query.field_coverage_complete = false;
                add_reason(&mut query.coverage_reasons, "transport_config");
            }
            summary.refresh(&checkpoint)?;
            summary.fail("transport_config");
            checkpoint.write_state(output, &summary.state()?)?;
            return Err(native_collection_resume_transport::sanitize(
                &detail,
                &options.key,
                &options.proxy,
            ));
        }
    };
    let mut candidates: HashMap<(String, String), Candidate> = HashMap::new();
    let mut logs = Vec::new();
    let mut halted: Option<(&'static str, String)> = None;
    for (index, query) in queries.iter().enumerate() {
        progress(index, queries.len(), &query.label);
        let effective_size = if query.query.contains("body=") {
            options.page_size.clamp(1, 500)
        } else {
            options.page_size.max(1)
        };
        let mut page = 1usize;
        let mut detail = String::new();
        let mut q = summary.query_summaries[index].clone();
        q.status = "running".into();
        let mut page_hashes = BTreeSet::new();
        let mut record_hashes = BTreeSet::new();
        loop {
            q.last_page = page;
            if cancel.load(Ordering::Relaxed) {
                halted = Some(("cancelled", "任务已取消；保留已完成页面供恢复".into()));
            }
            let result = (|| -> Result<native_collection_resume::PageEvidence, native_collection_resume_transport::Failure> {
                if checkpoint.retention_report().limit_reached {
                    return Err(native_collection_resume_transport::Failure::checkpoint(
                        "receipt_retention_limit: 当前任务已有收据超额；未追加请求或原文".into()));
                }
                let mut endpoint = fofa_endpoint(api_url, &options, query, page, effective_size, &fields)
                    .map_err(native_collection_resume_transport::Failure::checkpoint)?;
                let mut cached = cached_collection_page(&checkpoint, &cache_directory, &endpoint, &fields)
                    .map_err(native_collection_resume_transport::Failure::checkpoint)?;
                if cached.is_none() && fields != requested_fields {
                    let original = fofa_endpoint(api_url, &options, query, page, effective_size, &requested_fields)
                        .map_err(native_collection_resume_transport::Failure::checkpoint)?;
                    if let Some(saved) = cached_collection_page(&checkpoint, &cache_directory, &original, &requested_fields)
                        .map_err(native_collection_resume_transport::Failure::checkpoint)? {
                        fields = requested_fields.clone(); endpoint = original; cached = Some(saved);
                    }
                }
                if cached.is_none() && fields != safe_fields {
                    let fallback = fofa_endpoint(api_url, &options, query, page, effective_size, &safe_fields)
                        .map_err(native_collection_resume_transport::Failure::checkpoint)?;
                    if let Some(saved) = cached_collection_page(&checkpoint, &cache_directory, &fallback, &safe_fields)
                        .map_err(native_collection_resume_transport::Failure::checkpoint)? {
                        fields = safe_fields.clone(); endpoint = fallback; cached = Some(saved);
                    }
                }
                if let Some(saved) = cached { q.reused_pages += 1; return Ok(saved); }
                if let Some((category, message)) = &halted {
                    return Err(native_collection_resume_transport::Failure {
                        category, detail: message.clone(), retryable:false, stop_collection:true, attempts:0,
                    });
                }
                checkpoint.ensure_page_capacity().map_err(native_collection_resume_transport::Failure::checkpoint)?;
                let receipt = transport.request(&endpoint, cancel, |message| {
                    progress(index, queries.len(), &format!("FOFA 自动重试：{}；第 {page} 页；{message}", query.query));
                }).map_err(|failure| { q.request_attempts += failure.attempts; failure })?;
                q.request_attempts += receipt.attempts;
                let signature = format!("{:x}", Sha256::digest(endpoint.as_str().as_bytes()));
                if native_collection_resume::valid_page(&receipt.payload) {
                    checkpoint.save_page(&signature, &fields, &receipt.payload,
                        Some(&receipt.received_at), receipt.secrets_redacted, &receipt.redacted_locator_rows)
                        .map_err(native_collection_resume_transport::Failure::checkpoint)
                } else {
                    // Provider error envelopes can trigger a field downgrade;
                    // they are never checkpointed as successful pages.
                    Ok(native_collection_resume::PageEvidence {
                        payload_sha256: format!("{:x}", Sha256::digest(serde_json::to_vec(&receipt.payload)
                            .map_err(|e| native_collection_resume_transport::Failure::checkpoint(e.to_string()))?)),
                        payload:receipt.payload,received_at:Some(receipt.received_at),
                        request_sha256:signature,payload_redacted:receipt.secrets_redacted,redacted_locator_rows:receipt.redacted_locator_rows,
                    })
                }
            })();
            let receipt = match result {
                Ok(receipt) => receipt,
                Err(failure) => {
                    q.status = "error".into();
                    q.error_category = Some(failure.category.into());
                    detail = failure.detail;
                    add_reason(&mut q.coverage_reasons, failure.category);
                    if failure.category == "resource_limited" {
                        q.limited = true;
                        add_reason(&mut q.coverage_reasons, "retention_limit");
                    }
                    if failure.stop_collection {
                        halted = Some((failure.category, detail.clone()));
                    }
                    if failure.category != "cancelled" {
                        progress(
                            index,
                            queries.len(),
                            &format!(
                                "FOFA 查询失败：{}；第 {page} 页；[{}] {detail}",
                                query.query, failure.category
                            ),
                        );
                    }
                    break;
                }
            };
            let payload = &receipt.payload;
            if payload.get("error").and_then(serde_json::Value::as_bool) == Some(true) {
                let failure = native_collection_resume_transport::provider_failure(
                    payload,
                    &options.key,
                    &options.proxy,
                );
                if native_collection_resume_transport::field_restricted(&failure.detail)
                    && fields != safe_fields
                {
                    fields = safe_fields.clone();
                    progress(
                        index,
                        queries.len(),
                        "当前字段权限不足，回退基础资产字段后重试；字段覆盖将单独标记",
                    );
                    continue;
                }
                q.status = "error".into();
                q.error_category = Some(failure.category.into());
                detail = failure.detail;
                add_reason(&mut q.coverage_reasons, failure.category);
                if failure.stop_collection {
                    halted = Some((failure.category, detail.clone()));
                }
                progress(
                    index,
                    queries.len(),
                    &format!(
                        "FOFA 查询失败：{}；第 {page} 页；[{}] {detail}",
                        query.query, failure.category
                    ),
                );
                break;
            }
            if !q.actual_field_sets.contains(&fields) {
                q.actual_field_sets.push(fields.clone());
            }
            if fields != requested_fields {
                q.field_coverage_complete = false;
                add_reason(&mut q.coverage_reasons, "field_downgrade");
            }
            observe_received(
                &mut q.received_at_first,
                &mut q.received_at_last,
                receipt.received_at.as_deref(),
            );
            let raw_rows = payload
                .get("results")
                .and_then(serde_json::Value::as_array)
                .expect("transport and checkpoint require a results array");
            let raw_count = raw_rows.len();
            let total = payload
                .get("size")
                .and_then(serde_json::Value::as_u64)
                .and_then(|total| usize::try_from(total).ok());
            if let Some(total) = total {
                if !q.reported_totals.contains(&total) {
                    q.reported_totals.push(total);
                }
                if q.reported_totals.len() > 1 {
                    add_reason(&mut q.coverage_reasons, "total_changed");
                }
                q.total_matches = Some(total);
            } else {
                add_reason(&mut q.coverage_reasons, "unknown_total");
            }
            let page_hash = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(raw_rows).map_err(|e| e.to_string())?)
            );
            let repeated = raw_count > 0 && !page_hashes.insert(page_hash);
            if repeated {
                add_reason(&mut q.coverage_reasons, "duplicate_page");
            }
            for (row_index, raw) in raw_rows.iter().enumerate() {
                // Count provider rows, not canonical endpoint identities. Two
                // distinct observations of an endpoint are legitimate; an
                // identical raw row repeated across pages cannot prove total.
                let row_hash = format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(raw).map_err(|e| e.to_string())?)
                );
                if !record_hashes.insert(row_hash) {
                    add_reason(&mut q.coverage_reasons, "duplicate_record");
                }
                let shape_valid = raw.as_object().is_some()
                    || raw.as_array().is_some_and(|row| row.len() == fields.len());
                let record = flatten_fofa(&json!([raw]), &fields)
                    .pop()
                    .unwrap_or_default();
                let locator_redacted = receipt.redacted_locator_rows.contains(&row_index);
                if locator_redacted {
                    add_reason(&mut q.coverage_reasons, "redacted_locator");
                }
                if locator_redacted
                    || !shape_valid
                    || !super::asset_identity::valid_network_identity(
                        &text(&record, &["link"]),
                        &text(&record, &["host"]),
                        &text(&record, &["protocol"]),
                        &text(&record, &["ip"]),
                        &text(&record, &["port"]),
                    )
                {
                    q.rejected_rows += 1;
                    add_reason(&mut q.coverage_reasons, "invalid_records");
                    continue;
                }
                q.valid_rows += 1;
                if query.category == "seed_domain" {
                    let expected =
                        normalize_host(query.label.rsplit(':').next().unwrap_or_default());
                    // Match the actual endpoint first, as identity and probe do.
                    let candidate = normalize_host(&text(&record, &["link", "host"]));
                    if !expected.is_empty() && !host_matches_scope(&candidate, &expected) {
                        q.scope_filtered_rows += 1;
                        continue;
                    }
                }
                let asset_key = discovery_identity(&record);
                let mut incoming = record;
                for (key, value) in [
                    ("company", query.company.clone()),
                    ("discovery_seed_kind", query.seed_kind.into()),
                    ("asset_key", asset_key.clone()),
                    ("evidence", query.label.clone()),
                    ("matched_queries", query.query.clone()),
                    ("phases", query.phase.clone()),
                    ("score", query.score.to_string()),
                    (
                        "collection_received_at",
                        receipt.received_at.clone().unwrap_or_default(),
                    ),
                    (
                        "collection_binding_sha256",
                        checkpoint.binding_sha256().into(),
                    ),
                    (
                        "collection_time_status",
                        if receipt.received_at.is_some() {
                            "received"
                        } else {
                            "legacy_time_unknown"
                        }
                        .into(),
                    ),
                    (
                        "collection_actual_fields_json",
                        serde_json::to_string(&fields).map_err(|e| e.to_string())?,
                    ),
                    ("collection_payload_sha256", receipt.payload_sha256.clone()),
                    ("collection_request_sha256", receipt.request_sha256.clone()),
                    ("collection_page", page.to_string()),
                    ("collection_rule_version", COLLECTION_RULE_VERSION.into()),
                    (
                        "collection_payload_redacted",
                        receipt.payload_redacted.to_string(),
                    ),
                    (
                        "collection_payload_hash_semantics",
                        native_collection_resume::PAYLOAD_HASH_SEMANTICS.into(),
                    ),
                ] {
                    incoming.insert(key.into(), value);
                }
                incoming.insert(
                    "collection_provider_observed_at".into(),
                    text(&incoming, &["lastupdatetime"]),
                );
                let item = candidates
                    .entry((query.company.clone(), asset_key))
                    .or_default();
                if item.row.is_empty() {
                    item.row = incoming;
                } else {
                    merge_discovery_evidence(&mut item.row, &incoming);
                }
                item.scores
                    .entry(query.category.clone())
                    .and_modify(|score| *score = (*score).max(query.score))
                    .or_insert(query.score);
                item.evidence.insert(query.label.clone());
                item.phases.insert(query.phase.clone());
                item.queries.insert(query.query.clone());
            }
            q.fetched_rows += raw_count;
            if total.is_some_and(|total| q.fetched_rows > total) {
                add_reason(&mut q.coverage_reasons, "total_mismatch");
            }
            let reached_total = total.is_some_and(|total| q.fetched_rows >= total);
            let short = raw_count < effective_size;
            if short && total.is_some_and(|total| q.fetched_rows < total) {
                add_reason(&mut q.coverage_reasons, "short_page");
            }
            // A full page with no trustworthy total needs another page, bounded
            // by both the configured limit and the hard receipt page limit.
            let needs_more = !short && !reached_total && !repeated;
            if needs_more
                && ((options.max_pages > 0 && page >= options.max_pages)
                    || page >= native_collection_resume::MAX_RECEIPT_PAGES)
            {
                q.limited = true;
                add_reason(&mut q.coverage_reasons, "page_limit");
            }
            q.complete = true;
            q.status = "ok".into();
            q.coverage_complete = total.is_some() && reached_total && q.coverage_reasons.is_empty();
            summary.query_summaries[index] = q.clone();
            summary.refresh(&checkpoint)?;
            // Executing a query is still in progress until its terminal page.
            summary.status = "running".into();
            summary.complete = false;
            summary.coverage_complete = false;
            summary.candidates = candidates.len();
            summary.source_candidates = candidates.len();
            checkpoint.write_state(output, &summary.state()?)?;
            if !needs_more || q.limited {
                break;
            }
            q.complete = false;
            q.status = "running".into();
            page += 1;
        }
        q.coverage_complete = q.complete
            && q.error_category.is_none()
            && q.coverage_reasons.is_empty()
            && q.total_matches.is_some_and(|total| q.fetched_rows >= total);
        if q.status == "error" {
            q.complete = false;
        }
        let mut log = Record::new();
        for (key, value) in [
            ("company", query.company.clone()),
            ("phase", query.phase.clone()),
            ("category", query.category.clone()),
            ("label", query.label.clone()),
            ("score", query.score.to_string()),
            ("query", query.query.clone()),
            (
                "total_matches",
                q.total_matches.map(|n| n.to_string()).unwrap_or_default(),
            ),
            ("fetched", q.fetched_rows.to_string()),
            ("status", q.status.clone()),
            ("error", detail),
            (
                "error_category",
                q.error_category.clone().unwrap_or_default(),
            ),
            ("request_attempts", q.request_attempts.to_string()),
            ("reused_pages", q.reused_pages.to_string()),
            ("last_page", q.last_page.to_string()),
            ("partial", (!q.coverage_complete).to_string()),
            ("limited", q.limited.to_string()),
        ] {
            log.insert(key.into(), value);
        }
        logs.push(log);
        summary.query_summaries[index] = q;
        summary.refresh(&checkpoint)?;
        summary.status = "running".into();
        summary.complete = false;
        summary.coverage_complete = false;
        summary.candidates = candidates.len();
        summary.source_candidates = candidates.len();
        checkpoint.write_state(output, &summary.state()?)?;
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
    summary.source_candidates = source_candidates;
    summary.candidates = rows.len();
    summary.merged_duplicates = source_candidates - rows.len();
    summary.refresh(&checkpoint)?;
    let artifacts = (|| -> Result<(), String> {
        if summary
            .error_categories
            .iter()
            .any(|category| category == "resource_limited")
        {
            return Err(
                "resource_limited: 采集存储达到预算；保留已有页面和派生文件，不追加或覆盖输出"
                    .into(),
            );
        }

        write_collection_csv(
            &output.join("query_plan.csv"),
            &plan_fields,
            &plans,
            storage,
        )?;
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
            storage,
        )?;
        write_collection_csv(
            &output.join("candidates.csv"),
            &candidate_fields,
            &rows,
            storage,
        )?;
        write_collection_csv(
            &output.join("review_candidates.csv"),
            &candidate_fields,
            &rows
                .iter()
                .filter(|row| number(row, &["score"]) >= 30)
                .cloned()
                .collect::<Vec<_>>(),
            storage,
        )?;
        Ok(())
    })();
    if let Err(detail) = artifacts {
        let category = if detail.contains("resource_limited")
            || detail.starts_with("receipt_retention_limit:")
        {
            "resource_limited"
        } else {
            "artifact_io"
        };
        summary.fail(category);
        checkpoint.write_state(output, &summary.state()?)?;
        return Err(detail);
    }
    summary.refresh(&checkpoint)?;
    let cancelled = cancel.load(Ordering::Relaxed)
        || halted
            .as_ref()
            .is_some_and(|(category, _)| *category == "cancelled");
    if cancelled {
        summary.complete = false;
        summary.coverage_complete = false;
    }
    summary.status = if cancelled {
        "cancelled"
    } else if summary.complete {
        "completed"
    } else {
        "incomplete"
    }
    .into();
    checkpoint.write_state(output, &summary.state()?)?;
    if cancelled {
        return Err("__CANCELLED__".into());
    }
    if summary.failed_queries > 0 {
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
        return Err(format!("FOFA {}/{} 条查询未完成；已保留 {} 条候选、页面检查点和 query_log.csv，可原地恢复。首条失败：{first}",summary.failed_queries,queries.len(),rows.len()));
    }
    progress(
        queries.len(),
        queries.len(),
        "采集执行完成；覆盖状态见任务摘要",
    );
    summary.state()
}

#[cfg(test)]
#[path = "native_collection_resume_tests.rs"]
mod native_collection_resume_tests;
