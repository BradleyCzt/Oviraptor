// Union discovery provenance before refinement and again before network work.
// A source label is evidence of discovery, not an ownership decision.

fn discovery_identity(row: &Record) -> String {
    super::asset_identity::canonical_identity(
        &text(
            row,
            &["link", "host", "probe_input_url", "probe_effective_url"],
        ),
        &text(row, &["host"]),
        &text(row, &["protocol"]),
        &text(row, &["ip"]),
        &text(row, &["port"]),
        &text(row, &["asset_key"]),
    )
}

fn discovery_source(row: &Record) -> serde_json::Value {
    json!({
        "label": text(row, &["company"]),
        "kind": text(row, &["discovery_seed_kind"]),
        "evidence": text(row, &["evidence"]),
        "matchedQueries": text(row, &["matched_queries"]),
        "score": number(row, &["score"]),
        "assetKey": text(row, &["asset_key"]),
        "host": text(row, &["host"]), "link": text(row, &["link"]),
        "ip": text(row, &["ip"]), "port": text(row, &["port"]),
        "protocol": text(row, &["protocol"]),
        "observedAt": text(row, &["collection_provider_observed_at", "lastupdatetime"]),
        "receivedAt": text(row, &["collection_received_at"]),
        "timeStatus": text(row, &["collection_time_status"]),
        "requestSha256": text(row, &["collection_request_sha256"]),
        "payloadSha256": text(row, &["collection_payload_sha256"]),
        "page": text(row, &["collection_page"]),
        "ruleVersion": text(row, &["collection_rule_version"]),
        "actualFieldsJson": text(row, &["collection_actual_fields_json"]),
        "observation": super::asset_observation::source_snapshot(row),
    })
}

const DISCOVERY_SOURCE_LIMIT: usize = 128;
const DISCOVERY_SOURCE_BYTES: usize = 64 * 1024;

