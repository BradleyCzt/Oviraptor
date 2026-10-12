fn execute_source_tool_assignment(
    connection: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    slice: &JsonValue,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::{
        multi_agent::source,
        secrets::redact_json,
        store::stable_hash,
    };
    context.require_supervision(lease)?;
    let mut profile = agent_model_profile(context.environment, context.proxy)?;
    let capabilities = source::tool_capabilities(child.role)?;
    let tools = agent_tool_specs()
        .into_iter()
        .filter(|s| capabilities.iter().any(|n| n == s.name))
        .collect::<Vec<_>>();
    let input = crate::agent_runtime::multi_agent::directive::source_guidance::attach(
        connection,
        lease,
        child.role,
        true,
        json!({"sourceTask":slice,"maxModelRounds":3}),
    )?;
    let messages = source_assessment_messages(SOURCE_TOOLS_SYSTEM, &input);
    let (_, output) = source_assessment_budget(&messages, &profile)?;
    profile.max_output_tokens = Some(output);
    let frozen = redact_json(
        &json!({"schemaVersion":1,"messages":messages,"tools":tools.iter().map(|s|s.as_function_spec()).collect::<Vec<_>>(),
        "temperature":0.0,"model":profile.model,"endpointHash":stable_hash(&profile.endpoint),"proxyHash":stable_hash(profile.proxy.as_deref().unwrap_or_default()),
        "local":profile.local,"maxOutputTokens":profile.max_output_tokens,"maxContextTokens":profile.max_context_tokens}),
    );
    let client = AgentModelClient::new(profile.clone(), &[]);
    native_run_source_controlled_adapter(NativeSourceControlledAdapter {
        connection,
        context,
        lease,
        child,
        profile,
        tools,
        client,
        frozen,
        registered: capabilities,
        next_tool: 0,
        original_error: None,
    })
}

#[cfg(test)]
fn run_source_tool_phases(
    connection: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Vec<JsonValue>, String> {
    run_source_tool_phases_from(connection, context, lease, 0)
}

fn run_source_tool_phases_from(
    connection: &rusqlite::Connection,
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    start_index: usize,
) -> Result<Vec<JsonValue>, String> {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{scheduler, source},
    };
    if start_index > 1 {
        return Err("source_tool_phase_invalid_start_index".into());
    }
    let mut results = Vec::new();
    for (index, role) in [AgentRole::RepoMapper, AgentRole::SourceAnalyst]
        .into_iter()
        .enumerate()
        .skip(start_index)
    {
        context.require_supervision(lease)?;
        crate::agent_runtime::multi_agent::directive::source_guidance::freeze(
            connection, lease, role, true,
        )?;
        let revision:i64=connection.query_row("SELECT COALESCE(MAX(revision),1) FROM agent_evidence_revisions WHERE root_run_id=?1",[&lease.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        let slice = source::tool_task_slice(connection, lease, role, revision)?;
        let remaining:i64=connection.query_row("SELECT total_tokens-spent_tokens-reserved_tokens FROM agent_budget_ledger WHERE root_run_id=?1",[&lease.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        // Keep a share of the frozen token budget for the independent v2
        // Reviewer. This does not increase the root's authorized token limit.
        let review_share = i64::from(crate::agent_runtime::multi_agent::source_reviewer::enabled(
            connection, lease,
        )?) + i64::from(
            crate::agent_runtime::multi_agent::source_coverage_reviewer::enabled(
                connection, lease,
            )?,
        );
        let tokens = remaining / (2 + review_share - index as i64);
        if tokens < 1 {
            return Err("source_tool_phase_no_budget".into());
        }
        let child = scheduler::schedule_child(
            connection,
            lease,
            role,
            AgentLane::ReadOnlyAnalysis,
            "source_tools_ready",
            &slice,
            revision,
            &source::tool_capabilities(role)?,
            tokens,
            3,
        )?;
        scheduler::mark_child_running(connection, lease, &child)?;
        let result = execute_source_tool_assignment(connection, context, lease, &child, &slice)
            .map_err(|e| {
                // This exact return follows the atomic known-failure closure.
                // Other failures retain the existing unknown-usage cleanup.
                if e == "source_tool_phase_round_budget_exhausted_without_finish" {
                    e
                } else {
                    failed_specialist_error(connection, lease, &child, &e)
                }
            })?;
        results.push(json!({"role":role.as_str(),"assignmentId":child.assignment_id,"runId":child.run_id,"result":result}));
    }
    Ok(results)
}

include!("native_source_exhausted.rs");
