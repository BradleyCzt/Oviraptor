// Deterministic Rust admission over already captured/verified closed facts.

/// REM-A03 six named supervision trigger kinds + currently wired frame kinds.
/// Unknown kinds still map to `root_trigger_not_enabled`.
const NATIVE_WIRED_SUPERVISION_FRAME_KINDS: &[&str] = &[
    "new-evidence",
    "lease-expired",
    "protection-changed",
    "human-directive",
    "budget-allocation",
    "budget-consumption",
    "http-budget-consumption",
    "capability-changed",
    "authorization-output",
    "external-surface-output",
    "business-logic-output",
    "business-logic-input",
    "mapper-output",
    "client-side-output",
    "identity-session-output",
    "reviewer-decision",
    "investigator-proposal",
    // U176: admitted only because a real producer exists — the frozen, hash-bound,
    // re-verified U175 frame (`NativeCoordinatorFrame::input_parser_trigger`). It is
    // an advisory Root input only: no child, grant or dispatch follows (U177+).
    "input-parser-output",
];
const NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS: &[&str] = &["human-recovery"];

/// Future Root role-output frames (U61). Not wired producers yet — admission
/// returns named not_ready so they cannot be mistaken for unknown kinds.
/// U176: `input-parser-output` moved to the wired inventory (real frozen producer);
/// a display-only skeleton can no longer stand in for it.
const NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS: &[&str] = &[];

/// U74: reserved role-output frame kind for the four A04 Root-gated roles.
fn native_reserved_role_output_kind_for_role(
    role: crate::agent_runtime::contract::AgentRole,
) -> Option<&'static str> {
    use crate::agent_runtime::contract::AgentRole;
    match role {
        AgentRole::InputParser => Some("input-parser-output"),
        AgentRole::BusinessLogic => Some("business-logic-output"),
        AgentRole::Authorization => Some("authorization-output"),
        AgentRole::ExternalSurface => Some("external-surface-output"),
        _ => None,
    }
}

fn native_agent_role_for_reserved_role_output_kind(
    kind: &str,
) -> Option<crate::agent_runtime::contract::AgentRole> {
    use crate::agent_runtime::contract::AgentRole;
    match kind {
        "input-parser-output" => Some(AgentRole::InputParser),
        "business-logic-output" => Some(AgentRole::BusinessLogic),
        "authorization-output" => Some(AgentRole::Authorization),
        "external-surface-output" => Some(AgentRole::ExternalSurface),
        _ => None,
    }
}

/// U76: mint a reserved role-output skeleton from AgentRole (still not_ready).
fn native_reserved_role_output_frame_skeleton_for_role(
    role: crate::agent_runtime::contract::AgentRole,
    display: &str,
) -> Result<NativeCoordinatorFrame, String> {
    let kind = native_reserved_role_output_kind_for_role(role)
        .ok_or_else(|| "root_trigger_skeleton_kind_invalid".to_string())?;
    native_reserved_role_output_frame_skeleton(
        kind,
        json!({
            "display": display,
            "role": kind.trim_end_matches("-output").replace('-', "_"),
            "recovery": "not_ready",
        }),
    )
}

// Model strings never set utility features, risk, cost, scope or capabilities.
#[derive(Clone, Copy, Debug)]
enum NativeCoordinatorTrigger {
    ChildOutputEnvelope,
    BudgetAllocation,
    BudgetConsumption,
    HttpBudgetConsumption,
    CapabilityTransition,
    AuthorizationOutput,
    HumanDirective,
    ClientReadonlyOutput,
    IdentityReadonlyOutput,
    ReviewerDecision,
    InvestigatorProposal,
    /// U176: the U175 frozen InputParser trigger frame (AgentTriggerDecision basis).
    InputParserTrigger,
    BusinessLogicInput,
    NewEvidence,
    LeaseExpired,
    ProtectionChanged,
    /// DESIGN §5.4：同质主管评估帧。刻意不并入上面的旧 kind 盘点。
    HomogeneousDecision,
}
/// U49: single admission for supervision frame kinds (wired / not-ready / unknown).
fn native_supervision_frame_admission(kind: &str) -> Result<(), String> {
    // DESIGN §5.4：同质 Worker 帧单独准入，不进入旧 wired / not_ready / reserved 盘点。
    if kind == NATIVE_HOMOGENEOUS_FRAME_KIND {
        return Ok(());
    }
    if NATIVE_WIRED_SUPERVISION_FRAME_KINDS.contains(&kind) {
        return Ok(());
    }
    if NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS.contains(&kind) {
        return Err(format!("root_trigger_{}_not_ready", kind.replace('-', "_")));
    }
    if NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS.contains(&kind) {
        return Err(format!("root_trigger_{}_not_ready", kind.replace('-', "_")));
    }
    Err("root_trigger_not_enabled".into())
}