fn discovery_source_record(source: &serde_json::Value) -> Record {
    let has_snapshot = source
        .get("observation")
        .and_then(serde_json::Value::as_object)
        .is_some();
    let mut row: Record = source
        .get("observation")
        .and_then(serde_json::Value::as_object)
        .map(|snapshot| {
            snapshot
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    // Legacy sources contain only these original fields. Missing observations
    // remain missing; neither replay nor conversion mints a time or fact.
    for (field, key) in [
        ("host", "host"),
        ("link", "link"),
        ("ip", "ip"),
        ("port", "port"),
        ("protocol", "protocol"),
        ("collection_provider_observed_at", "observedAt"),
        ("collection_received_at", "receivedAt"),
        ("collection_time_status", "timeStatus"),
        ("collection_request_sha256", "requestSha256"),
        ("collection_payload_sha256", "payloadSha256"),
        ("collection_page", "page"),
        ("collection_rule_version", "ruleVersion"),
        ("collection_actual_fields_json", "actualFieldsJson"),
    ] {
        if has_snapshot && field != "collection_received_at" {
            continue;
        }
        if let Some(value) = source
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
        {
            row.entry(field.to_string())
                .or_insert_with(|| value.to_string());
        }
    }
    row
}

fn discovery_source_cmp(left: &serde_json::Value, right: &serde_json::Value) -> std::cmp::Ordering {
    super::asset_observation::source_observation_cmp(
        &discovery_source_record(left),
        &discovery_source_record(right),
    )
}

fn parse_discovery_sources(raw: &str) -> (Vec<serde_json::Value>, bool) {
    // Legacy JSON predates the retention budget. Do not allocate an unbounded
    // history on re-import; the original run's raw receipts remain separate.
    if raw.len() > 1024 * 1024 {
        return (Vec::new(), true);
    }
    match serde_json::from_str::<Vec<serde_json::Value>>(raw) {
        Ok(sources) => (sources, false),
        Err(_) => (Vec::new(), !raw.is_empty()),
    }
}

fn bounded_discovery_sources(sources: Vec<serde_json::Value>) -> (Vec<serde_json::Value>, bool) {
    let mut by_source = BTreeMap::<String, serde_json::Value>::new();
    let mut truncated = false;
    for source in sources {
        if !source.is_object() {
            truncated = true;
            continue;
        }
        let value = |key| {
            source
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
        };
        let endpoint = super::asset_identity::canonical_identity(
            value("link"),
            value("host"),
            value("protocol"),
            value("ip"),
            value("port"),
            value("assetKey"),
        );
        let query = if value("matchedQueries").is_empty() {
            value("evidence")
        } else {
            value("matchedQueries")
        };
        let key = serde_json::to_string(&[value("kind"), value("label"), query, endpoint.as_str()])
            .expect("source key serializes");
        if by_source.get(&key).is_none_or(|old| {
            discovery_source_cmp(&source, old)
                .then_with(|| source.to_string().cmp(&old.to_string()))
                .is_gt()
        }) {
            by_source.insert(key, source);
        }
    }
    let mut sources: Vec<_> = by_source.into_values().collect();
    sources.sort_by(|left, right| {
        discovery_source_cmp(right, left).then_with(|| left.to_string().cmp(&right.to_string()))
    });
    let mut retained = Vec::new();
    let mut bytes = 2usize;
    for source in sources {
        let size = source.to_string().len() + usize::from(!retained.is_empty());
        if retained.len() >= DISCOVERY_SOURCE_LIMIT || bytes + size > DISCOVERY_SOURCE_BYTES {
            truncated = true;
            continue;
        }
        bytes += size;
        retained.push(source);
    }
    (retained, truncated)
}

fn discovery_sources(row: &Record) -> Vec<serde_json::Value> {
    let (stored, _) = parse_discovery_sources(&text(row, &["discovery_sources_json"]));
    if stored.is_empty() {
        vec![discovery_source(row)]
    } else {
        stored
    }
}

fn discovery_source_count(sources: &[serde_json::Value]) -> usize {
    sources
        .iter()
        .filter_map(|source| {
            Some((
                source.get("kind")?.as_str()?,
                source.get("label")?.as_str()?,
            ))
        })
        .collect::<BTreeSet<_>>()
        .len()
}

pub(super) fn merge_persisted_discovery(stored: &str, incoming: &str) -> (String, usize, bool) {
    let (mut sources, old_truncated) = parse_discovery_sources(stored);
    let (incoming, new_truncated) = parse_discovery_sources(incoming);
    sources.extend(incoming);
    // Older interrupted re-probe CSVs have no provenance columns. Their
    // synthesized label with no discovery evidence is not a new source.
    let known_labels = sources
        .iter()
        .filter(|source| {
            source
                .get("kind")
                .and_then(|v| v.as_str())
                .is_some_and(|kind| !kind.is_empty())
        })
        .filter_map(|source| {
            source
                .get("label")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect::<BTreeSet<_>>();
    sources.retain(|source| {
        let value = |field| source.get(field).and_then(|v| v.as_str()).unwrap_or("");
        !(value("kind").is_empty()
            && value("evidence").is_empty()
            && value("matchedQueries").is_empty()
            && known_labels.contains(value("label")))
    });
    let (sources, truncated) = bounded_discovery_sources(sources);
    let count = discovery_source_count(&sources);
    (
        serde_json::to_string(&sources).expect("JSON values serialize"),
        count,
        old_truncated || new_truncated || truncated,
    )
}

fn store_discovery_sources(
    row: &mut Record,
    sources: Vec<serde_json::Value>,
    inherited_truncated: bool,
) {
    let (sources, truncated) = bounded_discovery_sources(sources);
    row.insert(
        "discovery_source_count".into(),
        discovery_source_count(&sources).to_string(),
    );
    row.insert(
        "discovery_sources_json".into(),
        serde_json::to_string(&sources).expect("JSON values serialize"),
    );
    row.insert(
        "discovery_sources_truncated".into(),
        (inherited_truncated || truncated).to_string(),
    );
}

fn source_history_truncated(row: &Record) -> bool {
    text(row, &["discovery_sources_truncated"]) == "true"
        || parse_discovery_sources(&text(row, &["discovery_sources_json"])).1
}

fn merge_discovery_evidence(target: &mut Record, incoming: &Record) {
    let mut sources = discovery_sources(target);
    sources.extend(discovery_sources(incoming));
    let replace = super::asset_observation::source_observation_cmp(incoming, target).is_gt();
    let inherited_truncated =
        source_history_truncated(target) || source_history_truncated(incoming);
    store_discovery_sources(target, sources, inherited_truncated);
    for field in ["evidence", "matched_queries", "phases"] {
        let values = [text(target, &[field]), text(incoming, &[field])]
            .into_iter()
            .flat_map(|value| value.split(" | ").map(str::to_string).collect::<Vec<_>>())
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>();
        target.insert(
            field.into(),
            values.into_iter().collect::<Vec<_>>().join(" | "),
        );
    }
    let score = number(target, &["score"]).max(number(incoming, &["score"]));
    if score > 0 {
        target.insert("score".into(), score.to_string());
    }
    if target.contains_key("confidence") {
        target.insert(
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
    }
    // Prefer an explicit company seed for the legacy display column, but keep
    // every label above. Never turn this preference into confirmed ownership.
    if text(target, &["discovery_seed_kind"]) != "company"
        && text(incoming, &["discovery_seed_kind"]) == "company"
    {
        target.insert("company".into(), text(incoming, &["company"]));
        target.insert("discovery_seed_kind".into(), "company".into());
    }
    if replace {
        // One whole observation wins, including absence. Filling a new missing
        // field from old evidence would create a snapshot that never existed.
        target.retain(|key, _| {
            !super::asset_observation::is_source_technical_field(key)
                && key != "collection_received_at"
                && key != "collection_observation_hash"
        });
        target.extend(super::asset_observation::source_snapshot(incoming));
        if let Some(received) = incoming.get("collection_received_at") {
            target.insert("collection_received_at".into(), received.clone());
        }
    }
}

fn merge_discovery_rows(rows: Vec<Record>) -> Vec<Record> {
    let mut merged = BTreeMap::<String, Record>::new();
    for mut row in rows {
        let key = discovery_identity(&row);
        // Seed the original source before replacing its raw asset_key.
        let sources = discovery_sources(&row);
        let truncated = source_history_truncated(&row);
        store_discovery_sources(&mut row, sources, truncated);
        if let Some(existing) = merged.get_mut(&key) {
            merge_discovery_evidence(existing, &row);
        } else {
            row.insert("asset_key".into(), key.clone());
            merged.insert(key, row);
        }
    }
    merged.into_values().collect()
}

// Preserve the first (highest priority) stage; union all later evidence onto
// that row. This also protects re-probes and old checkpoint input CSVs.
fn merge_probe_stages(stages: &mut [(String, Vec<String>, Vec<Record>)]) -> usize {
    let mut seen = HashMap::<String, (usize, usize)>::new();
    let mut duplicates = 0;
    for stage_index in 0..stages.len() {
        let rows = std::mem::take(&mut stages[stage_index].2);
        for mut row in rows {
            let key = discovery_identity(&row);
            if let Some(&(old_stage, old_row)) = seen.get(&key) {
                merge_discovery_evidence(&mut stages[old_stage].2[old_row], &row);
                duplicates += 1;
            } else {
                let sources = discovery_sources(&row);
                let truncated = source_history_truncated(&row);
                store_discovery_sources(&mut row, sources, truncated);
                seen.insert(key, (stage_index, stages[stage_index].2.len()));
                stages[stage_index].2.push(row);
            }
        }
    }
    for (_, fields, _) in stages {
        for field in [
            "discovery_seed_kind",
            "discovery_sources_json",
            "discovery_source_count",
            "discovery_sources_truncated",
        ] {
            if !fields.iter().any(|value| value == field) {
                fields.push(field.into());
            }
        }
    }
    duplicates
}

#[cfg(test)]
#[path = "native_discovery_tests.rs"]
mod discovery_tests;
