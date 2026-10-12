//! U178: InputParser readonly SDK input surface + strict output schema.
//!
//! The minted InputParser child (U177) analyzes only authorized, frozen, this-Root
//! this-scan Broker request facts (the U175 frame's cited records). The SDK input
//! is nothing but redacted metadata of those records — method, path, observed query
//! parameter names, whether a body was present — never raw values, no tools, no
//! target/browser grant. The SDK output is a strict parameter inventory: for each
//! observed parameter a type assumption and one controlled Broker variant template
//! id, chosen from fixed allowlists. It can never carry an arbitrary attack string,
//! an invented parameter, a live target request, or a `confirmed` impact claim.
use super::lease::CoordinatorLease;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub(crate) const PHASE: &str = "input_parser_readonly";
/// The only controlled, non-live Broker variant families the parser may select.
/// A live target request is never one of them (that is Broker/Executor work, not
/// the readonly parser's). Expanded only with a real Broker variant catalog later.
pub(crate) const BROKER_VARIANT_TEMPLATES: &[&str] = &[
    "controlled_boundary_empty",
    "controlled_boundary_overlong",
    "controlled_type_confusion",
    "controlled_enum_out_of_range",
    "controlled_encoding_variant",
];
/// Type assumptions the parser may record for an observed parameter.
pub(crate) const TYPE_ASSUMPTIONS: &[&str] =
    &["string", "integer", "boolean", "enum", "identifier", "opaque"];
/// Observed parameter locations the surface exposes. Only query parameter names
/// survive redaction; a request body is reported as presence only, never named.
pub(crate) const PARAMETER_LOCATIONS: &[&str] = &["query"];
/// The parser publishes assumptions only; a live probe is always still missing,
/// so a confirmed impact can never be claimed from this readonly surface alone.
pub(crate) const REQUIRED_GAP: &str = "missing_live_parameter_probe";
const ALLOWED_METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

fn bounded(s: &str, n: usize) -> bool {
    !s.is_empty() && s.len() <= n && !s.chars().any(|c| c.is_control())
}
fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Task {
    pub(crate) schema_version: u8,
    pub(crate) phase: String,
    pub(crate) root_run_id: String,
    pub(crate) scan_id: String,
    pub(crate) attempt_number: i64,
    pub(crate) target: String,
    pub(crate) coordinator_epoch: i64,
    pub(crate) coordinator_fence: String,
    pub(crate) decision_hash: String,
    pub(crate) target_requests_granted: u8,
    pub(crate) browser_actions_granted: u8,
    pub(crate) tools_granted: Vec<String>,
    pub(crate) surfaces: Vec<Surface>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Surface {
    pub(crate) evidence_ref: String,
    pub(crate) request_id: String,
    pub(crate) record_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) body_sha256: Option<String>,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) parameters: Vec<Parameter>,
    pub(crate) body_present: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Parameter {
    pub(crate) name: String,
    pub(crate) location: String,
}

impl Task {
    /// Re-validate a surface against its lease and the frame decision hash it was
    /// minted for. Any scope, grant or redaction drift fails closed.
    pub(crate) fn from_value(
        value: &Value,
        lease: &CoordinatorLease,
        decision_hash: &str,
    ) -> Result<Self, String> {
        let task: Self =
            serde_json::from_value(value.clone()).map_err(|_| "input_parser_task_invalid")?;
        if task.schema_version != 1
            || task.phase != PHASE
            || task.root_run_id != lease.root_run_id
            || task.scan_id != lease.scan_id
            || task.attempt_number != lease.attempt_number
            || task.target != lease.target_key
            || task.coordinator_epoch != lease.lease_epoch
            || task.coordinator_fence != lease.fencing_token
            || !hex64(&task.decision_hash)
            || task.decision_hash != decision_hash
            || task.target_requests_granted != 0
            || task.browser_actions_granted != 0
            || !task.tools_granted.is_empty()
            || task.surfaces.is_empty()
            || task.surfaces.len() > 16
        {
            return Err("input_parser_task_scope_invalid".into());
        }
        let mut refs = BTreeSet::new();
        for s in &task.surfaces {
            if !bounded(&s.evidence_ref, 128)
                || !refs.insert(s.evidence_ref.clone())
                || !bounded(&s.request_id, 256)
                || !hex64(&s.record_sha256)
                || s.body_sha256.as_ref().is_some_and(|h| !hex64(h))
                || !ALLOWED_METHODS.contains(&s.method.as_str())
                || !bounded(&s.path, 1024)
                || s.parameters.len() > 40
                || (s.parameters.is_empty() && !s.body_present)
            {
                return Err("input_parser_surface_invalid".into());
            }
            let mut names = BTreeSet::new();
            for p in &s.parameters {
                if !bounded(&p.name, 128)
                    || !PARAMETER_LOCATIONS.contains(&p.location.as_str())
                    || !names.insert((p.name.clone(), p.location.clone()))
                {
                    return Err("input_parser_surface_invalid".into());
                }
            }
        }
        Ok(task)
    }
    /// Every observed (name, location) pair the model is allowed to reference.
    fn observed(&self) -> BTreeSet<(String, String)> {
        self.surfaces
            .iter()
            .flat_map(|s| s.parameters.iter().map(|p| (p.name.clone(), p.location.clone())))
            .collect()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Assessment {
    summary: String,
    parameters: Vec<ParameterAssessment>,
    gaps: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParameterAssessment {
    name: String,
    location: String,
    type_assumption: String,
    broker_variant_template_id: String,
}

/// The parser's output must stay inside the readonly surface: real observed
/// parameters only, a controlled variant template per entry, assumptions not
/// impact. `deny_unknown_fields` already refuses `confirmed`/`candidates` and any
/// smuggled attack field; the checks below refuse invented names and live intents.
pub(crate) fn validate_assessment(text: &str, task: &Task) -> Result<(), String> {
    if text.len() > 4096 {
        return Err("input_parser_response_oversized".into());
    }
    let a: Assessment = serde_json::from_str(text).map_err(|_| "input_parser_response_invalid")?;
    let observed = task.observed();
    let mut seen = BTreeSet::new();
    if !bounded(&a.summary, 1024)
        || a.parameters.is_empty()
        || a.parameters.len() > 40
        || a.gaps.is_empty()
        || a.gaps.len() > 8
        || !a.gaps.iter().any(|g| g == REQUIRED_GAP)
        || a.gaps.iter().any(|g| !bounded(g, 128))
    {
        return Err("input_parser_readonly_assessment_insufficient".into());
    }
    for p in &a.parameters {
        if !PARAMETER_LOCATIONS.contains(&p.location.as_str())
            || !TYPE_ASSUMPTIONS.contains(&p.type_assumption.as_str())
            || !BROKER_VARIANT_TEMPLATES.contains(&p.broker_variant_template_id.as_str())
            || !observed.contains(&(p.name.clone(), p.location.clone()))
            || !seen.insert((
                p.name.clone(),
                p.location.clone(),
                p.broker_variant_template_id.clone(),
            ))
        {
            return Err("input_parser_readonly_assessment_insufficient".into());
        }
    }
    Ok(())
}
