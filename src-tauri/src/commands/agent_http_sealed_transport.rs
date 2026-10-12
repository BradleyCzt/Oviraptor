// Private actual transport lifetime. The boxed SQLite writer never moves between
// pre-send capture and the actual headers receipt. Saved JSON is not an issuer.
struct AgentHttpSealedTransport {
    writer: Option<Box<rusqlite::Connection>>,
    claim: Option<AgentHttpClaim>,
    witness: Option<http_budget_receipt::FreshHttpBudgetClaim>,
    actor: Option<crate::agent_runtime::multi_agent::lease::CoordinatorLease>,
    received: bool,
}

impl AgentHttpSealedTransport {
    fn claim(&self) -> Option<&AgentHttpClaim> {
        self.claim.as_ref()
    }

    // Invoke only after reqwest returned actual response headers. Neither a
    // recorded artifact nor an IPC value can recreate this transport lifetime.
    fn receive_headers(
        &mut self,
        context: &AgentRunContext,
        status: u16,
    ) -> Result<Option<String>, String> {
        let Some(claim) = self.claim.as_ref() else {
            return Ok(None);
        };
        if self.received || !(100..=599).contains(&status) {
            return Err("http_journal_receipt_verification_failed".into());
        }
        if context
            .run
            .as_ref()
            .is_none_or(|run| run.run_id != claim.run_id || run.db_path != context.db_path)
        {
            return Err("http_journal_receipt_context_invalid".into());
        }
        let writer = self
            .writer
            .as_ref()
            .ok_or("http_budget_receipt_original_writer_required")?;
        let _queue =
            crate::agent_runtime::execution_owner::queue_recovery_approval(&context.db_path)?;
        let tx =
            rusqlite::Transaction::new_unchecked(writer, rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| "http_journal_receipt_lock_failed")?;
        let work = || receive_agent_http_headers_on(&tx, context, claim, status);
        if claim.root_target.is_some() {
            single_http_transaction(&tx, true, work)?;
        } else {
            work()?;
        }
        // The scoped Single authorizer above has already been released. V8
        // receipt persistence uses this same transaction and original writer.
        let receipt_id = match (&self.witness, &self.actor) {
            (Some(witness), Some(actor)) => {
                let fresh=crate::agent_runtime::web_mode::http_budget_receipt::FreshHttpBudgetReceipt::from_transport_on(&tx,witness,status)?;
                let sealed = crate::agent_runtime::web_mode::http_budget_storage::insert_fresh_on(
                    &tx, actor, &fresh,
                )?;
                Some(sealed.receipt_id().to_owned())
            }
            (None, None) => None,
            _ => return Err("http_budget_receipt_original_writer_required".into()),
        };
        tx.commit()
            .map_err(|_| "http_journal_receipt_commit_failed")?;
        self.received = true;
        Ok(receipt_id)
    }
}

fn claim_agent_http_sealed_transport_with_headers(
    context: &AgentRunContext,
    runtime: &AgentToolRuntime,
    request: &AgentHttpRequest,
    url: &str,
    headers: &[(String, String)],
) -> Result<AgentHttpSealedTransport, String> {
    let Some(run) = context.run.as_ref() else {
        #[cfg(test)]
        {
            return Ok(AgentHttpSealedTransport {
                writer: None,
                claim: None,
                witness: None,
                actor: None,
                received: false,
            });
        }
        #[cfg(not(test))]
        {
            return Err("http_journal_run_required".into());
        }
    };
    if run.db_path != context.db_path || runtime.current_invocation.is_empty() {
        return Err("http_journal_context_invalid".into());
    }
    if !agent_http_claim_identity_valid(context, request) {
        return Err("http_journal_identity_invalid".into());
    }
    let writer = Box::new(db::open(&context.db_path)?);
    let queue = crate::agent_runtime::execution_owner::queue_recovery_approval(&context.db_path)?;
    let tx =
        rusqlite::Transaction::new_unchecked(&writer, rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| "http_journal_lock_failed")?;
    // Select the immutable original birth before writing a physical claim.
    let actor = native_http_sealed_protocol_actor_on(&tx, &run.run_id)?;
    let single = native_single_policy(&tx, context)?;
    let work = || claim_agent_http_request_on(&tx, context, runtime, request, url, headers);
    let claim = if single {
        single_http_transaction(&tx, false, work)?
    } else {
        work()?
    };
    tx.commit()
        .map_err(|_| "http_journal_claim_commit_failed")?;
    // A committed target claim is never rolled back merely because subsequent
    // capture fails. In that case no network send is allowed, and its original
    // reservation remains indeterminate until the real recovery protocol.
    let witness = if actor.is_some() {
        let tx =
            rusqlite::Transaction::new_unchecked(&writer, rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| "http_journal_lock_failed")?;
        let fresh =
            http_budget_receipt::FreshHttpBudgetClaim::before_send_on(&tx, context, &claim)?;
        tx.commit()
            .map_err(|_| "http_journal_claim_commit_failed")?;
        Some(fresh)
    } else {
        None
    };
    drop(queue);
    Ok(AgentHttpSealedTransport {
        writer: Some(writer),
        claim: Some(claim),
        witness,
        actor,
        received: false,
    })
}

fn native_http_sealed_protocol_actor_on(
    db: &rusqlite::Transaction<'_>,
    run: &str,
) -> Result<Option<crate::agent_runtime::multi_agent::lease::CoordinatorLease>, String> {
    let Some((actor, _)) = crate::agent_runtime::multi_agent::budget::target::child_owner(db, run)?
    else {
        return Ok(None);
    };
    let original = original_root_capture_mode(db, &actor.root_run_id)?;
    let declaration = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?
        .ok_or("http_budget_receipt_birth_missing")?;
    if declaration.fact() != original.fact() {
        return Err("http_budget_receipt_original_mode_changed".into());
    }
    crate::agent_runtime::web_mode::root::verify_birth(db, &actor.root_run_id, &original)?;
    let bootstrap = declaration.bootstrap_dispatch().unwrap_or(JsonValue::Null);
    let version = bootstrap["schemaVersion"]
        .as_i64()
        .ok_or("http_budget_receipt_original_v8_protocol_required")?;
    if (1..=7).contains(&version) && bootstrap.get("httpBudgetConsumptionRule").is_none() {
        return Ok(None);
    }
    crate::agent_runtime::web_mode::http_budget_receipt::require_protocol_on(
        db,
        &actor.root_run_id,
        &original,
    )?;
    Ok(Some(actor))
}
