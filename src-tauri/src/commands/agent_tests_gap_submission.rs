#[test]
fn gap_submission_reopens_pending_input_and_committed_task_without_creating_or_starting() {
    let f=followup_fixture();
    let input=followup_input(&f);
    {
        let db=db::open(&f.context.db_path).unwrap();
        stage_gap_submission(&db,&input).unwrap();
    }
    let db=db::open(&f.context.db_path).unwrap();
    let before=review_delivery_snapshot(&db);
    let restored=gap_followup_submission_in(&db,&input.source_scan_id,&input.assessment_message_id).unwrap().unwrap();
    assert_eq!(json!(restored.input),json!(input));
    assert!(restored.scan.is_none());
    assert_eq!(review_delivery_snapshot(&db),before,"read-only restoration");
    let scan=submit_agent_gap_followup_in(&db,&input).unwrap();
    drop(db);
    let db=db::open(&f.context.db_path).unwrap();
    let before=review_delivery_snapshot(&db);
    let restored=gap_followup_submission_in(&db,&input.source_scan_id,&input.assessment_message_id).unwrap().unwrap();
    assert_eq!(restored.scan.unwrap().id,scan.id);
    assert_eq!(submit_agent_gap_followup_in(&db,&input).unwrap().id,scan.id);
    assert_eq!(review_delivery_snapshot(&db),before);
    assert_eq!(scan.status,"draft");
}

#[test]
fn gap_submission_release_requires_observed_result_and_fences_delayed_creators() {
    let f=followup_fixture();let db=db::open(&f.context.db_path).unwrap();
    let input=followup_input(&f);
    stage_gap_submission(&db,&input).unwrap();
    release_gap_submission_in(&db,&input.request_id,None).unwrap();
    assert!(create_agent_gap_followup_in(&db,&input).err().unwrap().contains("submission_released"));
    assert!(submit_agent_gap_followup_in(&db,&input).err().unwrap().contains("submission_released"));
    assert!(gap_followup_submission_in(&db,&input.source_scan_id,&input.assessment_message_id).unwrap().is_none());
    let mut next=input.clone();next.request_id=Uuid::new_v4().to_string();
    let scan=submit_agent_gap_followup_in(&db,&next).unwrap();
    assert!(release_gap_submission_in(&db,&next.request_id,None).unwrap_err().contains("changed_reconcile"));
    assert!(release_gap_submission_in(&db,&next.request_id,Some("wrong-task")).is_err());
    release_gap_submission_in(&db,&next.request_id,Some(&scan.id)).unwrap();
    release_gap_submission_in(&db,&next.request_id,Some(&scan.id)).unwrap();
    assert_eq!(sentinel_scan_by_id(&db,&scan.id).unwrap().status,"draft","release preserves the task");
    assert_eq!(submit_agent_gap_followup_in(&db,&next).unwrap().id,scan.id,"old successful key never creates again");
}

#[test]
fn gap_submission_failed_creation_releases_only_proven_uncommitted_request() {
    let f=followup_fixture();let db=db::open(&f.context.db_path).unwrap();
    let input=followup_input(&f);
    db.execute_batch("CREATE TRIGGER submission_create_fault BEFORE INSERT ON agent_gap_followups BEGIN SELECT RAISE(ABORT,'fault'); END;").unwrap();
    let before:i64=db.query_row("SELECT COUNT(*) FROM sentinel_scans",[],|r|r.get(0)).unwrap();
    assert!(submit_agent_gap_followup_in(&db,&input).err().unwrap().starts_with("followup_rejected:"));
    let after:i64=db.query_row("SELECT COUNT(*) FROM sentinel_scans",[],|r|r.get(0)).unwrap();
    assert_eq!(before,after);
    assert!(gap_followup_submission_in(&db,&input.source_scan_id,&input.assessment_message_id).unwrap().is_none());
    db.execute_batch("DROP TRIGGER submission_create_fault").unwrap();
    let mut next=input.clone();next.request_id=Uuid::new_v4().to_string();
    assert_eq!(submit_agent_gap_followup_in(&db,&next).unwrap().status,"draft");
}

