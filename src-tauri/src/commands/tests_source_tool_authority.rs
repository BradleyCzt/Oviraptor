include!("tests_source_tools_born_fixture.rs");

fn native_source_tool_fixture(role:crate::agent_runtime::contract::AgentRole)
    -> (PathBuf,rusqlite::Connection,AgentRunContext,crate::agent_runtime::multi_agent::lease::CoordinatorLease) {
    native_source_tool_fixture_mode(role,"full")
}

fn native_source_tool_fixture_mode(role:crate::agent_runtime::contract::AgentRole,mode:&str)
    -> (PathBuf,rusqlite::Connection,AgentRunContext,crate::agent_runtime::multi_agent::lease::CoordinatorLease) {
    use crate::agent_runtime::{contract::AgentLane,multi_agent::{source,scheduler}};
    let (root,connection,record)=analysis_view_fixture(mode,true);
    analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(source_result_outcome(engine,scratch,"app.py","source-analysis"))).unwrap();
    let lease=source_tool_true_born_fixture_lease(&connection,&record);
    connection.execute("UPDATE agent_runs SET status='running',started_at=datetime('now','localtime') WHERE id=?1",
        [&lease.root_run_id]).unwrap();
    let slice=source::tool_task_slice(&connection,&lease,role,1).unwrap();
    let child=scheduler::schedule_child(&connection,&lease,role,AgentLane::ReadOnlyAnalysis,
        "source_tools_ready",&slice,1,&source::tool_capabilities(role).unwrap(),0,0).unwrap();
    scheduler::mark_child_running(&connection,&lease,&child).unwrap();
    let mut context=crate::scanning::agent_tests::test_context(&root.join("oviraptor.sqlite3"),&lease.target_key,Vec::new());
    context.scan_id=record.scan_id;
    context.attempt_number=1;
    context.run=Some(AgentRunLedger{db_path:context.db_path.clone(),run_id:child.run_id});
    // Deliberately impossible as a Web plan: source authorization must use its
    // actual published source contract, not this legacy context member.
    context.execution_plan.schema_version=255;
    (root,connection,context,lease)
}

