// Real v15 prepare: drift only after a committed original Root invoice.
struct MapperSourceDriftScope {
    database: PathBuf,
    root: String,
    scan: String,
    attempt: i64,
    target: String,
    artifact: PathBuf,
    original: Vec<u8>,
    actor: Option<crate::agent_runtime::multi_agent::lease::CoordinatorLease>,
    fired: bool,
}
thread_local! {
    static MAPPER_SOURCE_DRIFT_SCOPE: std::cell::RefCell<Option<MapperSourceDriftScope>> = const { std::cell::RefCell::new(None) };
}
struct MapperSourceDriftGuard;
impl MapperSourceDriftGuard {
    fn install(f: &RootTickFixture) -> Self {
        let artifact = f.context.target_dir.join("frontend-evidence.json");
        MAPPER_SOURCE_DRIFT_SCOPE.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(slot.is_none());
            *slot = Some(MapperSourceDriftScope {
                database: f.context.db_path.clone(),
                root: f.actor.root_run_id.clone(),
                scan: f.context.scan_id.clone(),
                attempt: f.context.attempt_number,
                target: f.context.target_url.clone(),
                original: std::fs::read(&artifact).unwrap(),
                artifact,
                actor: None,
                fired: false,
            });
        });
        Self
    }
    fn restore(&self) -> crate::agent_runtime::multi_agent::lease::CoordinatorLease {
        MAPPER_SOURCE_DRIFT_SCOPE.with(|slot| {
            let slot = slot.borrow();
            let scope = slot.as_ref().unwrap();
            assert!(scope.fired);
            std::fs::write(&scope.artifact, &scope.original).unwrap();
            scope.actor.clone().unwrap()
        })
    }
}
impl Drop for MapperSourceDriftGuard {
    fn drop(&mut self) {
        MAPPER_SOURCE_DRIFT_SCOPE.with(|slot| {
            if let Some(scope) = slot.borrow_mut().take() {
                std::fs::write(scope.artifact, scope.original).unwrap();
            }
        });
    }
}
pub(super) fn native_mapper_source_drift_post_original_invoice_checkpoint(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    round: i64,
) {
    MAPPER_SOURCE_DRIFT_SCOPE.with(|slot| {
        let mut slot=slot.borrow_mut(); let Some(scope)=slot.as_mut() else { return; };
        if scope.fired || context.db_path!=scope.database || actor.root_run_id!=scope.root || actor.scan_id!=scope.scan || actor.attempt_number!=scope.attempt || actor.target_key!=scope.target || context.scan_id!=scope.scan || context.attempt_number!=scope.attempt || context.target_url!=scope.target { return; }
        let db=db::open(&scope.database).unwrap();
        let phase:Option<String>=db.query_row("SELECT json_extract(fact_json,'$.request.basis.phase') FROM agent_root_tick_receipts WHERE root_run_id=?1 AND round=?2 AND phase='request'",params![scope.root,round],|r|r.get(0)).unwrap();
        if phase.as_deref()!=Some("mapper-output") { return; }
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND round=?2 AND phase='received'",params![scope.root,round],|r|r.get::<_,i64>(0)).unwrap(),1);
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(&db,&scope.root).unwrap().require_original_coordinator(&db,actor).unwrap();
        scope.actor=Some(actor.clone()); scope.fired=true;
        std::fs::write(&scope.artifact,b"{\"url\":\"https://authorized.example.test\",\"changedAfterPaidInvoice\":true}").unwrap();
    });
}
#[test]
fn mapper_loop_source_drift_after_actual_root_invoice_keeps_fee_and_denies_executor_without_resending(
) {
    use std::sync::Arc;
    let (port, seen, _stop) = spawn_endpoint(Arc::new(|request| {
        let wire: JsonValue =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        if wire["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("You are the Root Coordinator")
        {
            let input: JsonValue =
                serde_json::from_str(wire["messages"][1]["content"].as_str().unwrap()).unwrap();
            let step = match input["basis"]["phase"].as_str().unwrap() {
                "bootstrap" => "dispatch:spa_api_mapper",
                "mapper-output" => "dispatch:web_executor",
                phase => panic!("unexpected Root phase {phase}"),
            };
            (200,"application/json",proposal_model_response(&json!({"schemaVersion":1,"observed":["original frozen source"],"missing":[],"suggestions":[step],"costNotes":[],"risks":[]}).to_string()))
        } else {
            assert_eq!(wire["max_tokens"], 512);
            (
                200,
                "application/json",
                sdk_exact_model_body(
                    r#"{"summary":"original frozen frontend only","priorityContracts":[],"risks":[]}"#,
                ),
            )
        }
    }));
    let mut f = root_tick_fixture_from_web_mode_evidence_protocol(
        "mapper-loop-source-post-invoice-drift",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (60_000, 20),
        None,
        web_mode_fixture("multi", "https://authorized.example.test"),
        RootFixtureEvidence {
            mapper_v15: true,
            ..Default::default()
        },
    );
    let guard = MapperSourceDriftGuard::install(&f);
    drop(f.parent.take());
    assert!(
        multi_agent_prepare(&mut f.context).is_err(),
        "changed original source must refuse paid dispatch"
    );
    let actor = guard.restore();
    let db = db::open(&f.context.db_path).unwrap();
    let child=crate::agent_runtime::multi_agent::scheduler::ScheduledChild{assignment_id:db.query_row("SELECT id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='spa_api_mapper'",[&actor.root_run_id],|r|r.get(0)).unwrap(),run_id:db.query_row("SELECT child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='spa_api_mapper'",[&actor.root_run_id],|r|r.get(0)).unwrap(),role:crate::agent_runtime::contract::AgentRole::SpaApiMapper};
    assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='web_executor'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts p JOIN agent_root_tick_receipts q ON q.root_run_id=p.root_run_id AND q.round=p.round AND q.phase='request' WHERE p.root_run_id=?1 AND p.phase='publication' AND json_extract(q.fact_json,'$.request.basis.phase')='mapper-output'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    let round:i64=db.query_row("SELECT round FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request' AND json_extract(fact_json,'$.request.basis.phase')='mapper-output'",[&actor.root_run_id],|r|r.get(0)).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND round=?2 AND phase='received'",params![actor.root_run_id,round],|r|r.get::<_,i64>(0)).unwrap(),1);
    let paid:i64=db.query_row("SELECT COALESCE(sum(amount),0) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id='' AND dimension='model_requests' AND kind='consume'",[&actor.root_run_id],|r|r.get(0)).unwrap();
    assert_eq!(
        paid, 2,
        "bootstrap and rejected Mapper evaluation remain paid"
    );
    let tx = db.unchecked_transaction().unwrap();
    crate::agent_runtime::multi_agent::specialist::mapper_loop::final_seal::read_final_original_on(
        &tx, &actor, &child,
    )
    .unwrap()
    .expect("restoring exact bytes preserves the paid signed Mapper source");
    drop(tx);
    let count = seen.lock().unwrap().len();
    assert_eq!(count, 3, "two original Root SDKs and one Mapper SDK");
    let root = native_coordinator_root_context(&f.context, &actor);
    let replay = native_coordinator_mapper_decision(&root, &actor, &child);
    if let Ok((_, receipt)) = replay {
        assert!(receipt.replayed);
    }
    assert_eq!(
        seen.lock().unwrap().len(),
        count,
        "restored original source never resends paid SDK"
    );
    assert_eq!(db.query_row("SELECT COALESCE(sum(amount),0) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id='' AND dimension='model_requests' AND kind='consume'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),paid);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='web_executor'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
}