#[test]
fn gap_submission_journal_faults_prevent_creation_and_keep_release_uncertainty_recoverable() {
    for (timing,body) in [("BEFORE","SELECT RAISE(ABORT,'fault');"),("BEFORE","SELECT RAISE(IGNORE);"),
        ("AFTER","DELETE FROM agent_gap_followup_submissions WHERE request_id=NEW.request_id;"),
        ("AFTER","UPDATE agent_gap_followup_submissions SET state='released' WHERE request_id=NEW.request_id;")] {
        let f=followup_fixture();let db=db::open(&f.context.db_path).unwrap();let input=followup_input(&f);
        db.execute_batch(&format!("CREATE TRIGGER submission_write_fault {timing} INSERT ON agent_gap_followup_submissions BEGIN {body} END;")).unwrap();
        assert!(submit_agent_gap_followup_in(&db,&input).is_err());
        let count:i64=db.query_row("SELECT COUNT(*) FROM agent_gap_followups",[],|r|r.get(0)).unwrap();assert_eq!(count,0);
    }
    let f=followup_fixture();let db=db::open(&f.context.db_path).unwrap();let input=followup_input(&f);
    stage_gap_submission(&db,&input).unwrap();
    db.execute_batch("CREATE TRIGGER submission_release_fault BEFORE UPDATE ON agent_gap_followup_submissions BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(release_gap_submission_in(&db,&input.request_id,None).is_err());
    assert!(gap_followup_submission_in(&db,&input.source_scan_id,&input.assessment_message_id).unwrap().is_some());
    db.execute_batch("DROP TRIGGER submission_release_fault").unwrap();
    assert_eq!(submit_agent_gap_followup_in(&db,&input).unwrap().status,"draft");
}

#[test]
fn gap_submission_validates_immutable_input_and_exact_source_binding() {
    let f=followup_fixture();let db=db::open(&f.context.db_path).unwrap();let input=followup_input(&f);
    stage_gap_submission(&db,&input).unwrap();
    let mut changed=input.clone();changed.task_name="different".into();
    assert!(stage_gap_submission(&db,&changed).unwrap_err().contains("id_conflict"));
    changed.request_id=Uuid::new_v4().to_string();
    assert!(stage_gap_submission(&db,&changed).unwrap_err().contains("conflict_reconcile"));
    assert!(gap_followup_submission_in(&db,"different-source",&input.assessment_message_id).unwrap().is_none());
    assert!(gap_followup_submission_in(&db,&input.source_scan_id,"different-assessment").unwrap().is_none());
    assert!(db.execute("UPDATE agent_gap_followup_submissions SET input_json='{}'",[]).is_err());
    db.execute_batch("DROP TRIGGER agent_gap_submission_immutable").unwrap();
    db.execute("UPDATE agent_gap_followup_submissions SET input_hash='corrupt'",[]).unwrap();
    assert!(gap_followup_submission_in(&db,&input.source_scan_id,&input.assessment_message_id).is_err());
    assert!(release_gap_submission_in(&db,&input.request_id,None).is_err());
}

#[test]
fn gap_submission_two_windows_cannot_create_two_tasks_for_one_unacknowledged_source() {
    let f=followup_fixture();let input=followup_input(&f);
    let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));
    let results=std::thread::scope(|scope| {
        let handles:Vec<_>=(0..2).map(|_| {
            let mut input=input.clone();input.request_id=Uuid::new_v4().to_string();
            let gate=barrier.clone();let path=&f.context.db_path;
            scope.spawn(move || {let db=db::open(path).unwrap();gate.wait();submit_agent_gap_followup_in(&db,&input).is_ok()})
        }).collect();
        handles.into_iter().map(|h|h.join().unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|r|**r).count(),1);
    let db=db::open(&f.context.db_path).unwrap();
    let count:i64=db.query_row("SELECT COUNT(*) FROM agent_gap_followups",[],|r|r.get(0)).unwrap();assert_eq!(count,1);
    assert!(gap_followup_submission_in(&db,&input.source_scan_id,&input.assessment_message_id).unwrap().unwrap().scan.is_some());
}

#[test]
fn gap_submission_release_racing_creation_has_one_authoritative_winner() {
    let f=followup_fixture();let input=followup_input(&f);
    let db=db::open(&f.context.db_path).unwrap();stage_gap_submission(&db,&input).unwrap();
    let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));
    let (created,released)=std::thread::scope(|scope| {
        let gate=barrier.clone();let path=&f.context.db_path;let input=&input;
        let creator=scope.spawn(move || {let db=db::open(path).unwrap();gate.wait();submit_agent_gap_followup_in(&db,input).is_ok()});
        let gate=barrier.clone();
        let releaser=scope.spawn(move || {let db=db::open(path).unwrap();gate.wait();release_gap_submission_in(&db,&input.request_id,None).is_ok()});
        (creator.join().unwrap(),releaser.join().unwrap())
    });
    assert_ne!(created,released,"a release cannot report cancellation after an unseen commit");
    let count:i64=db.query_row("SELECT COUNT(*) FROM agent_gap_followups",[],|r|r.get(0)).unwrap();
    assert_eq!(count,i64::from(created));
}

