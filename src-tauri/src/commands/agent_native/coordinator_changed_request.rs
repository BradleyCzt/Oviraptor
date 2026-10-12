// Bootstrap request bytes remain exactly g2. Only a new captured output adds a
// new phase and permitted suggestion to its own independently paid round.
fn native_coordinator_basis_for_frame_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&NativeCoordinatorFrame>,
) -> Result<JsonValue, String> {
    let mut basis = native_coordinator_basis_on(db, context, actor)?;
    if frame.is_some() && basis.get("financialContinuation").is_some() {
        return Err("financial_continuation_changed_frame_denied".into());
    }
    if let Some(frame) = frame {
        // A raw observation reader is not a paid supervision input. Keeping the
        // same kind must never bypass the original closed-author proof.
        if frame.kind == "new-evidence" && frame.semantic.get("closedAuthor").is_none() {
            return Err("root_new_evidence_author_not_closed".into());
        }
        if frame.kind == "new-evidence" && basis.get("newEvidenceSupervision").is_none() {
            return Err("root_new_evidence_original_protocol_missing".into());
        }
        if frame.kind == "lease-expired" && basis.get("leaseExpirySupervision").is_none() {
            return Err("root_lease_expired_original_protocol_missing".into());
        }
        if frame.kind == "protection-changed" {
            if frame.semantic.get("closedAuthor").is_none() {
                return Err("root_protection_author_not_closed".into());
            }
            let mode = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?
                .ok_or("root_protection_original_protocol_missing")?;
            basis["protectionSupervision"] = mode.protection_supervision()
                .ok_or("root_protection_original_protocol_missing")?;
            native_coordinator_require_original_protection_fact(db, actor, &frame.fact())?;
        }
        frame.verify(db, context, actor)?;
        if frame.kind == NATIVE_HOMOGENEOUS_FRAME_KIND {
            // P1-1：同质主管轮沿用「旧 basis + changed fact」的既有形状，只追加决策卡。
            // 整块替换会让本地审议根轮的 localParents 证明消失（tick 可执行性检查），
            // 把付费轮误判成取消。候选漂移在这里同一函数里复算，注入 prompt 前拒掉。
            let snapshot = native_homogeneous_require_current_frame(db, actor, frame)?;
            basis["homogeneousTask"] = snapshot["homogeneousTask"].clone();
        }
        if frame.kind == "lease-expired" {
            native_coordinator_require_original_lease_fact(db, actor, &frame.fact())?;
        }
        basis["phase"] = frame.kind.into();
        basis["changedFact"] = frame.fact();
        basis["permittedSuggestion"] = frame.step().into();
    }
    if basis.to_string().len() > 256 * 1024 {
        return Err("root_tick_basis_too_large".into());
    }
    Ok(basis)
}