/// U59/U60: pure skeleton for a named not-ready supervision frame.
/// Does not open DB / CAS / SDK; admission still returns not_ready until a
/// real producer wires recovery.
#[cfg(test)]
fn native_not_ready_supervision_frame_skeleton(
    kind: &'static str,
    semantic: JsonValue,
) -> Result<NativeCoordinatorFrame, String> {
    if !NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS.contains(&kind) {
        return Err("root_trigger_skeleton_kind_invalid".into());
    }
    Ok(NativeCoordinatorFrame {
        kind,
        semantic,
        rows: Vec::new(),
        review_refs: None,
        review_meta: None,
    })
}

/// U143: explicit blockers before any A03 supervision `not_ready` kind may flip
/// `native_not_ready_supervision_producer_ready` to true. Emptying this list is
/// necessary but not sufficient — each item needs its own real producer + green slice.
const NATIVE_A03_PRODUCER_BLOCKERS: &[&str] = &[
    "lease_scan_source",
    "evidence_revision_feed",
    "protection_watch_source",
    "human_recovery_directive_feed",
    "zero_write_proof_on_failure",
];

/// U80: A03 not_ready kinds have skeletons but no live producers yet.
#[cfg(test)]
fn native_not_ready_supervision_producer_ready(kind: &str) -> bool {
    if !NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS.contains(&kind) {
        return false;
    }
    // Wired recovery producers land later; skeletons alone must not claim ready.
    false
}

/// U162: per-kind view of the A03 supervision producer blockers, mirroring the
/// A04 `native_root_prepare_a_blockers_remaining(role)` accessor. The four not_ready
/// supervision kinds (new-evidence / lease-expired / protection-changed / human-recovery)
/// share the same outstanding blocker inventory; wired / reserved / unknown kinds have
/// no pending A03 producer work, so they return None. This makes the blockers visible
/// per kind while `native_not_ready_supervision_producer_ready` never reports ready;
/// it stays hard-false.
#[cfg_attr(not(test), allow(dead_code))]
fn native_a03_producer_blockers_remaining(kind: &str) -> Option<&'static [&'static str]> {
    if NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS.contains(&kind) {
        Some(NATIVE_A03_PRODUCER_BLOCKERS)
    } else {
        None
    }
}

/// U86: single entry for future A03 producers — today always named not_ready.
#[cfg(test)]
fn native_try_produce_not_ready_supervision_frame(
    kind: &'static str,
    semantic: JsonValue,
) -> Result<NativeCoordinatorFrame, String> {
    let frame = native_not_ready_supervision_frame_skeleton(kind, semantic)?;
    if native_not_ready_supervision_producer_ready(kind) {
        // Future: bind rows / review refs / SDK. Keep branch for exhaustiveness.
        return Ok(frame);
    }
    // U170: keep the A03 producer blockers source-visible at the honest not_ready
    // site (mirrors U146's gate call); result discarded, behavior unchanged.
    let _ = native_a03_producer_blockers_remaining(kind);
    Err(native_supervision_frame_admission(kind).unwrap_err())
}

/// U91: named try_produce wrappers for A03 not_ready kinds (still not_ready).
#[cfg(test)]
fn native_try_produce_lease_expired_frame(
    assignment_id: &str,
    child_run_id: &str,
) -> Result<NativeCoordinatorFrame, String> {
    let _ = (assignment_id, child_run_id);
    Err("root_lease_expiry_original_fact_required".into())
}

#[cfg(test)]
fn native_try_produce_new_evidence_frame(
    _evidence_revision: i64,
) -> Result<NativeCoordinatorFrame, String> {
    Err("root_new_evidence_original_fact_required".into())
}

