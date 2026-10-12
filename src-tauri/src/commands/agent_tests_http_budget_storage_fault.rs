// Real localhost headers, production private writer, conservative rollback.
// Extra trigger is rejected by exact schema BEFORE receipt INSERT; this is not
// evidence of a natural SQLite disk/INSERT failure. No production fault switch.
type HttpBudgetStorageFaultApplied = std::sync::Arc<std::sync::Mutex<Option<Result<(), String>>>>;
fn http_budget_storage_fault_fixture(
    mode: u8,
) -> (BudgetTriggerFixture, HttpBudgetStorageFaultApplied) {
    let fault_path = std::sync::Arc::new(std::sync::Mutex::new(None::<std::path::PathBuf>));
    let site_fault = fault_path.clone();
    let applied: HttpBudgetStorageFaultApplied = std::sync::Arc::new(std::sync::Mutex::new(None));
    let site_applied = applied.clone();
    let mut h = fresh_multi_production_harness_with_site(
        "http-budget-storage-fault",
        move |request| {
            if let Some(path) = site_fault.lock().unwrap().take() {
                let result = (|| -> Result<(), String> {
                    let db = rusqlite::Connection::open_with_flags(
                        path,
                        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
                    )
                    .map_err(|e| e.to_string())?;
                    db.busy_timeout(std::time::Duration::from_secs(5))
                        .map_err(|e| e.to_string())?;
                    if mode == 1 {
                        db.execute_batch("CREATE TRIGGER http_budget_untrusted_insert BEFORE INSERT ON agent_http_budget_receipts BEGIN SELECT RAISE(ABORT,'injected_receipt_insert'); END;")
                            .map_err(|e| e.to_string())?;
                    } else {
                        // Cold damage in this disposable fixture only. Inventory
                        // exact original immutable guards, then restore every
                        // definition in the same transaction after damaging one row.
                        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
                        let guards: Vec<(String, String)> = {
                            let mut q = tx.prepare("SELECT name,sql FROM main.sqlite_master WHERE type='trigger' AND tbl_name='agent_root_mode_birth_seals' ORDER BY name")
                                .map_err(|e| e.to_string())?;
                            let rows = q
                                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                                .map_err(|e| e.to_string())?
                                .collect::<rusqlite::Result<_>>()
                                .map_err(|e| e.to_string())?;
                            rows
                        };
                        if guards.is_empty() {
                            return Err("original birth immutable guards missing".into());
                        }
                        for (name, _) in &guards {
                            tx.execute_batch(&format!(
                                "DROP TRIGGER main.\"{}\"",
                                name.replace('"', "\"\"")
                            ))
                            .map_err(|e| e.to_string())?;
                        }
                        let changed = tx
                            .execute(
                                "UPDATE agent_root_mode_birth_seals SET signature=zeroblob(32)",
                                [],
                            )
                            .map_err(|e| e.to_string())?;
                        if changed != 1 {
                            return Err(format!(
                                "expected one original birth row, changed {changed}"
                            ));
                        }
                        for (_, sql) in &guards {
                            tx.execute_batch(sql).map_err(|e| e.to_string())?;
                        }
                        let after: Vec<(String, String)> = {
                            let mut q = tx.prepare("SELECT name,sql FROM main.sqlite_master WHERE type='trigger' AND tbl_name='agent_root_mode_birth_seals' ORDER BY name")
                                .map_err(|e| e.to_string())?;
                            let rows = q
                                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                                .map_err(|e| e.to_string())?
                                .collect::<rusqlite::Result<_>>()
                                .map_err(|e| e.to_string())?;
                            rows
                        };
                        if after != guards {
                            return Err("original birth immutable guards changed".into());
                        }
                        tx.commit().map_err(|e| e.to_string())?;
                    }
                    Ok(())
                })();
                // Keep the actual server returning HTTP headers even if fixture
                // setup fails; the main test reports this exact fault error.
                *site_applied.lock().unwrap() = Some(result);
            }
            // Server returns actual successful headers only after the fault is in place.
            mock_site(request)
        },
    );
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|wire| {
        let response = if wire.contains("You are the Root Coordinator")
            && wire.contains("budget-consumption")
        {
            proposal_model_response(
                &json!({"schemaVersion":1,"observed":["original paid Web bill"],
                "missing":[],"suggestions":["assess:budget_consumption"],"costNotes":[],"risks":[]})
                .to_string(),
            )
        } else if let Some(response) = fresh_multi_root_wire_response(&wire) {
            response
        } else if wire.contains("SPA/API Mapper") {
            proposal_model_response(
                r#"{"summary":"original actual Mapper","priorityContracts":[],"risks":[]}"#,
            )
        } else {
            model_round(&[], 10)
        };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    h.context.execution_plan.hard_total_tokens = 60000;
    h.context.execution_plan.soft_uncached_tokens = 30000;
    h.context.execution_plan.hard_model_requests = 20;
    h.context.execution_plan.soft_model_requests = 10;
    h.context.execution_plan.max_turns = 8;
    freeze_fresh_multi_production_harness_http_v8(&mut h);
    let session = multi_agent_prepare(&mut h.context).unwrap();
    let f = BudgetTriggerFixture {
        h,
        session: Some(session),
        boundary: std::sync::Arc::new(std::sync::Mutex::new(None)),
        stop,
    };
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    let client = AgentModelClient::new(
        agent_model_profile(&f.h.context.environment, None).unwrap(),
        &[],
    );
    let reply = native_model_transport(
        &f.h.context,
        &client,
        vec![json!({"role":"user","content":"original Web call before faulty actual HTTP"})],
        &[],
        1,
    )
    .unwrap_or_else(|_| panic!("original actual SDK must settle"));
    f.h.context
        .run
        .as_ref()
        .unwrap()
        .model_round(1, &reply.usage, &["replay_http".into()])
        .unwrap();
    native_coordinator_budget_consumption_before_action(&f.h.context).unwrap();
    *fault_path.lock().unwrap() = Some(f.h.db_path.clone());
    (f, applied)
}

fn http_budget_storage_fault_actual_http(
    f: &BudgetTriggerFixture,
    runtime: &mut AgentToolRuntime,
) -> ToolExecutionResult {
    let args = json!({"identity":"anonymous","method":"GET",
        "url":format!("{}/", f.h.context.target_url),"family":"authorization"});
    let invocation = runtime.begin_invocation(1);
    f.h.context
        .run
        .as_ref()
        .unwrap()
        .begin_tool("replay_http", &args, &invocation)
        .unwrap();
    agent_execute_tool_at(&f.h.context, runtime, "replay_http", &args, &invocation)
}

fn http_budget_storage_fault_assert_unresolved(
    f: &BudgetTriggerFixture,
    db: &rusqlite::Connection,
) {
    let child = &f.session.as_ref().unwrap().executor;
    assert_eq!(
        f.h.site_seen.lock().unwrap().len(),
        1,
        "actual server returned headers once"
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_http_request_claims WHERE run_id=?1 AND response_status=0 AND received_at=''",
        [&child.run_id], |r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_http_request_claims WHERE run_id=?1 AND response_status>0",
            [&child.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let fees: Vec<(String,i64)> = db.prepare("SELECT kind,amount FROM agent_budget_entries WHERE assignment_id=?1 AND dimension='target_requests' ORDER BY rowid").unwrap()
        .query_map([&child.assignment_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(fees,vec![("reserve".into(),1),("forfeit".into(),1)], "known headers did not survive failed atomic seal; original cost stays unresolved, not settled");
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_http_budget_receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts WHERE phase='request' AND json_extract(fact_json,'$.request.basis.phase')='http-budget-consumption'",
        [],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result'",
        [&child.assignment_id],|r|r.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn http_budget_storage_fault_untrusted_insert_trigger_after_real_headers_keeps_original_debt_without_retry(
) {
    let _real = RealSpecialistTransport::enter();
    let (f, applied) = http_budget_storage_fault_fixture(1);
    let db = db::open(&f.h.db_path).unwrap();
    let child = &f.session.as_ref().unwrap().executor;
    let owners = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
        &db,
        "SELECT rowid,* FROM agent_assignment_attempts WHERE child_run_id=?1",
        [&child.run_id],
    )
    .unwrap();
    let mut runtime = AgentToolRuntime::default();
    let reply = http_budget_storage_fault_actual_http(&f, &mut runtime);
    assert_eq!(
        applied.lock().unwrap().as_ref(),
        Some(&Ok(())),
        "actual site fault must be applied before headers"
    );
    assert_eq!(
        value_first(&reply.model_view, &["code"]),
        "evidence_write_failed"
    );
    assert!(reply.model_view["error"]
        .as_str()
        .unwrap()
        .contains("http_budget_receipt_schema_conflict"));
    http_budget_storage_fault_assert_unresolved(&f, &db);
    assert!(
        crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
            &db,
            "SELECT rowid,* FROM agent_assignment_attempts WHERE child_run_id=?1",
            [&child.run_id]
        )
        .unwrap()
            == owners
    );
    db.execute_batch("DROP TRIGGER http_budget_untrusted_insert")
        .unwrap();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let again = http_budget_storage_fault_actual_http(&f, &mut runtime);
    assert!(!value_first(&again.model_view, &["code"]).is_empty());
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(
        native_coordinator_http_budget_consumption_before_action(&f.h.context).unwrap_err(),
        "budget_indeterminate_requires_reconciliation"
    );
    http_budget_storage_fault_assert_unresolved(&f, &db);
}

#[test]
fn http_budget_storage_fault_cold_original_birth_signature_after_real_headers_cannot_issue_seal_or_replay(
) {
    let _real = RealSpecialistTransport::enter();
    let (f, applied) = http_budget_storage_fault_fixture(2);
    let db = db::open(&f.h.db_path).unwrap();
    let child = &f.session.as_ref().unwrap().executor;
    let owners = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
        &db,
        "SELECT rowid,* FROM agent_assignment_attempts WHERE child_run_id=?1",
        [&child.run_id],
    )
    .unwrap();
    let mut runtime = AgentToolRuntime::default();
    let reply = http_budget_storage_fault_actual_http(&f, &mut runtime);
    assert_eq!(
        applied.lock().unwrap().as_ref(),
        Some(&Ok(())),
        "actual site fault must be applied before headers"
    );
    assert_eq!(
        value_first(&reply.model_view, &["code"]),
        "evidence_write_failed"
    );
    assert!(reply.model_view["error"]
        .as_str()
        .unwrap()
        .contains("root_mode_birth_signature_invalid"));
    http_budget_storage_fault_assert_unresolved(&f, &db);
    assert!(
        crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
            &db,
            "SELECT rowid,* FROM agent_assignment_attempts WHERE child_run_id=?1",
            [&child.run_id]
        )
        .unwrap()
            == owners
    );
    let sdk = f.h.model_seen.lock().unwrap().len();
    let again = http_budget_storage_fault_actual_http(&f, &mut runtime);
    assert!(!value_first(&again.model_view, &["code"]).is_empty());
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    http_budget_storage_fault_assert_unresolved(&f, &db);
}
