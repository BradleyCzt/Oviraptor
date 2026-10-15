// Exact response grouping is a presentation/analysis reduction. Every original
// request identity remains in all_probe_results.csv and is imported separately.
const RESPONSE_GROUP_FIELDS: &[&str] = &[
    "response_group_key",
    "response_group_representative_url",
    "response_group_representative_asset_key",
    "response_group_member_count",
    "response_group_members_json",
    "response_duplicate_of",
];

fn full_response_fingerprint(status: u16, bytes: usize, hash: &str, content_type: &str) -> String {
    let proof = json!(["http-full-body-v1", status, bytes, hash, content_type]).to_string();
    format!("{:x}", Sha256::digest(proof.as_bytes()))
}

fn exact_response_identity(row: &Record) -> Option<String> {
    let status = text(row, &["probe_status_code"]).parse::<u16>().ok()?;
    let length = text(row, &["probe_body_bytes"]).parse::<usize>().ok()?;
    let limit = text(row, &["probe_body_limit"]).parse::<usize>().ok()?;
    let hash = text(row, &["body_sha256"]);
    let encoding = text(row, &["probe_content_encoding"]);
    // Never turn a common gateway refusal, partial body, browser shell or a
    // missing legacy proof into evidence that two business sites are identical.
    if !(200..300).contains(&status)
        || status == 206
        || text(row, &["probe_outcome"]) != "web_alive"
        || text(row, &["probe_body_complete"]) != "true"
        || text(row, &["probe_body_truncated"]) != "false"
        || !text(row, &["probe_content_range"]).is_empty()
        || (!encoding.is_empty() && !encoding.eq_ignore_ascii_case("identity"))
        || length == 0
        || length > limit
        || hash.len() != 64
        || !hash.bytes().all(|value| value.is_ascii_hexdigit())
        || !text(row, &["probe_error"]).is_empty()
    {
        return None;
    }
    let expected =
        full_response_fingerprint(status, length, &hash, &text(row, &["probe_content_type"]));
    if text(row, &["probe_response_fingerprint"]) != expected {
        return None;
    }
    // A MIME change can change browser behavior even when the bytes match.
    // This extra guard is deliberately stricter than title/text similarity.
    Some(format!("http-response-exact-v1:{expected}"))
}

#[derive(Default)]
struct ExactResponseSummary {
    groups: usize,
    duplicates: usize,
}

fn response_member_trace(row: &Record) -> serde_json::Value {
    json!({
        "assetKey": text(row, &["asset_key"]),
        "inputUrl": text(row, &["probe_input_url", "link", "host"]),
        "effectiveUrl": text(row, &["probe_effective_url"]),
        "company": text(row, &["company"]),
        "host": text(row, &["host"]),
        "ip": text(row, &["ip"]),
        "port": text(row, &["port"]),
        "protocol": text(row, &["protocol"]),
        "discoverySources": discovery_sources(row),
        "statusCode": text(row, &["probe_status_code"]),
        "bodySha256": text(row, &["body_sha256"]),
        "bodyBytes": text(row, &["probe_body_bytes"]),
        "probeCheckedAt": text(row, &["probe_checked_at"]),
    })
}

fn annotate_exact_response_groups(rows: &mut [Record], selected: &[usize]) -> ExactResponseSummary {
    // Explicit blanks clear an older grouping after a later response changes.
    for row in rows.iter_mut() {
        for field in RESPONSE_GROUP_FIELDS {
            row.insert((*field).into(), String::new());
        }
    }
    let mut groups = BTreeMap::<String, Vec<usize>>::new();
    for &index in selected {
        if let Some(identity) = exact_response_identity(&rows[index]) {
            groups.entry(identity).or_default().push(index);
        }
    }
    let mut summary = ExactResponseSummary::default();
    for (identity, mut indices) in groups {
        if indices.len() < 2 {
            continue;
        }
        // Network completion order must never decide the displayed URL.
        indices.sort_by(|left, right| {
            rank(&rows[*right])
                .cmp(&rank(&rows[*left]))
                .then_with(|| endpoint(&rows[*left]).cmp(&endpoint(&rows[*right])))
                .then_with(|| {
                    text(&rows[*left], &["asset_key"]).cmp(&text(&rows[*right], &["asset_key"]))
                })
        });
        let representative = indices[0];
        let representative_url = text(&rows[representative], &["probe_input_url", "link", "host"]);
        let representative_asset_key = text(&rows[representative], &["asset_key"]);
        let members = indices
            .iter()
            .map(|&index| response_member_trace(&rows[index]))
            .collect::<Vec<_>>();
        let members_json = serde_json::to_string(&members).expect("JSON values serialize");
        for &index in &indices {
            let row = &mut rows[index];
            row.insert("response_group_key".into(), identity.clone());
            row.insert(
                "response_group_representative_url".into(),
                representative_url.clone(),
            );
            row.insert(
                "response_group_representative_asset_key".into(),
                representative_asset_key.clone(),
            );
            row.insert(
                "response_group_member_count".into(),
                indices.len().to_string(),
            );
            if index == representative {
                // Store the full trace once, keeping large identical welcome
                // groups linear in size rather than copying every member N times.
                row.insert("response_group_members_json".into(), members_json.clone());
            } else {
                row.insert("response_duplicate_of".into(), representative_url.clone());
            }
        }
        summary.groups += 1;
        summary.duplicates += indices.len() - 1;
    }
    summary
}

#[cfg(test)]
#[path = "native_response_identity_tests.rs"]
mod response_identity_tests;