#[cfg(test)]
fn native_try_produce_protection_changed_frame(
    _protection: &str,
) -> Result<NativeCoordinatorFrame, String> {
    Err("root_protection_original_fact_required".into())
}

#[cfg(test)]
fn native_try_produce_human_recovery_frame(
    directive_id: &str,
) -> Result<NativeCoordinatorFrame, String> {
    native_try_produce_not_ready_supervision_frame(
        "human-recovery",
        json!({
            "directiveId": directive_id,
            "recovery": "not_ready",
        }),
    )
}

/// U142: explicit blockers before Root prepare (a) may flip `prepare_implemented`.
/// Emptying this list is necessary but not sufficient — each item needs its own green slice.
const NATIVE_ROOT_PREPARE_A_BLOCKERS: &[&str] = &[
    "freeze_bound_basis",
    "readonly_sdk_input_surface",
    "scheduled_child_mint",
    "reviewer_floor_grant_wire",
    "zero_write_proof_on_failure",
];

/// U175: InputParser has a real frozen, hash-bound, re-verified trigger basis
/// (`NativeCoordinatorFrame::input_parser_trigger` + `verify_input_parser_trigger_source`,
/// proven by `agent_tests_input_parser_frame.rs`), so `freeze_bound_basis` is cleared for
/// InputParser only.
/// U177: `native_coordinator_mint_input_parser_child` mints the real child run with the
/// fixed 4000/1 grant behind Reviewer 15000/1 + Root supervision 16000/1 (proven by
/// `agent_tests_input_parser_mint.rs`), clearing `scheduled_child_mint` and
/// `reviewer_floor_grant_wire`. The other two items stay open, so prepare stays
/// not_ready and `native_root_role_prepare_implemented` stays false.
/// U178: the minted child runs the readonly SDK input surface
/// (`input_parser_build_surface` + `input_parser_verify_input` +
/// `input_parser::validate_assessment`, proven by `agent_tests_input_parser_sdk.rs`),
/// clearing `readonly_sdk_input_surface`.
/// U179: every failure branch of the readonly path (lost fence, tampered Broker
/// record, foreign-root fact, write/target capability, SDK HTTP error / unparseable
/// body, schema-invalid output) is proven zero-write — pre-send failures leave every
/// application table byte-identical and SDK failures publish no result and keep no
/// live capability lease (shared `failed_specialist_error` cleanup, proven by
/// `agent_tests_input_parser_zero_write.rs` + U178's invalid-output rollback),
/// clearing `zero_write_proof_on_failure`. InputParser's (a) blocker inventory is now
/// empty. U181 (after U180's end-to-end wiring) flipped
/// `native_root_role_prepare_implemented(InputParser)` to true.
const NATIVE_INPUT_PARSER_PREPARE_A_BLOCKERS: &[&str] = &[];

/// U137: honesty flag — true Root prepare (a) not shipped for the four gated roles.
/// U181: InputParser ships its true Root prepare (a): its blocker inventory
/// `NATIVE_INPUT_PARSER_PREPARE_A_BLOCKERS` is empty (U175/U177/U178/U179, each cleared
/// by its own real-run slice) and U180 proved the production end-to-end dispatch.
/// Only InputParser flips; BusinessLogic/Authorization/ExternalSurface still carry the
/// full shared `NATIVE_ROOT_PREPARE_A_BLOCKERS` and stay false (gates not_ready).
fn native_root_role_prepare_implemented(role: crate::agent_runtime::contract::AgentRole) -> bool {
    use crate::agent_runtime::contract::AgentRole::*;
    match role {
        InputParser => true,
        BusinessLogic | Authorization | ExternalSurface => false,
        _ => false,
    }
}

/// U145: per-role view of the Root prepare (a) blockers. The four A04 Root-gated
/// roles (InputParser/BusinessLogic/Authorization/ExternalSurface) expose their
/// outstanding blocker inventory (U175: InputParser has cleared `freeze_bound_basis`);
/// every other role has no pending (a) work, so it returns None. This makes the
/// blockers source-visible to the gates without ever flipping
/// `native_root_role_prepare_implemented` to true.
#[cfg_attr(not(test), allow(dead_code))]
fn native_root_prepare_a_blockers_remaining(
    role: crate::agent_runtime::contract::AgentRole,
) -> Option<&'static [&'static str]> {
    use crate::agent_runtime::contract::AgentRole;
    match role {
        AgentRole::InputParser => Some(NATIVE_INPUT_PARSER_PREPARE_A_BLOCKERS),
        AgentRole::BusinessLogic | AgentRole::Authorization | AgentRole::ExternalSurface => {
            Some(NATIVE_ROOT_PREPARE_A_BLOCKERS)
        }
        _ => None,
    }
}

