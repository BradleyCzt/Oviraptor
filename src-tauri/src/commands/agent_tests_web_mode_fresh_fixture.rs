// Production positive fixture: actual creator/startup/HMAC/fresh Root, no backfill.
fn multi_agent_new_task_root_for_target(
    name: &str,
    hard_tokens: i64,
    hard_requests: i64,
    target: &str,
) -> (
    PathBuf,
    PathBuf,
    String,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    multi_agent_task_root_for_target_protocol(name, hard_tokens, hard_requests, target, true)
}

// Legacy birth contract for direct Broker/Client tests without a Web SDK loop.
// Kept creation-only: never remove a protocol from an already born Root.
fn multi_agent_task_root_for_target_protocol(
    name: &str,
    hard_tokens: i64,
    hard_requests: i64,
    target: &str,
    new_evidence: bool,
) -> (PathBuf, PathBuf, String, crate::agent_runtime::multi_agent::lease::CoordinatorLease) {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("oviraptor-fresh-{name}-{}", Uuid::new_v4()));
    let path = db::initialize(&root).unwrap();
    let mut db = db::open(&path).unwrap();
    db.execute(
        "INSERT INTO projects(id,name) VALUES(9001,'Actual fresh fixture')",
        [],
    )
    .unwrap();
    let scan = web_mode_test_draft(&db, Some("multi"), target).unwrap();
    web_mode_test_start(&root, &mut db, &scan.id, WebStartMode::Confirm).unwrap();
    let mut plan = test_plan_for("standard", target).with_attempt(1);
    plan.hard_total_tokens = hard_tokens;
    plan.soft_uncached_tokens = hard_tokens.saturating_div(2);
    plan.hard_model_requests = hard_requests;
    plan.soft_model_requests = hard_requests.saturating_div(2);
    if new_evidence {
        persist_frozen_web_execution_plan(&path, &scan.id, 1, target, &plan).unwrap();
    } else {
        let proof = private_web_mode_on(&db, &scan.id, 1).unwrap();
        let mode = crate::agent_runtime::web_mode::root::NewRootModeDeclaration::from_verified(
            &proof, target, &plan.hash(),
        ).unwrap().with_local_deliberation().unwrap()
            .with_original_v5_bootstrap_for_test().unwrap();
        let budget = fresh_web_root_budget_declaration(&proof, target, &plan).unwrap();
        crate::agent_runtime::store::record_attempt_plan_with_declarations(
            &db, &scan.id, 1, target, plan.backend, &plan.hash(), &plan.as_json(),
            Some(&budget), Some(&mode),
        ).unwrap();
    }
    let mut context = test_context(&path, target, vec![]);
    context.scan_id = scan.id.clone();
    context.execution_plan = plan;
    let run = runtime_open_run(&path, &scan.id, &context.route).unwrap();
    let lease = crate::agent_runtime::multi_agent::lease::acquire_coordinator_lease(
        &db,
        &scan.id,
        1,
        target,
        &run.run_id,
        600,
    )
    .unwrap();
    (root, path, run.run_id, lease)
}
fn multi_agent_new_task_root(
    name: &str,
    hard_tokens: i64,
    hard_requests: i64,
) -> (
    PathBuf,
    PathBuf,
    String,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    multi_agent_new_task_root_for_target(
        name,
        hard_tokens,
        hard_requests,
        "https://authorized.example.test",
    )
}

fn web_mode_positive_plan_or(
    path: &Path,
    target: &str,
    fallback: AgentExecutionPlan,
) -> AgentExecutionPlan {
    let Ok(db) = db::open(path) else {
        return fallback;
    };
    let Ok(mut q)=db.prepare("SELECT r.plan_json FROM agent_runs r JOIN agent_root_mode_definitions d ON d.root_run_id=r.id
        WHERE r.target_url=?1 AND r.attempt_number=1 AND r.role='coordinator' AND r.backend='native' ORDER BY r.id LIMIT 2")else {return fallback};
    let Ok(rows) = q.query_map([target], |r| r.get::<_, String>(0)) else {
        return fallback;
    };
    let Ok(rows) = rows.collect::<rusqlite::Result<Vec<_>>>() else {
        return fallback;
    };
    if rows.len() != 1 {
        return fallback;
    }
    serde_json::from_str::<JsonValue>(&rows[0])
        .ok()
        .and_then(|p| AgentExecutionPlan::from_json(&p))
        .unwrap_or(fallback)
}
