//! Fresh v12 anonymous response proof. Actual caller selects only signed v12 birth.
use crate::agent_runtime::{contract::AgentRole, multi_agent::{attempts, budget, lease::CoordinatorLease, scheduler::ScheduledChild}, store, web_mode::{self, ModeFact, VerifiedMode, withdrawal_issuer::WithdrawalIssuer}};
use rusqlite::{params, Connection, OptionalExtension, Transaction, types::Value as SqlValue};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{io::Read, path::{Path, PathBuf}};
pub(crate) const RULE:&str="original_received_anonymous_public_surface_response_v1";
pub(crate) const OUTPUT_RULE:&str="original_closed_anonymous_entry_surface_output_advisory_v1";
pub(crate) const CAPACITY_RULE:&str="retain_one_original_surface_output_assessment_per_worker_v1";
pub(crate) const SCHEMA_SQL:&str=include_str!("external_surface_response_receipt.sql");
const DOMAIN:&str="oviraptor.original-public-surface-response.v1";

pub(crate) struct ClaimedPublicSurfaceWitness {
    connection:usize, actor:CoordinatorLease, child:ScheduledChild, worker:String,
    original:ModeFact, target_dir:PathBuf, contract:Value, claim:Value, fees:Vec<Vec<SqlValue>>,
    request_hash:String, proxy_binding:String, sent:bool, issuer:WithdrawalIssuer,
}
impl ClaimedPublicSurfaceWitness {
    /// Only this original unsent transport witness can identify its one own
    /// pending target charge. Saved rows or caller-supplied source names cannot.
    pub(crate) fn verify_unsent_on(&self,db:&Connection)->Result<&CoordinatorLease,String> {
        if db.is_autocommit() || db as *const Connection as usize!=self.connection || self.sent {
            return Err("public_surface_response_original_writer_changed".into());
        }
        let original = super::original_root_capture_mode(db,&self.actor.root_run_id)?;
        if original.fact()!=&self.original {
            return Err("public_surface_response_original_mode_changed".into());
        }
        require_protocol(db,&self.actor,&self.child,&original)?;
        require_claim_state(db,&self.actor,&self.child,"claimed")?;
        if claim_binding(db,&self.actor,&self.child)?!=self.claim
            || fee_rows(db,&self.actor,&self.child,&self.worker,false)?!=self.fees {
            return Err("public_surface_response_original_claim_changed".into());
        }
        Ok(&self.actor)
    }
}
pub(crate) struct FreshPublicSurfaceResponse {
    witness:ClaimedPublicSurfaceWitness, view:Value, body:Vec<u8>, sealed:Option<Value>,
}
impl FreshPublicSurfaceResponse {
    pub(crate) fn body(&self)->&[u8] { &self.body }
    pub(crate) fn view(&self)->&Value { &self.view }
    pub(crate) fn request(&self)->Value { json!({"url":self.witness.contract["url"],"method":"GET","identity":"anonymous","headers":[]}) }
    pub(crate) fn encoded_for_original(&self, original:&ModeFact)->Result<Vec<u8>,String> {
        if original!=&self.witness.original {return Err("public_surface_response_original_mode_changed".into());}
        encoded(self.sealed.as_ref().ok_or("public_surface_response_not_originally_received")?)
    }
}
pub(crate) struct VerifiedPublicSurfaceResponse {value:Value,id:String}
impl VerifiedPublicSurfaceResponse {
    pub(crate) fn fact(&self)->&Value {&self.value}
    pub(crate) fn receipt_id(&self)->&str {&self.id}
}