/// U172: per reserved-output-kind view of the Root prepare (a) blockers, mirroring
/// the A03 per-kind accessor `native_a03_producer_blockers_remaining(kind)`. A reserved
/// role-output kind maps to its AgentRole and reuses the per-role blockers; any wired /
/// A03 not_ready / unknown kind has no pending (a) work, so returns None. Never flips
/// `native_root_role_prepare_implemented` to true.
#[cfg_attr(not(test), allow(dead_code))]
fn native_root_prepare_a_blockers_remaining_for_kind(
    kind: &str,
) -> Option<&'static [&'static str]> {
    let role = native_agent_role_for_reserved_role_output_kind(kind)?;
    native_root_prepare_a_blockers_remaining(role)
}

/// U81: reserved A04 role-output kinds have skeletons/gates but no producers.
fn native_reserved_role_output_producer_ready(kind: &str) -> bool {
    if !NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS.contains(&kind) {
        return false;
    }
    false
}

/// U87: single entry for future A04 reserved role-output producers.
fn native_try_produce_reserved_role_output_frame(
    kind: &'static str,
    semantic: JsonValue,
) -> Result<NativeCoordinatorFrame, String> {
    let frame = native_reserved_role_output_frame_skeleton(kind, semantic)?;
    if native_reserved_role_output_producer_ready(kind) {
        return Ok(frame);
    }
    Err(native_supervision_frame_admission(kind).unwrap_err())
}

fn native_try_produce_reserved_role_output_for_role(
    role: crate::agent_runtime::contract::AgentRole,
    display: &str,
) -> Result<NativeCoordinatorFrame, String> {
    let kind = native_reserved_role_output_kind_for_role(role)
        .ok_or_else(|| "root_trigger_skeleton_kind_invalid".to_string())?;
    native_try_produce_reserved_role_output_frame(
        kind,
        json!({
            "display": display,
            "role": kind.trim_end_matches("-output").replace('-', "_"),
            "recovery": "not_ready",
        }),
    )
}

/// U94: named try_produce wrappers for reserved A04 role-output frames.
fn native_try_produce_input_parser_output_frame(
    display: &str,
) -> Result<NativeCoordinatorFrame, String> {
    use crate::agent_runtime::contract::AgentRole;
    native_try_produce_reserved_role_output_for_role(AgentRole::InputParser, display)
}

fn native_try_produce_business_logic_output_frame(
    display: &str,
) -> Result<NativeCoordinatorFrame, String> {
    use crate::agent_runtime::contract::AgentRole;
    native_try_produce_reserved_role_output_for_role(AgentRole::BusinessLogic, display)
}

fn native_try_produce_authorization_output_frame(
    display: &str,
) -> Result<NativeCoordinatorFrame, String> {
    use crate::agent_runtime::contract::AgentRole;
    native_try_produce_reserved_role_output_for_role(AgentRole::Authorization, display)
}

fn native_try_produce_external_surface_output_frame(
    display: &str,
) -> Result<NativeCoordinatorFrame, String> {
    use crate::agent_runtime::contract::AgentRole;
    native_try_produce_reserved_role_output_for_role(AgentRole::ExternalSurface, display)
}

#[cfg(test)]
fn native_lease_expired_frame_skeleton(
    assignment_id: &str,
    child_run_id: &str,
) -> NativeCoordinatorFrame {
    NativeCoordinatorFrame {
        kind: "lease-expired",
        semantic: json!({"assignmentId":assignment_id,"runId":child_run_id,
            "failureClass":"worker_lease_expired","recovery":"original_closed_source_required"}),
        rows: Vec::new(),
        review_refs: None,
        review_meta: None,
    }
}

