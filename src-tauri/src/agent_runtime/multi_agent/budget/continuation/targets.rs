//! Original durable target claims and exact original financial receipts.
use super::super::{historical::CostOwner, root::RootOwner};
use rusqlite::{params, Connection};
use serde_json::Value;

pub(super) fn verify(db: &Connection, root: &str) -> Result<(), String> {
    let bad_scope:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_http_request_claims c JOIN agent_runs r ON r.id=c.run_id
        WHERE (r.id=?1 OR r.root_run_id=?1) AND (c.scan_id<>r.scan_id OR c.attempt_number<>r.attempt_number
          OR c.target_url<>r.target_url OR c.budget_attempt<1 OR c.ordinal<1 OR c.request_index<1 OR length(c.request_hash)<>64
          OR NOT EXISTS(SELECT 1 FROM tool_invocations i WHERE i.run_id=c.run_id
            AND i.invocation_id=c.invocation_id AND i.tool_name=c.tool_name AND i.policy_decision='allow')))",
        [root],|r|r.get(0)).map_err(|e|e.to_string())?;
    if bad_scope {
        return Err("continuation_original_target_owner_conflict".into());
    }
    let mut q=db.prepare("SELECT c.run_id,c.invocation_id,c.request_index,c.request_hash,c.response_status,c.received_at
        FROM agent_http_request_claims c JOIN agent_runs r ON r.id=c.run_id
        WHERE r.id=?1 OR r.root_run_id=?1 ORDER BY c.rowid").map_err(|e|e.to_string())?;
    let claims = q
        .query_map([root], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut sources = std::collections::BTreeSet::new();
    for (run, invocation, index, hash, status, received) in claims {
        if !(100..=599).contains(&status) || received.is_empty() {
            return Err("continuation_original_target_receipt_required".into());
        }
        let (assignment, attempt) = original_owner(db, root, &run)?;
        let source = format!("http:{run}:{invocation}:{index}:{hash}");
        sources.insert((assignment.clone(), attempt.clone(), source.clone()));
        for (kind, suffix) in [
            ("reserve", "reserve"),
            ("forfeit", "dispatch"),
            ("reconcile", "receipt"),
        ] {
            let key = super::super::scope::stored_key(
                db,
                root,
                &assignment,
                &attempt,
                &format!("target:{source}:{suffix}"),
            )?;
            let exact: bool = db
                .query_row(
                    "SELECT count(*)=1 FROM agent_budget_entries WHERE root_run_id=?1
              AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='target_requests'
              AND kind=?4 AND amount=1 AND source_id=?5 AND idempotency_key=?6",
                    params![root, assignment, attempt, kind, source, key],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !exact {
                return Err("continuation_original_target_cost_conflict".into());
            }
        }
    }
    for (run, assignment, source) in public_receipts(db, root)? {
        let (bound, attempt) = original_owner(db, root, &run)?;
        if bound != assignment {
            return Err("continuation_original_target_owner_conflict".into());
        }
        for (kind, suffix) in [
            ("reserve", "reserve"),
            ("forfeit", "dispatch"),
            ("reconcile", "receipt"),
        ] {
            let key = super::super::scope::stored_key(
                db,
                root,
                &assignment,
                &attempt,
                &format!("target:{source}:{suffix}"),
            )?;
            let exact: bool = db
                .query_row(
                    "SELECT count(*)=1 FROM agent_budget_entries WHERE root_run_id=?1
                AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='target_requests'
                AND kind=?4 AND amount=1 AND source_id=?5 AND idempotency_key=?6",
                    params![root, assignment, attempt, kind, source, key],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !exact {
                return Err("continuation_original_target_cost_conflict".into());
            }
        }
        sources.insert((assignment, attempt, source));
    }
    let authorization: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_authorization_probe_claims c
        JOIN agent_runs r ON r.id=c.child_run_id WHERE r.root_run_id=?1)",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if authorization {
        return Err("continuation_authorization_original_receipt_not_supported".into());
    }
    let mut entries = db
        .prepare(
            "SELECT assignment_id,lease_attempt_id,source_id,kind,amount,idempotency_key
        FROM agent_budget_entries WHERE root_run_id=?1 AND dimension='target_requests'",
        )
        .map_err(|e| e.to_string())?;
    let rows = entries
        .query_map([root], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (assignment, attempt, source, kind, amount, key) = row.map_err(|e| e.to_string())?;
        let scope = (assignment, attempt, source);
        if !sources.contains(&scope) || amount != 1 {
            return Err("continuation_original_target_cost_conflict".into());
        }
        let suffix = match kind.as_str() {
            "reserve" => "reserve",
            "forfeit" => "dispatch",
            "reconcile" => "receipt",
            _ => return Err("continuation_original_target_cost_conflict".into()),
        };
        let expected = super::super::scope::stored_key(db, root, &scope.0, &scope.1,
            &format!("target:{}:{suffix}", scope.2))?;
        if key != expected {
            return Err("continuation_original_target_cost_conflict".into());
        }
    }
    Ok(())
}

fn original_owner(db: &Connection, root: &str, run: &str) -> Result<(String, String), String> {
    if let Some(owner) = CostOwner::for_run(db, run)? {
        owner.verify(db)?;
        if owner.lease.root_run_id != root {
            return Err("continuation_original_target_owner_conflict".into());
        }
        Ok((owner.assignment, owner.attempt.id))
    } else {
        if run != root {
            return Err("continuation_original_target_owner_conflict".into());
        }
        Ok((String::new(), RootOwner::load_single(db, root)?.id))
    }
}

fn public_receipts(db: &Connection, root: &str) -> Result<Vec<(String, String, String)>, String> {
    let mut q=db.prepare("SELECT c.child_run_id,c.assignment_id,c.state,c.artifact_id,c.response_json,c.response_hash,
        r.role,a.task_slice_json,c.scan_id,c.attempt_number,c.target_url,r.scan_id,r.attempt_number,r.target_url
        FROM agent_external_surface_captures c JOIN agent_runs r ON r.id=c.child_run_id
        JOIN agent_assignments a ON a.id=c.assignment_id AND a.child_run_id=r.id
        WHERE r.root_run_id=?1 AND a.coordinator_run_id=?1 ORDER BY c.rowid").map_err(|e|e.to_string())?;
    let rows = q
        .query_map([root], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, i64>(9)?,
                r.get::<_, String>(10)?,
                r.get::<_, String>(11)?,
                r.get::<_, i64>(12)?,
                r.get::<_, String>(13)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for (
        run,
        assignment,
        state,
        artifact,
        raw,
        hash,
        role,
        slice,
        scan,
        attempt,
        target,
        rscan,
        rattempt,
        rtarget,
    ) in rows
    {
        let v: Value = serde_json::from_str(&raw)
            .map_err(|_| "continuation_original_public_receipt_invalid")?;
        let canonical = v.to_string();
        let contract: Value = serde_json::from_str(&slice)
            .map_err(|_| "continuation_original_public_contract_invalid")?;
        let view = &v["capture"];
        let mut url = reqwest::Url::parse(&target)
            .map_err(|_| "continuation_original_public_contract_invalid")?;
        url.set_fragment(None);
        let slot = artifact
            .strip_suffix(".json")
            .ok_or("continuation_original_public_receipt_invalid")?;
        if state != "received"
            || role != "external_surface"
            || scan != rscan
            || attempt != rattempt
            || target != rtarget
            || artifact.is_empty()
            || slot.len() < 4
            || !slot.bytes().all(|b| b.is_ascii_digit())
            || canonical != raw
            || crate::agent_runtime::store::stable_hash(&raw) != hash
            || v.as_object().is_none_or(|o| o.len() != 3)
            || v["artifactId"] != artifact
            || v["targetRequests"] != 1
            || view["method"] != "GET"
            || view["identity"] != "anonymous"
            || view["redirectFollowed"] != false
            || view["status"]
                .as_i64()
                .is_none_or(|n| !(100..=599).contains(&n))
            || view["requestId"] != format!("public-surface:{assignment}")
            || view["url"] != contract["url"]
            || contract["url"] != url.as_str()
            || contract.as_object().is_none_or(|o| o.len() != 6)
            || contract["method"] != "GET"
            || contract["anonymousOnly"] != true
            || contract["maxRequests"] != 1
            || contract["followRedirects"] != false
            || contract["schemaVersion"] != 1
        {
            return Err("continuation_original_public_receipt_invalid".into());
        }
        let key = crate::agent_runtime::evidence_graph::store::evidence_natural_key(
            root,
            crate::agent_runtime::evidence_graph::contract::EvidenceNodeKind::RequestRecord,
            &format!("{run}:{artifact}"),
        );
        let refs = serde_json::json!([format!("agent-http/{artifact}")]).to_string();
        let fact:String=db.query_row("SELECT payload_json FROM agent_evidence_nodes WHERE root_run_id=?1
            AND created_by_run_id=?2 AND natural_key_hash=?3 AND revision=1 AND kind='request_record'
            AND provenance='observed' AND artifact_refs_json=?4",params![root,run,key,refs],|r|r.get(0))
            .map_err(|_|"continuation_original_public_fact_required")?;
        let node: Value =
            serde_json::from_str(&fact).map_err(|_| "continuation_original_public_fact_invalid")?;
        let canonical_node = node.to_string();
        if canonical_node != fact
            || node.as_object().is_none_or(|o| o.len() != 4)
            || node["requestId"] != view["requestId"]
            || node["status"] != view["status"]
            || node["bodySha256"] != view["bodySha256"]
            || !valid_hash(&node["bodySha256"])
            || !valid_hash(&node["recordSha256"])
        {
            return Err("continuation_original_public_fact_invalid".into());
        }
        result.push((run, assignment.clone(), format!("public:{assignment}")));
    }
    Ok(result)
}

fn valid_hash(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
