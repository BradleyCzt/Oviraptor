#[test]
fn coordinator_tick_expired_or_replaced_original_cannot_publish_paid_decision() {
    for replacement in [false, true] {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(&root_tick_valid_text("original fee")),
            )
        }));
        let mut f = root_tick_fixture(
            "tick-expired-original",
            &format!("http://127.0.0.1:{port}/v1"),
        );
        let db = db::open(&f.context.db_path).unwrap();
        db.execute_batch("CREATE TRIGGER tick_wait_original BEFORE INSERT ON agent_events WHEN NEW.event_type='model_round_completed' AND json_extract(NEW.payload_json,'$.rootTickVersion')=1 BEGIN SELECT RAISE(ABORT,'wait'); END;").unwrap();
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        db.execute_batch("DROP TRIGGER tick_wait_original;")
            .unwrap();
        db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 seconds') WHERE root_run_id=?1",[&f.actor.root_run_id]).unwrap();
        drop(f.parent.take());
        let actor = if replacement {
            crate::agent_runtime::multi_agent::lease::acquire_coordinator_lease(
                &db,
                &f.actor.scan_id,
                1,
                &f.actor.target_key,
                &f.actor.root_run_id,
                600,
            )
            .unwrap()
        } else {
            f.actor.clone()
        };
        if replacement {
            let parent = start_original_fixture_supervisor(
                &f.context.db_path,
                &actor,
            )
            .unwrap();
            f.context.supervision = Some(parent.ticket());
            f.parent = Some(parent);
        }
        let before = web_mode_test_rows(&db);
        assert!(native_coordinator_tick(&f.context, &actor).is_err());
        assert_eq!(seen.lock().unwrap().len(), 1);
        web_mode_assert_rows(&db, &before);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 1);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::balance(
                &db,
                &f.actor.root_run_id,
                None,
                "model_requests"
            )
            .unwrap()
            .consumed,
            1
        );
    }
}

#[test]
fn coordinator_tick_unknown_usage_keeps_original_resources_and_never_retries() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        let mut response: JsonValue = serde_json::from_str(&proposal_model_response(
            &root_tick_valid_text("semantic without bill"),
        ))
        .unwrap();
        response.as_object_mut().unwrap().remove("usage");
        (200, "application/json", response.to_string())
    }));
    let f = root_tick_fixture("tick-unknown-bill", &format!("http://127.0.0.1:{port}/v1"));
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_input_tokens"
        )
        .unwrap()
        .indeterminate
            > 0
    );
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
    let before = web_mode_test_rows(&db);
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    web_mode_assert_rows(&db, &before);
}

#[test]
fn coordinator_tick_last_semantic_write_failure_still_retains_original_invoice() {
    for fault in ["ignore", "collateral"] {
        let sql = if fault == "ignore" {
            "CREATE TRIGGER tick_bad_semantic BEFORE INSERT ON agent_root_tick_receipts WHEN NEW.phase='decision' BEGIN SELECT RAISE(IGNORE); END;"
        } else {
            "CREATE TRIGGER tick_bad_semantic AFTER INSERT ON agent_root_tick_receipts WHEN NEW.phase='decision' BEGIN UPDATE sentinel_scans SET current_checkpoint='collateral' WHERE id IN (SELECT scan_id FROM agent_runs WHERE id=NEW.root_run_id); END;"
        };
        let path = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
        let actual = path.clone();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
            // Original request has committed before real transport reaches us.
            // Install the semantic fault now; the received financial invoice
            // writes another table and must survive the failed semantic write.
            let database = db::open(actual.lock().unwrap().as_ref().unwrap()).unwrap();
            database.execute_batch(sql).unwrap();
            (
                200,
                "application/json",
                proposal_model_response(&root_tick_valid_text("verified before persistence")),
            )
        }));
        let f = root_tick_fixture(
            "tick-semantic-write",
            &format!("http://127.0.0.1:{port}/v1"),
        );
        let db = db::open(&f.context.db_path).unwrap();
        *path.lock().unwrap() = Some(f.context.db_path.clone());
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "{fault}: expected paid transport before fault"
        );
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[&f.actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 0);
        assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sentinel_scans WHERE current_checkpoint='collateral'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::balance(
                &db,
                &f.actor.root_run_id,
                None,
                "model_requests"
            )
            .unwrap()
            .consumed,
            1
        );
        db.execute_batch("DROP TRIGGER tick_bad_semantic;").unwrap();
        let before = web_mode_test_rows(&db);
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        assert_eq!(seen.lock().unwrap().len(), 1);
        web_mode_assert_rows(&db, &before);
    }
}
