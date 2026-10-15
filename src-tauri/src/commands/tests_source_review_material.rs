#[test]
fn source_review_material_preserves_accepted_analyzer_identity_and_origins() {
    use crate::agent_runtime::multi_agent::source_review;
    let (root,connection,record,_)=source_result_identical_engine_fixture();
    let lease=source_specialist_fixture_lease(&connection,&record);
    assert_eq!(source_review::capture(&connection,&lease,&[]).unwrap_err(),"source_review_material_transaction_required");
    let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
    let captured=source_review::capture(&tx,&lease,&[]).unwrap();
    let material=&captured["material"];
    assert_eq!(captured["digest"],crate::artifact_import::canonical::sha256_hex(material.to_string().as_bytes()));
    let candidates=material["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(),1);
    assert_eq!(candidates[0]["identity"]["kind"],"analyzer_revision");
    assert!(candidates[0]["identity"]["revisionHash"].as_str().is_some_and(|s|!s.is_empty()));
    assert!(candidates[0]["identity"].get("revision").is_none(),"importer hashes are not graph revision numbers");
    assert_eq!(candidates[0]["acceptedSources"].as_array().unwrap().iter().map(|s|s["engine"].as_str().unwrap()).collect::<std::collections::BTreeSet<_>>(),
        ["codeql","semgrep"].into_iter().collect());
    assert_eq!(material["independentReviewCompleted"],false);
    assert_eq!(material["hostActionsGranted"],0);
    assert_eq!(material["targetRequestsGranted"],0);
    assert_eq!(material["graphRevision"],0);
    assert_eq!(source_review::capture(&tx,&lease,&[]).unwrap(),captured);
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_assignments",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    tx.rollback().unwrap();
    drop(connection);fs::remove_dir_all(root).unwrap();
}

fn source_review_material_execution_fixture(damage:Option<&str>) -> (PathBuf,rusqlite::Connection,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,Result<JsonValue,String>) {
    source_review_material_execution_with_candidate(damage,json!({"title":"Candidate from actual source tool","rationale":"A suspicion, not a confirmed finding","path":"app.py","line":1}))
}

fn source_review_material_execution_with_candidate(damage:Option<&str>,candidate:JsonValue) -> (PathBuf,rusqlite::Connection,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,Result<JsonValue,String>) {
    let (port,seen,stop)=crate::scanning::agent_tests::spawn_endpoint(std::sync::Arc::new(move |request| {
        let body:JsonValue=serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let message=if body.get("tools").is_none() {json!({"role":"assistant","content":"Initial source assessment, not a review"})}
        else {
            let finished=body["messages"].as_array().unwrap().last().unwrap()["role"]=="tool";
            let analyst=body["tools"].as_array().unwrap().iter().any(|t|t["function"]["name"]=="evidence.submit_candidate");
            let (name,args)=if finished {("assignment.finish",json!({"summary":"Source work complete; awaiting independent review","gaps":[]}))}
                else if analyst {("evidence.submit_candidate",candidate.clone())}
                else {("repo.inventory",json!({}))};
            json!({"role":"assistant","tool_calls":[{"id":name,"type":"function","function":{"name":name,"arguments":args.to_string()}}]})
        };
        (200,"application/json",json!({"choices":[{"message":message,"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
    }));
    let (root,connection,record,mut lease)=source_specialist_true_born_model_fixture(&source_specialist_test_environment(port));
    if let Some(body)=damage {
        connection.execute_batch(&format!("CREATE TRIGGER source_material_closure_damage AFTER UPDATE ON agent_runs
            WHEN NEW.role='coordinator' AND NEW.terminal_state='completed_with_gaps' AND OLD.status<>'terminal'
            BEGIN {body} END;")).unwrap();
    }
    let result=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
    assert_eq!(seen.lock().unwrap().len(),6,"must use real model and tool rounds, without replaying them");
    let (epoch,fence)=connection.query_row("SELECT lease_epoch,fencing_token FROM agent_coordinator_leases WHERE root_run_id=?1",
        [&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    lease.lease_epoch=epoch;lease.fencing_token=fence;
    (root,connection,lease,result)
}

#[test]
fn source_review_material_preserves_broker_v1_identity_and_optional_arguments() {
    for args in [
        json!({"title":"  Candidate with whitespace  ","rationale":"  Needs review  ","path":"app.py","line":1}),
        json!({"title":"Named candidate","rationale":"Needs review","path":"app.py","line":1,
            "rule":"rule-1","severity":"high","cwe":"CWE-89","sourceAnalyzer":"model"}),
    ] {
        let (root,connection,lease,result)=source_review_material_execution_with_candidate(None,args.clone());
        assert!(result.is_ok(),"{result:?}");
        let node:(String,String,String)=connection.query_row("SELECT id,natural_key_hash,payload_json FROM agent_evidence_nodes WHERE kind='candidate_finding'",
            [],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        let payload:JsonValue=serde_json::from_str(&node.2).unwrap();
        // Pin the pre-existing on-disk identity independently of the shared
        // implementation, so a change to both producer and verifier fails.
        let legacy_identity=json!([lease.root_run_id,lease.scan_id,lease.attempt_number,
            payload["analysisManifestDigest"],"app.py",1,
            args["title"].as_str().unwrap().trim(),args["rationale"].as_str().unwrap().trim(),
            args.get("rule"),args.get("severity"),args.get("cwe"),args.get("sourceAnalyzer")]).to_string();
        assert_eq!(node.0,format!("cand-{}",&crate::agent_runtime::store::stable_hash(&legacy_identity)[..16]));
        assert_eq!(node.1,crate::agent_runtime::evidence_graph::store::evidence_natural_key(
            &lease.root_run_id,crate::agent_runtime::evidence_graph::contract::EvidenceNodeKind::CandidateFinding,&legacy_identity));
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

fn source_review_material_completion_bases(connection:&rusqlite::Connection)->Vec<(crate::agent_runtime::contract::AgentRole,JsonValue)> {
    [crate::agent_runtime::contract::AgentRole::RepoMapper,crate::agent_runtime::contract::AgentRole::SourceAnalyst].into_iter().map(|role| {
        let text:String=connection.query_row("SELECT task_slice_json FROM agent_assignments WHERE role=?1 AND json_extract(task_slice_json,'$.phase') IS NULL",
            [role.as_str()],|r|r.get(0)).unwrap();
        (role,serde_json::from_str(&text).unwrap())
    }).collect()
}

#[test]
fn source_review_material_real_execution_binds_graph_author_revision_and_receipts() {
    let (root,connection,lease,result)=source_review_material_execution_fixture(None);
    let result=result.unwrap();
    assert_eq!(result["status"],"source_analysis_completed_unreviewed");
    let closure:String=connection.query_row("SELECT payload_json FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",
        [&lease.root_run_id],|r|r.get(0)).unwrap();
    let closure:JsonValue=serde_json::from_str(&closure).unwrap();
    let captured=&closure["reviewMaterial"];
    let material=&captured["material"];
    assert_eq!(captured["digest"],crate::artifact_import::canonical::sha256_hex(material.to_string().as_bytes()));
    let candidates=material["candidates"].as_array().unwrap();
    assert!(candidates.iter().any(|c|c["identity"]["kind"]=="analyzer_revision"));
    let graph=candidates.iter().filter(|c|c["identity"]["kind"]=="graph_candidate").collect::<Vec<_>>();
    assert_eq!(graph.len(),1);
    let graph=graph[0];
    assert_eq!(graph["identity"]["revision"],1);
    assert_eq!(graph["payload"]["reviewState"],"candidate");
    assert_eq!(material["graphRevision"],1);
    let tool=material["toolEvidence"].as_array().unwrap().iter().find(|t|t["call"]["name"]=="evidence.submit_candidate").unwrap();
    assert_eq!(graph["authorRunId"],tool["authorRunId"]);
    assert_eq!(graph["identity"]["nodeId"],tool["output"]["id"]);
    assert_eq!(graph["toolEvidenceHashes"],json!([crate::artifact_import::canonical::sha256_hex(tool.to_string().as_bytes())]));
    assert_eq!(material["independentReviewCompleted"],false);
    let bases=source_review_material_completion_bases(&connection);
    for (label,mutation) in [
        ("unchanged","SELECT 1;"),
        ("lost_node","DELETE FROM agent_evidence_nodes;"),
        ("lost_revision","DELETE FROM agent_evidence_revisions;"),
        ("changed_payload","UPDATE agent_evidence_nodes SET payload_json=json_set(payload_json,'$.rationale','changed');"),
        ("changed_author","UPDATE agent_evidence_nodes SET created_by_run_id=root_run_id;"),
        ("changed_revision","UPDATE agent_evidence_nodes SET revision=2;"),
        ("changed_tree","UPDATE agent_evidence_nodes SET artifact_refs_json='[]';"),
        ("changed_natural_key","UPDATE agent_evidence_nodes SET natural_key_hash='changed';"),
        ("changed_provenance","UPDATE agent_evidence_nodes SET provenance='inferred';"),
        ("self_confirmation","UPDATE agent_evidence_nodes SET payload_json=json_set(payload_json,'$.reviewState','confirmed');"),
        ("lost_tool_receipt","DELETE FROM agent_source_tool_receipts WHERE tool_name='evidence.submit_candidate';"),
        ("lost_tool_event","DELETE FROM agent_events WHERE event_type='tool_invocation_completed' AND json_extract(payload_json,'$.call.name')='evidence.submit_candidate';"),
    ] {
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
        tx.execute_batch(mutation).unwrap();
        let audited=source_assessment_completion(&tx,&lease,&bases);
        assert_eq!(audited.is_ok(),label=="unchanged","{label}: {audited:?}");
        if let Ok(proof)=audited {assert_eq!(&proof.review_material,captured);}
        tx.rollback().unwrap();
    }
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_review_material_terminal_damage_rolls_back_without_replaying_calls() {
    for damage in [
        "DELETE FROM agent_evidence_nodes;",
        "UPDATE agent_evidence_nodes SET payload_json=json_set(payload_json,'$.rationale','changed');",
        "UPDATE agent_evidence_nodes SET created_by_run_id=root_run_id;",
    ] {
        let (root,connection,lease,result)=source_review_material_execution_fixture(Some(damage));
        assert!(result.is_err(),"{damage}: {result:?}");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",
            [&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_evidence_nodes WHERE json_extract(payload_json,'$.rationale')='A suspicion, not a confirmed finding' AND created_by_run_id<>root_run_id",
            [],|r|r.get::<_,i64>(0)).unwrap(),1,"damage must roll back");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE acknowledged_at<>''",[],|r|r.get::<_,i64>(0)).unwrap(),4);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}
