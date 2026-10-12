// Strict Root/C/parent binding, stable Rust-derived bootstrap basis and schema.
struct NativeCoordinatorTickReceipt {
    summary:
        crate::agent_runtime::multi_agent::budget::root::model::tick::decision::DecisionSummary,
    event_sequence: i64,
    replayed: bool,
    tick: crate::agent_runtime::multi_agent::budget::root::model::tick::Tick,
    saved: crate::agent_runtime::multi_agent::budget::root::model::tick::SavedDecision,
}
struct NativeCoordinatorRequest {
    _hook: Option<crate::llm_hook::LlmHookHandle>,
    client: AgentModelClient,
    request: crate::agent_runtime::model::gateway::ModelRequest,
    basis: JsonValue,
    fact: JsonValue,
    estimate: i64,
}
fn native_coordinator_messages_for_basis(
    basis: &JsonValue,
) -> (Vec<JsonValue>, Vec<AgentToolSpec>) {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::decision::DecisionSummary;
    let local = basis.get("localDeliberation").is_some();
    let specs = if local {
        let mut tools =
            crate::agent_runtime::multi_agent::budget::root::model::tick::local::schemas();
        if basis.get("liveBudgetObservation").is_some() {
            tools[2].description="Read the captured pre-dispatch ledger snapshot: Root plus child consumption, active reservations, indeterminate cost, gross model headroom and original deadline. This observation is frozen for this paid call, conveys no grant and cannot authorize execution; Rust checks current balances at admission.";
        }
        tools
    } else {
        Vec::new()
    };
    let system = if basis.get("financialContinuation").is_some() {
        crate::agent_runtime::multi_agent::budget::root::model::tick::financial_purpose::SYSTEM
    } else if local {
        "You are the Root Coordinator. Use only the registered bounded local tools to inspect frozen facts and propose the offered step or defer, then return a strict DecisionSummary JSON. At most 3 model rounds and 4 local calls. No target actions, commands, arbitrary paths, credentials, reasoning or assistant drafts. Suggestions are advisory; Rust decides dispatch and authorization. Hard ceilings are not remaining balance or grants. Every list <=16; every string <=512 characters; schemaVersion=1."
    } else {
        "You are the Root Coordinator. Inspect only frozen facts. Return one strict DecisionSummary JSON. No reasoning, assistant drafts, tools, commands or credentials. Suggestions are advisory: Rust decides dispatch and authorization. Every list <=16; every string <=512 characters; schemaVersion=1."
    };
    let messages = vec![
        json!({"role":"system","content":
        system}),
        json!({"role":"user","content":json!({"basis":basis,"requiredOutput":DecisionSummary::schema()}).to_string()}),
    ];
    (messages, specs)
}
fn native_coordinator_request(
    context: &AgentRunContext,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<NativeCoordinatorRequest, String> {
    use crate::agent_runtime::{
        multi_agent::budget::root::model::tick::decision::DecisionSummary, store,
    };
    let database = db::open(&context.db_path)?;
    let basis = {
        let snapshot = database
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        native_coordinator_tick_authority(&snapshot, context, lease)?;
        native_coordinator_basis_on(&snapshot, context, lease)?
    };
    let (messages, specs) = native_coordinator_messages_for_basis(&basis);
    let tools: Vec<JsonValue> = specs.iter().map(AgentToolSpec::as_function_spec).collect();
    let original = agent_model_profile(&context.environment, context.proxy.as_deref())?;
    let mut root_context = context.clone();
    root_context.environment.prompt_audit_mode = "off".into();
    let (hook, client) = native_model_setup(&root_context, &specs)?;
    let output = client.profile().max_output_tokens.unwrap_or(1024).min(2048);
    if output == 0 {
        return Err("root_tick_output_invalid".into());
    }
    let mut profile = client.profile().clone();
    profile.max_output_tokens = Some(output);
    let client = AgentModelClient::new(profile, &specs);
    let request = agent_model_request(messages, &specs);
    let bytes = serde_json::to_vec(&json!({"messages":request.messages,"tools":tools}))
        .map_err(|e| e.to_string())?
        .len();
    let estimate = i64::try_from(bytes)
        .ok()
        .and_then(|bytes| bytes.checked_add(i64::try_from(output).ok()?))
        .filter(|n| *n > 0)
        .ok_or("root_tick_estimate_invalid")?;
    // Local hook transport ports can change on a legitimate restart. Bind the
    // configured original provider/model/credential/proxy and exact payload,
    // not the hook's ephemeral loopback port. No raw credential is persisted.
    let mut fact = json!({"version":1,"purpose":"coordinator_decision_v1","root":lease.root_run_id,
        "basisHash":store::stable_hash(&basis.to_string()),"messages":request.messages,"tools":tools,
        "schemaHash":store::stable_hash(&DecisionSummary::schema().to_string()),"model":original.model,
        "endpointBinding":store::stable_hash(&original.endpoint),"deployment":if original.local {"local"}else {"cloud"},
        "credentialBinding":store::stable_hash(original.bearer().unwrap_or("")),
        "proxyBinding":store::stable_hash(original.proxy.as_deref().unwrap_or("")),
        "maxContextTokens":original.max_context_tokens,"maxOutputTokens":output,
        "temperature":request.temperature,"timeoutPolicy":"original_root_clock",
        "providerOptions":client.profile().bounded_wire_options()});
    if let Some(binding) = basis.get("financialContinuation") {
        fact["purpose"] = crate::agent_runtime::multi_agent::budget::root::model::tick::financial_purpose::PURPOSE.into();
        fact["financialContinuation"] = binding.clone();
        native_coordinator_require_original_model_on(
            &database,
            binding["sourceRootRunId"]
                .as_str()
                .ok_or("financial_continuation_birth_invalid")?,
            &fact,
        )?;
    }
    Ok(NativeCoordinatorRequest {
        _hook: hook,
        client,
        request,
        basis,
        fact,
        estimate,
    })
}
fn native_coordinator_tick_authority(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(), String> {
    if db.is_autocommit() {
        let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return native_coordinator_tick_authority(&snapshot, context, actor);
    }
    use crate::agent_runtime::multi_agent::lease;
    let run = context.run.as_ref().ok_or("root_tick_run_missing")?;
    if context.target_url.starts_with("source:")
        || run.run_id != actor.root_run_id
        || run.db_path != context.db_path
        || context.scan_id != actor.scan_id
        || context.attempt_number != actor.attempt_number
        || context.target_url != actor.target_key
    {
        return Err("root_tick_context_conflict".into());
    }
    if crate::agent_runtime::multi_agent::budget::root::model::tick::financial_purpose::binding_on(
        db,
        &run.run_id,
    )?
    .is_some()
    {
        let birth = verify_financial_continuation_birth_on(db, &run.run_id)?;
        native_financial_continuation_require_authority_in(db, &birth)?;
    }
    agent_require_frozen_web_plan(db, context).map_err(str::to_string)?;
    native_coordinator_frozen_evidence(db, context)?;
    if native_frozen_web_root_mode_on(db, context)?
        != crate::agent_runtime::web_mode::WebMode::Multi
    {
        return Err("root_tick_frozen_mode_denied".into());
    }
    lease::validate_coordinator_lease(db, actor)?;
    lease::require_executable_coordinator(db, actor)?;
    let original: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1
        AND root_run_id=id AND parent_run_id IS NULL AND assignment_id='' AND role='coordinator'
        AND backend='native' AND orchestration_policy='multi' AND status='running'
        AND scan_id=?2 AND attempt_number=?3 AND target_url=?4 AND cancel_requested_at='')",
            params![
                actor.root_run_id,
                actor.scan_id,
                actor.attempt_number,
                actor.target_key
            ],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !original {
        return Err("root_tick_original_scope_conflict".into());
    }
    context
        .supervision
        .as_ref()
        .ok_or("worker_supervisor_stopped")?
        .check_actor(&context.db_path, actor)
}
fn native_coordinator_initialize_control(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget::root::model::RootModelCall;
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, context, actor)?;
    RootModelCall::initialize_coordinator_control(&tx, actor)?;
    native_coordinator_tick_authority(&tx, context, actor)?;
    tx.commit().map_err(|e| e.to_string())
}
fn native_coordinator_bootstrap(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<NativeCoordinatorTickReceipt, String> {
    native_coordinator_initialize_control(context, actor)?;
    native_coordinator_tick(context, actor)
}

#[cfg(test)]
thread_local! {
    static ROOT_TICK_AFTER_PAID_TEST_BOUNDARY: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn native_coordinator_tick_paid_checkpoint() {
    let boundary = ROOT_TICK_AFTER_PAID_TEST_BOUNDARY.with(|slot|slot.borrow_mut().take());
    if let Some(boundary) = boundary { boundary(); }
    if let Some(path) = std::env::var_os("OVIRAPTOR_ROOT_TICK_PAID_READY_FILE") {
        std::fs::write(path, "root-tick-paid-ready").unwrap();
        use std::io::Read;
        let mut byte = [0];
        std::io::stdin().read_exact(&mut byte).unwrap();
        panic!("test parent must SIGKILL after paid receipt and before publication");
    }
}
