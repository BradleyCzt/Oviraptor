#[test]
fn source_financial_exit_actual_sdk_stop_preserves_original_fee_and_only_observes_elapsed() {
    use crate::agent_runtime::multi_agent::budget;
    use std::sync::{atomic::Ordering, mpsc, Arc, Mutex};
    for stop_kind in ["scan_pause", "attempt_rotate", "coordinator_replace"] {
        let (arrive, arrived) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let released = Arc::new(Mutex::new(released));
        let (port, seen, stop) = crate::scanning::agent_tests::spawn_endpoint(Arc::new(
            move |_| {
                arrive.send(()).unwrap();
                let _ = released
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10));
                (200,"application/json",json!({"choices":[{"message":{"role":"assistant","content":"late Source response must not grant work"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
            },
        ));
        let (root, db, record) =
            source_dispatch_fixture(&source_specialist_test_environment(port), None);
        analysis_view_run(&root, &record, |_, engine, _, _, scratch, _| {
            Ok(source_regression_outcome(engine, scratch))
        })
        .unwrap();
        let work = root.join("attempt-0001");
        let path = root.join("oviraptor.sqlite3");
        let registered = prepare_native_source_coordinator(&db, &record.scan_id, 1, &work).unwrap();
        let c =
            native_source_fresh_finance::original_for_execution(&db, &registered.run_id).unwrap();
        let plan = NativeSourcePlan::load(&db, &record.scan_id, 1).unwrap();
        let wall = source_exit_wall(&db, &c.root_run_id);
        let result = std::thread::scope(|scope| {
            let (done, finished) = mpsc::channel();
            let (path, work, scan) = (&path, &work, &record.scan_id);
            scope.spawn(move || {
                let _ = done.send(run_native_source_assessments(path, scan, 1, work));
            });
            arrived
                .recv_timeout(Duration::from_secs(5))
                .expect("actual original Source SDK must arrive");
            std::thread::sleep(Duration::from_millis(50));
            match stop_kind {
                "scan_pause" => {
                    assert_eq!(request_sentinel_pause(path, &record.scan_id).unwrap(), 1);
                }
                "attempt_rotate" => {
                    db.execute(
                        "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
                        [&record.scan_id],
                    )
                    .unwrap();
                }
                "coordinator_replace" => {
                    db.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token=?2 WHERE root_run_id=?1",params![c.root_run_id,Uuid::new_v4().to_string()]).unwrap();
                }
                _ => unreachable!(),
            }
            let result = finished.recv_timeout(Duration::from_secs(3));
            let _ = release.send(());
            result.expect("stop must cancel actual SDK before the provider responds")
        });
        assert!(result.is_err(), "{stop_kind}: {result:?}");
        // The endpoint journals after releasing its delayed response. Await
        // that actual late delivery, then prove it cannot change any DB row.
        let before_late = source_exit_snapshot(&db);
        let drained = std::time::Instant::now() + Duration::from_secs(2);
        while seen.lock().unwrap().is_empty() {
            assert!(
                std::time::Instant::now() < drained,
                "endpoint did not finish: {stop_kind}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            source_exit_snapshot(&db),
            before_late,
            "late response changed stopped work: {stop_kind}"
        );
        assert!(
            arrived.try_recv().is_err(),
            "unexpected second SDK: {stop_kind}"
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "no replay or replacement SDK: {stop_kind}"
        );
        let cost = budget::balance(&db, &c.root_run_id, None, "model_requests").unwrap();
        assert_eq!(
            cost.consumed + cost.indeterminate + cost.reserved,
            1,
            "original uncertain fee survives: {stop_kind}"
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_messages WHERE root_run_id=?1",
                [&c.root_run_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_ne!(
            db.query_row(
                "SELECT status FROM agent_runs WHERE id=?1",
                [&c.root_run_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "terminal"
        );
        assert_eq!(
            NativeSourcePlan::load(&db, &record.scan_id, 1).unwrap(),
            plan
        );
        if stop_kind == "scan_pause" {
            assert!(finish_sentinel_pause(&path, &record.scan_id, 1).unwrap());
            assert_eq!(
                db.query_row(
                    "SELECT status FROM sentinel_scans WHERE id=?1",
                    [&record.scan_id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "paused"
            );
        }
        if stop_kind == "coordinator_replace" {
            source_exit_require_fact(&db, &c, false);
        } else {
            assert!(
                source_exit_wall(&db, &c.root_run_id) > wall,
                "known elapsed lost: {stop_kind}"
            );
        }
        stop.store(true, Ordering::SeqCst);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
