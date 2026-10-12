#[test]
fn gap_reverse_assessment_original_source_rejects_damage_without_writes() {
    let f=followup_fixture();
    let db=db::open(&f.context.db_path).unwrap();
    let actor=db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",[&f.root_run_id],|row|Ok(crate::agent_runtime::multi_agent::lease::CoordinatorLease {
        scan_id:row.get(0)?,attempt_number:row.get(1)?,target_key:row.get(2)?,root_run_id:row.get(3)?,lease_epoch:row.get(4)?,fencing_token:row.get(5)?,lease_expires_at:row.get(6)?,
    })).unwrap();
    let root=native_coordinator_root_context(&f.context,&actor);
    let (candidate,revision,missing)=paid_proposal_candidate(&db);
    let original:(String,i64,String,String,String)=db.query_row("SELECT dedup_key,delivery_attempts,artifact_refs_json,acknowledged_at,payload_json FROM agent_messages WHERE id=?1",[&f.assessment_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).unwrap();
    let before=super::tests::application_table_snapshot(&db);
    let tx=db.unchecked_transaction().unwrap();
    let baseline=NativeCoordinatorFrame::investigator(&tx,&root,&actor,&candidate,revision,&missing).unwrap();
    tx.commit().unwrap();
    assert!(super::tests::application_table_snapshot(&db)==before);
    for mutation in ["dedup_key='foreign-original'","delivery_attempts=2","artifact_refs_json='[\"foreign\"]'","acknowledged_at=''","payload_json=json_set(payload_json,'$.unexpectedGrant',1)"] {
        db.execute(&format!("UPDATE agent_messages SET {mutation} WHERE id=?1"),[&f.assessment_id]).unwrap();
        let changed=super::tests::application_table_snapshot(&db);
        let tx=db.unchecked_transaction().unwrap();
        assert!(NativeCoordinatorFrame::investigator(&tx,&root,&actor,&candidate,revision,&missing).is_err(),"{mutation}");
        tx.commit().unwrap();
        assert!(super::tests::application_table_snapshot(&db)==changed,"pure source cannot repair a damaged ACK: {mutation}");
        db.execute("UPDATE agent_messages SET dedup_key=?2,delivery_attempts=?3,artifact_refs_json=?4,acknowledged_at=?5,payload_json=?6 WHERE id=?1",params![f.assessment_id,original.0,original.1,original.2,original.3,original.4]).unwrap();
        // Fixture restoration may emit its own update event; the reader must
        // preserve the state that exists after that external mutation.
        let restored_before=super::tests::application_table_snapshot(&db);
        let tx=db.unchecked_transaction().unwrap();
        let restored=NativeCoordinatorFrame::investigator(&tx,&root,&actor,&candidate,revision,&missing).unwrap();
        assert_eq!(restored.fact(),baseline.fact(),"{mutation}");
        tx.commit().unwrap();
        assert!(super::tests::application_table_snapshot(&db)==restored_before,"{mutation}");
    }
}

#[test]
fn gap_original_published_dispatch_rejects_forged_frame_and_policy_binding() {
    let f=followup_fixture();
    let db=db::open(&f.context.db_path).unwrap();
    let task:String=db.query_row("SELECT task_slice_json FROM agent_assignments WHERE coordinator_run_id=?1 AND role='deep_investigator'",[&f.root_run_id],|row|row.get(0)).unwrap();
    let task:JsonValue=serde_json::from_str(&task).unwrap();
    let decision=&task["rootDecision"];
    let candidate=task["candidateId"].as_str().unwrap();
    let revision=task["revision"].as_i64().unwrap();
    let before=super::tests::application_table_snapshot(&db);
    let tx=db.unchecked_transaction().unwrap();
    gap_receipt_root_decision_valid(&tx,&f.root_run_id,decision,candidate,revision).unwrap();
    tx.commit().unwrap();
    for policy in [false,true] {
        let mut forged=decision.clone();
        if policy {forged["rustPolicy"]["factHash"]="forged".into();}
        else {forged["frameHash"]="forged".into();}
        let tx=db.unchecked_transaction().unwrap();
        assert!(gap_receipt_root_decision_valid(&tx,&f.root_run_id,&forged,candidate,revision).is_err(),"policy={policy}");
        tx.commit().unwrap();
        assert!(super::tests::application_table_snapshot(&db)==before);
    }
    for (candidate,revision) in [("foreign-candidate",revision),(candidate,revision+1)] {
        let tx=db.unchecked_transaction().unwrap();
        assert!(gap_receipt_root_decision_valid(&tx,&f.root_run_id,decision,candidate,revision).is_err());
        tx.commit().unwrap();
        assert!(super::tests::application_table_snapshot(&db)==before);
    }
}