// Root-only protocol selection. Missing legacy definitions are old no-op;
// an original v12 birth cannot downgrade when a definition is cold changed.
pub(crate) fn enabled_on(db:&Connection,root:&str)->Result<bool,String> {
    if db.is_autocommit() {return Err("public_surface_response_snapshot_required".into());}
    let birth:Option<String>=db.query_row("SELECT binding_json FROM agent_root_mode_birth_seals WHERE root_run_id=?1",[root],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    let birth=birth.map(|raw|serde_json::from_str::<Value>(&raw).map_err(|_|"public_surface_response_original_protocol_missing")).transpose()?;
    let mode=web_mode::root::read(db,root)?;
    let current=mode.as_ref().and_then(|m|m.bootstrap_dispatch()).unwrap_or(Value::Null);
    let original=birth.as_ref().map(|b|&b["definition"]["bootstrapDispatch"]);
    let has_new_rule=|p:&Value|matches!(p["schemaVersion"].as_i64(),Some(12..=15)) || p.get("externalSurfaceResponseRule").is_some();
    if !has_new_rule(&current) && !original.is_some_and(has_new_rule) {return Ok(false);}
    let mode=mode.ok_or("public_surface_response_original_protocol_missing")?;
    let proof=super::original_root_capture_mode(db,root)?;
    web_mode::root::verify_birth(db,root,&proof)?;
    budget::root::RootOwner::load_original(db,root)?;
    if mode.mode()!=web_mode::WebMode::Multi || mode.fact()!=proof.fact() || !matches!(current["schemaVersion"].as_i64(),Some(12..=15))
        || current["externalSurfaceResponseRule"]!=RULE || current["externalSurfaceOutputRule"]!=OUTPUT_RULE
        || current["externalSurfaceOutputRequestCapacityRule"]!=CAPACITY_RULE {
        return Err("public_surface_response_original_protocol_missing".into());
    }
    Ok(true)
}

// The factory owns INSERT and original fee claim. It cannot adopt a saved row.
// Caller must hold a stable Box<Connection>, commit, then call before_send_on.
pub(super) fn claim_on(tx:&Transaction<'_>, context:&super::AgentRunContext, actor:&CoordinatorLease, child:&ScheduledChild)->Result<ClaimedPublicSurfaceWitness,String> {
    with_savepoint(tx, ResponseWritePhase::Claim, || claim_in(tx,context,actor,child))
}
fn claim_in(tx:&Transaction<'_>, context:&super::AgentRunContext, actor:&CoordinatorLease, child:&ScheduledChild)->Result<ClaimedPublicSurfaceWitness,String> {
    let original=super::original_root_capture_mode(tx,&actor.root_run_id)?;
    require_protocol(tx,actor,child,&original)?;
    verify_schema_on(tx)?;
    budget::supervision_capacity::require_external_surface_probe_headroom_on(tx,actor)?;
    let contract=super::public_surface_authority(tx,context,actor,child)?;
    let usage=super::agent_request_accounting(tx,&actor.scan_id,actor.attempt_number,&actor.target_key)?;
    let ceiling=context.execution_plan.hard_model_requests.max(1).saturating_mul(4).min(400);
    if usage.budget_committed>=ceiling {return Err("public_surface_request_budget_exhausted".into());}
    let worker=attempts::require_live_for_run(tx,&child.run_id)?;
    if worker.lease_epoch!=1 {return Err("public_surface_response_original_worker_changed".into());}
    let directory=super::resolve_agent_evidence_location(tx,&actor.root_run_id)?;
    if directory!=context.target_dir {return Err("public_surface_response_directory_changed".into());}
    let n=tx.execute("INSERT INTO agent_external_surface_captures(scan_id,attempt_number,target_url,assignment_id,child_run_id,state) VALUES(?1,?2,?3,?4,?5,'claimed')",params![actor.scan_id,actor.attempt_number,actor.target_key,child.assignment_id,child.run_id]).map_err(|_|"public_surface_already_claimed_requires_reconciliation")?;
    if n!=1 {return Err("public_surface_claim_postcondition".into());}
    budget::target::claim(tx,actor,&child.assignment_id,&format!("public:{}",child.assignment_id))?;
    require_claim_state(tx,actor,child,"claimed")?;
    let claim=claim_binding(tx,actor,child)?;
    let fees=fee_rows(tx,actor,child,&worker.id,false)?;
    let issuer=WithdrawalIssuer::from_verified_birth_on(tx,&actor.root_run_id,original.clone())?;
    issuer.require_actor(tx,actor)?;
    let proxy_binding=store::stable_hash(context.proxy.as_deref().unwrap_or(""));
    let request_hash=request_hash(&contract,&proxy_binding);
    Ok(ClaimedPublicSurfaceWitness {connection: &**tx as *const Connection as usize,actor:actor.clone(),child:child.clone(),worker:worker.id,original:original.fact().clone(),target_dir:directory,contract,claim,fees,request_hash,proxy_binding,sent:false,issuer})
}

pub(super) fn before_send_on(db:&Connection, context:&super::AgentRunContext, witness:&mut ClaimedPublicSurfaceWitness)->Result<(),String> {
    if !db.is_autocommit() || db as *const Connection as usize!=witness.connection || witness.sent {return Err("public_surface_response_original_writer_changed".into());}
    let tx=db.unchecked_transaction().map_err(|e|e.to_string())?;
    if store::stable_hash(context.proxy.as_deref().unwrap_or(""))!=witness.proxy_binding
        || super::public_surface_authority(&tx,context,&witness.actor,&witness.child)?!=witness.contract
        || claim_binding(&tx,&witness.actor,&witness.child)?!=witness.claim
        || fee_rows(&tx,&witness.actor,&witness.child,&witness.worker,false)?!=witness.fees {
        return Err("public_surface_response_original_claim_changed".into());
    }
    require_claim_state(&tx,&witness.actor,&witness.child,"claimed")?;
    budget::supervision_capacity::require_external_surface_presend_headroom_on(&tx,witness)?;
    witness.sent=true;
    Ok(())
}

// This leaf owns the exact anonymous request builder and actual send. A
// caller cannot pass an authenticated Response or reconstruct a saved body.
pub(super) fn send_transport(db:&Connection, context:&super::AgentRunContext, mut witness:ClaimedPublicSurfaceWitness)->Result<FreshPublicSurfaceResponse,String> {
    before_send_on(db,context,&mut witness)?;
    let cancel=super::agent_scan_cancel_token(&context.db_path,&context.scan_id,context.attempt_number);
    if cancel.is_cancelled() {return Err("public_surface_cancelled".into());}
    let mut builder=reqwest::blocking::Client::builder().redirect(reqwest::redirect::Policy::none()).no_proxy()
        .connect_timeout(std::time::Duration::from_secs(10)).timeout(std::time::Duration::from_secs(20));
    if let Some(proxy)=&context.proxy {builder=builder.proxy(reqwest::Proxy::all(proxy).map_err(|_|"public_surface_proxy_invalid")?);}
    let client=builder.build().map_err(|_|"public_surface_client_unavailable")?;
    let url=witness.contract["url"].as_str().ok_or("public_surface_url_missing")?;
    let timeout=budget::target::transport_timeout(db,&witness.child.run_id,std::time::Duration::from_secs(20))?;
    let response=client.get(url).header(reqwest::header::USER_AGENT,"oviraptor-public-surface/1.0")
        .header(reqwest::header::CACHE_CONTROL,"no-store").timeout(timeout).send()
        .map_err(|_|"public_surface_transport_outcome_unknown")?;
    receive_transport(witness,response)
}

// Only the above real fixed transport can construct body/response provenance.
fn receive_transport(witness:ClaimedPublicSurfaceWitness, mut response:reqwest::blocking::Response)->Result<FreshPublicSurfaceResponse,String> {
    if !witness.sent || Some(response.url().as_str())!=witness.contract["url"].as_str() {return Err("public_surface_response_transport_binding_changed".into());}
    let status=response.status().as_u16();
    let content_type=response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|s|s.to_str().ok()).unwrap_or_default().to_string();
    let headers=super::agent_security_relevant_headers(response.headers());
    let mut body=Vec::new();
    Read::by_ref(&mut response).take(super::AGENT_MAX_RESPONSE_BYTES as u64+1).read_to_end(&mut body).map_err(|_|"public_surface_body_incomplete")?;
    let truncated=body.len()>super::AGENT_MAX_RESPONSE_BYTES;
    body.truncate(super::AGENT_MAX_RESPONSE_BYTES);
    let text=String::from_utf8_lossy(&body);
    let protected=status==429 || (status!=404 && super::is_directory_block_signal(&format!("http {status} {content_type} {}",text.chars().take(4000).collect::<String>())));
    let view=crate::agent_runtime::secrets::redact_json(&json!({"requestId":format!("public-surface:{}",witness.child.assignment_id),"url":witness.contract["url"],"method":"GET","identity":"anonymous","status":status,"contentType":content_type,"securityRelevantHeaders":headers,"truncated":truncated,"bodySha256":format!("{:x}",Sha256::digest(&body)),"bodyBytes":body.len(),"untrustedTextExcerpt":text.chars().take(12000).collect::<String>(),"redirectFollowed":false,"protected":protected,"confirmedFinding":false}));
    Ok(FreshPublicSurfaceResponse {witness,view,body,sealed:None})
}

