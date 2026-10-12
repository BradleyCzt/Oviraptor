// A new attempt is not a financial continuation contract. This read-only gate
// runs before startup mutates the scan or allocates files, inside its snapshot.
// Even settled paid history cannot mint another fresh allowance here: a future
// explicit remaining-budget migration must bind the original owners first.
fn require_web_start_financial_history_in(
    db: &rusqlite::Connection,
    scan: &str,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget::{self, root::RootOwner};
    if db.is_autocommit() {
        return Err("web_start_finance_requires_transaction".into());
    }
    // Include durable owners as well as live rows. Deleting/moving an old Root
    // must not conceal its financial history from a new attempt of this scan.
    let roots: Vec<String> = db
        .prepare(
            "SELECT id FROM agent_runs WHERE scan_id=?1 AND backend='native'
               AND role='coordinator' AND parent_run_id IS NULL AND assignment_id=''
             UNION SELECT root_run_id FROM agent_root_budget_attempts
               WHERE json_extract(contract_json,'$.root.scan')=?1 ORDER BY 1",
        )
        .map_err(|e| e.to_string())?
        .query_map([scan], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())?;
    let mut paid_history = false;
    for root in roots {
        let scope_valid: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND scan_id=?2
             AND backend='native' AND role='coordinator' AND parent_run_id IS NULL
             AND assignment_id='' AND attempt_number>0)",
                params![root, scan],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !scope_valid {
            return Err("web_start_original_finance_scope_missing".into());
        }
        // A dispatch/reservation is already an obligation, including uncertain
        // transport, child costs and old coarse counters without an owner.
        let progressed: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1)
             OR EXISTS(SELECT 1 FROM agent_root_model_journal WHERE root_run_id=?1)
             OR EXISTS(SELECT 1 FROM agent_web_model_journal WHERE root_run_id=?1)
             OR EXISTS(SELECT 1 FROM agent_specialist_calls WHERE root_run_id=?1)
             OR EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE root_run_id=?1)
             OR EXISTS(SELECT 1 FROM agent_model_cost_facts WHERE root_run_id=?1)
             OR EXISTS(SELECT 1 FROM agent_budget_ledger WHERE root_run_id=?1
               AND (spent_tokens<>0 OR spent_requests<>0 OR reserved_tokens<>0 OR reserved_requests<>0))
             OR EXISTS(SELECT 1 FROM agent_runs WHERE (id=?1 OR root_run_id=?1)
               AND (used_tokens<>0 OR used_cached_tokens<>0 OR used_requests<>0
                    OR reserved_tokens<>0 OR reserved_requests<>0))
             OR EXISTS(SELECT 1 FROM agent_http_request_claims c JOIN agent_runs r ON r.id=c.run_id
               WHERE r.id=?1 OR r.root_run_id=?1)
             OR EXISTS(SELECT 1 FROM tool_invocations i JOIN agent_runs r ON r.id=i.run_id
               WHERE r.id=?1 OR r.root_run_id=?1)
             OR EXISTS(SELECT 1 FROM agent_events e JOIN agent_runs r ON r.id=e.run_id
               WHERE (r.id=?1 OR r.root_run_id=?1) AND e.event_type='model_round_completed')",
            [&root], |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        if !progressed {
            continue;
        }
        RootOwner::load_original(db, &root)?;
        budget::admission::require_determinate(db, &root)?;
        budget::root::remaining_child_capacity(db, &root)?;
        // Existing paid publications have an authoritative physical proof
        // reader. Unpublished invoices confer no permission through this gate;
        // the final rejection remains unconditional even if no publication exists.
        let events: Vec<i64> = db
            .prepare(
                "SELECT json_extract(fact_json,'$.eventSequence') FROM agent_root_tick_receipts
             WHERE root_run_id=?1 AND phase='publication' ORDER BY round",
            )
            .map_err(|e| e.to_string())?
            .query_map([&root], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<_>>()
            .map_err(|e| e.to_string())?;
        for event in events {
            budget::root::model::tick::Tick::project_publication(db, &root, event)?;
        }
        paid_history = true;
    }
    if paid_history {
        return Err("web_start_paid_history_requires_explicit_financial_contract".into());
    }
    Ok(())
}
