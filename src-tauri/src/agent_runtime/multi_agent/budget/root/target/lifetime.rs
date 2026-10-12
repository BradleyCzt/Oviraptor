//! Original Single HTTP invocation must have returned, including body/evidence.
use super::*;
use crate::agent_runtime::execution_owner;
pub(crate) const INVOCATION_KIND:&str="single-target-http";

pub(crate) fn require_idle_original(
    db:&Connection, owner:&RootOwner,
)->Result<Vec<execution_owner::NativeInvocationOwner>,String> {
    if db.is_autocommit() {return Err("single_target_lifetime_transaction_required".into());}
    let path=db.path().filter(|p|!p.is_empty()).ok_or("single_target_lifetime_database_missing")?;
    let original=RootOwner::load_single(db,&owner.root)?;
    if original.id!=owner.id || original.contract!=owner.contract {return Err("budget_root_original_owner_conflict".into());}
    let mut q=db.prepare("SELECT scan_id,attempt_number,target_url,budget_attempt,ordinal,invocation_id,request_index,
        request_hash,tool_name,identity_handle,response_status,received_at FROM agent_http_request_claims WHERE run_id=?1 ORDER BY rowid").map_err(|e|e.to_string())?;
    let rows=q.query_map([&owner.root],|r|Ok((RootTargetRequest {scan:r.get(0)?,attempt:r.get(1)?,target:r.get(2)?,
        budget_attempt:r.get(3)?,ordinal:r.get(4)?,invocation:r.get(5)?,request_index:r.get(6)?,request_hash:r.get(7)?},
        r.get::<_,String>(8)?,r.get::<_,String>(9)?,r.get::<_,i64>(10)?,r.get::<_,String>(11)?)))
        .map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let mut guards=vec![];let scope=&owner.contract["root"];
    for (request,tool,identity,status,received) in rows {
        if scope["scan"]!=request.scan || scope["attempt"]!=request.attempt || scope["target"]!=request.target
            || request.budget_attempt<=0 || request.ordinal<=0 || request.request_index<=0 || request.invocation.is_empty()
            || request.request_hash.len()!=64 || !request.request_hash.bytes().all(|b|b.is_ascii_hexdigit())
            || (status==0)!=received.is_empty() || (status!=0 && !(100..=599).contains(&status)) || identity.is_empty() {
            return Err("single_target_lifetime_original_call_conflict".into());
        }
        let bound:bool=db.query_row("SELECT count(*)=1 FROM tool_invocations WHERE run_id=?1 AND invocation_id=?2
            AND tool_name=?3 AND policy_decision='allow' AND status IN ('running','completed','failed','cancelled')",
            params![owner.root,request.invocation,tool],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !bound {return Err("single_target_lifetime_original_invocation_conflict".into());}
        let source=format!("http:{}:{}:{}:{}",owner.root,request.invocation,request.request_index,request.request_hash);
        let call=RootTargetCall {owner:owner.clone(),request,source,original:vec![]};
        call.verify_dispatch(db)?;
        let guard=execution_owner::probe_native_invocation(std::path::Path::new(path),&call.request.scan,
            call.request.attempt,INVOCATION_KIND,&call.source).map_err(|e|format!("single_target_transport_not_idle:{e}"))?
            .ok_or("single_target_transport_original_exit_proof_missing")?;
        guards.push(guard);
    }
    // Empty original HTTP history has no inode to manufacture.
    Ok(guards)
}