// Caller writes the genuine fresh artifact, then enters the original writer's
// receipt transaction. This function owns receive/cost/signature persistence.
pub(super) fn issue_received_on(tx:&Transaction<'_>, context:&super::AgentRunContext, fresh:FreshPublicSurfaceResponse, written:&super::FreshHttpArtifactWritten)->Result<VerifiedPublicSurfaceResponse,String> {
    with_savepoint(tx, ResponseWritePhase::Receive, || issue_received_in(tx,context,fresh,written))
}
fn issue_received_in(tx:&Transaction<'_>, context:&super::AgentRunContext, mut fresh:FreshPublicSurfaceResponse, written:&super::FreshHttpArtifactWritten)->Result<VerifiedPublicSurfaceResponse,String> {
    let w=&fresh.witness;
    let body_hash=format!("{:x}",Sha256::digest(&fresh.body));
    if &**tx as *const Connection as usize!=w.connection || written.scan_id()!=w.actor.scan_id || written.attempt_number()!=w.actor.attempt_number || written.target_url()!=w.actor.target_key || written.run_id()!=Some(w.child.run_id.as_str()) || written.directory()!=w.target_dir.join(super::AGENT_HTTP_DIRECTORY) || written.body_bytes()!=fresh.body.len() || written.body_sha256()!=body_hash {return Err("public_surface_response_original_artifact_changed".into());}
    if super::agent_scan_cancel_token(&context.db_path,&context.scan_id,context.attempt_number).is_cancelled() {return Err("public_surface_cancelled_after_capture".into());}
    if super::public_surface_authority(tx,context,&w.actor,&w.child)?!=w.contract || claim_binding(tx,&w.actor,&w.child)?!=w.claim || fee_rows(tx,&w.actor,&w.child,&w.worker,false)?!=w.fees {return Err("public_surface_response_original_claim_changed".into());}
    let payload=json!({"capture":fresh.view,"artifactId":written.artifact_id(),"targetRequests":1});
    let raw=payload.to_string();
    let n=tx.execute("UPDATE agent_external_surface_captures SET state='received',artifact_id=?1,response_json=?2,response_hash=?3 WHERE assignment_id=?4 AND child_run_id=?5 AND state='claimed' AND artifact_id=''",params![written.artifact_id(),raw,store::stable_hash(&raw),w.child.assignment_id,w.child.run_id]).map_err(|e|e.to_string())?;
    if n!=1 {return Err("public_surface_receipt_postcondition".into());}
    budget::target::receive(tx,&w.actor,&w.child.assignment_id,&format!("public:{}",w.child.assignment_id))?;
    super::record_public_surface_fact(tx,context,&w.actor,&w.child,written.artifact_id(),&fresh.view)?;
    let fees=fee_rows(tx,&w.actor,&w.child,&w.worker,true)?;
    let value=json!({"schemaVersion":1,"purpose":"original_received_anonymous_public_surface_response","rootRunId":w.actor.root_run_id,"scanId":w.actor.scan_id,"attemptNumber":w.actor.attempt_number,"targetKey":w.actor.target_key,"assignmentId":w.child.assignment_id,"childRunId":w.child.run_id,"workerAttemptId":w.worker,"coordinatorEpoch":w.actor.lease_epoch,"coordinatorFence":w.actor.fencing_token,"originalModeReceiptId":w.original.receipt_id,"contract":w.contract,"requestHash":w.request_hash,"proxyBinding":w.proxy_binding,"claim":w.claim,"response":payload,"artifactId":written.artifact_id(),"recordSha256":written.record_sha256(),"recordBytes":written.record_bytes(),"bodySha256":written.body_sha256(),"bodyBytes":written.body_bytes(),"retainedBytesOnly":true,"originalTargetFeesHash":store::stable_hash(&format!("{fees:?}")),"executionAllowed":false});
    verify_artifact(&w.target_dir,&value)?;
    let canonical=value.to_string();
    let id=store::stable_hash(&canonical);
    fresh.sealed=Some(value);
    let tag=fresh.witness.issuer.sign_fresh_public_surface_response_receipt(&fresh)?;
    let w=&fresh.witness;
    let n=tx.execute("INSERT INTO main.agent_public_surface_response_receipts(receipt_id,root_run_id,assignment_id,child_run_id,worker_attempt_id,claim_rowid,artifact_id,fact_json,fact_hash,signature) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?1,?9)",params![id,w.actor.root_run_id,w.child.assignment_id,w.child.run_id,w.worker,w.claim["rowid"].as_i64(),written.artifact_id(),canonical,tag]).map_err(|e|e.to_string())?;
    if n!=1 {return Err("public_surface_response_write_unconfirmed".into());}
    read_original_on(tx,&w.actor,&w.child)?.ok_or_else(||"public_surface_response_write_unconfirmed".into())
}

