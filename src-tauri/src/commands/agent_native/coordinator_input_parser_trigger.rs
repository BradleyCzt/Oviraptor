// REM-A04 U174: deterministic InputParser trigger predicate (requirements §5.6/§8).
// Rust, not the model, decides eligibility. It reads only this Root's verified,
// observed Broker HTTP request facts and writes nothing. Eligibility is
// necessary, not sufficient: it grants no child, budget, capability or target
// request, and `native_coordinator_prepare_input_parser` stays not_ready while
// `NATIVE_ROOT_PREPARE_A_BLOCKERS` remain.
const NATIVE_INPUT_PARSER_TRIGGER_FACT_LIMIT: i64 = 64;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeInputParserTriggerDecision {
    role: &'static str,
    eligible: bool,
    reason_codes: Vec<String>,
    evidence_refs: Vec<String>,
    contract_keys: Vec<String>,
    missing_prerequisites: Vec<String>,
}

/// Pure core over (evidence node id, record metadata) pairs. Only observed
/// controllable inputs count: a non-empty query parameter name in the actually
/// requested URL, or a non-empty request body. Header names exist on every
/// request and path parameters can only be inferred, so neither triggers.
#[cfg_attr(not(test), allow(dead_code))]
fn native_input_parser_trigger_from_records(
    records: &[(String, JsonValue)],
) -> NativeInputParserTriggerDecision {
    use std::collections::BTreeSet;
    let mut reasons = BTreeSet::new();
    let mut refs = BTreeSet::new();
    let mut keys = BTreeSet::new();
    for (id, record) in records {
        let request = &record["request"];
        let mut observed = false;
        if request["url"]
            .as_str()
            .and_then(|url| reqwest::Url::parse(url).ok())
            .is_some_and(|url| url.query_pairs().any(|(name, _)| !name.trim().is_empty()))
        {
            reasons.insert("observed_query_parameter".to_string());
            observed = true;
        }
        if request["bodyBytes"].as_u64().is_some_and(|bytes| bytes > 0) {
            reasons.insert("observed_request_body".to_string());
            observed = true;
        }
        if observed {
            refs.insert(id.clone());
            if let Some(key) = request["contractKey"].as_str().filter(|key| !key.trim().is_empty()) {
                keys.insert(key.to_string());
            }
        }
    }
    let eligible = !refs.is_empty();
    NativeInputParserTriggerDecision {
        role: crate::agent_runtime::contract::AgentRole::InputParser.as_str(),
        eligible,
        reason_codes: reasons.into_iter().collect(),
        evidence_refs: refs.into_iter().collect(),
        contract_keys: keys.into_iter().collect(),
        missing_prerequisites: if eligible {
            Vec::new()
        } else {
            vec!["observed_controllable_input".into()]
        },
    }
}

/// Reads the newest bounded set of this Root's observed request facts. Every
/// fact must still match its hashed Broker artifacts; a tampered or foreign
/// fact fails closed instead of being skipped, exactly like ClientSide capture.
#[cfg_attr(not(test), allow(dead_code))]
fn native_input_parser_trigger_decision(
    db: &rusqlite::Connection,
    root_run_id: &str,
    target_dir: &Path,
) -> Result<NativeInputParserTriggerDecision, String> {
    use crate::agent_runtime::evidence_graph::{
        contract::{EvidenceNodeKind, EvidenceProvenance},
        store::load_evidence_node,
    };
    let ids: Vec<String> = db
        .prepare(
            "SELECT id FROM agent_evidence_nodes WHERE root_run_id=?1 AND kind='request_record'
             AND provenance='observed' ORDER BY revision DESC,id LIMIT ?2",
        )
        .map_err(|e| e.to_string())?
        .query_map(params![root_run_id, NATIVE_INPUT_PARSER_TRIGGER_FACT_LIMIT], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let mut records = Vec::with_capacity(ids.len());
    for id in ids {
        let node = load_evidence_node(db, &id)?.ok_or("input_parser_trigger_fact_missing")?;
        if node.root_run_id != root_run_id
            || node.kind != EvidenceNodeKind::RequestRecord
            || node.provenance != EvidenceProvenance::Observed
            || node.revision <= 0
            || !verified_web_http_review_fact(target_dir, &node.artifact_refs, &node.payload)
        {
            return Err("input_parser_trigger_fact_invalid".into());
        }
        let [path] = node.artifact_refs.as_slice() else {
            return Err("input_parser_trigger_fact_invalid".into());
        };
        let name = path
            .strip_prefix("agent-http/")
            .ok_or("input_parser_trigger_fact_invalid")?;
        let directory = open_agent_artifact_directory(&target_dir.join(AGENT_HTTP_DIRECTORY))
            .ok_or("input_parser_trigger_artifact_root_invalid")?;
        let (bytes, _) = read_agent_artifact_file(&directory, name, 1_048_576)
            .ok_or("input_parser_trigger_record_missing")?;
        let record: JsonValue =
            serde_json::from_slice(&bytes).map_err(|_| "input_parser_trigger_record_invalid")?;
        records.push((node.id, record));
    }
    Ok(native_input_parser_trigger_from_records(&records))
}