/// U67: named convenience skeletons for remaining A03 not_ready kinds.
#[cfg(test)]
fn native_new_evidence_frame_skeleton(
    _evidence_revision: i64,
) -> Result<NativeCoordinatorFrame, String> {
    Err("root_new_evidence_original_fact_required".into())
}

#[cfg(test)]
fn native_protection_changed_frame_skeleton(
    _protection: &str,
) -> Result<NativeCoordinatorFrame, String> {
    Err("root_protection_original_fact_required".into())
}

#[cfg(test)]
fn native_human_recovery_frame_skeleton(directive_id: &str) -> NativeCoordinatorFrame {
    native_not_ready_supervision_frame_skeleton(
        "human-recovery",
        json!({
            "directiveId": directive_id,
            "recovery": "not_ready",
        }),
    )
    .expect("human-recovery is in not_ready inventory")
}

/// U64: pure skeleton for reserved Root role-output frames (A04).
/// No prepare/child/SDK; admission stays named not_ready until producers exist.
fn native_reserved_role_output_frame_skeleton(
    kind: &'static str,
    semantic: JsonValue,
) -> Result<NativeCoordinatorFrame, String> {
    if !NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS.contains(&kind) {
        return Err("root_trigger_skeleton_kind_invalid".into());
    }
    Ok(NativeCoordinatorFrame {
        kind,
        semantic,
        rows: Vec::new(),
        review_refs: None,
        review_meta: None,
    })
}

/// U66/U78: named convenience skeletons delegate to role→kind mint (still not_ready).
/// U176: InputParser's kind is wired with a real frozen producer, so its display-only
/// skeleton is refused (`root_trigger_skeleton_kind_invalid`) instead of minted.
#[cfg_attr(not(test), allow(dead_code))]
fn native_input_parser_output_frame_skeleton(
    display: &str,
) -> Result<NativeCoordinatorFrame, String> {
    use crate::agent_runtime::contract::AgentRole;
    native_reserved_role_output_frame_skeleton_for_role(AgentRole::InputParser, display)
}