pub(crate) fn read_original_on(db:&Connection,actor:&CoordinatorLease,child:&ScheduledChild)->Result<Option<VerifiedPublicSurfaceResponse>,String> {
    if db.is_autocommit() {return Err("public_surface_response_snapshot_required".into());}
    verify_schema_on(db)?;
    let original=super::original_root_capture_mode(db,&actor.root_run_id)?;
    require_protocol(db,actor,child,&original)?;
    let worker=attempts::current(db,actor,&child.assignment_id)?;
    type Row=(String,String,String,Vec<u8>,String,String,String,String,i64,String);
    let row:Option<Row>=db.query_row("SELECT receipt_id,fact_json,fact_hash,signature,root_run_id,assignment_id,child_run_id,worker_attempt_id,claim_rowid,artifact_id FROM main.agent_public_surface_response_receipts WHERE worker_attempt_id=?1",[&worker.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).optional().map_err(|e|e.to_string())?;
    let Some((id,raw,hash,tag,root,assignment,run,wid,rowid,artifact))=row else{return Ok(None)};
    let value:Value=serde_json::from_str(&raw).map_err(|_|"public_surface_response_original_fact_changed")?;
    let issuer=WithdrawalIssuer::from_verified_birth_on(db,&actor.root_run_id,original.clone())?;
    issuer.verify_public_surface_response_receipt(&encoded(&value)?,&tag)?;
    let canonical=value.to_string();
    if canonical!=raw || hash!=store::stable_hash(&raw) || id!=hash || root!=actor.root_run_id || assignment!=child.assignment_id || run!=child.run_id || wid!=worker.id || value["schemaVersion"]!=1 || value["purpose"]!="original_received_anonymous_public_surface_response" || value["rootRunId"]!=root || value["scanId"]!=actor.scan_id || value["attemptNumber"]!=actor.attempt_number || value["targetKey"]!=actor.target_key || value["assignmentId"]!=assignment || value["childRunId"]!=run || value["workerAttemptId"]!=wid || value["coordinatorEpoch"]!=actor.lease_epoch || value["coordinatorFence"]!=actor.fencing_token || value["originalModeReceiptId"]!=original.fact().receipt_id || value["artifactId"]!=artifact || value["claim"]["rowid"]!=rowid || value["executionAllowed"]!=false || value["retainedBytesOnly"]!=true {return Err("public_surface_response_original_binding_changed".into());}
    let proxy_binding=value["proxyBinding"].as_str().ok_or("public_surface_response_original_binding_changed")?;
    if value["requestHash"]!=request_hash(&value["contract"],proxy_binding) {return Err("public_surface_response_original_binding_changed".into());}
    if claim_binding(db,actor,child)?!=value["claim"] {return Err("public_surface_response_original_claim_changed".into());}
    require_claim_state(db,actor,child,"received")?;
    let (payload,hash):(String,String)=db.query_row("SELECT response_json,response_hash FROM agent_external_surface_captures WHERE assignment_id=?1 AND child_run_id=?2",params![assignment,run],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    let canonical_payload=value["response"].to_string();
    if payload!=canonical_payload || hash!=store::stable_hash(&payload) {return Err("public_surface_response_original_fact_changed".into());}
    let fees=fee_rows(db,actor,child,&worker.id,true)?;
    if value["originalTargetFeesHash"]!=store::stable_hash(&format!("{fees:?}")) {return Err("public_surface_response_original_fees_changed".into());}
    let directory=super::resolve_agent_evidence_location(db,&actor.root_run_id)?;
    verify_artifact(&directory,&value)?;
    Ok(Some(VerifiedPublicSurfaceResponse {value,id}))
}

fn require_protocol(db:&Connection,actor:&CoordinatorLease,child:&ScheduledChild,proof:&VerifiedMode)->Result<(),String> {
    if db.is_autocommit() || child.role!=AgentRole::ExternalSurface {return Err("public_surface_response_scope_invalid".into());}
    web_mode::root::verify_birth(db,&actor.root_run_id,proof)?;
    let mode=web_mode::root::read(db,&actor.root_run_id)?.ok_or("public_surface_response_birth_missing")?;
    let p=mode.bootstrap_dispatch().unwrap_or(Value::Null);
    if mode.fact()!=proof.fact() || mode.mode()!=web_mode::WebMode::Multi || !matches!(p["schemaVersion"].as_i64(),Some(12..=15)) || p["externalSurfaceResponseRule"]!=RULE || p["externalSurfaceOutputRule"]!=OUTPUT_RULE || p["externalSurfaceOutputRequestCapacityRule"]!=CAPACITY_RULE {return Err("public_surface_response_original_protocol_missing".into());}
    budget::root::RootOwner::load_original(db,&actor.root_run_id)?.require_original_coordinator(db,actor)?;
    let worker=attempts::current(db,actor,&child.assignment_id)?;
    if worker.lease_epoch!=1 || worker.child_run_id!=child.run_id {return Err("public_surface_response_original_worker_changed".into());}
    let task:String=db.query_row("SELECT a.task_slice_json FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND a.child_run_id=?2 AND a.coordinator_run_id=?3 AND a.role='external_surface' AND a.lane='target_touching' AND a.lease_epoch=?4 AND a.fencing_token=?5 AND r.assignment_id=a.id AND r.root_run_id=?3 AND r.parent_run_id=?3 AND r.role=a.role AND r.lane=a.lane AND r.backend='native' AND r.orchestration_policy='multi' AND r.scan_id=?6 AND r.attempt_number=?7 AND r.target_url=?8 AND a.target_key=?8",params![child.assignment_id,child.run_id,actor.root_run_id,actor.lease_epoch,actor.fencing_token,actor.scan_id,actor.attempt_number,actor.target_key],|r|r.get(0)).map_err(|_|"public_surface_response_original_contract_changed")?;
    let task:Value=serde_json::from_str(&task).map_err(|_|"public_surface_response_original_contract_changed")?;
    let mut url=reqwest::Url::parse(&actor.target_key).map_err(|_|"public_surface_response_original_contract_changed")?;url.set_fragment(None);
    if !matches!(url.scheme(),"http"|"https") || !url.username().is_empty() || url.password().is_some() || task!=json!({"schemaVersion":1,"url":url.as_str(),"method":"GET","anonymousOnly":true,"maxRequests":1,"followRedirects":false}) {return Err("public_surface_response_original_contract_changed".into());}
    Ok(())
}
fn claim_binding(db:&Connection,actor:&CoordinatorLease,child:&ScheduledChild)->Result<Value,String> {
    db.query_row("SELECT rowid,created_at FROM agent_external_surface_captures WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND assignment_id=?4 AND child_run_id=?5",params![actor.scan_id,actor.attempt_number,actor.target_key,child.assignment_id,child.run_id],|r|Ok(json!({"rowid":r.get::<_,i64>(0)?,"createdAt":r.get::<_,String>(1)?,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"assignmentId":child.assignment_id,"childRunId":child.run_id}))).map_err(|_|"public_surface_response_original_claim_missing".into())
}
fn require_claim_state(db:&Connection,actor:&CoordinatorLease,child:&ScheduledChild,state:&str)->Result<(),String> {
    let ok:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_external_surface_captures WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND assignment_id=?4 AND child_run_id=?5 AND state=?6 AND (?6<>'claimed' OR (artifact_id='' AND response_json='{}' AND response_hash='')))",params![actor.scan_id,actor.attempt_number,actor.target_key,child.assignment_id,child.run_id,state],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !ok {return Err("public_surface_response_original_claim_changed".into());}Ok(())
}
fn fee_rows(db:&Connection,actor:&CoordinatorLease,child:&ScheduledChild,worker:&str,received:bool)->Result<Vec<Vec<SqlValue>>,String> {
    let source=format!("public:{}",child.assignment_id);
    let rows=rows(db,"SELECT kind,amount,idempotency_key FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND source_id=?4 ORDER BY kind",&[&actor.root_run_id,&child.assignment_id,worker,&source])?;
    let mut expected=vec![vec![SqlValue::Text("forfeit".into()),SqlValue::Integer(1),SqlValue::Text(format!("target:{source}:dispatch"))],vec![SqlValue::Text("reserve".into()),SqlValue::Integer(1),SqlValue::Text(format!("target:{source}:reserve"))]];
    if received {expected.insert(1,vec![SqlValue::Text("reconcile".into()),SqlValue::Integer(1),SqlValue::Text(format!("target:{source}:receipt"))]);}
    let unrelated:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE source_id=?1 AND (root_run_id<>?2 OR assignment_id<>?3 OR lease_attempt_id<>?4 OR dimension<>'target_requests'))",params![source,actor.root_run_id,child.assignment_id,worker],|r|r.get(0)).map_err(|e|e.to_string())?;
    if unrelated || rows!=expected {return Err("public_surface_response_original_fees_changed".into());}
    self::rows(db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND source_id=?4 ORDER BY kind",&[&actor.root_run_id,&child.assignment_id,worker,&source])
}
fn verify_artifact(directory:&Path,fact:&Value)->Result<(),String> {
    let artifact=fact["artifactId"].as_str().ok_or("public_surface_response_original_artifact_changed")?;
    let payload=&fact["response"]["capture"];
    let bound=json!({"recordSha256":fact["recordSha256"],"bodySha256":fact["bodySha256"],"requestId":payload["requestId"],"status":payload["status"]});
    if !super::verified_web_http_review_fact(directory,&[format!("{}/{artifact}",super::AGENT_HTTP_DIRECTORY)],&bound) {return Err("public_surface_response_original_artifact_changed".into());}
    let handle=super::open_agent_artifact_directory(&directory.join(super::AGENT_HTTP_DIRECTORY)).ok_or("public_surface_response_original_artifact_changed")?;
    let (bytes,_)=super::read_agent_artifact_file(&handle,artifact,1048576).ok_or("public_surface_response_original_artifact_changed")?;
    let record:Value=serde_json::from_slice(&bytes).map_err(|_|"public_surface_response_original_artifact_changed")?;
    if bytes.len() as u64!=fact["recordBytes"].as_u64().ok_or("public_surface_response_original_artifact_changed")? || record["response"]!=*payload || record["request"]!=json!({"url":fact["contract"]["url"],"method":"GET","identity":"anonymous","headerNames":[],"contractKey":null,"bodyBytes":null}) || record["payloadBytes"]!=fact["bodyBytes"] || payload["bodyBytes"]!=fact["bodyBytes"] || payload["bodySha256"]!=fact["bodySha256"] || payload["identity"]!="anonymous" || payload["method"]!="GET" || payload["redirectFollowed"]!=false || payload["confirmedFinding"]!=false {return Err("public_surface_response_original_artifact_changed".into());}Ok(())
}
fn encoded(value:&Value)->Result<Vec<u8>,String> {
    let canonical=serde_json::to_string(value).map_err(|e|e.to_string())?;
    Ok(format!("{DOMAIN}:{canonical}").into_bytes())
}
fn rows(db:&Connection,sql:&str,keys:&[&str])->Result<Vec<Vec<SqlValue>>,String> {
    let mut q=db.prepare(sql).map_err(|e|e.to_string())?;let count=q.column_count();
    let result=q.query_map(rusqlite::params_from_iter(keys.iter()),|r|(0..count).map(|i|r.get(i)).collect::<rusqlite::Result<Vec<SqlValue>>>()).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string());
    result
}
pub(crate) fn verify_schema_on(db:&Connection)->Result<(),String> {
    let expected=Connection::open_in_memory().map_err(|e|e.to_string())?;expected.execute_batch(SCHEMA_SQL).map_err(|e|e.to_string())?;
    let sql="SELECT type,name,sql FROM main.sqlite_master WHERE tbl_name='agent_public_surface_response_receipts' ORDER BY type,name";
    let shadow:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE tbl_name='agent_public_surface_response_receipts' OR name LIKE 'public_surface_response_%')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if shadow || rows(db,sql,&[])?!=rows(&expected,sql,&[])? {return Err("public_surface_response_schema_conflict".into());}Ok(())
}