// A physical withdrawal has one immutable paid fact. Mutable metadata may
// invalidate that fact, but cannot turn its receipt into another paid round.
fn native_coordinator_require_original_lease_fact(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    current: &JsonValue,
) -> Result<(), String> {
    let mut query = db.prepare("SELECT fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request'")
        .map_err(|e| e.to_string())?;
    let rows = query.query_map([&actor.root_run_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    for row in rows {
        let text = row.map_err(|e| e.to_string())?;
        let fact: JsonValue = serde_json::from_str(&text)
            .map_err(|_| "root_lease_closed_original_request_invalid")?;
        let canonical_fact = fact.to_string();
        if canonical_fact != text {
            return Err("root_lease_closed_original_request_invalid".into());
        }
        let basis = &fact["request"]["basis"];
        let original = &basis["changedFact"];
        if basis["phase"] == "lease-expired"
            && original["semantic"]["receiptId"] == current["semantic"]["receiptId"]
            && original["semantic"]["workerAttemptId"] == current["semantic"]["workerAttemptId"]
            && original != current
        {
            return Err("root_lease_closed_original_fact_changed".into());
        }
    }
    Ok(())
}
fn native_coordinator_verify_frame_basis(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&NativeCoordinatorFrame>,
    captured: &JsonValue,
) -> Result<(), String> {
    if frame.is_none() {
        return native_coordinator_verify_original_basis(db, context, actor, captured);
    }
    if native_coordinator_basis_for_frame_on(db, context, actor, frame)? != *captured {
        return Err("root_tick_original_basis_changed".into());
    }
    Ok(())
}

pub(crate) fn native_coordinator_require_original_model_on(
    db: &rusqlite::Connection,
    root: &str,
    request: &JsonValue,
) -> Result<(), String> {
    let text: String = db
        .query_row(
            "SELECT fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1
        AND round=1 AND phase='request'",
            [root],
            |r| r.get(0),
        )
        .map_err(|_| "root_tick_original_model_missing")?;
    let fact: JsonValue =
        serde_json::from_str(&text).map_err(|_| "root_tick_original_model_invalid")?;
    let canonical_fact = fact.to_string();
    if canonical_fact != text {
        return Err("root_tick_original_model_invalid".into());
    }
    let original = &fact["request"]["request"];
    // Legacy providers had no extension. A newly selected extension cannot
    // silently change an original paid model contract on continuation.
    let empty_options = json!({});
    if original.get("providerOptions").unwrap_or(&empty_options)
        != request.get("providerOptions").unwrap_or(&empty_options)
    {
        return Err("root_tick_original_model_changed".into());
    }
    for key in [
        "model",
        "endpointBinding",
        "deployment",
        "credentialBinding",
        "proxyBinding",
        "maxContextTokens",
        "maxOutputTokens",
        "temperature",
        "timeoutPolicy",
        "schemaHash",
    ] {
        if original.get(key).is_none() || original[key] != request[key] {
            return Err("root_tick_original_model_changed".into());
        }
    }
    Ok(())
}
fn native_coordinator_request_for_frame(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&NativeCoordinatorFrame>,
) -> Result<NativeCoordinatorRequest, String> {
    if let Some(frame) = frame {
        NativeCoordinatorTrigger::classify(frame)?;
    }
    let mut prepared = native_coordinator_request(context, actor)?;
    let Some(frame) = frame else {
        return Ok(prepared);
    };
    let db = db::open(&context.db_path)?;
    prepared.basis = native_coordinator_basis_for_frame_on(&db, context, actor, Some(frame))?;
    if frame.kind == "authorization-output" {
        // Retain the born local contract/history for original proof checking,
        // but this dedicated assessment has no local-tool continuation.
        prepared.request.tools.clear();
        prepared.fact["tools"] = json!([]);
        prepared.request.messages[0] = json!({"role":"system","content":"You are the Root Coordinator. Assess only the original frozen Authorization output. Return one strict DecisionSummary JSON with the offered assessment suggestion. No tools, target actions, commands, credentials, reasoning or assistant drafts. Suggestions are advisory and grant no scope or dispatch. Every list <=16; every string <=512 characters; schemaVersion=1."});
    }
    if frame.kind == "external-surface-output" {
        prepared.request.tools.clear();
        prepared.fact["tools"] = json!([]);
        prepared.request.messages[0] = json!({"role":"system","content":"You are the Root Coordinator. Assess only the original frozen anonymous ExternalSurface output. Return one strict DecisionSummary JSON with the offered assessment suggestion. No tools, target actions, commands, credentials, reasoning or assistant drafts. Suggestions are advisory and grant no scope or dispatch. Every list <=16; every string <=512 characters; schemaVersion=1."});
    }
    if frame.kind == "business-logic-output" {
        prepared.request.tools.clear();
        prepared.fact["tools"] = json!([]);
        prepared.request.messages[0] = json!({"role":"system","content":"You are the Root Coordinator. Assess only the original frozen readonly BusinessLogic output. Return one strict DecisionSummary JSON with the offered assessment suggestion. No tools, target actions, commands, credentials, reasoning or assistant drafts. Suggestions are advisory and grant no scope or dispatch. Every list <=16; every string <=512 characters; schemaVersion=1."});
    }
    if frame.kind == "business-logic-input" {
        prepared.request.tools.clear();
        prepared.fact["tools"] = json!([]);
        prepared.request.messages[0] = json!({"role":"system","content":"You are the Root Coordinator. Decide only whether the original frozen eligible BusinessLogic input warrants one readonly candidate assessment. Return one strict DecisionSummary JSON. Select only the offered dispatch:business_logic suggestion or defer with an empty suggestions list. No tools, target actions, commands, credentials, reasoning or assistant drafts. Rust alone authorizes the fixed readonly child; no target action or additional scope is granted. Every list <=16; every string <=512 characters; schemaVersion=1."});
    }
    prepared.request.messages[1] = json!({"role":"user","content":json!({"basis":prepared.basis,
        "requiredOutput":crate::agent_runtime::multi_agent::budget::root::model::tick::decision::DecisionSummary::schema(),
        "permittedSuggestion":frame.step(),"instruction":"Use the permitted suggestion exactly only if useful. Empty suggestions means defer. This suggestion grants no new scope, capability, budget or target request; Rust validates the actual dispatch."}).to_string()});
    if frame.kind == NATIVE_HOMOGENEOUS_FRAME_KIND {
        // P1-1：同质主管轮次注入自己的系统提示（SPEC §7.1 原文）与决策卡；
        // basis 已由 `native_coordinator_basis_for_frame_on` 追加 homogeneousTask 段，
        // 这里不再改写它，保证「复算 == 已捕获」的原始事实绑定仍然成立。
        // 只给候选 id，不授予任何执行权，也不放宽封闭集合。
        prepared.request.tools.clear();
        prepared.fact["tools"] = json!([]);
        prepared.request.messages[0] = json!({"role":"system","content":crate::agent_runtime::multi_agent::homogeneous::prompt::system_prompt()});
        prepared.request.messages[1] = json!({"role":"user","content":json!({
            "basis": prepared.basis,
            "instruction": "Output exactly one CoordinatorDecision JSON object as defined in the system contract. No markdown fence, no extra keys, no second action. Choose only ids listed in basis.homogeneousTask.candidates and an action listed in basis.homogeneousTask.allowedActions."
        }).to_string()});
    }
    prepared.fact["purpose"] = "coordinator_changed_fact_v1".into();
    prepared.fact["basisHash"] =
        crate::agent_runtime::store::stable_hash(&prepared.basis.to_string()).into();
    prepared.fact["messages"] = json!(prepared.request.messages);
    let bytes = serde_json::to_vec(
        &json!({"messages":prepared.request.messages,"tools":prepared.fact["tools"]}),
    )
    .map_err(|e| e.to_string())?
    .len();
    let output = prepared
        .client
        .profile()
        .max_output_tokens
        .ok_or("root_tick_output_invalid")?;
    prepared.estimate = i64::try_from(bytes)
        .ok()
        .and_then(|n| n.checked_add(i64::try_from(output).ok()?))
        .filter(|n| *n > 0)
        .ok_or("root_tick_estimate_invalid")?;
    Ok(prepared)
}

fn native_coordinator_tick_for_frame(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: &NativeCoordinatorFrame,
) -> Result<NativeCoordinatorTickReceipt, String> {
    native_coordinator_tick_with_frame(context, actor, Some(frame))
}

// One signed original response identity cannot become a second paid basis
// when mutable metadata drifts. Invalid provenance stops before Root charging.
fn native_coordinator_require_original_protection_fact(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    current: &JsonValue,
) -> Result<(), String> {
    let mut q = db.prepare("SELECT fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request'")
        .map_err(|e|e.to_string())?;
    let rows = q.query_map([&actor.root_run_id], |r|r.get::<_,String>(0))
        .map_err(|e|e.to_string())?;
    for row in rows {
        let raw = row.map_err(|e|e.to_string())?;
        let fact: JsonValue = serde_json::from_str(&raw).map_err(|_|"root_protection_original_request_invalid")?;
        let canonical = fact.to_string();
        if canonical != raw { return Err("root_protection_original_request_invalid".into()); }
        let basis = &fact["request"]["basis"];
        let original = &basis["changedFact"];
        if basis["phase"] == "protection-changed"
            && original["semantic"]["receiptId"] == current["semantic"]["receiptId"]
            && original != current {
            return Err("root_protection_original_fact_changed".into());
        }
    }
    Ok(())
}
