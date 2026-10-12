#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn homogeneous_parser_paid_original_worker_executes_real_approved_code_and_preserves_raw_material()
{
    use crate::agent_runtime::{
        evidence_graph::{
            contract::{EvidenceNode, EvidenceNodeKind, EvidenceProvenance},
            store::{evidence_natural_key, insert_evidence_node},
        },
        multi_agent::homogeneous::{apply, decompose, model_worker, parser_execution, pool, store},
        store::stable_hash,
    };
    use crate::tool_supply::{container_parser, frozen_plan::tests::TestSupplyFixture};
    let runtime = container_parser::Runtime::inspect().unwrap();
    let (mut f, supply, _) = frozen_tool_supply_original_birth_with_mode(
        Some(true),
        |path| TestSupplyFixture::approve_container_on(path, runtime.digest()),
        true,
    );
    let approval = supply.approval_id().to_string();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
        let input: JsonValue =
            serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(body["max_tokens"], 1024);
        assert!(body
            .get("tools")
            .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty)));
        assert_eq!(input["source_analysis_tools"][0]["approval_id"], approval);
        assert!(
            body.to_string().contains("session=fixture-account-a"),
            "scoped evidence: {}",
            input["evidence"]
        );
        let refs: Vec<_> = input["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["evidence_ref"].clone())
            .collect();
        let response = json!({"output":{"summary_zh":"按原材料生成只读解析辅助代码","findings":[],"coverage_notes":["解析不等于目标验证"]},"evidence_refs":refs,"terminal_hint":"completed","tool_request":{"name":"source_analysis","approval_id":approval,"program":"def analyze(data):\n return data","evidence_refs":refs}});
        (
            200,
            "application/json",
            proposal_model_response(&response.to_string()),
        )
    }));
    f.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    f.context.environment.api_key = "mock-key".into();
    let db = db::open(&f.context.db_path).unwrap();
    store::set_run_scheduling_model(&db, &f.actor.root_run_id, "homogeneous_workers").unwrap();
    let task_id = store::activate_task_for_lease(&db, &f.actor)
        .unwrap()
        .unwrap();
    let task = store::load_task(&db, &task_id).unwrap().unwrap();
    assert!(
        crate::agent_runtime::multi_agent::homogeneous::material_policy::plaintext(
            &db,
            &f.actor.scan_id,
            1
        )
        .unwrap()
    );
    pool::provision_workers(&db, &task, f.actor.lease_epoch, 3).unwrap();
    let identity = "original fixture frontend data";
    let id = stable_hash(identity);
    let tx = db.unchecked_transaction().unwrap();
    insert_evidence_node(&tx,&EvidenceNode{id:id.clone(),root_run_id:f.actor.root_run_id.clone(),revision:1,kind:EvidenceNodeKind::Endpoint,provenance:EvidenceProvenance::Observed,natural_key_hash:evidence_natural_key(&f.actor.root_run_id,EvidenceNodeKind::Endpoint,identity),payload:json!({"url":format!("{}/api/orders",f.actor.target_key),"path":"/api/orders","Cookie":"session=fixture-account-a","Token":"fixture-account-b","password":"fixture-password","target_request_executed":false}),artifact_refs:vec![],created_by_run_id:f.actor.root_run_id.clone(),supersedes_id:String::new(),created_at:String::new()}).unwrap();
    tx.commit().unwrap();
    let slice = decompose::slice_id_for(
        &task.task_id,
        "path_prefix",
        "/api/orders",
        task.evidence_revision_base,
    );
    let first = homogeneous_product_envelope(
        1,
        &task.task_id,
        "decompose",
        json!({"slices":[{"slice_id":slice,"scope_kind":"path_prefix","scope_value":"/api/orders","lane":"read_only_analysis","evidence_revision_base":task.evidence_revision_base,"worker_capability_profile_id":"homogeneous_v1","max_retries":1}]}),
        homogeneous_product_refs(),
    );
    assert!(
        apply::run_coordinator_turn(&db, &f.actor, &first.to_string())
            .unwrap()
            .applied()
    );
    let worker = pool::worker_ids_in_state(&db, &f.actor.root_run_id, "idle").unwrap()[0].clone();
    let mut refs = homogeneous_product_refs();
    refs["slice_ids"] = json!([slice]);
    refs["worker_ids"] = json!([worker]);
    refs["tool_ids"] = json!(["evidence.read", "source_analysis"]);
    let second = homogeneous_product_envelope(
        2,
        &task.task_id,
        "assign",
        json!({"slice_id":slice,"worker_id":worker,"reserved_tokens":20000,"reserved_requests":1,"capability_lease":["evidence.read","source_analysis"],"lane":"read_only_analysis","attempt":1}),
        refs,
    );
    let assigned = apply::run_coordinator_turn(&db, &f.actor, &second.to_string()).unwrap();
    assert!(assigned.applied(), "{assigned:?}");
    let assignment = store::list_assignment_ids_in_state(&db, &f.actor.root_run_id, &["assigned"])
        .unwrap()[0]
        .clone();
    let transport = SpecialistTransportContext {
        supervision: None,
        db_path: &f.context.db_path,
        scan_id: &f.actor.scan_id,
        attempt_number: f.actor.attempt_number,
        target_key: &f.actor.target_key,
        run_id: &f.actor.root_run_id,
        environment: &f.context.environment,
        proxy: None,
        usage_dir: &f.context.target_dir,
        deadline: None,
    };
    native_homogeneous_model_worker(
        &transport,
        &f.actor,
        &assignment,
        &std::sync::Mutex::new(()),
        Some(supply.configuration()),
    )
    .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    let row = store::load_assignment(&db, &assignment).unwrap().unwrap();
    assert_eq!(row.state, "reported");
    let report = store::load_report(&db, &store::report_id_for(&assignment, 1))
        .unwrap()
        .unwrap();
    assert_eq!(report.terminal_hint, "completed");
    assert_eq!(report.consumed_requests, 1);
    let fact: String = db
        .query_row(
            "SELECT fact_json FROM agent_homogeneous_parser_results WHERE assignment_id=?1",
            [&assignment],
            |r| r.get(0),
        )
        .unwrap();
    let fact: JsonValue = serde_json::from_str(&fact).unwrap();
    assert_eq!(fact["receipt"]["exitCode"], 0);
    assert_eq!(fact["receipt"]["cleanupVerified"], true);
    assert_eq!(fact["vulnerabilityConfirmed"], false);
    let parsed: JsonValue =
        serde_json::from_str(fact["receipt"]["stdout"].as_str().unwrap()).unwrap();
    assert_eq!(
        parsed["result"]["evidence"][0]["observed"]["Cookie"],
        "session=fixture-account-a"
    );
    assert_eq!(
        parsed["result"]["evidence"][0]["observed"]["Token"],
        "fixture-account-b"
    );
    assert_eq!(
        parsed["result"]["evidence"][0]["observed"]["password"],
        "fixture-password"
    );
    assert!(report.output["findings"].as_array().unwrap().is_empty());
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment.clone(),
        run_id: db
            .query_row(
                "SELECT child_run_id FROM agent_assignments WHERE id=?1",
                [&assignment],
                |r| r.get(0),
            )
            .unwrap(),
        role: crate::agent_runtime::contract::AgentRole::DeepInvestigator,
    };
    let before = super::tests::application_table_snapshot(&db);
    let error = parser_execution::execute(
        &db,
        supply.configuration(),
        &f.actor,
        &child,
        &parser_execution::ToolRequest {
            name: "source_analysis".into(),
            approval_id: supply.approval_id().into(),
            program: "def analyze(data): return data".into(),
            evidence_refs: report.evidence_refs,
        },
        &|| false,
    )
    .unwrap_err();
    assert_eq!(error, "assignment_attempt_expired_or_not_executable");
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(model_worker::enabled(&db, &f.actor.scan_id, 1).unwrap());
    let task = store::load_task(&db, &task_id).unwrap().unwrap();
    let slice = store::load_slice(&db, &row.slice_id).unwrap().unwrap();
    let report = store::load_report(&db, &store::report_id_for(&assignment, 1))
        .unwrap()
        .unwrap();
    let checks = crate::agent_runtime::multi_agent::homogeneous::machine::recompute(
        &db, &task, &row, &slice, &report,
    );
    assert!(checks.all_true(), "{checks:?}");
    let mut refs = homogeneous_product_refs();
    refs["slice_ids"] = json!([slice.slice_id]);
    refs["assignment_ids"] = json!([assignment]);
    let mut accepted = homogeneous_product_envelope(
        3,
        &task_id,
        "accept",
        json!({"assignment_id":assignment,"slice_id":slice.slice_id,"report_id":report.report_id,"verdict":"accept","machine_checks":checks,"model_judgement":{"adequate":true,"question_zh":"实际执行和明文是否保持","answer_zh":"实际容器执行退出、清理回执与原文可核验"},"reason_codes":["goal_reached"],"evidence_refs":report.evidence_refs}),
        refs,
    );
    accepted["zero_intent"] = json!({"reason":"covered_by_inflight","evidence_assignment_ids":[assignment],"note_zh":"主管验收已有在途解析汇报"});
    let result = apply::run_coordinator_turn(&db, &f.actor, &accepted.to_string()).unwrap();
    assert!(result.applied(), "{result:?}");
    assert_eq!(
        store::load_assignment(&db, &assignment)
            .unwrap()
            .unwrap()
            .state,
        "accepted"
    );
}
