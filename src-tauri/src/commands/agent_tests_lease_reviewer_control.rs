#[test]
fn lease_reviewer_actual_three_controls_confirmed_natural_expiry_and_zero_replay() {
    lease_reviewer_control_audit(false);
}

#[test]
fn lease_reviewer_actual_three_controls_rejected_natural_expiry_and_zero_replay() {
    lease_reviewer_control_audit(true);
}

#[test]
fn lease_authorization_original_expired_c_refuses_reentry_without_renewal_or_writes() {
    lease_reviewer_control_audit_mode(false,true,false);
}

#[test]
fn lease_authorization_original_signed_source_and_captured_frame_reject_cold_drift() {
    lease_reviewer_control_audit_mode(false,false,true);
}

fn lease_reviewer_control_audit(rejected: bool) {
    lease_reviewer_control_audit_mode(rejected,false,false);
}

fn lease_reviewer_control_audit_mode(rejected: bool, expire_authorization_c: bool, diagnostic: bool) {
    run_authorization_control_to_review_audit(
        false,
        false,
        0,
        0,
        rejected,
        false,
        false,
        true,
        expire_authorization_c,
        AuthorizationReviewFault::None,
        |context, session, db, models, targets, reviewed| {
            use crate::agent_runtime::multi_agent::{attempts, budget, specialist};
            let root_id = session.lease.root_run_id.clone();
            let root = &root_id;
            let verdict = if rejected { "rejected" } else { "confirmed" };
            assert!(
                matches!(reviewed, AgentTargetOutcome::Incomplete(_)),
                "the original target remains incomplete: {reviewed:?}"
            );
            let (candidate, revision, raw, assignment, run): (String,i64,String,String,String) = db.query_row(
                "SELECT candidate_id,candidate_revision,candidate_json,assignment_id,reviewer_run_id FROM agent_review_requests WHERE root_run_id=?1 AND status=?2",
                params![root,verdict],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
            ).unwrap();
            let frozen: JsonValue = serde_json::from_str(&raw).unwrap();
            let findings = frozen["findingCandidates"].as_array().unwrap();
            assert_eq!(findings.len(), 1);
            assert_eq!(
                findings[0]["record"]["kind"],
                "authorization_cross_object_read"
            );
            assert_eq!(
                findings[0]["record"]["verification"],
                "operator_bound_three_side_native_control"
            );
            assert_eq!(findings[0]["record"]["sides"].as_array().unwrap().len(), 3);
            assert_eq!(
                targets.lock().unwrap().as_slice(),
                [
                    "GET /api/orders?id=owner HTTP/1.1",
                    "GET /api/orders?id=owner HTTP/1.1",
                    "GET /api/orders?id=tester HTTP/1.1",
                ]
            );
            let claims: (i64,i64) = db.query_row(
                "SELECT count(*),sum(artifact_id<>'') FROM agent_authorization_probe_claims WHERE scan_id=?1 AND attempt_number=?2",
                params![context.scan_id,context.attempt_number],|r|Ok((r.get(0)?,r.get(1)?)),
            ).unwrap();
            assert_eq!(
                claims,
                (3, 3),
                "all three real HTTP replies must have original artifacts"
            );
            let settled: (String,bool,i64) = db.query_row(
                "SELECT c.status,c.published_at<>'',(SELECT count(*) FROM sentinel_findings f WHERE f.scan_id=c.scan_id AND f.target_url=c.target_url AND f.stage=c.stage AND f.kind=c.kind AND f.record_key=c.record_key AND f.title=c.title AND f.severity=c.severity AND f.record_json=c.record_json) FROM agent_finding_candidates c WHERE c.id=?1 AND c.root_run_id=?2 AND c.candidate_revision=?3 AND c.reviewer_run_id=?4",
                params![findings[0]["id"].as_str().unwrap(),root,revision,run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            ).unwrap();
            assert_eq!(
                settled,
                if rejected {
                    ("rejected".into(), false, 0)
                } else {
                    ("published".into(), true, 1)
                }
            );
            let reviewer = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
                assignment_id: assignment,
                run_id: run,
                role: crate::agent_runtime::contract::AgentRole::EvidenceReviewer,
            };
            let worker = attempts::current(db, &session.lease, &reviewer.assignment_id).unwrap();
            assert_eq!(worker.state, "expired");
            assert!(!worker.finished_at.is_empty());
            let lease_calls: Vec<String> = db.prepare(
                "SELECT call_id FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request' AND json_extract(fact_json,'$.request.basis.phase')='lease-expired' AND json_extract(fact_json,'$.request.basis.changedFact.semantic.role')='evidence_reviewer' ORDER BY round",
            ).unwrap().query_map([root],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
            assert_eq!(lease_calls.len(), 1);
            let call = &lease_calls[0];
            assert_eq!(
                lease_deep_publication_counts(db, root, call),
                (1, 1, 1, 1),
                "publication/event/timeline/collaboration must each be original and unique"
            );
            let (received,reported,input,output,total,requests): (i64,bool,i64,i64,i64,i64) = db.query_row(
                "SELECT count(*),json_extract(receipt_json,'$.usageReported'),json_extract(receipt_json,'$.usage.inputTokens'),json_extract(receipt_json,'$.usage.outputTokens'),json_extract(receipt_json,'$.usage.totalTokens'),json_extract(receipt_json,'$.usage.modelRequests') FROM agent_root_model_journal WHERE root_run_id=?1 AND call_id=?2 AND phase='received'",
                params![root,call],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
            ).unwrap();
            assert_eq!((received, reported, requests), (1, true, 1));
            assert_eq!(input.checked_add(output), Some(total));
            assert!(total > 0);
            let root_fee: (i64,i64) = db.query_row(
                "SELECT count(*),sum(amount) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id='' AND source_id=?2 AND dimension='model_requests' AND kind='consume'",
                params![root,call],|r|Ok((r.get(0)?,r.get(1)?)),
            ).unwrap();
            assert_eq!(root_fee, (1, 1));
            assert_eq!(
                budget::balance(db, root, Some(&reviewer.assignment_id), "model_requests")
                    .unwrap()
                    .consumed,
                1
            );
            let c: (i64,String,String) = db.query_row(
                "SELECT lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",[root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            ).unwrap();
            // The held original parent may renew a live C under its heartbeat
            // proof. A session deadline cache is not the persisted authority.
            assert_eq!((c.0,c.1),(session.lease.lease_epoch,session.lease.fencing_token.clone()));
            let sdk_before = models.lock().unwrap().len();
            let target_before = targets.lock().unwrap().clone();
            let before = super::tests::application_table_snapshot(db);
            let tx = db.unchecked_transaction().unwrap();
            let receipt =
                specialist::received_for_local_delivery(&tx, &session.lease, &reviewer).unwrap();
            assert_eq!(
                validated_review_decision(&receipt.text).unwrap().verdict,
                verdict
            );
            assert_eq!(receipt.usage.model_requests, 1);
            let root_context = native_coordinator_root_context(context, &session.lease);
            let frame = NativeCoordinatorFrame::lease_closed_mapper(
                &tx,
                &root_context,
                &session.lease,
                &reviewer,
            )
            .unwrap();
            assert_eq!(frame.semantic["originalDecision"]["verdict"], verdict);
            assert_eq!(frame.semantic["dispatchGranted"], false);
            assert_eq!(frame.semantic["targetRequestsGranted"], 0);
            frame.verify(&tx, &root_context, &session.lease).unwrap();
            let cost =
                budget::root::model::tick::Tick::audit_cost_for_continuation(&tx, root, call)
                    .unwrap();
            assert_eq!(cost["phase"], "received");
            assert_eq!(cost["originalSemanticAvailable"], true);
            tx.commit().unwrap();
            assert!(super::tests::application_table_snapshot(db) == before);
            let replay = replay_frozen_review(
                context,
                session,
                &candidate,
                revision,
                &raw,
                AgentTargetOutcome::incomplete("fixture executor finished without a finding"),
            )
            .unwrap();
            assert!(matches!(replay, AgentTargetOutcome::Incomplete(_)));
            assert!(super::tests::application_table_snapshot(db) == before);
            let whole = multi_agent_review(
                context,
                session,
                AgentTargetOutcome::incomplete("fixture executor finished without a finding"),
            );
            assert!(
                matches!(whole, AgentTargetOutcome::Incomplete(_)),
                "{whole:?}"
            );
            assert!(
                super::tests::application_table_snapshot(db) == before,
                "whole review must not write, renew or reissue"
            );
            assert_eq!(models.lock().unwrap().len(), sdk_before);
            assert_eq!(*targets.lock().unwrap(), target_before);
            assert_eq!(lease_deep_publication_counts(db, root, call), (1, 1, 1, 1));
            // The actual three GETs issued three birth-bound original seals.
            // Verify historical signatures and original rows after legal closure.
            {
                use sha2::Digest;
                let original=super::tests::application_table_snapshot(db);
                let tx=db.unchecked_transaction().unwrap();
                let issuer=original_withdrawal_issuer_on(&tx,&session.lease).unwrap().unwrap();
                let ids:Vec<String> = tx.prepare("SELECT receipt_id FROM agent_authorization_side_receipts WHERE root_run_id=?1 ORDER BY side").unwrap()
                    .query_map([root],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
                assert_eq!(ids.len(),3);
                for id in &ids {
                    let raw=issuer.verify_authorization_side_issued(&tx,id).unwrap();
                    let fact:JsonValue=serde_json::from_str(&raw).unwrap();
                    for row in fact["originalRows"].as_array().unwrap() {
                        let frozen=NativeCoordinatorFrozenRows {sql:row["sql"].as_str().unwrap().into(),
                            keys:serde_json::from_value(row["keys"].clone()).unwrap(),hash:row["hash"].as_str().unwrap().into(),allow_empty:false};
                        frozen.verify(&tx).unwrap();
                    }
                    let directory=context.target_dir.join(AGENT_HTTP_DIRECTORY);
                    let metadata=std::fs::read(directory.join(fact["artifactId"].as_str().unwrap())).unwrap();
                    let descriptor:JsonValue=serde_json::from_slice(&metadata).unwrap();
                    let body=std::fs::read(directory.join(descriptor["payloadFile"].as_str().unwrap())).unwrap();
                    assert_eq!(fact["recordSha256"],format!("{:x}",sha2::Sha256::digest(&metadata)));
                    assert_eq!(fact["bodySha256"],format!("{:x}",sha2::Sha256::digest(&body)));
                    assert_eq!(fact["recordBytes"],metadata.len());
                    assert_eq!(fact["bodyBytes"],body.len());
                }
                assert!(super::tests::application_table_snapshot(&tx)==original);
                if diagnostic {
                    // Simulate offline corruption only in this disposable TX.
                    // Public hashes may be recomputed; private HMAC cannot.
                    let ddl:String=tx.query_row("SELECT sql FROM sqlite_master WHERE name='authorization_side_receipt_no_update'",[],|r|r.get(0)).unwrap();
                    tx.execute("DROP TRIGGER authorization_side_receipt_no_update",[]).unwrap();
                    let raw=issuer.verify_authorization_side_issued(&tx,&ids[0]).unwrap();
                    let mut fact:JsonValue=serde_json::from_str(&raw).unwrap();
                    fact["bodySha256"]=serde_json::json!("0".repeat(64));
                    let changed=fact.to_string();
                    tx.execute("UPDATE agent_authorization_side_receipts SET fact_json=?1,fact_hash=?2 WHERE receipt_id=?3",
                        params![changed,crate::agent_runtime::store::stable_hash(&changed),ids[0]]).unwrap();
                    tx.execute_batch(&ddl).unwrap();
                    let damaged=super::tests::application_table_snapshot(&tx);
                    assert_eq!(issuer.verify_authorization_side_issued(&tx,&ids[0]).unwrap_err(),"authorization_side_original_signature_invalid");
                    assert!(super::tests::application_table_snapshot(&tx)==damaged);
                }
                tx.rollback().unwrap();
                assert!(super::tests::application_table_snapshot(db)==original);
            }
            if diagnostic {
                let original=super::tests::application_table_snapshot(db);
                let tx=db.unchecked_transaction().unwrap();
                let source=native_authorization_original_finding_on(&tx,&session.lease,&context.target_dir,&findings[0]).unwrap();
                assert_eq!(source.bounded_facts["originalArtifactSealed"],true);
                for row in &source.rows {row.verify(&tx).unwrap();}
                assert!(super::tests::application_table_snapshot(&tx)==original);
                assert_eq!(tx.execute("UPDATE agent_authorization_probe_claims SET artifact_id='' WHERE scan_id=?1 AND attempt_number=?2 AND side='owner'",params![context.scan_id,context.attempt_number]).unwrap(),1);
                let damaged=super::tests::application_table_snapshot(&tx);
                assert!(native_authorization_original_finding_on(&tx,&session.lease,&context.target_dir,&findings[0]).is_err());
                assert!(source.rows.iter().any(|row|row.verify(&tx).is_err()));
                assert!(super::tests::application_table_snapshot(&tx)==damaged);
                tx.rollback().unwrap();
                assert!(super::tests::application_table_snapshot(db)==original);
                assert_eq!(models.lock().unwrap().len(),sdk_before);
                assert_eq!(*targets.lock().unwrap(),target_before);
                let tx=db.unchecked_transaction().unwrap();
                let artifact:String=tx.query_row("SELECT artifact_id FROM agent_authorization_probe_claims WHERE scan_id=?1 AND attempt_number=?2 AND side='owner'",params![context.scan_id,context.attempt_number],|r|r.get(0)).unwrap();
                let body_path=context.target_dir.join(AGENT_HTTP_DIRECTORY).join(artifact.replace(".json",".body"));
                let original_body=std::fs::read(&body_path).unwrap();
                let replacement=String::from_utf8(original_body.clone()).unwrap().replace("order-owner","order-OTHER");
                assert_ne!(replacement.as_bytes(),original_body);
                assert_eq!(replacement.len(),original_body.len());
                std::fs::write(&body_path,replacement).unwrap();
                let unchanged=super::tests::application_table_snapshot(&tx);
                assert_eq!(native_authorization_original_finding_on(&tx,&session.lease,&context.target_dir,&findings[0]).err().unwrap(),"authorization_original_signed_artifact_changed");
                assert!(frame.verify(&tx,&root_context,&session.lease).is_err());
                assert!(replay_frozen_review(context,session,&candidate,revision,&raw,
                    AgentTargetOutcome::incomplete("original review replay")).is_err());
                assert_eq!(models.lock().unwrap().len(),sdk_before);
                assert_eq!(*targets.lock().unwrap(),target_before);
                assert!(super::tests::application_table_snapshot(&tx)==unchanged);
                std::fs::write(&body_path,&original_body).unwrap();
                frame.verify(&tx,&root_context,&session.lease).unwrap();
                let ddl:String=tx.query_row("SELECT sql FROM sqlite_master WHERE name='authorization_side_receipt_no_delete'",[],|r|r.get(0)).unwrap();
                tx.execute("DROP TRIGGER authorization_side_receipt_no_delete",[]).unwrap();
                assert_eq!(tx.execute("DELETE FROM agent_authorization_side_receipts WHERE scan_id=?1 AND attempt_number=?2 AND side='owner'",params![context.scan_id,context.attempt_number]).unwrap(),1);
                tx.execute_batch(&ddl).unwrap();
                let damaged=super::tests::application_table_snapshot(&tx);
                assert!(native_authorization_original_finding_on(&tx,&session.lease,&context.target_dir,&findings[0]).is_err());
                assert!(frame.verify(&tx,&root_context,&session.lease).is_err());
                assert!(super::tests::application_table_snapshot(&tx)==damaged);
                tx.rollback().unwrap();
                assert!(super::tests::application_table_snapshot(db)==original);
                assert_eq!(models.lock().unwrap().len(),sdk_before);
                assert_eq!(*targets.lock().unwrap(),target_before);
            }
            if expire_authorization_c {
                db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1",[root]).unwrap();
                let stopped=super::tests::application_table_snapshot(db);
                assert!(multi_agent_authorization(context,session).is_err());
                assert_eq!(models.lock().unwrap().len(),sdk_before);
                assert_eq!(*targets.lock().unwrap(),target_before);
                assert!(super::tests::application_table_snapshot(db)==stopped);
            }

        },
    );
}
#[test]
fn lease_authorization_actual_reviewer_sdk_ack_drift_preserves_known_bill_and_zero_reentry() {
    authorization_reviewer_sdk_ack_fault_audit(AuthorizationReviewFault::OriginalAckDuringReviewerSdk);
}

#[test]
fn lease_authorization_actual_reviewer_sdk_cold_ack_restore_recovers_same_paid_receipt_once() {
    authorization_reviewer_sdk_ack_fault_audit(AuthorizationReviewFault::OriginalAckColdDuringReviewerSdk);
}

fn authorization_reviewer_sdk_ack_fault_audit(fault: AuthorizationReviewFault) {
    run_authorization_control_to_review_audit(
        false, false, 0, 0, false, false, false, true, false,
        fault,
        |context, session, db, models, targets, reviewed| {
            use crate::agent_runtime::multi_agent::{budget, specialist};
            assert!(matches!(reviewed, AgentTargetOutcome::Incomplete(_))
                && reviewed.detail().contains("review_gate_failed:"), "{reviewed:?}");
            let root = &session.lease.root_run_id;
            let (candidate, revision, raw, assignment, run): (String, i64, String, String, String) = db.query_row(
                "SELECT candidate_id,candidate_revision,candidate_json,assignment_id,reviewer_run_id FROM agent_review_requests WHERE root_run_id=?1 AND status='failed'",
                [root], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
            ).unwrap();
            let reviewer = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
                assignment_id: assignment,
                run_id: run,
                role: crate::agent_runtime::contract::AgentRole::EvidenceReviewer,
            };
            assert_eq!(models.lock().unwrap().iter().filter(|request|
                request.contains("你是独立 Evidence Reviewer。你没有网络")).count(), 1);
            assert_eq!(targets.lock().unwrap().as_slice(), [
                "GET /api/orders?id=owner HTTP/1.1", "GET /api/orders?id=owner HTTP/1.1",
                "GET /api/orders?id=tester HTTP/1.1",
            ]);
            let accounting = agent_request_accounting(db, &context.scan_id,
                context.attempt_number, &context.target_url).unwrap();
            assert_eq!((accounting.authorization.received, accounting.authorization.unresolved), (3, 0));
            let unpublished: (i64, i64, i64, i64) = db.query_row(
                "SELECT (SELECT count(*) FROM agent_review_decisions WHERE root_run_id=?1),(SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND kind='review_decision'),(SELECT count(*) FROM agent_finding_candidates WHERE root_run_id=?1 AND (status='published' OR published_at<>'')),(SELECT count(*) FROM sentinel_findings WHERE scan_id=?2 AND record_json LIKE '%authorization_cross_object_read%')",
                params![root,context.scan_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            ).unwrap();
            assert_eq!(unpublished, (0, 0, 0, 0));
            let original = super::tests::application_table_snapshot(db);
            let tx = db.unchecked_transaction().unwrap();
            // This is a paid failed delivery, so audit the original received
            // financial receipt without inventing a completed child or grant.
            let receipt = specialist::received_for_reconciliation(&tx, &session.lease, &reviewer).unwrap();
            assert_eq!(receipt.rejection, "");
            assert_eq!(validated_review_decision(&receipt.text).unwrap().verdict, "confirmed");
            assert_eq!((receipt.usage.input_tokens, receipt.usage.cached_input_tokens,
                receipt.usage.output_tokens, receipt.usage.total_tokens,
                receipt.usage.model_requests), (10, 0, 10, 20, 1));
            let (calls, reported, state): (i64, bool, String) = tx.query_row(
                "SELECT count(*),json_extract(response_json,'$.usageReported'),state FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3",
                params![reviewer.assignment_id,reviewer.run_id,root], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
            ).unwrap();
            assert_eq!((calls, reported, state.as_str()), (1, true, "received"));
            // The received fee is determinate; post-SDK rejection precedes
            // scalar settlement, so keep the original remaining reservation.
            for (dimension, consumed, reserved) in [
                ("model_input_tokens", 10, 7_990),
                ("model_cached_tokens", 0, 8_000),
                ("model_output_tokens", 10, 7_990),
                ("model_requests", 1, 0),
            ] {
                let balance = budget::balance(&tx, root, Some(&reviewer.assignment_id), dimension).unwrap();
                assert_eq!((balance.consumed, balance.reserved, balance.indeterminate), (consumed, reserved, 0));
            }
            let held: (String, i64, i64, String) = tx.query_row(
                "SELECT state,reserved_tokens,reserved_requests,budget_settled_at FROM agent_assignments WHERE id=?1 AND child_run_id=?2",
                params![reviewer.assignment_id,reviewer.run_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            ).unwrap();
            assert_eq!(held, ("paused".into(), 8_000, 1, "".into()));
            tx.commit().unwrap();
            assert!(super::tests::application_table_snapshot(db) == original);
            let sdk_before = models.lock().unwrap().len();
            let target_before = targets.lock().unwrap().clone();
            for _ in 0..2 {
                let whole = multi_agent_review(context, session,
                    AgentTargetOutcome::incomplete("fixture executor finished without a finding"));
                assert!(matches!(whole, AgentTargetOutcome::Failed(_)), "{whole:?}");
                assert!(super::tests::application_table_snapshot(db) == original,
                    "invalid original ACK must reject whole review without writes");
                assert!(replay_frozen_review(context, session, &candidate, revision, &raw,
                    AgentTargetOutcome::incomplete("fixture executor finished without a finding")).is_err());
                assert!(super::tests::application_table_snapshot(db) == original,
                    "same original paid call must not recover through a damaged Auth ACK");
                assert_eq!(models.lock().unwrap().len(), sdk_before);
                assert_eq!(*targets.lock().unwrap(), target_before);
            }
            if fault == AuthorizationReviewFault::OriginalAckColdDuringReviewerSdk {
                authorization_reviewer_saved_ack_recovery_audit(context, session, db, models, targets);
            }
        },
    );
}
