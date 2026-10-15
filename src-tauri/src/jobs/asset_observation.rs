//! Projection of original source/probe observations. Import time is never an
//! observation clock; provider timestamps remain literal source evidence.
use chrono::{DateTime, FixedOffset};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
};

pub(super) fn parse_clock(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value.trim()).ok()
}

fn source_derived(key: &str) -> bool {
    matches!(
        key,
        "company"
            | "asset_key"
            | "discovery_seed_kind"
            | "evidence"
            | "matched_queries"
            | "phases"
            | "score"
            | "confidence"
            | "review_tier"
            | "attribution_reason"
            | "activity_state"
            | "evidence_types"
            | "company_match_count"
    )
}
fn optimizer_derived(key: &str) -> bool {
    matches!(key, "auto_excluded" | "exclude_reason")
}
fn probe_field(key: &str) -> bool {
    key.starts_with("probe_")
        || matches!(
            key,
            "body_sha256"
                | "has_login"
                | "has_register"
                | "content_category"
                | "content_risk_score"
                | "content_matches"
        )
}
fn provenance(key: &str) -> bool {
    matches!(
        key,
        "discovery_sources_json" | "discovery_source_count" | "discovery_sources_truncated"
    )
}
fn internal(key: &str) -> bool {
    key.starts_with("response_group_")
        || key == "response_duplicate_of"
        || matches!(
            key,
            "collection_observation_hash" | "probe_observation_hash" | "imported_at"
        )
}
pub(super) fn is_source_technical_field(key: &str) -> bool {
    key != "collection_received_at"
        && !source_derived(key)
        && !optimizer_derived(key)
        && !probe_field(key)
        && !provenance(key)
        && !internal(key)
}
pub(super) fn source_snapshot(row: &HashMap<String, String>) -> BTreeMap<String, String> {
    row.iter()
        .filter(|(key, value)| is_source_technical_field(key) && !value.is_empty())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
fn hash(snapshot: &BTreeMap<String, String>) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(snapshot).expect("string map serializes"))
    )
}
/// Shared with collection deduplication. Provider dates do not order receipts.
/// Equal-clock conflicts select an entire snapshot with a stable tie breaker.
pub(super) fn source_observation_cmp(
    left: &HashMap<String, String>,
    right: &HashMap<String, String>,
) -> Ordering {
    let clock = |row: &HashMap<String, String>| {
        row.get("collection_received_at")
            .and_then(|v| parse_clock(v))
    };
    clock(left)
        .cmp(&clock(right))
        .then_with(|| hash(&source_snapshot(left)).cmp(&hash(&source_snapshot(right))))
        .then_with(|| {
            left.get("collection_received_at")
                .cmp(&right.get("collection_received_at"))
        })
}
fn strings(extra: &Map<String, Value>) -> HashMap<String, String> {
    extra
        .iter()
        .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_string())))
        .collect()
}
fn text<'a>(extra: &'a Map<String, Value>, key: &str) -> &'a str {
    extra.get(key).and_then(Value::as_str).unwrap_or("")
}
fn truth(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value.as_bool() == Some(true) || matches!(value.as_str(), Some("1" | "true"))
    })
}

