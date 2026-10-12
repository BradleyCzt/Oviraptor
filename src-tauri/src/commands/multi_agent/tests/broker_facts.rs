#[test]
fn brokered_http_record_becomes_one_attributable_review_fact_only_while_executor_is_live() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (root, path, root_run_id, lease) = multi_agent_test_root("http-graph", 100, 10);
    let connection = db::open(&path).unwrap();
    let child = scheduler::schedule_child(
        &connection, &lease, AgentRole::WebExecutor, AgentLane::TargetTouching,
        "http-graph", &serde_json::json!({}), 2, &["replay_http".into()], 10, 1,
    ).unwrap();
    scheduler::mark_child_running(&connection, &lease, &child).unwrap();
    let mut context = test_context(&path, &lease.target_key, vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.run = Some(AgentRunLedger { db_path: path.clone(), run_id: child.run_id.clone() });
    let view = serde_json::json!({
        "requestId":"req-0001", "status":200,
        "bodySha256":format!("{:x}", Sha256::digest(b"safe body")), "redaction":{"applied":true},
    });
    let artifact = agent_write_http_record(&context, 1,
        &serde_json::json!({"method":"GET","url":lease.target_key,"identity":"anonymous"}),
        &view, b"safe body",
    ).unwrap();
    let record_fact = || agent_record_http_observation(&context, "replay_http", &artifact, &view, "req-0001");
    record_fact().unwrap();
    record_fact().unwrap();
    let nodes: (i64, String, i64, String, String) = connection.query_row(
        "SELECT COUNT(*),MAX(root_run_id),MAX(revision),MAX(created_by_run_id),MAX(artifact_refs_json) \
         FROM agent_evidence_nodes WHERE kind='request_record'", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    ).unwrap();
    assert_eq!(nodes.0, 1);
    assert_eq!(nodes.1, root_run_id);
    assert_eq!(nodes.2, 2);
    assert_eq!(nodes.3, child.run_id);
    assert_eq!(nodes.4, r#"["agent-http/0001.json"]"#);
    assert_eq!(review_fact_refs(&connection, &root_run_id, &context.target_dir).unwrap().len(), 1);
    // Even a valid on-disk Broker record must not be attributed to a forged
    // graph revision, a different assignment revision or a fabricated node
    // identity. The Reviewer must freeze the exact child/assignment fact.
    connection.execute("UPDATE agent_evidence_nodes SET revision=3 WHERE kind='request_record'", []).unwrap();
    assert!(review_fact_refs(&connection, &root_run_id, &context.target_dir).unwrap().is_empty());
    connection.execute("UPDATE agent_evidence_nodes SET revision=2 WHERE kind='request_record'", []).unwrap();
    connection.execute("UPDATE agent_evidence_nodes SET natural_key_hash='forged' WHERE kind='request_record'", []).unwrap();
    assert!(review_fact_refs(&connection, &root_run_id, &context.target_dir).unwrap().is_empty());
    connection.execute("UPDATE agent_evidence_nodes SET natural_key_hash=?1 WHERE kind='request_record'", [&crate::agent_runtime::evidence_graph::store::evidence_natural_key(
        &root_run_id, crate::agent_runtime::evidence_graph::contract::EvidenceNodeKind::RequestRecord,
        &format!("{}:{artifact}", child.run_id),
    )]).unwrap();
    assert_eq!(review_fact_refs(&connection, &root_run_id, &context.target_dir).unwrap().len(), 1);
    let body_path = context.target_dir.join("agent-http").join("0001.body");
    std::fs::write(&body_path, b"fake body").unwrap();
    assert!(review_fact_refs(&connection, &root_run_id, &context.target_dir).unwrap().is_empty(),
        "a same-length body replacement cannot keep a verified review fact");
    std::fs::write(&body_path, b"safe body").unwrap();
    assert_eq!(review_fact_refs(&connection, &root_run_id, &context.target_dir).unwrap().len(), 1);
    #[cfg(unix)]
    {
        let backup = body_path.with_extension("body-backup");
        std::fs::rename(&body_path, &backup).unwrap();
        std::fs::hard_link(&backup, &body_path).unwrap();
        assert!(review_fact_refs(&connection, &root_run_id, &context.target_dir).unwrap().is_empty(),
            "a matching hardlinked body is not an isolated Broker artifact");
        assert_eq!(record_fact().unwrap_err(), "http_observation_payload_missing");
        std::fs::remove_file(&body_path).unwrap();
        std::fs::rename(&backup, &body_path).unwrap();
        let directory = context.target_dir.join("agent-http");
        let backup = context.target_dir.join("agent-http-backup");
        std::fs::rename(&directory, &backup).unwrap();
        std::os::unix::fs::symlink(&backup, &directory).unwrap();
        assert!(review_fact_refs(&connection, &root_run_id, &context.target_dir).unwrap().is_empty(),
            "a redirected artifact directory must not become a Reviewer fact");
        assert_eq!(record_fact().unwrap_err(), "http_observation_artifact_missing");
        std::fs::remove_file(&directory).unwrap();
        std::fs::rename(&backup, &directory).unwrap();
        assert_eq!(review_fact_refs(&connection, &root_run_id, &context.target_dir).unwrap().len(), 1);
    }

    let missing = agent_record_http_observation(&context, "replay_http", "9999.json", &view, "req-0001");
    assert_eq!(missing.unwrap_err(), "http_observation_artifact_missing");
    let mismatch = agent_record_http_observation(&context, "replay_http", &artifact, &view, "req-fake");
    assert_eq!(mismatch.unwrap_err(), "http_observation_record_invalid");
    let fake_view = serde_json::json!({"requestId":"req-0001","status":201,"bodySha256":view["bodySha256"]});
    assert_eq!(agent_record_http_observation(&context, "replay_http", &artifact, &fake_view, "req-0001").unwrap_err(),
        "http_observation_artifact_mismatch");
    let fake_hash_view = serde_json::json!({"requestId":"req-0001","status":200,"bodySha256":"a".repeat(64)});
    assert_eq!(agent_record_http_observation(&context, "replay_http", &artifact, &fake_hash_view, "req-0001").unwrap_err(),
        "http_observation_payload_mismatch");
    // Capability/fencing loss is decided by the one Broker authority gate that
    // also guards dispatch and the send boundary, so the producer reports the
    // canonical authority code (an execution-authorization stop), never an
    // evidence-write failure. Observation-only conditions keep their own code.
    let authority_stop = |code: &str| agent_tool_terminal_outcome(&agent_http_claim_error(code.to_string()))
        .map(|outcome| outcome.terminal_code().to_string());
    assert_eq!(agent_record_http_observation(&context, "browser_action", &artifact, &view, "req-0001").unwrap_err(),
        "tool_capability_or_fencing_denied");
    assert_eq!(authority_stop("tool_capability_or_fencing_denied"), Some(terminal_code::EXECUTION_AUTHORIZATION_DENIED.to_string()));
    connection.execute("UPDATE agent_assignments SET evidence_revision=0 WHERE id=?1", [&child.assignment_id]).unwrap();
    assert_eq!(record_fact().unwrap_err(), "http_observation_assignment_or_lease_denied");
    connection.execute("UPDATE agent_assignments SET evidence_revision=2 WHERE id=?1", [&child.assignment_id]).unwrap();
    connection.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE child_run_id=?1", [&child.run_id]).unwrap();
    assert_eq!(record_fact().unwrap_err(), "tool_capability_or_fencing_denied");
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM agent_evidence_nodes WHERE kind='request_record'", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 1);
    let binary_body = b"safe\xffbody";
    let binary_view = serde_json::json!({
        "requestId":"req-0002", "status":200,
        "bodySha256":format!("{:x}", Sha256::digest(binary_body)),
    });
    let binary_artifact = agent_write_http_record(&context, 2,
        &serde_json::json!({"method":"GET","url":lease.target_key,"identity":"anonymous"}),
        &binary_view, binary_body,
    ).unwrap();
    let binary_slot = binary_artifact.strip_suffix(".json").unwrap();
    assert_eq!(std::fs::read(context.target_dir.join("agent-http").join(format!("{binary_slot}.body"))).unwrap(), binary_body);
    std::fs::remove_file(context.target_dir.join("agent-http").join(&artifact)).unwrap();
    assert!(review_fact_refs(&connection, &root_run_id, &context.target_dir).unwrap().is_empty());
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(context.target_dir).unwrap();
}

#[test]
fn reviewer_fact_manifest_uses_only_verified_current_node_versions() {
    use crate::agent_runtime::{
        contract::{AgentBackendKind, AgentLane, AgentRole, AgentRunStatus, AssignmentState, MultiAgentPolicy},
        evidence_graph::{
            contract::{EvidenceNode, EvidenceNodeKind, EvidenceProvenance},
            store::{evidence_natural_key, insert_evidence_node},
        },
        multi_agent::{assignment::{insert_assignment, AgentAssignment}, scheduler},
        store::{self, AgentRunRow},
    };
    let (root, path, root_run_id, lease) = multi_agent_test_root("fact-supersession", 100, 10);
    let connection = db::open(&path).unwrap();
    let child = scheduler::schedule_child(
        &connection, &lease, AgentRole::WebExecutor, AgentLane::TargetTouching,
        "original-fact", &serde_json::json!({}), 2, &["replay_http".into()], 10, 1,
    ).unwrap();
    scheduler::mark_child_running(&connection, &lease, &child).unwrap();
    let mut context = test_context(&path, &lease.target_key, vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.run = Some(AgentRunLedger { db_path: path.clone(), run_id: child.run_id.clone() });
    let view = serde_json::json!({
        "requestId":"req-original", "status":200,
        "bodySha256":format!("{:x}", Sha256::digest(b"original")),
    });
    let artifact = agent_write_http_record(&context, 1,
        &serde_json::json!({"method":"GET","url":lease.target_key,"identity":"anonymous"}),
        &view, b"original",
    ).unwrap();
    agent_record_http_observation(&context, "replay_http", &artifact, &view, "req-original").unwrap();
    let original = review_fact_refs(&connection, &root_run_id, &context.target_dir).unwrap();
    assert_eq!(original.len(), 1);

    // A same-root inferred node must not retire a Broker observation, even if
    // it claims to supersede it. Reject it at the graph writer boundary.
    let inferred = EvidenceNode {
        id: "ev-inferred-successor".into(), root_run_id: root_run_id.clone(), revision: 3,
        kind: EvidenceNodeKind::RequestRecord, provenance: EvidenceProvenance::Inferred,
        natural_key_hash: store::stable_hash("inferred-successor"),
        payload: serde_json::json!({}), artifact_refs: Vec::new(),
        created_by_run_id: child.run_id.clone(), supersedes_id: original[0].clone(),
        created_at: String::new(),
    };
    assert!(insert_evidence_node(&connection, &inferred).unwrap_err()
        .contains("provenance_downgrade"));
    assert_eq!(review_fact_refs(&connection, &root_run_id, &context.target_dir).unwrap(), original);

    // Model the next, separately attributed revision without sending a second
    // request in this test. The fixture builds an independently revalidated
    // Broker-format artifact and child/assignment linkage, not a live scan.
    let next_run_id = "run-verified-successor";
    let next_assignment_id = "asg-verified-successor";
    let mut next_assignment = AgentAssignment::new(next_assignment_id, &root_run_id,
        AgentRole::WebExecutor, AgentLane::TargetTouching, &lease.target_key, "verified-successor");
    next_assignment.child_run_id = next_run_id.into();
    next_assignment.evidence_revision = 3;
    next_assignment.state = AssignmentState::Completed;
    insert_assignment(&connection, &next_assignment).unwrap();
    let mut next_run = AgentRunRow::new(next_run_id, &lease.scan_id, 1, &lease.target_key,
        AgentBackendKind::Native, AgentRole::WebExecutor, "plan", "evidence");
    next_run.root_run_id = root_run_id.clone();
    next_run.assignment_id = next_assignment_id.into();
    next_run.orchestration_policy = MultiAgentPolicy::Multi;
    next_run.lane = Some(AgentLane::TargetTouching);
    next_run.status = AgentRunStatus::Terminal;
    store::create_run(&connection, &next_run).unwrap();
    let next_view = serde_json::json!({
        "requestId":"req-successor", "status":200,
        "bodySha256":format!("{:x}", Sha256::digest(b"successor")),
    });
    let next_artifact = agent_write_http_record(&context, 2,
        &serde_json::json!({"method":"GET","url":lease.target_key,"identity":"anonymous"}),
        &next_view, b"successor",
    ).unwrap();
    let slot = format!("agent-http/{next_artifact}");
    let key = evidence_natural_key(&root_run_id, EvidenceNodeKind::RequestRecord,
        &format!("{next_run_id}:{next_artifact}"));
    let successor = EvidenceNode {
        id: format!("ev-{}", &key[..32]), root_run_id: root_run_id.clone(), revision: 3,
        kind: EvidenceNodeKind::RequestRecord, provenance: EvidenceProvenance::Observed,
        natural_key_hash: key,
        payload: serde_json::json!({
            "requestId":"req-successor", "status":200, "bodySha256":next_view["bodySha256"],
            "recordSha256":format!("{:x}", Sha256::digest(
                std::fs::read(context.target_dir.join(&slot)).unwrap())),
        }),
        artifact_refs: vec![slot], created_by_run_id: next_run_id.into(),
        supersedes_id: original[0].clone(), created_at: String::new(),
    };
    insert_evidence_node(&connection, &successor).unwrap();
    assert_eq!(review_fact_refs(&connection, &root_run_id, &context.target_dir).unwrap(), vec![successor.id.clone()]);

    // A delayed import with a smaller revision is a separate branch, not an
    // ancestor of the latest frozen candidate. Its valid artifact must not
    // reopen the Reviewer or enter that candidate's fact manifest.
    let late_run_id = "run-late-nonancestor";
    let mut late_assignment = AgentAssignment::new("asg-late-nonancestor", &root_run_id,
        AgentRole::WebExecutor, AgentLane::TargetTouching, &lease.target_key, "late-nonancestor");
    late_assignment.child_run_id = late_run_id.into();
    late_assignment.evidence_revision = 1;
    late_assignment.state = AssignmentState::Completed;
    insert_assignment(&connection, &late_assignment).unwrap();
    let mut late_run = AgentRunRow::new(late_run_id, &lease.scan_id, 1, &lease.target_key,
        AgentBackendKind::Native, AgentRole::WebExecutor, "plan", "evidence");
    late_run.root_run_id = root_run_id.clone();
    late_run.assignment_id = late_assignment.id.clone();
    late_run.orchestration_policy = MultiAgentPolicy::Multi;
    late_run.lane = Some(AgentLane::TargetTouching);
    late_run.status = AgentRunStatus::Terminal;
    store::create_run(&connection, &late_run).unwrap();
    let late_view = serde_json::json!({
        "requestId":"req-late", "status":200,
        "bodySha256":format!("{:x}", Sha256::digest(b"late")),
    });
    let late_artifact = agent_write_http_record(&context, 3,
        &serde_json::json!({"method":"GET","url":lease.target_key,"identity":"anonymous"}),
        &late_view, b"late",
    ).unwrap();
    let late_slot = format!("agent-http/{late_artifact}");
    let late_key = evidence_natural_key(&root_run_id, EvidenceNodeKind::RequestRecord,
        &format!("{late_run_id}:{late_artifact}"));
    let late_fact = EvidenceNode {
        id: format!("ev-{}", &late_key[..32]), root_run_id: root_run_id.clone(), revision: 1,
        kind: EvidenceNodeKind::RequestRecord, provenance: EvidenceProvenance::Observed,
        natural_key_hash: late_key,
        payload: serde_json::json!({
            "requestId":"req-late", "status":200, "bodySha256":late_view["bodySha256"],
            "recordSha256":format!("{:x}", Sha256::digest(
                std::fs::read(context.target_dir.join(&late_slot)).unwrap())),
        }),
        artifact_refs: vec![late_slot], created_by_run_id: late_run_id.into(),
        supersedes_id: String::new(), created_at: String::new(),
    };
    insert_evidence_node(&connection, &late_fact).unwrap();
    assert_eq!(review_fact_refs(&connection, &root_run_id, &context.target_dir).unwrap(), vec![successor.id]);
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(context.target_dir).unwrap();
}