#[test]
fn source_tool_authority_diff_cannot_read_or_submit_outside_selected_view() {
    use crate::agent_runtime::contract::AgentRole;
    let (root,connection,context,_)=native_source_tool_fixture_mode(AgentRole::SourceAnalyst,"diff");
    let inventory=agent_execute_source_tool(&context,"repo.inventory",&json!({}));
    assert!(inventory.get("error").is_none(),"{inventory}");
    assert!(inventory.to_string().contains("app.py"));
    assert!(!inventory.to_string().contains("untouched.py"));
    let selected=agent_execute_source_tool(&context,"repo.read_slice",&json!({"path":"app.py"}));
    assert!(selected.to_string().contains("print('changed')"),"{selected}");
    for path in ["untouched.py","../untouched.py","/etc/passwd"] {
        let read=agent_execute_source_tool(&context,"repo.read_slice",&json!({"path":path}));
        assert!(read.get("error").is_some(),"{path}: {read}");
        let candidate=agent_execute_source_tool(&context,"evidence.submit_candidate",
            &json!({"title":"out of scope","rationale":"must reject","path":path,"line":1}));
        assert!(candidate.get("error").is_some(),"{path}: {candidate}");
    }
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_evidence_nodes",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_tool_authority_real_native_child_reads_and_submits_without_web_plan() {
    use crate::agent_runtime::contract::AgentRole;
    let (root,connection,context,lease)=native_source_tool_fixture(AgentRole::SourceAnalyst);
    let plan=crate::agent_runtime::store::attempt_plan(&connection,&context.scan_id,1,&context.target_url).unwrap();
    assert!(AgentExecutionPlan::from_json(&plan).is_none(),"source root is not a Web plan");
    assert!(agent_run_has_frozen_source(&context));
    let tools=agent_tool_specs_for(&context);
    assert!(tools.iter().any(|spec|spec.name=="evidence.submit_candidate"));
    assert!(tools.iter().all(|spec|agent_is_source_tool(spec.name)));
    let inventory=agent_execute_source_tool(&context,"repo.inventory",&json!({}));
    assert!(inventory.get("error").is_none(),"{inventory}");
    let read=agent_execute_source_tool(&context,"repo.read_slice",&json!({"path":"app.py"}));
    assert!(read.to_string().contains("print('changed')"),"{read}");
    for tool in ["replay_http","browser_action","shell.execute","review.write"] {
        assert!(agent_authorize_tool_on(&connection,&context,tool).is_err(),"{tool}");
    }
    let args=json!({"title":"native source candidate","rationale":"needs independent review","path":"app.py","line":1});
    let first=agent_execute_source_tool(&context,"evidence.submit_candidate",&args);
    assert!(first["id"].is_string(),"{first}");
    let replay=agent_execute_source_tool(&context,"evidence.submit_candidate",&args);
    assert_eq!(first["id"],replay["id"]);
    let stored:(String,String,i64)=connection.query_row("SELECT root_run_id,created_by_run_id,revision FROM agent_evidence_nodes WHERE id=?1",
        [first["id"].as_str().unwrap()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(stored,(lease.root_run_id,context.run.as_ref().unwrap().run_id.clone(),1));
    let finish=agent_execute_source_tool(&context,"assignment.finish",&json!({"summary":"analysis complete","gaps":[]}));
    assert_eq!(finish["conclusive"],false,"{finish}");
    assert!(finish["gaps"].as_array().unwrap().contains(&json!("source_review_not_completed")));
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_review_decisions",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_tool_authority_initial_assessments_and_mapper_cannot_gain_candidate_writes() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{source,scheduler}};
    let (root,connection,context,_)=native_source_tool_fixture(AgentRole::RepoMapper);
    assert!(agent_authorize_tool_on(&connection,&context,"repo.read_slice").is_ok());
    assert!(agent_authorize_tool_on(&connection,&context,"evidence.submit_candidate").is_err());
    assert!(!agent_tool_specs_for(&context).iter().any(|spec|spec.name=="evidence.submit_candidate"));
    drop(connection);fs::remove_dir_all(root).unwrap();
    let (root,connection,_record,lease)=source_specialist_fixture();
    let role=AgentRole::SourceAnalyst;
    let initial=source::task_slice(&connection,&lease,role).unwrap();
    let capabilities=source::tool_capabilities(role).unwrap();
    assert!(scheduler::schedule_child(&connection,&lease,role,AgentLane::ReadOnlyAnalysis,"source_tools_ready",
        &initial,1,&capabilities,0,0).is_err());
    let slice=source::tool_task_slice(&connection,&lease,role,1).unwrap();
    assert!(scheduler::prepare_readonly_child(&connection,&lease,role,"source_results_ready",&slice,8000).is_err());
    let child=scheduler::prepare_readonly_child(&connection,&lease,role,"source_results_ready",&initial,8000).unwrap();
    assert!(source::authorize_tool(&connection,&lease.scan_id,1,&lease.target_key,&child.run_id,"repo.read_slice").is_err());
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_tool_authority_generic_dispatch_does_not_spend_web_request_budget() {
    use crate::agent_runtime::contract::AgentRole;
    let (root,connection,context,_)=native_source_tool_fixture(AgentRole::RepoMapper);
    let mut runtime=AgentToolRuntime {target_requests:usize::MAX,..Default::default()};
    let result=agent_execute_tool_inner(&context,&mut runtime,"repo.read_slice",&json!({"path":"app.py"}));
    assert!(result.get("error").is_none(),"{result}");
    assert!(result.to_string().contains("print('changed')"));
    assert_eq!(runtime.target_requests,usize::MAX);
    connection.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE capability='repo.read_slice'",[]).unwrap();
    assert!(!agent_tool_specs_for(&context).iter().any(|spec|spec.name=="repo.read_slice"));
    assert!(agent_execute_tool_inner(&context,&mut runtime,"repo.read_slice",&json!({"path":"app.py"})).get("error").is_some());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_tool_authority_rejects_scope_role_revision_or_capability_forgery() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{source,scheduler}};
    let (root,connection,_record,lease)=source_specialist_fixture();
    let role=AgentRole::SourceAnalyst;
    let slice=source::tool_task_slice(&connection,&lease,role,1).unwrap();
    let caps=source::tool_capabilities(role).unwrap();
    for field in ["analysisDigest","analysisResultsDigest","candidateReceipts","rootRunId","toolsGranted","role","phase"] {
        let mut changed=slice.clone();changed[field]=json!("changed");
        assert!(scheduler::schedule_child(&connection,&lease,role,AgentLane::ReadOnlyAnalysis,"source_tools_ready",
            &changed,1,&caps,0,0).is_err(),"{field}");
    }
    for caps in [vec!["evidence.read".into()],vec!["repo.read_slice".into()],vec!["shell.execute".into()]] {
        assert!(scheduler::schedule_child(&connection,&lease,role,AgentLane::ReadOnlyAnalysis,"source_tools_ready",
            &slice,1,&caps,0,0).is_err());
    }
    assert!(source::tool_task_slice(&connection,&lease,role,2).is_err());
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_tool_authority_rechecks_live_permissions_and_runtime_on_every_read() {
    use crate::agent_runtime::contract::AgentRole;
    for mutation in [
        "UPDATE sentinel_scans SET status='paused'",
        "UPDATE sentinel_scan_attempts SET status='paused'",
        "UPDATE agent_runs SET cancel_requested_at='requested'",
        "UPDATE agent_runs SET backend='retired' WHERE parent_run_id IS NOT NULL",
        "UPDATE agent_assignments SET state='paused'",
        "UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime')",
        "UPDATE agent_capability_leases SET lease_expires_at='2000-01-01'",
        "UPDATE agent_coordinator_leases SET fencing_token='replaced'",
        "DELETE FROM agent_lane_leases",
        "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles[0].apiKey','different')",
    ] {
        let (root,connection,context,_)=native_source_tool_fixture(AgentRole::SourceAnalyst);
        assert!(agent_authorize_tool_on(&connection,&context,"repo.read_slice").is_ok());
        connection.execute_batch(mutation).unwrap();
        assert!(agent_authorize_tool_on(&connection,&context,"repo.read_slice").is_err(),"{mutation}");
        let answer=agent_execute_source_tool(&context,"repo.read_slice",&json!({"path":"app.py"}));
        assert!(answer.get("error").is_some(),"{mutation}: {answer}");
        assert!(!answer.to_string().contains("print('changed')"));
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_tool_authority_candidate_transaction_rolls_back_revocation_and_runtime_drift() {
    use crate::agent_runtime::contract::AgentRole;
    for mutation in ["UPDATE agent_runs SET cancel_requested_at='requested'",
        "UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime')",
        "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles[0].apiKey','changed')",
        "UPDATE agent_assignments SET evidence_revision=2",
        "UPDATE agent_runs SET created_at='2000-01-01 00:00:00' WHERE role='coordinator'"] {
        let (root,connection,context,_)=native_source_tool_fixture(AgentRole::SourceAnalyst);
        connection.execute_batch(&format!("CREATE TRIGGER revoke_native_source AFTER INSERT ON agent_evidence_nodes BEGIN {mutation}; END;")).unwrap();
        let answer=agent_execute_source_tool(&context,"evidence.submit_candidate",
            &json!({"title":"suspect","rationale":"unreviewed","path":"app.py","line":1}));
        assert_eq!(answer["code"],"source_authorization_changed","{mutation}: {answer}");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_evidence_nodes",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_evidence_revisions",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert!(agent_run_has_frozen_source(&context),"failed write must restore authority");
        drop(connection);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_tool_authority_requires_running_root_and_started_boundary() {
    use crate::agent_runtime::contract::AgentRole;
    for mutation in ["status='prepared'", "started_at=''"] {
        let (root, db, context, actor) = native_source_tool_fixture(AgentRole::SourceAnalyst);
        assert!(agent_native_source_tool_authority(&db, &context, "repo.read_slice").is_ok());
        db.execute(&format!("UPDATE agent_runs SET {mutation} WHERE id=?1"), [&actor.root_run_id]).unwrap();
        let before = crate::scanning::web_mode_test_rows(&db);
        assert_eq!(agent_native_source_tool_authority(&db, &context, "repo.read_slice").err(), Some("source_tool_root_not_running"));
        assert!(agent_tool_specs_for(&context).is_empty());
        let answer = agent_execute_source_tool(&context, "repo.read_slice", &json!({"path":"app.py"}));
        assert!(answer.get("error").is_some());
        assert!(!answer.to_string().contains("print('changed')"));
        crate::scanning::web_mode_assert_rows(&db, &before);
        drop(db); fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn source_tool_authority_remaining_time_uses_original_origin_not_started_metadata() {
    use crate::agent_runtime::contract::AgentRole;
    for stamp in ["invalid", "2099-01-01 00:00:00", "2000-01-01 00:00:00"] {
        let (root, db, context, actor) = native_source_tool_fixture(AgentRole::SourceAnalyst);
        let origin: String = db.query_row("SELECT started_at FROM agent_budget_clock_origins WHERE root_run_id=?1",
            [&actor.root_run_id], |r| r.get(0)).unwrap();
        db.execute("UPDATE agent_runs SET started_at=?2 WHERE id=?1", params![actor.root_run_id, stamp]).unwrap();
        let before = crate::scanning::web_mode_test_rows(&db);
        assert!(crate::agent_runtime::multi_agent::budget::clock::remaining(&db, &actor.root_run_id).is_ok());
        assert!(agent_native_source_tool_authority(&db, &context, "repo.read_slice").is_ok(), "{stamp}");
        assert_eq!(db.query_row("SELECT started_at FROM agent_budget_clock_origins WHERE root_run_id=?1",
            [&actor.root_run_id], |r| r.get::<_, String>(0)).unwrap(), origin);
        crate::scanning::web_mode_assert_rows(&db, &before);
        drop(db); fs::remove_dir_all(root).unwrap();
    }
}