pub(super) struct Stored {
    pub extra: Map<String, Value>,
    pub last_seen: String,
    pub last_alive: Option<String>,
    ip: String,
    endpoint: String,
    probe_outcome: String,
}
impl Stored {
    pub fn load(db: &Connection, key: &str) -> Result<Option<Self>, String> {
        db.query_row("SELECT extra_json,last_seen,last_alive,ip,link,host,protocol,port,probe_outcome FROM assets WHERE asset_key=?1", [key], |row| {
            let raw: String = row.get(0)?;
            let link: String = row.get(4)?;
            let host: String = row.get(5)?;
            let protocol: String = row.get(6)?;
            let ip: String = row.get(3)?;
            let port: String = row.get(7)?;
            let mut extra = serde_json::from_str::<Value>(&raw).ok().and_then(|v|v.as_object().cloned()).unwrap_or_default();
            // Old databases may have network columns but no mirrored CSV fields.
            for (name,value) in [("link",&link),("host",&host),("protocol",&protocol),("ip",&ip),("port",&port)] {
                if !value.is_empty() { extra.entry(name.to_string()).or_insert_with(||Value::String(value.clone())); }
            }
            let endpoint = super::asset_identity::endpoint_url(&link,&host,&protocol,&ip,&port).unwrap_or(link);
            Ok(Self { extra, last_seen: row.get(1)?, last_alive: row.get(2)?, ip, endpoint, probe_outcome: row.get(8)? })
        }).optional().map_err(|error|error.to_string())
    }
}
pub(super) struct Plan {
    pub source_replace: bool,
    pub source_current: bool,
    pub probe_replace: bool,
    pub probe_current: bool,
    pub classification_current: bool,
    pub seen: String,
    pub last_alive: Option<String>,
    pub extra_json: String,
}
/// Preserve unknown legacy values; only a precise incoming observation can
/// initialize/advance an observation clock. No caller supplies `now` here.
pub(super) fn advance_seen(previous: &str, incoming: &str) -> String {
    match (parse_clock(previous), parse_clock(incoming)) {
        (Some(old), Some(new)) if new > old => new
            .to_utc()
            .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
        (None, Some(new)) => new
            .to_utc()
            .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
        _ => previous.to_string(),
    }
}
fn probe_snapshot(extra: &Map<String, Value>) -> BTreeMap<String, String> {
    strings(extra)
        .into_iter()
        .filter(|(key, value)| {
            probe_field(key)
                && !matches!(
                    key.as_str(),
                    "probe_checked_at" | "probe_history_reason" | "probe_observation_hash"
                )
                && !value.is_empty()
        })
        .collect()
}
fn selection(old_clock: &str, new_clock: &str, tie: Ordering) -> (bool, bool) {
    match (parse_clock(old_clock), parse_clock(new_clock)) {
        (Some(old), Some(new)) => match new.cmp(&old) {
            Ordering::Greater => (true, true),
            Ordering::Less => (false, false),
            Ordering::Equal => (tie == Ordering::Greater, tie != Ordering::Less),
        },
        (Some(_), None) => (false, false),
        // Historical CSVs without a provable clock keep their old compatibility
        // behavior, while all their observation timestamps remain unknown.
        _ => (true, true),
    }
}
fn positive(outcome: &str) -> bool {
    matches!(
        outcome,
        "web_alive"
            | "web_restricted"
            | "browser_render_required"
            | "virtual_host_required"
            | "alive_clean"
            | "tcp_alive_non_http"
    )
}
fn probe_binding(extra: &mut Map<String, Value>, source_ip: &str, fallback_endpoint: &str) {
    let explicit_endpoint = !text(extra, "probe_input_url").is_empty();
    let endpoint = if explicit_endpoint {
        text(extra, "probe_input_url").to_string()
    } else {
        fallback_endpoint.to_string()
    };
    let observed_ip = if !text(extra, "probe_observed_ip").is_empty() {
        text(extra, "probe_observed_ip").to_string()
    } else {
        reqwest::Url::parse(&endpoint)
            .ok()
            .and_then(|url| {
                url.host_str()
                    .and_then(|v| v.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            })
            .map(|v| v.to_string())
            .unwrap_or_default()
    };
    extra.insert("probe_observed_endpoint".into(), Value::String(endpoint));
    extra.insert("probe_observed_ip".into(), Value::String(observed_ip));
    extra.insert(
        "probe_observed_endpoint_basis".into(),
        Value::String(
            if explicit_endpoint {
                "probe_input_url"
            } else {
                "legacy_candidate_endpoint"
            }
            .into(),
        ),
    );
    // A source-reported IP is not a measured TCP peer (DNS may differ). Keep it
    // separately to detect source changes without inventing remote_addr proof.
    extra.insert("probe_source_ip".into(), Value::String(source_ip.into()));
}
pub(super) fn plan(
    stored: Option<&Stored>,
    mut incoming: Map<String, Value>,
    is_probe: bool,
    endpoint: &str,
    ip: &str,
    outcome: &str,
) -> Plan {
    let empty = Map::new();
    let old = stored.map(|v| &v.extra).unwrap_or(&empty);
    let new_received = text(&incoming, "collection_received_at").to_string();
    let old_received = text(old, "collection_received_at");
    let source_tie = source_observation_cmp(&strings(&incoming), &strings(old));
    let (source_replace, source_current) = selection(old_received, &new_received, source_tie);
    if is_probe {
        probe_binding(&mut incoming, ip, endpoint);
    }
    let new_probe = text(&incoming, "probe_checked_at").to_string();
    let old_probe = text(old, "probe_checked_at");
    let probe_tie = hash(&probe_snapshot(&incoming)).cmp(&hash(&probe_snapshot(old)));
    let (probe_replace, probe_current) = if is_probe {
        selection(old_probe, &new_probe, probe_tie)
    } else {
        (false, false)
    };
    let mut extra = old.clone();
    if source_replace {
        extra.retain(|key, _| !is_source_technical_field(key) && key != "collection_received_at");
    }
    if probe_replace {
        extra.retain(|key, _| !probe_field(key));
    }
    for (key, value) in &incoming {
        let keep = if internal(key) {
            false
        } else if provenance(key) {
            false
        } else if optimizer_derived(key) {
            false
        } else if probe_field(key) {
            probe_replace
        } else if source_derived(key) {
            source_current
        } else {
            source_replace
        };
        if keep {
            if value.is_null() {
                extra.remove(key);
            } else {
                extra.insert(key.clone(), value.clone());
            }
        }
    }
    // These traces are project/run-local even for old cached probe CSVs.
    if probe_current {
        extra
            .retain(|key, _| !key.starts_with("response_group_") && key != "response_duplicate_of");
    }
    let current_ip = if source_replace {
        ip
    } else {
        stored.map(|v| v.ip.as_str()).unwrap_or(ip)
    };
    let has_probe = if probe_replace {
        !outcome.is_empty()
    } else {
        stored.is_some_and(|v| !v.probe_outcome.is_empty())
    };
    if has_probe {
        if text(&extra, "probe_observed_endpoint").is_empty() {
            // Preserve the old endpoint/IP before the new source is projected.
            probe_binding(
                &mut extra,
                stored.map(|v| v.ip.as_str()).unwrap_or(ip),
                stored.map(|v| v.endpoint.as_str()).unwrap_or(endpoint),
            );
        }
        let source_changed = !text(&extra, "probe_source_ip").is_empty()
            && !current_ip.is_empty()
            && text(&extra, "probe_source_ip") != current_ip;
        let reason = if source_changed {
            "endpoint_changed"
        } else if parse_clock(text(&extra, "probe_checked_at")).is_none() {
            "unknown_probe_time"
        } else if matches!((parse_clock(text(&extra,"collection_received_at")),parse_clock(text(&extra,"probe_checked_at"))),(Some(received),Some(checked)) if received>checked)
        {
            "collection_after_probe"
        } else {
            ""
        };
        if reason.is_empty() {
            extra.remove("probe_history_reason");
        } else {
            extra.insert("probe_history_reason".into(), Value::String(reason.into()));
        }
    } else {
        extra.remove("probe_history_reason");
    }
    let historical = matches!(
        text(&extra, "probe_history_reason"),
        "endpoint_changed" | "collection_after_probe"
    );
    let classification_current = if is_probe {
        probe_current && !historical
    } else {
        source_current
    };
    if classification_current {
        for key in ["auto_excluded", "exclude_reason"] {
            if let Some(value) = incoming.get(key) {
                extra.insert(key.into(), value.clone());
            }
        }
    }
    // Union and bound provenance independently of which technical observation
    // wins; omitted or false incoming flags can never clear prior truncation.
    let previous_sources = text(old, "discovery_sources_json");
    let incoming_sources = text(&incoming, "discovery_sources_json");
    let (sources, count, truncated) =
        super::native_workers::merge_persisted_discovery(previous_sources, incoming_sources);
    if !previous_sources.is_empty() || !incoming_sources.is_empty() {
        extra.insert("discovery_sources_json".into(), Value::String(sources));
        extra.insert(
            "discovery_source_count".into(),
            Value::String(count.to_string()),
        );
    }
    if truncated
        || truth(old.get("discovery_sources_truncated"))
        || truth(incoming.get("discovery_sources_truncated"))
    {
        extra.insert(
            "discovery_sources_truncated".into(),
            Value::String("true".into()),
        );
    }
    let seen = advance_seen(
        stored.map(|v| v.last_seen.as_str()).unwrap_or(""),
        &new_received,
    );
    let mut last_alive = stored.and_then(|v| v.last_alive.clone());
    // Alive is an aggregation of actual positive measurements, independently
    // of which (possibly negative) probe is the current projection. A delayed
    // positive can establish its original historical time, never import time.
    if is_probe && positive(outcome) && parse_clock(&new_probe).is_some() {
        let value = advance_seen(last_alive.as_deref().unwrap_or(""), &new_probe);
        last_alive = Some(value);
    }
    Plan {
        source_replace,
        source_current,
        probe_replace,
        probe_current,
        classification_current,
        seen,
        last_alive,
        extra_json: Value::Object(extra).to_string(),
    }
}

/// A legacy company/raw key can describe another protocol. Recompute the
/// persisted identity before reuse, including deterministic hash-key conflicts.
pub(super) fn choose_key(
    db: &Connection,
    candidate: &str,
    company: &str,
    canonical: &str,
) -> Result<String, String> {
    let digest = format!("{:x}", Sha256::digest(canonical.as_bytes()));
    for attempt in 0..34 {
        let key = match attempt {
            0 => candidate.to_string(),
            1 => format!("{company}\u{1f}canonical-sha256:{digest}"),
            _ => format!("{company}\u{1f}canonical-sha256:{digest}:{}", attempt - 1),
        };
        let stored: Option<(String, String, String, String, String)> = db
            .query_row(
                "SELECT link,host,protocol,ip,port FROM assets WHERE asset_key=?1",
                [&key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if stored.is_none_or(|(link, host, protocol, ip, port)| {
            super::asset_identity::canonical_identity(&link, &host, &protocol, &ip, &port, &key)
                == canonical
        }) {
            return Ok(key);
        }
    }
    Err("资产身份键冲突过多，未覆盖现有资产".into())
}
