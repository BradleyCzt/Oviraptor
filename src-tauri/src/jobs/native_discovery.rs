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
    })
}

fn discovery_sources(row: &Record) -> Vec<serde_json::Value> {
    let stored = text(row, &["discovery_sources_json"]);
    serde_json::from_str::<Vec<serde_json::Value>>(&stored)
        .ok()
        .filter(|sources| !sources.is_empty())
        .unwrap_or_else(|| vec![discovery_source(row)])
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

pub(super) fn merge_persisted_discovery(stored: &str, incoming: &str) -> (String, usize) {
    let mut sources = serde_json::from_str::<Vec<serde_json::Value>>(stored).unwrap_or_default();
    sources.extend(serde_json::from_str::<Vec<serde_json::Value>>(incoming).unwrap_or_default());
    sources.sort_by_key(serde_json::Value::to_string);
    sources.dedup();
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
    let count = discovery_source_count(&sources);
    (
        serde_json::to_string(&sources).expect("JSON values serialize"),
        count,
    )
}

fn merge_discovery_evidence(target: &mut Record, incoming: &Record) {
    let mut sources = discovery_sources(target);
    sources.extend(discovery_sources(incoming));
    sources.sort_by_key(serde_json::Value::to_string);
    sources.dedup();
    let count = discovery_source_count(&sources);
    target.insert("discovery_source_count".into(), count.to_string());
    target.insert(
        "discovery_sources_json".into(),
        serde_json::to_string(&sources).expect("JSON values serialize"),
    );
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
    for (key, value) in incoming {
        if !value.is_empty() && target.get(key).is_none_or(|old| old.is_empty()) {
            target.insert(key.clone(), value.clone());
        }
    }
}

fn merge_discovery_rows(rows: Vec<Record>) -> Vec<Record> {
    let mut merged = BTreeMap::<String, Record>::new();
    for mut row in rows {
        let key = discovery_identity(&row);
        // Seed the original source before replacing its raw asset_key.
        let sources = discovery_sources(&row);
        row.insert(
            "discovery_sources_json".into(),
            serde_json::to_string(&sources).expect("JSON values serialize"),
        );
        if let Some(existing) = merged.get_mut(&key) {
            merge_discovery_evidence(existing, &row);
        } else {
            row.insert("asset_key".into(), key.clone());
            row.insert(
                "discovery_source_count".into(),
                discovery_source_count(&sources).to_string(),
            );
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
                row.insert(
                    "discovery_sources_json".into(),
                    serde_json::to_string(&sources).expect("JSON values serialize"),
                );
                row.insert(
                    "discovery_source_count".into(),
                    discovery_source_count(&sources).to_string(),
                );
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
