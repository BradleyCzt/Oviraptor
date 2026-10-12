// A truly new Web creator/HMAC/Root/C/parent fixture; no old authority backfill.
struct RootTickFixture {
    parent: Option<crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor>,
    context: AgentRunContext,
    actor: crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    f: WebModeFixture,
}
#[derive(Default)]
struct RootFixtureEvidence<'a> {
    max_turns: Option<i64>,
    evidence_patch: Option<&'a JsonValue>,
    original_v5: bool,
    mapper_v15: bool,
}
fn root_tick_fixture(tag: &str, model_endpoint: &str) -> RootTickFixture {
    root_tick_fixture_protocol(tag, model_endpoint, true)
}
fn root_tick_fixture_protocol(tag: &str, model_endpoint: &str, local: bool) -> RootTickFixture {
    root_tick_fixture_protocol_limits(tag, model_endpoint, local, (60_000, 20))
}
fn root_tick_fixture_protocol_limits(
    tag: &str,
    model_endpoint: &str,
    local: bool,
    limits: (i64, i64),
) -> RootTickFixture {
    root_tick_fixture_protocol_limits_timeout(tag, model_endpoint, local, limits, None)
}
fn root_tick_fixture_original_v5_limits(
    tag: &str,
    model_endpoint: &str,
    limits: (i64, i64),
) -> RootTickFixture {
    let f = web_mode_fixture("multi", "https://authorized.example.test");
    root_tick_fixture_from_web_mode_evidence_protocol(
        tag,
        model_endpoint,
        true,
        limits,
        None,
        f,
        RootFixtureEvidence {
            original_v5: true,
            ..Default::default()
        },
    )
}
fn root_tick_fixture_protocol_limits_timeout(
    tag: &str,
    model_endpoint: &str,
    local: bool,
    limits: (i64, i64),
    timeout: Option<u64>,
) -> RootTickFixture {
    let f = web_mode_fixture("multi", "https://authorized.example.test");
    root_tick_fixture_from_web_mode(tag, model_endpoint, local, limits, timeout, f)
}
fn root_tick_fixture_from_web_mode(
    tag: &str,
    model_endpoint: &str,
    local: bool,
    limits: (i64, i64),
    timeout: Option<u64>,
    f: WebModeFixture,
) -> RootTickFixture {
    root_tick_fixture_from_web_mode_turns(tag, model_endpoint, local, limits, timeout, f, None)
}
fn root_tick_fixture_from_web_mode_turns(
    tag: &str,
    model_endpoint: &str,
    local: bool,
    limits: (i64, i64),
    timeout: Option<u64>,
    f: WebModeFixture,
    max_turns: Option<i64>,
) -> RootTickFixture {
    root_tick_fixture_from_web_mode_evidence(
        tag,
        model_endpoint,
        local,
        limits,
        timeout,
        f,
        RootFixtureEvidence {
            max_turns,
            ..Default::default()
        },
    )
}
fn root_tick_fixture_from_web_mode_evidence(
    tag: &str,
    model_endpoint: &str,
    local: bool,
    limits: (i64, i64),
    timeout: Option<u64>,
    f: WebModeFixture,
    evidence: RootFixtureEvidence<'_>,
) -> RootTickFixture {
    root_tick_fixture_from_web_mode_evidence_protocol(
        tag,
        model_endpoint,
        local,
        limits,
        timeout,
        f,
        evidence,
    )
}
fn root_tick_fixture_from_web_mode_evidence_protocol(
    tag: &str,
    model_endpoint: &str,
    local: bool,
    limits: (i64, i64),
    timeout: Option<u64>,
    f: WebModeFixture,
    evidence: RootFixtureEvidence<'_>,
) -> RootTickFixture {
    let RootFixtureEvidence {
        max_turns,
        evidence_patch,
        original_v5,
        mapper_v15,
    } = evidence;
    assert!(
        !(original_v5 && mapper_v15),
        "one original creation protocol"
    );
    let mut context = test_context(&f.path, &f.target, vec![AgentIdentity::anonymous()]);
    if let Some(patch) = evidence_patch {
        for (key, value) in patch.as_object().expect("fixture evidence object") {
            context
                .evidence
                .as_object_mut()
                .unwrap()
                .insert(key.clone(), value.clone());
        }
    }
    context.scan_id = f.scan.clone();
    context.target_dir = f.work.join("url-pipeline/target-00001");
    fs::create_dir_all(&context.target_dir).unwrap();
    fs::write(context.target_dir.join(".oviraptor-scan-id"), &f.scan).unwrap();
    fs::write(
        context.target_dir.join("frontend-evidence.json"),
        context.evidence.to_string(),
    )
    .unwrap();
    context.log_path = f.root.join(format!("{tag}.log"));
    context.environment.api_base = model_endpoint.into();
    if let Some(timeout) = timeout {
        context.execution_plan.timeout_seconds = timeout;
    }
    if let Some(turns) = max_turns {
        context.execution_plan.max_turns = turns;
    }
    context.execution_plan.hard_total_tokens = limits.0;
    context.execution_plan.soft_uncached_tokens = limits.0.min(30_000);
    context.execution_plan.hard_model_requests = limits.1;
    context.execution_plan.soft_model_requests = limits.1.min(10);
    context.execution_plan = native_frozen_tool_supply_prebirth_fixture_plan_for_test(
        &f.path, &f.scan, 1, &f.target, &context.execution_plan,
    ).unwrap();
    // All native ceilings/evidence/model input exist before the real first
    // Root INSERT. This fixture cannot relabel or re-sign an existing Root.
    let db = db::open(&f.path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_runs WHERE scan_id=?1",
            [&f.scan],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    if mapper_v15 {
        let proof = private_web_mode_on(&db, &f.scan, 1).unwrap();
        let mode = crate::agent_runtime::web_mode::root::NewRootModeDeclaration::from_verified(
            &proof,
            &f.target,
            &context.execution_plan.hash(),
        )
        .unwrap()
        .with_local_deliberation()
        .unwrap()
        .with_mapper_v15_bootstrap_for_test()
        .unwrap()
        .with_new_evidence_supervision()
        .unwrap()
        .with_lease_expiry_supervision()
        .unwrap()
        .with_protection_supervision()
        .unwrap();
        let budget =
            fresh_web_root_budget_declaration(&proof, &f.target, &context.execution_plan).unwrap();
        crate::agent_runtime::store::record_attempt_plan_with_declarations(
            &db,
            &f.scan,
            1,
            &f.target,
            context.execution_plan.backend,
            &context.execution_plan.hash(),
            &context.execution_plan.as_json(),
            Some(&budget),
            Some(&mode),
        )
        .unwrap();
    } else if original_v5 {
        let proof = private_web_mode_on(&db, &f.scan, 1).unwrap();
        let mode = crate::agent_runtime::web_mode::root::NewRootModeDeclaration::from_verified(
            &proof,
            &f.target,
            &context.execution_plan.hash(),
        )
        .unwrap()
        .with_local_deliberation()
        .unwrap()
        .with_original_v5_bootstrap_for_test()
        .unwrap()
        .with_new_evidence_supervision()
        .unwrap()
        .with_lease_expiry_supervision()
        .unwrap()
        .with_protection_supervision()
        .unwrap();
        let budget =
            fresh_web_root_budget_declaration(&proof, &f.target, &context.execution_plan).unwrap();
        crate::agent_runtime::store::record_attempt_plan_with_declarations(
            &db,
            &f.scan,
            1,
            &f.target,
            context.execution_plan.backend,
            &context.execution_plan.hash(),
            &context.execution_plan.as_json(),
            Some(&budget),
            Some(&mode),
        )
        .unwrap();
    } else if local {
        persist_frozen_web_execution_plan(&f.path, &f.scan, 1, &f.target, &context.execution_plan)
            .unwrap();
    } else {
        let proof = private_web_mode_on(&db, &f.scan, 1).unwrap();
        let mode = crate::agent_runtime::web_mode::root::NewRootModeDeclaration::from_verified(
            &proof,
            &f.target,
            &context.execution_plan.hash(),
        )
        .unwrap();
        let budget =
            fresh_web_root_budget_declaration(&proof, &f.target, &context.execution_plan).unwrap();
        crate::agent_runtime::store::record_attempt_plan_with_declarations(
            &db,
            &f.scan,
            1,
            &f.target,
            context.execution_plan.backend,
            &context.execution_plan.hash(),
            &context.execution_plan.as_json(),
            Some(&budget),
            Some(&mode),
        )
        .unwrap();
    }
    let run = runtime_open_run(&f.path, &f.scan, &context.route).unwrap();
    context.run = Some(run.clone());
    // Pure read of the C1 already born with its original control/10 ceilings.
    // No acquire, extra ledger INSERT, initializer or cfg(test) finance issuer.
    let actor=db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at
        FROM agent_coordinator_leases WHERE root_run_id=?1",[&run.run_id],|r|Ok(crate::agent_runtime::multi_agent::lease::CoordinatorLease {
            scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,
            lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?,
        })).unwrap();
    assert_eq!(actor.lease_epoch, 1);
    assert!(Uuid::parse_str(&actor.fencing_token).is_ok());
    crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(&db, &run.run_id)
        .unwrap()
        .require_original_coordinator(&db, &actor)
        .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
            [&run.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        10
    );
    assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_clock_origins o JOIN agent_runs r ON r.id=o.root_run_id
        WHERE r.id=?1 AND r.created_at=o.started_at)",[&run.run_id],|r|r.get::<_,bool>(0)).unwrap());
    let original_plan: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&run.run_id],
            |r| r.get(0),
        )
        .unwrap();
    let original_value: JsonValue = serde_json::from_str(&original_plan).unwrap();
    if original_value.get("toolSupply").is_some() {
        // The scoped test request adds only its typed supply selection before
        // birth. Every other original input must still equal this fixture.
        let mut without_supply = original_value.clone();
        without_supply.as_object_mut().unwrap().remove("toolSupply");
        assert_eq!(without_supply, context.execution_plan.as_json());
        context.execution_plan = AgentExecutionPlan::from_json(&original_value).unwrap();
    }
    assert_eq!(original_plan, context.execution_plan.as_json().to_string());
    bind_agent_evidence_location(&context).unwrap();
    let parent = start_original_fixture_supervisor(&f.path, &actor).unwrap();
    context.supervision = Some(parent.ticket());
    let original_rows = web_mode_test_rows(&db);
    native_coordinator_initialize_control(&context, &actor).unwrap();
    web_mode_assert_rows(&db, &original_rows); // Pure wrapper must not mint authority.
    assert_eq!(
        fs::read_to_string(context.target_dir.join("frontend-evidence.json")).unwrap(),
        context.evidence.to_string()
    );
    RootTickFixture {
        parent: Some(parent),
        context,
        actor,
        f,
    }
}
fn root_tick_valid_text(mark: &str) -> String {
    json!({"schemaVersion":1,"observed":[mark],"missing":[],"suggestions":[],"costNotes":[],"risks":[]}).to_string()
}
fn root_tick_count(db: &rusqlite::Connection, root: &str, phase: &str) -> i64 {
    db.query_row(
        "SELECT count(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase=?2",
        params![root, phase],
        |r| r.get(0),
    )
    .unwrap()
}
fn root_tick_write_probe(f: &RootTickFixture, path: &Path) {
    let c = &f.context;
    fs::write(path,json!({"root":f.actor.root_run_id,"db":c.db_path,"scan":c.scan_id,"target":c.target_url,
        "targetDir":c.target_dir,"log":c.log_path,"evidence":c.evidence,"capabilities":c.capabilities,
        "apiBase":c.environment.api_base,"llm":c.environment.llm,"deployment":c.environment.deployment,
        "epoch":f.actor.lease_epoch,"fence":f.actor.fencing_token}).to_string()).unwrap();
}
fn root_tick_reload_probe(
    path: &Path,
) -> (
    AgentRunContext,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    let v: JsonValue = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let text = |key: &str| v[key].as_str().unwrap();
    let db_path = PathBuf::from(text("db"));
    let db = db::open(&db_path).unwrap();
    // Read the original current row only to compare; do not acquire or adopt it.
    let actor=db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at
        FROM agent_coordinator_leases WHERE root_run_id=?1",[text("root")],|r|Ok(crate::agent_runtime::multi_agent::lease::CoordinatorLease {
            scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,
            lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?})).unwrap();
    assert_eq!(actor.lease_epoch, v["epoch"].as_i64().unwrap());
    assert_eq!(actor.fencing_token, text("fence"));
    let mut c = test_context(&db_path, text("target"), vec![AgentIdentity::anonymous()]);
    c.scan_id = text("scan").into();
    c.target_dir = PathBuf::from(text("targetDir"));
    c.log_path = PathBuf::from(text("log"));
    c.evidence = v["evidence"].clone();
    c.capabilities = v["capabilities"].clone();
    c.environment.api_base = text("apiBase").into();
    c.environment.llm = text("llm").into();
    c.environment.deployment = text("deployment").into();
    let plan: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [text("root")],
            |r| r.get(0),
        )
        .unwrap();
    c.execution_plan =
        AgentExecutionPlan::from_json(&serde_json::from_str(&plan).unwrap()).unwrap();
    c.run = Some(AgentRunLedger {
        db_path: db_path.clone(),
        run_id: text("root").into(),
    });
    native_frozen_web_root_mode_on(&db, &c).unwrap();
    let owner = crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
        &db,
        text("root"),
    )
    .unwrap();
    owner.require_original_coordinator(&db, &actor).unwrap();
    (c, actor)
}

// Uses the actual private issuer bridge; never fabricates a signed worker.
fn start_original_fixture_supervisor(
    path: &Path,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor, String> {
    use crate::agent_runtime::multi_agent::{
        parent_invocation_owner::ParentInvocationOwner, supervisor::WorkerSupervisor,
    };
    let db = db::open(path)?;
    match original_withdrawal_issuer_on(&db, actor)? {
        Some(issuer) => {
            let owner = ParentInvocationOwner::claim(
                path,
                &actor.scan_id,
                actor.attempt_number,
                &actor.root_run_id,
            )?;
            WorkerSupervisor::start_owned_signed(path, actor, owner, issuer)
        }
        None => WorkerSupervisor::start(path, actor),
    }
}