#[test]
fn gap_submission_deleted_tasks_keep_permanent_receipts_and_paid_source_remains_protected() {
    let f=followup_fixture();let db=db::open(&f.context.db_path).unwrap();let input=followup_input(&f);
    let scan=submit_agent_gap_followup_in(&db,&input).unwrap();
    db.execute("DELETE FROM sentinel_scans WHERE id=?1",[&scan.id]).unwrap();
    let restored=gap_followup_submission_in(&db,&input.source_scan_id,&input.assessment_message_id).unwrap().unwrap();
    assert_eq!(restored.created_scan_id,scan.id);
    assert!(restored.scan.is_none());
    assert!(submit_agent_gap_followup_in(&db,&input).is_err());
    let count:i64=db.query_row("SELECT COUNT(*) FROM agent_gap_followups WHERE request_id=?1",[&input.request_id],|r|r.get(0)).unwrap();assert_eq!(count,0);
    assert!(release_gap_submission_in(&db,&input.request_id,None).is_err());
    release_gap_submission_in(&db,&input.request_id,Some(&scan.id)).unwrap();
    assert!(submit_agent_gap_followup_in(&db,&input).is_err(),"released tombstone cannot recreate a deleted task");
    // This source has an original running Root and seven paid SDK calls. A
    // followup tombstone cannot authorize removing its execution/cost evidence.
    let before=web_mode_test_rows(&db);
    assert!(db.execute("DELETE FROM sentinel_scans WHERE id=?1",[&input.source_scan_id]).is_err());
    web_mode_assert_rows(&db,&before);
    assert!(delete_sentinel_scan_inner(&f.context.db_path,&input.source_scan_id).is_err());
    web_mode_assert_rows(&db,&before);
    let preserved:String=db.query_row("SELECT created_scan_id FROM agent_gap_followup_submissions WHERE request_id=?1",[&input.request_id],|r|r.get(0)).unwrap();
    assert_eq!(preserved,scan.id);
}

#[test]
fn gap_submission_result_receipt_write_failure_rolls_back_task_and_provenance() {
    let f=followup_fixture();let db=db::open(&f.context.db_path).unwrap();let input=followup_input(&f);
    db.execute_batch("CREATE TRIGGER submission_result_fault BEFORE UPDATE OF created_scan_id ON agent_gap_followup_submissions BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let before:i64=db.query_row("SELECT COUNT(*) FROM sentinel_scans",[],|r|r.get(0)).unwrap();
    assert!(submit_agent_gap_followup_in(&db,&input).err().unwrap().starts_with("followup_rejected:"));
    let after:i64=db.query_row("SELECT COUNT(*) FROM sentinel_scans",[],|r|r.get(0)).unwrap();assert_eq!(before,after);
    let count:i64=db.query_row("SELECT COUNT(*) FROM agent_gap_followups",[],|r|r.get(0)).unwrap();assert_eq!(count,0);
}

/// Guard still bites (`root_paid_open_run_retained`): an open paid original
/// Root's run row cannot be removed by raw SQL, directly or through the scan
/// cascade; the refusal names the guard and every row survives.
#[test]
fn gap_submission_open_paid_root_run_rejects_direct_and_cascading_delete() {
    let f=followup_fixture();let db=db::open(&f.context.db_path).unwrap();let input=followup_input(&f);
    let root:String=db.query_row("SELECT r.id FROM agent_runs r WHERE r.scan_id=?1 AND r.role='coordinator' AND r.root_run_id=r.id
        AND r.status<>'terminal' AND EXISTS(SELECT 1 FROM agent_root_budget_attempts a WHERE a.root_run_id=r.id)",
        [&input.source_scan_id],|r|r.get(0)).unwrap();
    let before=web_mode_test_rows(&db);
    for sql in ["DELETE FROM agent_runs WHERE id=?1","DELETE FROM sentinel_scans WHERE id=(SELECT scan_id FROM agent_runs WHERE id=?1)"] {
        let error=db.execute(sql,[&root]).unwrap_err().to_string();
        assert!(error.contains("root_paid_open_run_retained"),"{sql}: {error}");
        web_mode_assert_rows(&db,&before);
    }
}
