fn source_specialist_fixture() -> (PathBuf,rusqlite::Connection,WorkbenchStartRecord,crate::agent_runtime::multi_agent::lease::CoordinatorLease) {
    source_specialist_fixture_model(None)
}

fn source_specialist_fixture_model(model: Option<&ModelRuntimeEnv>) -> (PathBuf,rusqlite::Connection,WorkbenchStartRecord,crate::agent_runtime::multi_agent::lease::CoordinatorLease) {
    let (root,connection,record)=analysis_view_fixture_configure("full",true,|connection,record| {
        if let Some(model)=model {
            connection.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles',json(?1),'$.activeModelProfileId','source-fixture') WHERE id=(SELECT id FROM config_profiles ORDER BY is_default DESC,id LIMIT 1)",
                [json!([{"id":"source-fixture","llm":model.llm,"apiKey":model.api_key,"apiBase":model.api_base,"deployment":model.deployment}]).to_string()]).unwrap();
            record.llm_policy=source_model_policy(model);
        }
    });
    analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_result_outcome(engine,scratch,"app.py","source-analysis"))).unwrap();
    let lease=source_specialist_fixture_lease(&connection,&record);
    (root,connection,record,lease)
}

fn source_specialist_fixture_lease(connection:&rusqlite::Connection,record:&WorkbenchStartRecord)->crate::agent_runtime::multi_agent::lease::CoordinatorLease {
    let work=Path::new(connection.path().unwrap()).parent().unwrap().join("attempt-0001");
    // These regressions pin the original v1 contract, including rejection of
    // unplanned Reviewer calls. New v2 execution has separate end-to-end tests.
    let root=prepare_native_source_coordinator_version(connection,&record.scan_id,1,&work,false).unwrap();
    crate::agent_runtime::multi_agent::lease::acquire_coordinator_lease(
        connection,&record.scan_id,1,&root.target_key,&root.run_id,600).unwrap()
}

#[test]
fn source_specialists_rotated_fence_cannot_schedule_second_role_in_same_attempt() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{lease,scheduler,source,specialist}};
    let (root,connection,record,original)=source_specialist_fixture();
    let mapper_slice=source::task_slice(&connection,&original,AgentRole::RepoMapper).unwrap();
    let mapper=scheduler::prepare_readonly_child(&connection,&original,AgentRole::RepoMapper,
        "source_results_ready",&mapper_slice,8_000).unwrap();
    let request=source_specialist_request(&connection,&original,AgentRole::RepoMapper);
    let specialist::Start::Dispatch(call)=specialist::start(&connection,&original,&mapper,&request).unwrap() else {panic!()};
    let usage=AgentTokenUsage {input_tokens:10,cached_input_tokens:0,output_tokens:5,total_tokens:15,model_requests:1};
    specialist::record_received(&connection,&call,"completed old mapper",false,&usage).unwrap();
    complete_readonly_assessment(&connection,&original,&mapper,&usage,
        &json!({"sourceTask":mapper_slice,"summary":"completed old mapper"})).unwrap();
    connection.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 second') WHERE root_run_id=?1",
        [&original.root_run_id]).unwrap();
    let replacement=lease::acquire_coordinator_lease(&connection,&record.scan_id,1,
        &original.target_key,&original.root_run_id,600).unwrap();
    assert_ne!(replacement.fencing_token,original.fencing_token);
    let analyst_slice=source::task_slice(&connection,&replacement,AgentRole::SourceAnalyst).unwrap();
    let error=scheduler::prepare_readonly_child(&connection,&replacement,AgentRole::SourceAnalyst,
        "source_results_ready",&analyst_slice,8_000).unwrap_err();
    assert_eq!(error,"readonly_fencing_changed_requires_fresh_attempt");
    let assignments:i64=connection.query_row("SELECT COUNT(*) FROM agent_assignments WHERE coordinator_run_id=?1",
        [&original.root_run_id],|row|row.get(0)).unwrap();
    assert_eq!(assignments,1,"rejection must not reserve a second budget or grant another child");
    let saved:(i64,String)=connection.query_row("SELECT lease_epoch,fencing_token FROM agent_assignments WHERE id=?1",
        [&mapper.assignment_id],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(saved,(original.lease_epoch,original.fencing_token));
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_specialists_group_shared_revision_but_keep_both_accepted_origins() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler,specialist}};
    let (root,connection,record,_)=source_result_identical_engine_fixture();
    let lease=source_specialist_fixture_lease(&connection,&record);
    for role in [AgentRole::RepoMapper,AgentRole::SourceAnalyst] {
        let input=source::assessment_input(&connection,&lease,role).unwrap();
        assert_eq!(input["candidateRecords"].as_array().unwrap().len(),1);
        let origins=input["candidateRecords"][0]["acceptedSources"].as_array().unwrap();
        assert_eq!(origins.iter().map(|o|o["engine"].as_str().unwrap()).collect::<std::collections::BTreeSet<_>>(),["codeql","semgrep"].into_iter().collect());
        assert_eq!(input["sourceTask"]["candidateReceipts"].as_array().unwrap().len(),2,"the task still binds each accepted source");
        assert_eq!(input["sourceTask"]["targetRequestsGranted"],0);
        assert_eq!(input["sourceTask"]["toolsGranted"],json!([]));
        let slice=source::task_slice(&connection,&lease,role).unwrap();
        let child=scheduler::prepare_readonly_child(&connection,&lease,role,"source_results_ready",&slice,8_000).unwrap();
        let request=source_specialist_request(&connection,&lease,role);
        let specialist::Start::Dispatch(call)=specialist::start(&connection,&lease,&child,&request).unwrap() else {panic!()};
        let usage=AgentTokenUsage {input_tokens:10,cached_input_tokens:0,output_tokens:5,total_tokens:15,model_requests:1};
        specialist::record_received(&connection,&call,"one candidate, two recorded origins",false,&usage).unwrap();
        let payload=json!({"summary":"one candidate, two recorded origins","sourceTask":slice});
        complete_readonly_assessment(&connection,&lease,&child,&usage,&payload).unwrap();
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND acknowledged_at<>''",[&child.assignment_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
    drop(connection);fs::remove_dir_all(root).unwrap();
}