impl NativeCoordinatorTrigger {
    fn classify(frame: &NativeCoordinatorFrame) -> Result<Self, String> {
        native_supervision_frame_admission(frame.kind)?;
        match frame.kind {
            "human-directive" => Ok(Self::HumanDirective),
            "budget-allocation" => Ok(Self::BudgetAllocation),
            "budget-consumption" => Ok(Self::BudgetConsumption),
            "http-budget-consumption" => Ok(Self::HttpBudgetConsumption),
            "capability-changed" => Ok(Self::CapabilityTransition),
            "authorization-output" => Ok(Self::AuthorizationOutput),
            "external-surface-output" => Ok(Self::ChildOutputEnvelope),
            "business-logic-output" => Ok(Self::ChildOutputEnvelope),
            "business-logic-input" => Ok(Self::BusinessLogicInput),
            "mapper-output" => Ok(Self::ChildOutputEnvelope),
            "client-side-output" => Ok(Self::ClientReadonlyOutput),
            "identity-session-output" => Ok(Self::IdentityReadonlyOutput),
            "reviewer-decision" => Ok(Self::ReviewerDecision),
            "investigator-proposal" => Ok(Self::InvestigatorProposal),
            "input-parser-output" => Ok(Self::InputParserTrigger),
            "new-evidence" => Ok(Self::NewEvidence),
            "lease-expired" => Ok(Self::LeaseExpired),
            "protection-changed" => Ok(Self::ProtectionChanged),
            NATIVE_HOMOGENEOUS_FRAME_KIND => Ok(Self::HomogeneousDecision),
            // Wired inventory is authoritative; unreachable if admission stays in sync.
            other => Err(format!("root_trigger_wired_unmapped:{other}")),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::ChildOutputEnvelope
            | Self::AuthorizationOutput
            | Self::ClientReadonlyOutput
            | Self::IdentityReadonlyOutput
            | Self::InvestigatorProposal => "child_output_envelope",
            Self::BudgetAllocation
            | Self::BudgetConsumption
            | Self::HttpBudgetConsumption
            | Self::CapabilityTransition => "capability_budget_state_changed",
            Self::HumanDirective => "human_directive",
            Self::ReviewerDecision => "reviewer_decision",
            // Distinct name: a deterministic trigger decision, not a child output envelope.
            Self::InputParserTrigger => "input_parser_trigger",
            Self::BusinessLogicInput => "business_logic_input",
            Self::NewEvidence => "new_evidence",
            Self::LeaseExpired => "lease_expired",
            Self::ProtectionChanged => "protection_changed",
            Self::HomogeneousDecision => "homogeneous_decision",
        }
    }
}
fn native_coordinator_cost_weight(amount: i64, limit: Option<i64>) -> i64 {
    match limit {
        Some(limit) if limit > 0 => ((i128::from(amount) * 20 + i128::from(limit) - 1)
            / i128::from(limit))
        .clamp(0, 20) as i64,
        Some(0) if amount == 0 => 0,
        _ => 20,
    }
}
fn native_coordinator_rust_dispatch_policy(
    context: &AgentRunContext,
    frame: &NativeCoordinatorFrame,
    decision: &NativeCoordinatorTickReceipt,
    role: crate::agent_runtime::contract::AgentRole,
    tokens: i64,
    requests: i64,
    limits: [Option<i64>; 3],
) -> Result<JsonValue, String> {
    native_coordinator_rust_dispatch_policy_with_rule(
        context,
        frame,
        decision,
        role,
        tokens,
        requests,
        limits,
        NativeCoordinatorModelCostRule::OriginalConservative,
    )
}
#[allow(clippy::too_many_arguments)]
fn native_coordinator_rust_dispatch_policy_with_rule(
    context: &AgentRunContext,
    frame: &NativeCoordinatorFrame,
    decision: &NativeCoordinatorTickReceipt,
    role: crate::agent_runtime::contract::AgentRole,
    tokens: i64,
    requests: i64,
    limits: [Option<i64>; 3],
    model_cost_rule: NativeCoordinatorModelCostRule,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::{contract::AgentRole, store};
    if tokens < 0 || requests < 0 {
        return Err("root_decision_cost_invalid".into());
    }
    let trigger = NativeCoordinatorTrigger::classify(frame)?;
    let target = match (trigger, role) {
        (NativeCoordinatorTrigger::ChildOutputEnvelope, AgentRole::WebExecutor) => true,
        (NativeCoordinatorTrigger::ReviewerDecision, AgentRole::DeepInvestigator) => false,
        _ => return Err("root_trigger_role_conflict".into()),
    };
    let summary = decision.summary.as_json();
    let proposals = summary["suggestions"]
        .as_array()
        .ok_or("root_decision_schema_invalid")?;
    let offered = proposals
        .iter()
        .filter(|v| v.as_str() == Some(frame.step()))
        .count();
    if offered == 0 {
        return Err("root_decision_did_not_select_step".into());
    }
    // Same fixed local step = one overlap key. Merge before assigning a lane;
    // duplicate advisory text neither increases benefit nor spends twice.
    let merged = 1;
    let missing = frame.semantic["missingEvidence"]
        .as_array()
        .map_or(0, Vec::len);
    let required = context.execution_plan.frozen_json()["coverage"]["requiredFamilies"]
        .as_array()
        .map_or(0, Vec::len)
        .min(8) as i64;
    // These weights describe permitted coverage work, not a vulnerability
    // severity claim. The independent closed origin earns only provenance;
    // Mapper/model priority strings are not target evidence or information gain.
    let impact = if target { 24 } else { 16 };
    let evidence = 8;
    let information = if !target && missing > 0 { 8 } else { 0 };
    let coverage = if target { required * 4 } else { 4 };
    let blocker = 8;
    let expected_target = if target {
        context.execution_plan.contract_limit.clamp(0, 32)
    } else {
        0
    };
    let target_cost = native_coordinator_cost_weight(expected_target, limits[2]);
    let model_cost = (model_cost_rule.weight(tokens, limits[0])?
        + model_cost_rule.weight(requests, limits[1])?
        + 1)
        / 2;
    let risk = if target { 20 } else { 0 };
    let overlap = 0; // All identical fixed-step proposals have already merged.
    let uncertainty = if target { 8 } else { 12 };
    let score = (impact + evidence + information + coverage + blocker
        - target_cost
        - model_cost
        - risk
        - overlap
        - uncertainty)
        .clamp(-100, 100);
    if score <= 0 {
        return Err("root_decision_utility_nonpositive".into());
    }
    Ok(
        json!({"schemaVersion":1,"trigger":trigger.name(),"factHash":store::stable_hash(&frame.fact().to_string()),
        "step":frame.step(),"offeredCount":offered,"mergedProposalCount":merged,
        "ignoredAdvisoryCount":proposals.len()-offered,"score":score,"selection":"accepted",
        "reasonCode":"verified_fact_bounded_coverage_work",
        "features":{"impactCeilingWeight":impact,"evidenceStrength":evidence,
            "informationGain":information,"coverageGain":coverage,"blockerReleaseValue":blocker,
            "targetRequestCost":target_cost,"modelCost":model_cost,"sideEffectRisk":risk,
            "overlapPenalty":overlap,"uncertaintyPenalty":uncertainty},
        "costBasis":{"reservedModelTokens":tokens,"reservedModelRequests":requests,
            "expectedTargetRequests":expected_target,"originalLimits":limits},
        "advisoryOnly":true,"targetEvidenceProven":false}),
    )
}

