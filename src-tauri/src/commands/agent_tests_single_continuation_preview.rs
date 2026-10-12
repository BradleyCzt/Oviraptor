// Read-only proposal evidence from an actual paid localhost Single source.
#[test]
fn single_continuation_preview_actual_paid_source_is_readonly_and_not_authority() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    let (f, context, seen) = single_model_inflight_paid();
    let db = db::open(&f.path).unwrap();
    let root = &context.run.as_ref().unwrap().run_id;
    RootOwner::load_single(&db, root).unwrap().close_single_finance(&db).unwrap();
    assert_eq!(db.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&f.scan]).unwrap(), 1);
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(native_single_continuation_preview_in(&db, &f.scan, 1, root).unwrap_err(),
        "single_continuation_requires_snapshot");
    let snapshot = db.unchecked_transaction().unwrap();
    let proposal = native_single_continuation_preview_in(&snapshot, &f.scan, 1, root).unwrap();
    assert_eq!(proposal["schemaVersion"], 1);
    assert_eq!(proposal["purpose"], "single_resume_proposal_readonly");
    assert_eq!(proposal["scanId"], f.scan);
    assert_eq!(proposal["attemptNumber"], 1);
    assert_eq!(proposal["rootRunId"], *root);
    assert_eq!(proposal["previousStatus"], "paused");
    for field in ["executionAllowed", "approvalImplemented", "birthImplemented"] {
        assert_eq!(proposal[field], false, "{field}");
    }
    assert_eq!(proposal["approvalBlocked"], true);
    assert_eq!(proposal["modelContract"]["modelConfigurationState"], "unavailable");
    assert_eq!(proposal["modelContract"]["modelCandidates"], json!([]));
    assert_eq!(proposal["modelContract"]["approvalBlocked"], true);
    assert_eq!(proposal["modelContract"]["executionAllowed"], false);
    assert_eq!(proposal["sourceHistory"]["executionAllowed"], false);
    assert_eq!(proposal["remaining"]["executionAllowed"], false);
    assert_eq!(proposal["originalDeadline"], proposal["remaining"]["originalDeadline"]);
    assert_eq!(proposal["approvalBlockReasons"], json!([
        "frozen_single_model_output_cap_missing",
        "frozen_complete_model_input_upper_bound_missing"
    ]));
    assert_eq!(native_single_continuation_preview_in(&snapshot, &f.scan, 1, root).unwrap(), proposal);
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn single_continuation_preview_scope_drift_cannot_offer_another_source() {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    let (f, context, seen) = single_model_inflight_paid();
    let db = db::open(&f.path).unwrap();
    let root = &context.run.as_ref().unwrap().run_id;
    RootOwner::load_single(&db, root).unwrap().close_single_finance(&db).unwrap();
    assert_eq!(db.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&f.scan]).unwrap(), 1);
    for (scan, attempt, selected) in [
        ("foreign-scan", 1, root.as_str()),
        (f.scan.as_str(), 1, "foreign-root"),
        (f.scan.as_str(), 2, root.as_str()),
    ] {
        let before = super::tests::application_table_snapshot(&db);
        let snapshot = db.unchecked_transaction().unwrap();
        assert_eq!(native_single_continuation_preview_in(&snapshot, scan, attempt, selected).unwrap_err(),
            "single_continuation_current_scope_invalid");
        snapshot.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    assert_eq!(db.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1", [&f.scan]).unwrap(), 1);
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(native_single_continuation_preview_in(&snapshot, &f.scan, 1, root).unwrap_err(),
        "single_continuation_current_scope_invalid");
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(db.execute("UPDATE sentinel_scans SET attempt_count=1 WHERE id=?1", [&f.scan]).unwrap(), 1);
    assert_eq!(db.execute("UPDATE projects SET status='archived' WHERE id=(SELECT project_id FROM sentinel_scans WHERE id=?1)",
        [&f.scan]).unwrap(), 1);
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(native_single_continuation_preview_in(&snapshot, &f.scan, 1, root).unwrap_err(),
        "single_continuation_current_scope_invalid");
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
