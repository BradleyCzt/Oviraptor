// Independent paid Workers, using the same transport, receipts and owner lifetime
// as existing tool-free children. Only assign wakes a carrier; idle IDs cost nothing.
fn native_homogeneous_seed_observations(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(), String> {
    use crate::agent_runtime::{
        evidence_graph::{
            contract::{EvidenceNode, EvidenceNodeKind, EvidenceProvenance},
            store::{evidence_natural_key, insert_evidence_node},
        },
        multi_agent::homogeneous::model_worker,
        store::stable_hash,
    };
    let db = db::open(&context.db_path)?;
    if !model_worker::enabled(&db, &actor.scan_id, actor.attempt_number)? {
        return Ok(());
    }
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let (observed, hash) = native_coordinator_frozen_evidence(&tx, context)?;
    let origin =
        reqwest::Url::parse(&context.target_url).map_err(|_| "homogeneous_target_invalid")?;
    let revision: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(revision),0)+1 FROM agent_evidence_revisions WHERE root_run_id=?1",
            [&actor.root_run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut facts = vec![(
        EvidenceNodeKind::PageState,
        json!({"url":context.target_url,"host":origin.host_str(),"path":origin.path(),"surface":observed["surface"],"capture_sha256":hash,"observation_kind":"frozen_frontend_capture_metadata","vulnerability_confirmed":false}),
    )];
    for candidate in observed["apiCandidates"]
        .as_array()
        .into_iter()
        .flatten()
        .take(32)
    {
        let Some(raw) = candidate["url"]
            .as_str()
            .or_else(|| candidate["path"].as_str())
        else {
            continue;
        };
        let Ok(endpoint) = origin.join(raw) else {
            continue;
        };
        if endpoint.origin() != origin.origin() || raw.len() > 2048 {
            continue;
        }
        facts.push((EvidenceNodeKind::Endpoint,json!({"url":endpoint.as_str(),"host":endpoint.host_str(),"path":endpoint.path(),"observation":{
            "source":candidate["source"].as_str().unwrap_or("").chars().take(128).collect::<String>(),
            "method":candidate["method"].as_str().unwrap_or("").chars().take(16).collect::<String>(),
            "confidence":candidate["confidence"].as_str().unwrap_or("").chars().take(32).collect::<String>(),
            "candidateOnly":candidate["candidateOnly"].as_bool()},"capture_sha256":hash,"observation_kind":"observed_frontend_api_reference","target_request_executed":false,"vulnerability_confirmed":false})));
    }
    for (kind, payload) in facts {
        let identity = format!("homogeneous_frontend:{}:{}", hash, payload.to_string());
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_evidence_nodes WHERE root_run_id=?1 AND natural_key_hash=?2)",params![actor.root_run_id,evidence_natural_key(&actor.root_run_id,kind,&identity)],|r|r.get(0)).map_err(|e|e.to_string())?;
        if exists {
            continue;
        }
        insert_evidence_node(
            &tx,
            &EvidenceNode {
                id: stable_hash(&format!("{}:{identity}", actor.root_run_id)),
                root_run_id: actor.root_run_id.clone(),
                revision,
                kind,
                provenance: EvidenceProvenance::Observed,
                natural_key_hash: evidence_natural_key(&actor.root_run_id, kind, &identity),
                payload,
                artifact_refs: vec![],
                created_by_run_id: actor.root_run_id.clone(),
                supersedes_id: String::new(),
                created_at: String::new(),
            },
        )?;
    }
    tx.commit().map_err(|e| e.to_string())
}

fn native_homogeneous_model_worker(
    transport: &SpecialistTransportContext<'_>,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    id: &str,
    control: &std::sync::Mutex<()>,
    configuration:Option<&crate::tool_supply::ToolAdministratorConfiguration>,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::homogeneous::{model_worker, store};
    transport.require_supervision(actor)?;
    let db = db::open(transport.db_path)?;
    let lane:String=db.query_row("SELECT lane FROM agent_assignments WHERE id=?1",[id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if lane=="review" {return native_homogeneous_review_worker(transport,actor,id,control);}
    let (child, input) = {
        let _guard = control.lock().map_err(|_| "homogeneous_control_poisoned")?;
        model_worker::claim(&db, actor, id)?
    };
    let expected = input.clone();
    let execution=crate::agent_runtime::multi_agent::homogeneous::execution_policy::read_original_on(&db,&actor.root_run_id)?.is_some();
    let system=if execution{crate::agent_runtime::multi_agent::homogeneous::execution_worker::SYSTEM}else{model_worker::SYSTEM};
    let result = specialist_round_transport_bound(
        transport,
        actor,
        &child,
        system,
        input,
        None,
        |tx| {
            if !model_worker::enabled(tx, &actor.scan_id, actor.attempt_number)? {
                return Err("homogeneous_worker_frozen_mode_missing".into());
            }
            let task = store::load_task_by_root_run(tx, &actor.root_run_id)
                .map_err(|e| e.detail)?
                .ok_or("homogeneous_task_missing")?;
            let assignment = store::load_assignment(tx, id)
                .map_err(|e| e.detail)?
                .ok_or("homogeneous_assignment_missing")?;
            let slice = store::load_slice(tx, &assignment.slice_id)
                .map_err(|e| e.detail)?
                .ok_or("homogeneous_slice_missing")?;
            crate::agent_runtime::multi_agent::homogeneous::model_guidance::verify_frozen_input(
                &model_worker::input(tx, &task, &slice)?,
                &expected,
            )?;
            Ok(())
        },
    );
    // Even a paid semantic refusal has a verifiable receipt and a failed report.
    // Transport-unknown remains fenced and cannot become a retryable free report.
    if execution{
        let configuration=configuration.ok_or("homogeneous_execution_configuration_unavailable")?;
        let cancel=specialist_model_cancel_token(transport.db_path,actor,&child,transport.supervision.clone());
        return crate::agent_runtime::multi_agent::homogeneous::execution_worker::deliver(&db,configuration,actor,&child,&||cancel.is_cancelled());
    }
    let _guard = control.lock().map_err(|_| "homogeneous_control_poisoned")?;
    match model_worker::deliver(&db, actor, &child) {
        Ok(()) => Ok(()),
        Err(delivery) => Err(format!(
            "homogeneous_worker_reconciliation_required:{};{delivery}",
            result.err().unwrap_or_default()
        )),
    }
}

fn native_homogeneous_review_worker(transport:&SpecialistTransportContext<'_>,actor:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,id:&str,control:&std::sync::Mutex<()>)->Result<(),String>{
    let db=db::open(transport.db_path)?;
    let (child,input)={let _guard=control.lock().map_err(|_|"homogeneous_control_poisoned")?;crate::commands::homogeneous_execution::claim_review(&db,actor,id)?};
    let expected=input.clone();
    let result=specialist_round_transport_bound(transport,actor,&child,crate::commands::homogeneous_execution::REVIEW_SYSTEM,input,None,|tx|{
        if crate::commands::homogeneous_execution::review_input(tx,actor,id)?!=expected {return Err("homogeneous_review_original_bundle_changed".into());}Ok(())
    });
    let _guard=control.lock().map_err(|_|"homogeneous_control_poisoned")?;
    crate::commands::homogeneous_execution::deliver_review(&db,actor,&child).map_err(|e|format!("homogeneous_review_reconciliation_required:{};{e}",result.err().unwrap_or_default()))
}
