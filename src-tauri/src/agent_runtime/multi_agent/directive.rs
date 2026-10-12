// Legacy sibling modules still import their shared SQL/model helpers from this
// boundary. Keep these imports until those modules own explicit dependencies.
use super::lease::{
    require_active_attempt, require_executable_coordinator, validate_coordinator_lease,
    CoordinatorLease,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::json;

mod confirmation;
mod controlled_action_guard;
mod delivery;
mod draft_store;
pub(crate) mod human_review;
mod human_fact;
pub(crate) use human_fact::confirmed_fact_for_root;
mod inbox_claim;
mod supervised_inbox;
pub(crate) use supervised_inbox::{collect_for_original_parent, defer_for_original_parent};
mod interpretation;
pub(crate) mod ordered_plan;
pub(crate) mod ordered_execution;
#[cfg(test)]
pub use confirmation::confirm_draft;
/// DESIGN §5.6：Phase 1 的引导闸门复用现状规则初筛，但不把 `DraftInterpretation`
/// 这个内部结构外泄——只回传闸门需要的四个字段。
pub(crate) fn screen_directive_text(
    text: &str,
    target_key: &str,
) -> (String, String, Vec<String>, String) {
    let interpreted = interpretation::interpret(text, target_key);
    (
        interpreted.status,
        interpreted.coordinator_decision,
        interpreted.reason_codes,
        interpreted.safe_execution_text,
    )
}

pub(crate) fn screen_directive_material(text: &str, target: &str, plaintext: bool)
    -> (String, String, Vec<String>, String)
{
    if !plaintext { return screen_directive_text(text, target); }
    let result = interpretation::interpret_with_material(text, target, true);
    (result.status, result.coordinator_decision, result.reason_codes, result.safe_execution_text)
}
pub use confirmation::{cancel_draft, confirm_bound_draft};
use delivery::prepare_model_context_in_transaction;
pub use delivery::{prepare_model_context, record_model_delivery};
pub(crate) use draft_store::create_draft_in_transaction;
pub use draft_store::list_open_drafts;
use draft_store::{confirmed_payload, draft_integrity_valid, load_draft, validate_thread_key};
#[cfg(test)]
pub use draft_store::{create_draft, create_draft_in_thread};
#[cfg(test)]
pub use inbox_claim::claim_pending_directives;
#[cfg(test)]
pub use inbox_claim::transition_directive;
use inbox_claim::{claim_pending_in_transaction, transition_directive_in_transaction};
pub(crate) mod queue_actions;
pub(crate) use queue_actions::apply_for_original_parent as apply_queue_for_original_parent;
#[cfg(test)]
pub use queue_actions::apply_queue_actions;
pub mod proposals;
pub mod reconciliation;
pub(crate) mod source_guidance;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserDirective {
    pub id: String,
    pub text: String,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanDirectiveDraft {
    pub id: String,
    pub source_message_id: String,
    pub scan_id: String,
    pub attempt_number: i64,
    pub root_run_id: String,
    pub target_key: String,
    pub recipient_role: String,
    pub thread_key: String,
    pub text: String,
    pub intent: String,
    pub requested_roles: Vec<String>,
    pub referenced_fact_ids: Vec<String>,
    pub requested_contracts: Vec<String>,
    pub priority_changes: Vec<String>,
    pub proposed_scope_change: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readonly_assessment_plan: Option<ordered_plan::OrderedAssessmentPlan>,
    pub estimated_tokens: i64,
    pub estimated_requests: i64,
    pub side_effect_class: String,
    pub required_approvals: Vec<String>,
    pub validation_result: String,
    pub reason_codes: Vec<String>,
    pub coordinator_decision: String,
    pub confirmation_required: bool,
    pub safe_execution_text: String,
    pub revision: i64,
    pub draft_hash: String,
    #[serde(skip_serializing)]
    pub bound_lease_epoch: i64,
    #[serde(skip_serializing)]
    pub bound_fencing_token: String,
    pub status: String,
    pub confirmed_directive_id: String,
}

#[derive(Clone, Debug)]
struct DraftInterpretation {
    intent: String,
    requested_roles: Vec<String>,
    referenced_fact_ids: Vec<String>,
    requested_contracts: Vec<String>,
    priority_changes: Vec<String>,
    proposed_scope_change: Option<String>,
    estimated_tokens: i64,
    estimated_requests: i64,
    side_effect_class: String,
    required_approvals: Vec<String>,
    validation_result: String,
    reason_codes: Vec<String>,
    coordinator_decision: String,
    confirmation_required: bool,
    safe_execution_text: String,
    status: String,
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|item| item == value) {
        values.push(value.to_string());
    }
}

fn extract_fact_ids(text: &str) -> Vec<String> {
    let mut facts = Vec::new();
    for token in text.split(|character: char| {
        !(character.is_ascii_alphanumeric() || character == '-' || character == '_')
    }) {
        // G-17 and similar chat labels are not evidence-graph node IDs.
        // Keep them in the redacted text, never in structured fact refs.
        if valid_graph_fact_id(token) {
            push_unique(&mut facts, token);
        }
    }
    facts
}

fn valid_graph_fact_id(id: &str) -> bool {
    id.len() == 35
        && id.starts_with("ev-")
        && id[3..]
            .bytes()
            .all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
}

/// A text reference does not create evidence. Revalidate during drafting,
/// confirmation and claim so a superseded or cross-root node never becomes a
/// trusted machine reference. Old G-17 drafts remain readable but not claimable.
fn fact_refs_current(
    connection: &Connection,
    root_run_id: &str,
    refs: &[String],
) -> Result<bool, String> {
    for id in refs {
        if !valid_graph_fact_id(id) {
            return Ok(false);
        }
        let Some(node) =
            crate::agent_runtime::evidence_graph::store::load_evidence_node(connection, id)?
        else {
            return Ok(false);
        };
        if node.root_run_id != root_run_id
            || node.provenance
                == crate::agent_runtime::evidence_graph::contract::EvidenceProvenance::Inferred
            || matches!(
                node.kind,
                crate::agent_runtime::evidence_graph::contract::EvidenceNodeKind::Hypothesis
                    | crate::agent_runtime::evidence_graph::contract::EvidenceNodeKind::Contract
                    | crate::agent_runtime::evidence_graph::contract::EvidenceNodeKind::CandidateFinding
                    | crate::agent_runtime::evidence_graph::contract::EvidenceNodeKind::Finding
            )
        {
            return Ok(false);
        }
        let superseded: i64 = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_evidence_nodes WHERE root_run_id=?1 AND supersedes_id=?2)",
                params![root_run_id, id],
                |row| row.get(0),
            )
            .map_err(|error| format!("无法检查用户引用的 evidence 版本：{error}"))?;
        if superseded != 0 {
            return Ok(false);
        }
    }
    Ok(true)
}
