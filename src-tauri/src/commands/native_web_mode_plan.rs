// Read-only route classification; this does not grant recovery authority.
fn ordinary_web_recovery_scope_on(db: &rusqlite::Connection, scan: &str) -> Result<bool, String> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.native_protection_recovery_drafts WHERE scan_id=?1)
        OR EXISTS(SELECT 1 FROM main.native_protection_recovery_cancellations WHERE scan_id=?1)
        OR EXISTS(SELECT 1 FROM main.native_protection_recovery_scope_budgets WHERE scan_id=?1)
        OR EXISTS(SELECT 1 FROM main.native_web_mode_actions WHERE scan_id=?1 AND json_extract(action_json,'$.kind')='protection_recovery')",
        [scan], |r| r.get(0),
    ).map_err(|e| e.to_string())
}

include!("native_web_root_budget.rs");

// Ordinary Web entry only. Source's old plan publisher never calls this helper.
#[cfg(test)]
fn persist_frozen_web_execution_plan(
    db_path: &Path,
    scan: &str,
    attempt: i64,
    target: &str,
    plan: &AgentExecutionPlan,
) -> Result<(), String> {
    persist_frozen_web_execution_plan_with_tool_configuration(db_path,scan,attempt,target,plan,None)
}

fn persist_frozen_web_execution_plan_with_tool_configuration(
    db_path: &Path, scan: &str, attempt: i64, target: &str,
    plan: &AgentExecutionPlan,
    configuration: Option<&crate::tool_supply::ToolAdministratorConfiguration>,
) -> Result<(), String> {
    use crate::agent_runtime::{store, web_mode::root};
    let db = db::open(db_path)?;
    // Recovery scope needs its own signed budget/model/identity consumer. Never
    // let this ordinary publisher mint schema2 default ceilings for it, even
    // if a future startup has already produced a valid private Mode receipt.
    // The retained action and cancellation also deny a lost draft relation.
    if ordinary_web_recovery_scope_on(&db, scan)? {
        return Err("protection_recovery_requires_dedicated_root_budget".into());
    }
    let proof = private_web_mode_on(&db, scan, attempt)?;
    if !proof.fact().targets.iter().any(|v| v == target) {
        return Err("web_mode_target_not_frozen".into());
    }
    let existing: Vec<String> = {
        let mut statement=db.prepare("SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND role='coordinator' ORDER BY id")
            .map_err(|e|e.to_string())?;
        let ids = statement
            .query_map(params![scan, attempt, target], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<_>>()
            .map_err(|e| e.to_string())?;
        ids
    };
    if existing.is_empty() {
        #[cfg(test)]
        let prepared_plan = AgentExecutionPlan::from_json(
            &agent_tests::native_frozen_tool_supply_plan_for_test(
                db_path, scan, attempt, target, plan.as_json(),
            )?,
        ).ok_or("tool_supply_test_original_plan_invalid")?;
        #[cfg(test)]
        let plan = &prepared_plan;
        let declaration =
            root::NewRootModeDeclaration::from_verified(&proof, target, &plan.hash())?.with_local_deliberation()?.with_new_evidence_supervision()?.with_lease_expiry_supervision()?.with_protection_supervision()?;
        #[cfg(test)]
        let declaration = agent_tests::native_mapper_physical_new_birth_for_test(
            &db, db_path, scan, attempt, target, plan, declaration,
        )?;
        let budget = fresh_web_root_budget_declaration(&proof, target, plan)?;
        let native_plan = plan.as_json();
        return store::record_attempt_plan_with_declarations_and_tool_configuration(
            &db,
            scan,
            attempt,
            target,
            plan.backend,
            &plan.hash(),
            &native_plan,
            Some(&budget),
            Some(&declaration),
            configuration,
        );
    }
    if existing.len() != 1 {
        return Err("web_mode_existing_root_ambiguous".into());
    }
    let declaration =
        root::read(&db, &existing[0])?.ok_or("web_mode_existing_root_missing_declaration")?;
    if declaration.fact() != proof.fact() {
        return Err("web_mode_existing_root_original_fact_changed".into());
    }
    // Reentry compares its original sidecar first; it never supplies Some(mode)
    // or silently fills missing mode/budget authority on an existing Root.
    persist_agent_execution_plan(db_path, scan, attempt, target, plan)
}

fn native_frozen_web_root_mode(
    context: &AgentRunContext,
) -> Result<crate::agent_runtime::web_mode::WebMode, String> {
    let db = db::open(&context.db_path)?;
    native_frozen_web_root_mode_on(&db, context)
}

fn native_frozen_web_root_mode_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
) -> Result<crate::agent_runtime::web_mode::WebMode, String> {
    let run = context.run.as_ref().ok_or("web_mode_root_missing")?;
    let declaration = crate::agent_runtime::web_mode::root::read(db, &run.run_id)?
        .ok_or("web_mode_root_declaration_missing")?;
    let fact = private_web_mode_on(db, &context.scan_id, context.attempt_number)?;
    let text:String=db.query_row("SELECT plan_json FROM agent_runs WHERE id=?1 AND scan_id=?2 AND attempt_number=?3
        AND target_url=?4 AND role='coordinator' AND backend='native' AND parent_run_id IS NULL AND assignment_id=''",
        params![run.run_id,context.scan_id,context.attempt_number,context.target_url],|r|r.get(0)).map_err(|_|"web_mode_root_scope_changed")?;
    let plan: JsonValue = serde_json::from_str(&text).map_err(|_| "web_mode_root_plan_invalid")?;
    if declaration.fact() != fact.fact() || plan != context.execution_plan.as_json() {
        return Err("web_mode_original_root_fact_changed".into());
    }
    Ok(declaration.mode())
}

// This narrow test/read gate is not mode selection. A Multi worker keeps the
// existing inbox flow; a Single run never acquires a Coordinator for chatting.
fn native_single_directives_disabled(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
) -> Result<bool, String> {
    let Some(run) = context.run.as_ref() else {
        return Ok(false);
    };
    let (policy, role): (String, String) = db
        .query_row(
            "SELECT orchestration_policy,role FROM agent_runs WHERE id=?1
        AND scan_id=?2 AND attempt_number=?3 AND target_url=?4",
            params![
                run.run_id,
                context.scan_id,
                context.attempt_number,
                context.target_url
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "web_mode_directive_scope_missing")?;
    if policy == "single" && role == "coordinator" {
        // Do not mint a collaboration fence for legacy Single diagnostic runs.
        return Ok(true);
    }
    if policy != "multi" {
        return Err("web_mode_directive_policy_invalid".into());
    }
    Ok(false)
}

// Pure original owner load; no current C or zero-use history issues authority.
fn native_web_original_finance_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    let root = &context
        .run
        .as_ref()
        .ok_or("web_finance_root_missing")?
        .run_id;
    let present: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !present {
        return Err("web_root_original_finance_missing".into());
    }
    RootOwner::load_original(db, root)?.require_executable(db)
}