fn source_specialist_request(connection:&rusqlite::Connection,lease:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,role:crate::agent_runtime::contract::AgentRole)->JsonValue {
    let input=crate::agent_runtime::multi_agent::source::assessment_input(connection,lease,role).unwrap();
    json!({"messages":[{"role":"user","content":crate::agent_runtime::secrets::redact_json(&input).to_string()}],"tools":[]})
}

#[test]
fn source_surface_contract_rejects_coverage_injected_during_model_claim_without_dispatch() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler,specialist}};
    let (root,connection,_record,lease)=source_specialist_fixture();
    let role=AgentRole::SourceAnalyst;
    let slice=source::task_slice(&connection,&lease,role).unwrap();
    let child=scheduler::prepare_readonly_child(&connection,&lease,role,"source_results_ready",&slice,8_000).unwrap();
    let request=source_specialist_request(&connection,&lease,role);
    let before=source_coordinator_row(&connection,&lease.root_run_id);
    connection.execute_batch("CREATE TRIGGER source_coverage_claim_change AFTER INSERT ON agent_specialist_calls BEGIN
        UPDATE agent_runs SET plan_json=json_set(plan_json,'$.sourceCoveragePhaseVersion',1) WHERE id=NEW.root_run_id; END;").unwrap();
    assert_eq!(specialist::start(&connection,&lease,&child,&request).unwrap_err(),"source_surface_frozen_plan_invalid");
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_specialist_calls",[],|r|r.get::<_,i64>(0)).unwrap(),0,
        "a rejected claim must not leave an unknown dispatched call");
    assert_eq!(source_coordinator_row(&connection,&lease.root_run_id),before,"post-write mutation must roll back");
    connection.execute_batch("DROP TRIGGER source_coverage_claim_change").unwrap();
    assert!(matches!(specialist::start(&connection,&lease,&child,&request).unwrap(),specialist::Start::Dispatch(_)));
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_surface_contract_rejects_unplanned_reviewer_and_web_roles() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::scheduler};
    let (root,connection,_record,lease)=source_specialist_fixture();
    for (role,lane,capabilities) in [
        (AgentRole::EvidenceReviewer,AgentLane::Review,vec!["evidence.read".into(),"review.write".into()]),
        (AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,vec!["evidence.read".into()]),
        (AgentRole::DeepInvestigator,AgentLane::ReadOnlyAnalysis,vec!["evidence.read".into()]),
        (AgentRole::WebExecutor,AgentLane::TargetTouching,vec!["http_request".into()]),
    ] {
        let error=scheduler::schedule_child(&connection,&lease,role,lane,"unplanned_source_phase",
            &json!({"surface":"web","candidateId":"unbound"}),1,&capabilities,100,1)
            .expect_err("source roots must not accept a generic Web or Reviewer assignment");
        assert_eq!(error,"source_phase_role_not_in_frozen_plan");
    }
    for table in ["agent_assignments","agent_capability_leases","agent_lane_leases","agent_specialist_calls"] {
        assert_eq!(connection.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
    let reserved:(i64,i64)=connection.query_row("SELECT COALESCE(SUM(reserved_tokens),0),COALESCE(SUM(reserved_requests),0) FROM agent_budget_ledger WHERE root_run_id=?1",
        [&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(reserved,(0,0));
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_surface_contract_rechecks_old_children_before_start_and_dispatch() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{source,scheduler,specialist}};
    let (root,connection,_record,lease)=source_specialist_fixture();
    let slice=source::task_slice(&connection,&lease,AgentRole::SourceAnalyst).unwrap();
    let mut child=scheduler::schedule_child(&connection,&lease,AgentRole::SourceAnalyst,AgentLane::ReadOnlyAnalysis,
        "source_results_ready",&slice,1,&["evidence.read".into()],100,1).unwrap();
    // Reproduce a legacy generic Reviewer row, without using the now-guarded
    // scheduler to grant one. All assignment/run/lane identities agree.
    connection.execute("UPDATE agent_assignments SET role='evidence_reviewer',lane='review',task_slice_json='{}' WHERE id=?1",[&child.assignment_id]).unwrap();
    connection.execute("UPDATE agent_runs SET role='evidence_reviewer',lane='review' WHERE id=?1",[&child.run_id]).unwrap();
    connection.execute("UPDATE agent_lane_leases SET lane='review' WHERE assignment_id=?1",[&child.assignment_id]).unwrap();
    child.role=AgentRole::EvidenceReviewer;
    assert_eq!(scheduler::mark_child_running(&connection,&lease,&child).unwrap_err(),"source_phase_role_not_in_frozen_plan");
    assert_eq!(connection.query_row("SELECT state FROM agent_assignments WHERE id=?1",[&child.assignment_id],|r|r.get::<_,String>(0)).unwrap(),"leased");
    connection.execute("UPDATE agent_assignments SET state='running' WHERE id=?1",[&child.assignment_id]).unwrap();
    connection.execute("UPDATE agent_runs SET status='running' WHERE id=?1",[&child.run_id]).unwrap();
    let request=json!({"messages":[{"role":"user","content":"unbound review"}],"tools":[]});
    let result=specialist::start_authorized(&connection,&lease,&child,&request,|_| {
        // The frozen-surface check precedes provider/runtime authorization.
        panic!("legacy source Reviewer reached dispatch authorization")
    });
    assert_eq!(result.unwrap_err(),"source_phase_role_not_in_frozen_plan");
    // Keep this assertion about the actual durable dispatch journal, not just
    // the returned error. A rejected request must not become an unknown call.
    assert!(!connection.query_row("SELECT EXISTS(SELECT 1 FROM agent_specialist_calls)",[],|r|r.get::<_,bool>(0)).unwrap());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_surface_contract_rejects_changed_versions_grants_and_rolls_back_postwrite() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{source,scheduler}};
    let (root,connection,_record,lease)=source_specialist_fixture();
    let role=AgentRole::SourceAnalyst;
    let slice=source::task_slice(&connection,&lease,role).unwrap();
    let original:String=connection.query_row("SELECT plan_json FROM agent_runs WHERE id=?1",[&lease.root_run_id],|r|r.get(0)).unwrap();
    for (key,value) in [
        ("surface",json!("web")),("schemaVersion",json!(2)),("sourceToolsPhaseVersion",json!(2)),
        ("sourceReviewPhaseVersion",json!(1)),("modelRequestLimit",json!(9)),
        ("sourceCoveragePhaseVersion",json!(1)),("sourceCoveragePhaseVersion",JsonValue::Null),
        ("sourceCoverageDecisionPhaseVersion",json!(1)),
        ("targetRequestsGranted",json!(1)),("hostActionsGranted",json!(1)),
    ] {
        connection.execute("UPDATE agent_runs SET plan_json=json_set(plan_json,?2,json(?3)) WHERE id=?1",
            params![lease.root_run_id,format!("$.{key}"),value.to_string()]).unwrap();
        let error=scheduler::schedule_child(&connection,&lease,role,AgentLane::ReadOnlyAnalysis,
            "source_results_ready",&slice,1,&["evidence.read".into()],100,1).unwrap_err();
        assert_eq!(error,"source_surface_frozen_plan_invalid","{key}");
        connection.execute("UPDATE agent_runs SET plan_json=?2 WHERE id=?1",params![lease.root_run_id,original]).unwrap();
    }
    connection.execute("UPDATE agent_runs SET root_run_id='',orchestration_policy='single' WHERE id=?1",[&lease.root_run_id]).unwrap();
    assert_eq!(scheduler::schedule_child(&connection,&lease,role,AgentLane::ReadOnlyAnalysis,
        "source_results_ready",&slice,1,&["evidence.read".into()],100,1).unwrap_err(),"source_surface_frozen_plan_invalid");
    connection.execute("UPDATE agent_runs SET root_run_id=id,orchestration_policy='multi' WHERE id=?1",[&lease.root_run_id]).unwrap();
    connection.execute_batch("CREATE TRIGGER source_surface_test_change AFTER INSERT ON agent_capability_leases BEGIN
        UPDATE agent_runs SET plan_json=json_set(plan_json,'$.sourceReviewPhaseVersion',1) WHERE id=NEW.root_run_id; END;").unwrap();
    assert_eq!(scheduler::schedule_child(&connection,&lease,role,AgentLane::ReadOnlyAnalysis,
        "source_results_ready",&slice,1,&["evidence.read".into()],100,1).unwrap_err(),"source_surface_frozen_plan_invalid");
    for table in ["agent_assignments","agent_capability_leases","agent_lane_leases"] {
        assert_eq!(connection.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
    assert_eq!(source::task_slice(&connection,&lease,role).unwrap(),slice,"root mutation must roll back too");
    connection.execute_batch("DROP TRIGGER source_surface_test_change").unwrap();
    let child=scheduler::prepare_readonly_child(&connection,&lease,role,"source_results_ready",&slice,100).unwrap();
    assert_eq!(child.role,role);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_surface_contract_start_and_model_claim_recheck_after_write_triggers() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{source,scheduler,specialist}};
    let (root,connection,_record,lease)=source_specialist_fixture();
    let role=AgentRole::SourceAnalyst;
    let slice=source::task_slice(&connection,&lease,role).unwrap();
    let child=scheduler::schedule_child(&connection,&lease,role,AgentLane::ReadOnlyAnalysis,
        "source_results_ready",&slice,1,&["evidence.read".into()],8_000,1).unwrap();
    connection.execute_batch("CREATE TRIGGER source_surface_start_change AFTER UPDATE ON agent_runs
        WHEN NEW.role='source_analyst' AND NEW.status='running' BEGIN
        UPDATE agent_runs SET hard_request_budget=9 WHERE id=NEW.root_run_id; END;").unwrap();
    assert_eq!(scheduler::mark_child_running(&connection,&lease,&child).unwrap_err(),"source_surface_frozen_plan_invalid");
    assert_eq!(connection.query_row("SELECT state FROM agent_assignments WHERE id=?1",[&child.assignment_id],|r|r.get::<_,String>(0)).unwrap(),"leased");
    assert_eq!(connection.query_row("SELECT status FROM agent_runs WHERE id=?1",[&child.run_id],|r|r.get::<_,String>(0)).unwrap(),"prepared");
    assert_eq!(connection.query_row("SELECT hard_request_budget FROM agent_runs WHERE id=?1",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),8);
    connection.execute_batch("DROP TRIGGER source_surface_start_change").unwrap();
    scheduler::mark_child_running(&connection,&lease,&child).unwrap();
    let request=source_specialist_request(&connection,&lease,role);
    connection.execute_batch("CREATE TRIGGER source_surface_claim_change AFTER INSERT ON agent_specialist_calls BEGIN
        UPDATE agent_runs SET plan_json=json_set(plan_json,'$.surface','web') WHERE id=NEW.root_run_id; END;").unwrap();
    assert_eq!(specialist::start(&connection,&lease,&child,&request).unwrap_err(),"source_surface_frozen_plan_invalid");
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_specialist_calls",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(source::task_slice(&connection,&lease,role).unwrap(),slice);
    connection.execute_batch("DROP TRIGGER source_surface_claim_change").unwrap();
    assert!(matches!(specialist::start(&connection,&lease,&child,&request).unwrap(),specialist::Start::Dispatch(_)));
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_specialists_assignment_response_mailbox_and_budget_are_real_and_idempotent() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler,specialist}};
    let (root,connection,_record,lease)=source_specialist_fixture();
    for role in [AgentRole::RepoMapper,AgentRole::SourceAnalyst] {
        let slice=source::task_slice(&connection,&lease,role).unwrap();
        assert_eq!(slice["targetRequestsGranted"],0);
        assert!(!slice["candidateReceipts"].as_array().unwrap().is_empty());
        let child=scheduler::prepare_readonly_child(&connection,&lease,role,"source_results_ready",&slice,8_000).unwrap();
        assert_eq!(scheduler::prepare_readonly_child(&connection,&lease,role,"source_results_ready",&slice,8_000).unwrap(),child);
        let request=source_specialist_request(&connection,&lease,role);
        let specialist::Start::Dispatch(call)=specialist::start(&connection,&lease,&child,&request).unwrap() else {panic!("first dispatch must be new")};
        let usage=AgentTokenUsage {input_tokens:10,cached_input_tokens:0,output_tokens:5,total_tokens:15,model_requests:1};
        let response=specialist::record_received(&connection,&call,"examined this source attempt",false,&usage).unwrap();
        let payload=json!({"summary":response.text,"sourceTask":slice});
        assert_eq!(complete_readonly_assessment(&connection,&lease,&child,&usage,&payload).unwrap(),payload);
        // Completion is a durable receipt, not a second model dispatch or a
        // fresh reservation when the caller reconciles the same assignment.
        let replay=scheduler::prepare_readonly_child(&connection,&lease,role,"source_results_ready",&slice,8_000).unwrap();
        assert_eq!(replay,child);
        assert!(matches!(specialist::start(&connection,&lease,&child,&request).unwrap(),specialist::Start::Received(_)));
        complete_readonly_assessment(&connection,&lease,&child,&usage,&payload).unwrap();
        let result:(i64,i64,i64)=connection.query_row(
            "SELECT count(*),sum(delivery_attempts),sum(delivered_at<>'' AND acknowledged_at<>'') FROM agent_messages WHERE assignment_id=?1",
            [&child.assignment_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(result,(1,1,1));
        let terminal:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND a.state='completed' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND r.status='terminal' AND r.used_tokens=15 AND r.used_requests=1)",
            [&child.assignment_id],|r|r.get(0)).unwrap();assert!(terminal);
    }
    let ledger:(i64,i64,i64,i64)=connection.query_row("SELECT spent_tokens,spent_requests,reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",[&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(ledger,(30,2,0,0));
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_specialists_cannot_escalate_lanes_capabilities_or_frozen_contract() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{source,scheduler}};
    let (root,connection,_record,lease)=source_specialist_fixture();
    for role in [AgentRole::RepoMapper,AgentRole::SourceAnalyst] {
        let slice=source::task_slice(&connection,&lease,role).unwrap();
        for (lane,capabilities,revision,input) in [
            (AgentLane::TargetTouching,vec!["evidence.read".into()],1,slice.clone()),
            (AgentLane::Review,vec!["evidence.read".into()],1,slice.clone()),
            (AgentLane::ReadOnlyAnalysis,vec!["shell.execute".into()],1,slice.clone()),
            (AgentLane::ReadOnlyAnalysis,vec!["review.write".into()],1,slice.clone()),
            (AgentLane::ReadOnlyAnalysis,vec!["evidence.read".into()],2,slice.clone()),
            (AgentLane::ReadOnlyAnalysis,vec!["evidence.read".into()],1,json!({"target":"https://unrelated.invalid"})),
        ] {
            assert!(scheduler::schedule_child(&connection,&lease,role,lane,"source_results_ready",&input,revision,&capabilities,8_000,1).is_err());
        }
        for field in ["analysisResultsDigest","analysisDigest","rootRunId","attemptNumber","candidateReceipts","toolsGranted"] {
            let mut changed=slice.clone();changed[field]=json!("replacement");
            assert!(scheduler::prepare_readonly_child(&connection,&lease,role,"source_results_ready",&changed,8_000).is_err(),"{field}");
        }
    }
    let count:i64=connection.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1",[&lease.root_run_id],|r|r.get(0)).unwrap();
    assert_eq!(count,0,"denials must not allocate children or budgets");
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_specialists_reject_fake_finish_and_different_response_receipt() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler,specialist}};
    let (root,connection,_record,lease)=source_specialist_fixture();
    let role=AgentRole::SourceAnalyst;
    let slice=source::task_slice(&connection,&lease,role).unwrap();
    let child=scheduler::prepare_readonly_child(&connection,&lease,role,"source_results_ready",&slice,8_000).unwrap();
    let usage=AgentTokenUsage {input_tokens:10,cached_input_tokens:0,output_tokens:5,total_tokens:15,model_requests:1};
    let payload=json!({"sourceTask":slice,"summary":"actual"});
    assert!(complete_readonly_assessment(&connection,&lease,&child,&usage,&payload).is_err());
    let request=source_specialist_request(&connection,&lease,role);
    let specialist::Start::Dispatch(call)=specialist::start(&connection,&lease,&child,&request).unwrap() else {panic!()};
    specialist::record_received(&connection,&call,"actual",false,&usage).unwrap();
    let mut forged=payload.clone();forged["summary"]=json!("fabricated");
    assert_eq!(complete_readonly_assessment(&connection,&lease,&child,&usage,&forged).unwrap_err(),"source_assessment_receipt_mismatch");
    forged=payload.clone();forged["sourceTask"]["analysisResultsDigest"]=json!("old-attempt");
    assert!(complete_readonly_assessment(&connection,&lease,&child,&usage,&forged).is_err());
    complete_readonly_assessment(&connection,&lease,&child,&usage,&payload).unwrap();
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_specialists_cannot_dispatch_after_stop_or_revision_damage() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler,specialist}};
    for mutation in ["stop","attempt","scope","envelope","view","target","root_plan","root_evidence","root_cancel","child_cancel","policy"] {
        let (root,connection,record,mut lease)=source_specialist_fixture();
        let role=AgentRole::RepoMapper;
        let slice=source::task_slice(&connection,&lease,role).unwrap();
        let child=scheduler::prepare_readonly_child(&connection,&lease,role,"source_results_ready",&slice,8_000).unwrap();
        let request=source_specialist_request(&connection,&lease,role);
        match mutation {
            "stop"=>{connection.execute("UPDATE sentinel_scans SET status='stopped' WHERE id=?1",[&record.scan_id]).unwrap();},
            "attempt"=>{connection.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",[&record.scan_id]).unwrap();},
            "scope"=>{connection.execute("UPDATE sentinel_scans SET source_path='/nonexistent-replacement' WHERE id=?1",[&record.scan_id]).unwrap();},
            "envelope"=>{connection.execute_batch("DROP TRIGGER import_revisions_are_append_only; UPDATE import_record_revisions SET envelope_json='{}' WHERE record_kind='finding_candidate';").unwrap();},
            "view"=>{let view=source_result_view(&connection,&record);let file=view.repository().join("app.py");fs::remove_file(&file).unwrap();fs::write(file,"changed after scheduling").unwrap();},
            "target"=>{lease.target_key="https://web.example.invalid".into();},
            "root_plan"=>{connection.execute("UPDATE agent_runs SET plan_hash='web-plan' WHERE id=?1",[&lease.root_run_id]).unwrap();},
            "root_evidence"=>{connection.execute("UPDATE agent_runs SET evidence_hash='old-results' WHERE id=?1",[&lease.root_run_id]).unwrap();},
            "root_cancel"=>{connection.execute("UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1",[&lease.root_run_id]).unwrap();},
            "child_cancel"=>{connection.execute("UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1",[&child.run_id]).unwrap();},
            "policy"=>{connection.execute("UPDATE agent_runs SET orchestration_policy='single' WHERE id=?1",[&lease.root_run_id]).unwrap();},
            _=>unreachable!(),
        }
        assert!(specialist::start(&connection,&lease,&child,&request).is_err(),"{mutation}");
        let calls:i64=connection.query_row("SELECT count(*) FROM agent_specialist_calls WHERE assignment_id=?1",[&child.assignment_id],|r|r.get(0)).unwrap();assert_eq!(calls,0);
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_specialists_unreviewed_pipeline_cannot_report_ci_passed() {
    let (root,connection,record)=analysis_view_fixture("full",true);
    let report=analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_result_outcome(engine,scratch,"app.py","candidate"))).unwrap();
    assert_eq!(report["gate"]["status"],"coverage_incomplete");
    assert_eq!(report["gate"]["counts"]["decisions"],0);
    assert!(report["gaps"].as_array().unwrap().contains(&json!("source_review_not_completed")));
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_specialists_real_transport_delivers_once_and_refuses_unbound_model_input() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler}};
    let (port,seen,stop)=crate::scanning::agent_tests::spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",
        json!({"id":"source-loopback","choices":[{"message":{"role":"assistant","content":"source assessment recorded"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())));
    let environment=source_specialist_test_environment(port);
    let (root,connection,record,lease)=source_specialist_true_born_model_fixture(&environment);
    let db_path=root.join("oviraptor.sqlite3");
    let context=SpecialistTransportContext { supervision: None,db_path:&db_path,scan_id:&record.scan_id,attempt_number:1,
        target_key:&lease.target_key,run_id:&lease.root_run_id,environment:&environment,proxy:None,usage_dir:&root,deadline:None};
    for role in [AgentRole::RepoMapper,AgentRole::SourceAnalyst] {
        let slice=source::task_slice(&connection,&lease,role).unwrap();
        let input=source::assessment_input(&connection,&lease,role).unwrap();
        let (tokens,_)=source_assessment_budget(&source_assessment_messages("read-only source assessment",&input),&agent_model_profile(&environment,None).unwrap()).unwrap();
        let child=scheduler::prepare_readonly_child(&connection,&lease,role,"source_results_ready",&slice,tokens).unwrap();
        let mut wrong=input.clone();wrong["candidateRecords"]=json!([]);
        let before=seen.lock().unwrap().len();
        assert!(specialist_round_transport(&context,&lease,&child,"read-only source assessment",wrong).is_err());
        assert_eq!(seen.lock().unwrap().len(),before,"invalid source evidence must not reach the model");
        let (text,usage)=specialist_round_transport(&context,&lease,&child,"read-only source assessment",input.clone()).unwrap();
        let payload=json!({"sourceTask":slice,"summary":text});
        complete_readonly_assessment(&connection,&lease,&child,&usage,&payload).unwrap();
        assert_eq!(specialist_round_transport(&context,&lease,&child,"read-only source assessment",input).unwrap(),(text,usage));
        assert_eq!(seen.lock().unwrap().len(),before+1);
    }
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
    let counts:(i64,i64)=connection.query_row("SELECT (SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND acknowledged_at<>''),(SELECT count(*) FROM agent_events e JOIN agent_runs r ON r.id=e.run_id WHERE r.root_run_id=?1 AND e.event_type='model_round_completed')",[&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(counts,(2,2));
    drop(connection);fs::remove_dir_all(root).unwrap();
}

fn source_specialist_test_environment(port:u16)->ModelRuntimeEnv {
    ModelRuntimeEnv {llm:"openai/test-model".into(),api_key:"test-key".into(),
        api_base:format!("http://127.0.0.1:{port}/v1"),deployment:"cloud".into(),
        full_power:false,prompt_audit_mode:"off".into()}
}

