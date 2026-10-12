fn review_delivery_snapshot(connection: &rusqlite::Connection) -> Vec<String> {
    [
        "agent_review_requests", "agent_review_decisions", "agent_finding_candidates",
        "sentinel_findings", "agent_runs", "agent_assignments", "agent_messages",
        "agent_budget_ledger", "agent_capability_leases", "agent_lane_leases",
        "agent_specialist_calls", "agent_events", "agent_snapshots", "sentinel_scans",
        "agent_collaboration_events",
    ].iter().map(|table| {
        let mut query = connection.prepare(&format!("SELECT * FROM {table} ORDER BY rowid")).unwrap();
        let columns = query.column_count();
        let rows = query.query_map([], |row| (0..columns)
            .map(|i| row.get::<_, rusqlite::types::Value>(i)).collect::<rusqlite::Result<Vec<_>>>())
            .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        format!("{table}:{rows:?}")
    }).collect()
}

#[test]
fn reviewer_delivery_rolls_back_all_business_writes_and_reuses_received_result() {
    use crate::agent_runtime::{contract::{AgentLane,AgentRole}, multi_agent::scheduler};
    for action in ["IGNORE", "ABORT,'delivery fault'"] {
        for target in [
            "BEFORE INSERT ON agent_review_decisions",
            "BEFORE UPDATE OF decision_id ON agent_review_requests",
            "BEFORE INSERT ON agent_messages WHEN NEW.kind='review_decision'",
            "BEFORE UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='review_decision'",
            "BEFORE UPDATE OF state ON agent_assignments WHEN NEW.state='completed'",
            "BEFORE UPDATE OF status ON agent_runs WHEN NEW.status='terminal'",
            "BEFORE UPDATE OF revoked_at ON agent_capability_leases",
            "BEFORE DELETE ON agent_lane_leases",
            "BEFORE UPDATE OF candidate_revision ON agent_finding_candidates WHEN NEW.record_key='second'",
            "BEFORE INSERT ON sentinel_findings WHEN NEW.record_key='second'",
            "BEFORE UPDATE OF title ON sentinel_findings WHEN NEW.record_key='first'",
            "BEFORE UPDATE OF status ON agent_finding_candidates WHEN NEW.record_key='second'",
        ] {
            let (root,path,run_id,lease) = multi_agent_test_root("review-atomic",60_000,20);
            let connection = db::open(&path).unwrap();
            let mut context = test_context(&path,&lease.target_key,vec![]);
            context.scan_id = lease.scan_id.clone();
            context.run = Some(AgentRunLedger { db_path:path,run_id:run_id.clone() });
            for key in ["first","second","third"] {
                stage_agent_finding(&context,"web","evidence",key,key,"info",&serde_json::json!({"fact":key})).unwrap();
            }
            connection.execute(
                "INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json) \
                 VALUES(?1,?2,'web','evidence','first','older finding','info','{}')",
                params![lease.scan_id,lease.target_key],
            ).unwrap();
            let child = scheduler::schedule_child(&connection,&lease,AgentRole::EvidenceReviewer,AgentLane::Review,
                "candidate_ready",&serde_json::json!({"candidateId":"atomic","candidateRevision":1}),1,
                &["evidence.read".into(),"review.write".into()],8000,1).unwrap();
            scheduler::mark_child_running(&connection,&lease,&child).unwrap();
            let bundle = serde_json::json!({"findingCandidates":pending_agent_finding_candidates(&connection,&run_id).unwrap()});
            open_review_request(&connection,&lease,&child,"review-atomic","atomic",1,&bundle.to_string(),&context.target_dir).unwrap();
            let response = serde_json::json!({"verdict":"confirmed","reasonCodes":["fixture_evidence"],"missingEvidence":[],"confidence":0.9,"summary":"reviewed"}).to_string();
            let (port,seen,_stop) = spawn_endpoint(std::sync::Arc::new(move |_|(200,"application/json",proposal_model_response(&response))));
            context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
            let (text,usage) = multi_agent_child_round_transport(&context,&lease,&child,"review fixture",bundle).unwrap();
            settle_child_usage(&connection,&lease,&child,&usage).unwrap();
            let decision = validated_review_decision(&text).unwrap();
            let before = review_delivery_snapshot(&connection);
            connection.execute_batch(&format!("CREATE TRIGGER fault {target} BEGIN SELECT RAISE({action}); END;")).unwrap();
            let error = complete_review_delivery(&connection,&lease,&child,"review-atomic","atomic",1,&decision,&context.target_dir).unwrap_err();
            if action.starts_with("ABORT") { assert!(error.contains("delivery fault"),"{target}: {error}"); }
            assert_eq!(review_delivery_snapshot(&connection),before,"{target}/{action}");
            connection.execute_batch("DROP TRIGGER fault").unwrap();
            complete_review_delivery(&connection,&lease,&child,"review-atomic","atomic",1,&decision,&context.target_dir).unwrap();
            let final_state:(i64,i64,i64,i64,i64,i64) = connection.query_row(
                "SELECT (SELECT COUNT(*) FROM agent_finding_candidates WHERE status='published'), \
                 (SELECT COUNT(*) FROM sentinel_findings WHERE record_json<>'{}'), \
                 (SELECT COUNT(*) FROM agent_messages WHERE kind='review_decision' AND delivery_attempts=1 AND acknowledged_at<>''), \
                 spent_tokens,spent_requests,(SELECT COUNT(*) FROM agent_specialist_calls WHERE state='received') FROM agent_budget_ledger",
                [], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
            ).unwrap();
            assert_eq!(final_state,(3,3,1,20,1,1),"{target}/{action}");
            assert_eq!(seen.lock().unwrap().len(),1,"delivery retry must not call the model");
            drop(connection);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn investigator_delivery_is_atomic_without_refunding_or_redispatching_the_model() {
    for action in ["IGNORE", "ABORT,'gap delivery fault'"] {
        for target in [
            "BEFORE INSERT ON agent_messages WHEN NEW.kind='gap_proposed'",
            "BEFORE UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='gap_proposed'",
            "BEFORE INSERT ON agent_messages WHEN NEW.kind='proposal_assessed'",
            "BEFORE UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='proposal_assessed'",
            "BEFORE UPDATE OF state ON agent_assignments WHEN NEW.role='deep_investigator' AND NEW.state='completed'",
            "BEFORE UPDATE OF status ON agent_runs WHEN NEW.role='deep_investigator' AND NEW.terminal_state='completed'",
            "BEFORE UPDATE OF revoked_at ON agent_capability_leases",
            "BEFORE DELETE ON agent_lane_leases",
            "", // Successful real-transport delivery and read-only replay.
        ] {
            let response = serde_json::json!({
                "summary":"needs operator review", "nextStep":"manual_review", "gapCode":"missing_control",
                "supportingFactRefs":[], "missingEvidence":["missing control"], "prerequisites":[],
                "proposedContracts":[], "expectedInformationGain":0.0, "impactCeiling":"low",
                "estimatedCost":{"modelTokens":100,"modelRequests":1,"targetRequests":0},
                "sideEffectClass":"read_only", "overlapKeys":[],
                "falsificationCondition":"control refutes claim", "stopCondition":"operator decision",
            }).to_string();
            // Actual Root/C/worker fixture; its frozen model endpoint is this one.
            let (port,seen,_stop) = specialist_original_endpoint(std::sync::Arc::new(move |_|(200,"application/json",proposal_model_response(&response))));
            let (root,context,session) = specialist_original_gap_fixture(&format!("http://127.0.0.1:{port}/v1"));
            let connection = db::open(&context.db_path).unwrap();
            if !target.is_empty() {
                connection.execute_batch(&format!("CREATE TRIGGER fault {target} BEGIN SELECT RAISE({action}); END;")).unwrap();
            }
            let _real = RealSpecialistTransport::enter();
            let result = multi_agent_investigate_review_gap(&context,&session,"candidate-gap",2,&serde_json::json!(["missing control"]));
            assert_eq!(result.is_ok(),target.is_empty(),"{target}/{action}: {result:?}");
            if !target.is_empty() && action.starts_with("ABORT") {
                assert!(result.as_ref().unwrap_err().contains("gap delivery fault"),"{target}: {result:?}");
            }
            let saved:(i64,i64,i64,i64,i64) = connection.query_row(
                "SELECT (SELECT COUNT(*) FROM agent_messages WHERE kind IN ('gap_proposed','proposal_assessed')), \
                 (SELECT COUNT(*) FROM agent_assignments WHERE role='deep_investigator' AND state='completed'), \
                 r.used_tokens,r.used_requests,(SELECT COUNT(*) FROM agent_specialist_calls WHERE state='received' AND role='deep_investigator') \
                 FROM agent_runs r WHERE r.role='deep_investigator'",
                [], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
            ).unwrap();
            let expected = if target.is_empty() { (2,1,20,1,1) } else { (0,0,20,1,1) };
            assert_eq!(saved,expected,"{target}/{action}");
            if !target.is_empty() { connection.execute_batch("DROP TRIGGER fault").unwrap(); }
            let before = review_delivery_snapshot(&connection);
            let replay = multi_agent_investigate_review_gap(&context,&session,"candidate-gap",2,&serde_json::json!(["missing control"]));
            if target.contains("revoked_at") || target.contains("agent_lane_leases") {
                // Cleanup itself was prevented. Recovery must not seize a child
                // still marked running or silently accept unreleased authority.
                assert!(replay.is_err());
                assert_eq!(before,review_delivery_snapshot(&connection));
                let (assignment_id,run_id) = connection.query_row(
                    "SELECT id,child_run_id FROM agent_assignments WHERE role='deep_investigator'",
                    [],|row|Ok((row.get(0)?,row.get(1)?)),
                ).unwrap();
                let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
                    assignment_id,run_id,role:crate::agent_runtime::contract::AgentRole::DeepInvestigator,
                };
                stop_failed_child_preserving_usage(&connection,&session.lease,&child,"explicit cleanup after fault removal").unwrap();
                multi_agent_investigate_review_gap(&context,&session,"candidate-gap",2,&serde_json::json!(["missing control"])).unwrap();
            } else {
            assert!(replay.is_ok(),"saved receipt must recover after removing the fault: {target}/{action}: {replay:?}");
            if target.is_empty() { assert_eq!(before,review_delivery_snapshot(&connection)); }
            }
            let delivered: (i64,i64,i64,i64) = connection.query_row(
                "SELECT (SELECT COUNT(*) FROM agent_messages WHERE kind IN ('gap_proposed','proposal_assessed') AND delivery_attempts=1 AND acknowledged_at<>''), \
                 (SELECT COUNT(*) FROM agent_assignments WHERE role='deep_investigator' AND state='completed' AND budget_settled_at<>'' AND reserved_tokens=0 AND reserved_requests=0), \
                 used_tokens,used_requests FROM agent_runs WHERE role='deep_investigator'",
                [], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            ).unwrap();
            assert_eq!(delivered,(2,1,20,1),"{target}/{action}");
            let before = review_delivery_snapshot(&connection);
            multi_agent_investigate_review_gap(&context,&session,"candidate-gap",2,&serde_json::json!(["missing control"])).unwrap();
            assert_eq!(before, review_delivery_snapshot(&connection));
            assert_eq!(specialist_handler_call_count(&seen),1);
            drop(connection);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
