use super::*;
use crate::commands::mapper_evidence_lookup::{
    MapperEvidenceInventory, MapperLookupLimits, MapperLookupObservation,
};

pub(crate) struct VerifiedMapperLookupAudit {
    first_id: String,
    fact: Value,
    observation: MapperLookupObservation,
}
impl VerifiedMapperLookupAudit {
    pub(crate) fn observation(&self) -> &Value {
        self.observation.value()
    }
    pub(crate) fn require_on(
        &self,
        db: &Connection,
        actor: &CoordinatorLease,
        child: &ScheduledChild,
    ) -> Result<(), String> {
        let first = read_received_on(db, actor, child, &self.first_id)?;
        let (fact, _) = original_lookup(db, actor, child, &first)?;
        if fact != self.fact || read(db, &self.first_id, "lookup")?.as_ref() != Some(&fact) {
            return Err("mapper_loop_original_lookup_changed".into());
        }
        Ok(())
    }
}
fn original_lookup(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    first: &VerifiedMapperRound,
) -> Result<(Value, MapperLookupObservation), String> {
    first.require_on(db, actor, child)?;
    if first.round != 1 || !first.rejection().is_empty() {
        return Err("mapper_loop_lookup_paid_round_invalid".into());
    }
    let [call] = first.response.tool_calls.as_slice() else {
        return Err("mapper_loop_lookup_call_group_invalid".into());
    };
    if call.name != "evidence.lookup" {
        return Err("mapper_loop_lookup_tool_invalid".into());
    }
    let inventory = original_inventory_on(db, actor, child)?;
    let observation = inventory.lookup_on(db, actor, &call.arguments)?;
    let fact = json!({"schemaVersion":1,"purpose":"original_paid_mapper_lookup","firstCallId":first.id,"firstReceivedHash":store::stable_hash(&first.fact.to_string()),"inventoryHash":inventory.hash(),"call":{"id":call.id,"name":call.name,"arguments":call.arguments},"observation":observation.value()});
    Ok((fact, observation))
}
pub(super) fn original_inventory_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<MapperEvidenceInventory, String> {
    let grant = budget::model::original_mapper_loop_grant_on(db, actor, &child.assignment_id)?;
    let input = grant.input().fact();
    let original = &input["inventory"];
    let limits = MapperLookupLimits::new(
        original["limits"]["sourceMaxBytes"]
            .as_u64()
            .ok_or("mapper_loop_inventory_limits_invalid")?,
        original["limits"]["observationMaxBytes"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("mapper_loop_inventory_limits_invalid")?,
    )?;
    let directory = crate::commands::resolve_agent_evidence_location(db, &actor.root_run_id)?;
    let inventory = MapperEvidenceInventory::capture_on(
        db,
        actor,
        &directory,
        original["evidenceRevision"]
            .as_i64()
            .ok_or("mapper_loop_inventory_revision_invalid")?,
        limits,
    )?;
    if inventory.fact() != original
        || inventory.hash()
            != input["inventoryHash"]
                .as_str()
                .ok_or("mapper_loop_inventory_hash_invalid")?
    {
        return Err("mapper_loop_original_inventory_changed".into());
    }
    Ok(inventory)
}
pub(crate) fn audit_lookup_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    bounds: &VerifiedMapperLoopRequestBounds,
    first: &VerifiedMapperRound,
) -> Result<VerifiedMapperLookupAudit, String> {
    writer::run(db, |writer| {
        let tx = Transaction::new_unchecked(writer, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        live(&tx, actor, child, bounds, bounds.second_estimate())?;
        let (fact, observation) = original_lookup(&tx, actor, child, first)?;
        match read(&tx, &first.id, "lookup")? {
            Some(saved) if saved == fact => {}
            Some(_) => return Err("mapper_loop_original_lookup_changed".into()),
            None => {
                let text = fact.to_string();
                tx.execute("INSERT INTO main.agent_mapper_loop_journal(call_id,phase,root_run_id,assignment_id,child_run_id,worker_attempt_id,round_number,request_hash,fact_json,fact_hash) VALUES(?1,'lookup',?2,?3,?4,?5,1,?6,?7,?8)",params![first.id,actor.root_run_id,child.assignment_id,child.run_id,first.worker,first.request_hash,text,store::stable_hash(&text)]).map_err(|e|e.to_string())?;
            }
        }
        let audit = VerifiedMapperLookupAudit {
            first_id: first.id.clone(),
            fact,
            observation,
        };
        audit.require_on(&tx, actor, child)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(audit)
    })
}
pub(crate) fn begin_second_for_transport(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    bounds: &VerifiedMapperLoopRequestBounds,
    first: &VerifiedMapperRound,
    audit: &VerifiedMapperLookupAudit,
) -> Result<Start, String> {
    writer::run(db, |writer| {
        let tx = Transaction::new_unchecked(writer, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        first.require_on(&tx, actor, child)?;
        audit.require_on(&tx, actor, child)?;
        if audit.first_id != first.id {
            return Err("mapper_loop_original_lookup_scope_changed".into());
        }
        let grant = budget::model::original_mapper_loop_grant_on(&tx, actor, &child.assignment_id)?;
        let request = bounds.second_request(&first.response, &audit.observation)?;
        let request_fact = json!({"messages":request.messages,"tools":request.tools.iter().map(|t|t.as_function_spec()).collect::<Vec<_>>()});
        let hash = store::stable_hash(&request_fact.to_string());
        let id=store::stable_hash(&json!({"root":actor.root_run_id,"assignment":child.assignment_id,"worker":grant.worker_attempt_id(),"child":child.run_id,"round":2,"requestHash":hash}).to_string());
        if read(&tx, &id, "dispatch")?.is_some() {
            return if read(&tx, &id, "received")?.is_some() {
                Ok(Start::Received(read_received_on(&tx, actor, child, &id)?))
            } else {
                Err("mapper_loop_original_outcome_requires_reconciliation".into())
            };
        }
        live(&tx, actor, child, bounds, bounds.second_estimate())?;
        let owner = execution_owner::claim_native_invocation(
            Path::new(writer.path().ok_or("mapper_loop_database_missing")?),
            &actor.scan_id,
            actor.attempt_number,
            INVOCATION_KIND,
            &child.run_id,
        )?;
        budget::clock::sample(&tx, actor, &child.assignment_id)?;
        let pending = PendingRound {
            actor: actor.clone(),
            child: child.clone(),
            worker: grant.worker_attempt_id().into(),
            id,
            request_hash: hash,
            dispatch: json!({"schemaVersion":1,"scope":{"rootRunId":actor.root_run_id,"assignmentId":child.assignment_id,"childRunId":child.run_id,"workerAttemptId":grant.worker_attempt_id(),"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"epoch":actor.lease_epoch,"fence":actor.fencing_token},"round":2,"inputHash":grant.input().hash(),"request":request_fact,"estimate":bounds.second_estimate(),"configuration":bounds.original_configuration_binding(),"firstCallId":first.id,"lookupHash":store::stable_hash(&audit.fact.to_string())}),
            estimate: bounds.second_estimate(),
            round: 2,
            request,
            stage: FinancialStage::Unsettled,
            financial_failure: None,
            _owner: owner,
        };
        insert(&tx, &pending, "dispatch", &pending.dispatch)?;
        original(&tx, &pending)?;
        live(&tx, actor, child, bounds, pending.estimate)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Start::Dispatch(pending))
    })
}
pub(super) fn verify_second_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    dispatch: &Value,
) -> Result<(), String> {
    let first_id = dispatch["firstCallId"]
        .as_str()
        .ok_or("mapper_loop_first_receipt_missing")?;
    let first = read_received_on(db, actor, child, first_id)?;
    let (fact, observation) = original_lookup(db, actor, child, &first)?;
    if read(db, first_id, "lookup")?.as_ref() != Some(&fact)
        || dispatch["lookupHash"] != store::stable_hash(&fact.to_string())
    {
        return Err("mapper_loop_original_lookup_changed".into());
    }
    let grant = budget::model::original_mapper_loop_grant_on(db, actor, &child.assignment_id)?;
    let mut messages = grant.input().fact()["request"]["messages"]
        .as_array()
        .ok_or("mapper_loop_request_invalid")?
        .clone();
    let call = &first.response.tool_calls[0];
    messages.push(json!({"role":"assistant","content":"","tool_calls":[{"id":call.id,"type":"function","function":{"name":"evidence.lookup","arguments":call.arguments.to_string()}}]}));
    messages.push(
        json!({"role":"tool","tool_call_id":call.id,"content":observation.value().to_string()}),
    );
    if dispatch["request"]
        != json!({"messages":messages,"tools":grant.input().fact()["request"]["tools"]})
        || dispatch["estimate"] != grant.input().fact()["secondRoundEstimatedTokens"]
    {
        return Err("mapper_loop_original_second_wire_changed".into());
    }
    Ok(())
}

pub(super) fn verify_lookup_original_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    first: &VerifiedMapperRound,
) -> Result<(), String> {
    let (fact, _) = original_lookup(db, actor, child, first)?;
    if read(db, &first.id, "lookup")?.as_ref() != Some(&fact) {
        return Err("mapper_loop_original_lookup_changed".into());
    }
    Ok(())
}