#[derive(Clone, Copy)]
enum ResponseWritePhase { Claim, Receive }

// The only write through a trigger is the original immutable graph revision
// registration. Its name alone is insufficient: authenticate the stored DDL
// against SQLite's own normalization of the original migration statement.
fn verify_original_revision_trigger_on(db:&Connection)->Result<(),String> {
    const NAME:&str="agent_evidence_nodes_revision_insert";
    let temporary:bool=db.query_row(
        "SELECT EXISTS(SELECT 1 FROM temp.sqlite_master WHERE type='trigger' AND name=?1)",
        [NAME],|row|row.get(0)).map_err(|e|e.to_string())?;
    if temporary { return Err("public_surface_revision_trigger_temporary_shadow".into()); }
    let reference=Connection::open_in_memory().map_err(|e|e.to_string())?;
    reference.execute_batch("CREATE TABLE agent_evidence_nodes(id TEXT,root_run_id TEXT,revision INTEGER);
             CREATE TABLE agent_evidence_revisions(root_run_id TEXT,revision INTEGER,parent_revision INTEGER,cause_event_id TEXT,manifest_hash TEXT);
             CREATE TRIGGER IF NOT EXISTS agent_evidence_nodes_revision_insert
             AFTER INSERT ON agent_evidence_nodes
             BEGIN
                 INSERT OR IGNORE INTO agent_evidence_revisions
                     (root_run_id,revision,parent_revision,cause_event_id,manifest_hash)
                 VALUES(NEW.root_run_id,NEW.revision,
                     (SELECT MAX(revision) FROM agent_evidence_revisions
                      WHERE root_run_id=NEW.root_run_id AND revision<NEW.revision),
                     'node:' || NEW.id,'');
             END;").map_err(|e|e.to_string())?;
    let expected:String=reference.query_row(
        "SELECT sql FROM main.sqlite_master WHERE type='trigger' AND name=?1",[NAME],|row|row.get(0))
        .map_err(|e|e.to_string())?;
    let actual:Option<String>=db.query_row(
        "SELECT sql FROM main.sqlite_master WHERE type='trigger' AND name=?1 AND tbl_name='agent_evidence_nodes'",
        [NAME],|row|row.get(0)).optional().map_err(|e|e.to_string())?;
    if actual.as_deref()!=Some(expected.as_str()) {
        return Err("public_surface_revision_trigger_original_ddl_invalid".into());
    }
    Ok(())
}

fn with_savepoint<T>(tx:&Transaction<'_>, phase:ResponseWritePhase, work:impl FnOnce()->Result<T,String>)->Result<T,String> {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let name=format!("public_surface_receipt_{}",uuid::Uuid::new_v4().simple());
    tx.execute_batch(&format!("SAVEPOINT {name}")).map_err(|e|e.to_string())?;
    let verified=if matches!(phase,ResponseWritePhase::Receive) {
        verify_original_revision_trigger_on(tx)
    } else { Ok(()) };
    let installed = verified.and_then(|()|tx.authorizer(Some(move |a:AuthContext<'_>| {
        let direct=a.database_name==Some("main") && a.accessor.is_none();
        let allowed=match a.action {
            AuthAction::Insert { table_name } => (matches!(phase,ResponseWritePhase::Receive)
                && a.database_name==Some("main")
                && a.accessor==Some("agent_evidence_nodes_revision_insert")
                && table_name=="agent_evidence_revisions") || (direct && match phase {
                ResponseWritePhase::Claim => matches!(table_name,"agent_external_surface_captures"|"agent_budget_entries"),
                ResponseWritePhase::Receive => matches!(table_name,"agent_budget_entries"|"agent_evidence_nodes"|"agent_public_surface_response_receipts"),
            }),
            AuthAction::Update { table_name, column_name } => direct
                && matches!(phase,ResponseWritePhase::Receive)
                && table_name=="agent_external_surface_captures"
                && matches!(column_name,"state"|"artifact_id"|"response_json"|"response_hash"),
            AuthAction::Read { .. } | AuthAction::Select | AuthAction::Function { .. }
                | AuthAction::Transaction { .. } | AuthAction::Savepoint { .. }
                | AuthAction::Recursive => true,
            _ => false,
        };
        if allowed { Authorization::Allow } else { Authorization::Deny }
    })).map_err(|e|format!("public_surface_response_writer_guard_unavailable:{e}")));
    let result=match installed { Ok(())=>work(), Err(error)=>Err(error) };
    // Clear before rollback: an injected write must never leave partial finance
    // or capture state, and an authorizer cleanup error cannot be discarded.
    let cleared=tx.authorizer(None::<fn(AuthContext<'_>)->Authorization>)
        .map_err(|e|format!("public_surface_response_writer_guard_clear_failed:{e}"));
    match (result,cleared) {
        (Ok(value),Ok(()))=>{
            tx.execute_batch(&format!("RELEASE SAVEPOINT {name}")).map_err(|e|e.to_string())?;
            Ok(value)
        },
        (result,clear)=>{
            let error=match (result,clear) {
                (Err(error),Ok(()))=>error,
                (Ok(_),Err(error))=>error,
                (Err(error),Err(clear))=>format!("{error}; {clear}"),
                (Ok(_),Ok(()))=>unreachable!(),
            };
            tx.execute_batch(&format!("ROLLBACK TO SAVEPOINT {name}; RELEASE SAVEPOINT {name}"))
                .map_err(|rollback|format!("{error}; public_surface_response_rollback_failed:{rollback}"))?;
            Err(error)
        }
    }
}

fn request_hash(contract:&Value,proxy_binding:&str)->String {
    store::stable_hash(&json!({"url":contract["url"],"method":"GET","identity":"anonymous",
        "followRedirects":false,"userAgent":"oviraptor-public-surface/1.0","cacheControl":"no-store",
        "authenticatedHeaders":[],"proxyBinding":proxy_binding}).to_string())
}