// ---------------------------------------------------------------------------
// DESIGN §5.4：同质 Worker 主管决策帧（只新增，不改旧 kind 的 step 与打分逻辑）。
// ---------------------------------------------------------------------------

/// 新帧 kind。刻意不进 `NATIVE_WIRED_SUPERVISION_FRAME_KINDS`：那张清单有存量盘点断言。
pub(crate) const NATIVE_HOMOGENEOUS_FRAME_KIND: &str = "homogeneous-decision";
// 付费 tick 已接上 producer（`native_homogeneous_frame_for_root`）；step 字符串由
// `coordinator_frame.rs` 的映射表给出，这里保留规格用名的单一来源，只被盘点断言引用。
#[cfg_attr(not(test), allow(dead_code))]
/// 规格 §9 给出的新 step。
pub(crate) const NATIVE_HOMOGENEOUS_FRAME_STEP: &str = "assess:homogeneous_decision";

// producer 组装决策卡走 `gate::allowed_actions`（同一次事务内），这里只是付费前的
// 只读复核入口，DESIGN §5.4 的 action 归属检查在 `apply::run_coordinator_turn` 里也是同一函数。
#[cfg_attr(not(test), allow(dead_code))]
/// 同质任务当前可执行的动作子集（供 producer 组装决策卡、也供护栏复核）。
fn native_homogeneous_allowed_actions(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    target_key: &str,
) -> Result<Vec<String>, String> {
    use crate::agent_runtime::multi_agent::homogeneous::{gate, store};
    let task = store::load_task_by_target(connection, scan_id, attempt_number, target_key)
        .map_err(|error| error.reason_code.to_string())?;
    let Some(task) = task else {
        // 与旧路径一致：不可寻的任务绝不放行。
        return Err("root_homogeneous_task_missing".into());
    };
    gate::allowed_actions(connection, &task).map_err(|error| error.reason_code.to_string())
}

#[cfg_attr(not(test), allow(dead_code))]
/// 新 step 的策略护栏：`action ∈ allowed_actions`；JSON schema 由
/// `homogeneous::decision::parse_decision_envelope` 把住（DESIGN §9）。
/// 返回值仍然带 `advisoryOnly:true`：副作用只发生在 `apply_decision` 里。
fn native_homogeneous_action_gate(
    connection: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    action: &str,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::multi_agent::homogeneous::{reason, types};
    if !types::is_member(action, &types::ACTIONS) {
        return Err(reason::UNKNOWN_ACTION.to_string());
    }
    let allowed = native_homogeneous_allowed_actions(
        connection,
        &actor.scan_id,
        actor.attempt_number,
        &actor.target_key,
    )?;
    let selection = if allowed.iter().any(|candidate| candidate == action) {
        "accepted"
    } else {
        "refused"
    };
    Ok(json!({
        "schemaVersion": 1,
        "step": NATIVE_HOMOGENEOUS_FRAME_STEP,
        "action": action,
        "allowedActions": allowed,
        "selection": selection,
        "reasonCode": if selection == "accepted" { reason::GOAL_REACHED } else { reason::UNKNOWN_ACTION },
        "advisoryOnly": true,
    }))
}
