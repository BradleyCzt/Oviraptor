// Storage/closure tests still start a signed Root and retain its real parent.
fn proposal_fixture(
    name: &str,
    text: &str,
) -> (PathBuf, AgentRunContext, String, LiveProposalGuard) {
    live_proposal_fixture_limits(name, text, (20_000, 20))
}

fn proposal_model_response(content: &str) -> String {
    serde_json::json!({"id":"proposal-loopback","choices":[{"message":{"role":"assistant","content":content},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string()
}

fn valid_proposal_text() -> &'static str {
    r#"{"summary":"独立评估已有路由","suggestions":["建议核对访问控制证据"],"limitations":["未进行目标请求"]}"#
}

#[test]
fn directive_proposal_silent_checkpoint_writes_do_not_grant_dispatch_or_lose_response() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    for (stage,trigger) in [
        ("start","CREATE TRIGGER ignore_checkpoint BEFORE UPDATE OF state ON agent_directive_proposals WHEN NEW.state='executing' BEGIN SELECT RAISE(IGNORE); END;"),
        ("response","CREATE TRIGGER ignore_checkpoint BEFORE UPDATE OF state ON agent_directive_proposals WHEN NEW.state='received' BEGIN SELECT RAISE(IGNORE); END;"),
        ("event","CREATE TRIGGER ignore_checkpoint BEFORE INSERT ON agent_events WHEN NEW.event_type='model_round_completed' BEGIN SELECT RAISE(IGNORE); END;"),
    ] {
        let (root,context,id, _original_parent)=proposal_fixture("proposal-checkpoint-ignore","@mapper 请分析已有证据");
        let lease=take_human_directives(&context).unwrap().lease.unwrap();
        let connection=db::open(&context.db_path).unwrap();
        let job=proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
        consume_proposal_mailbox(&connection,&lease,&job.child.run_id,&job.request_message_id,"human_assessment_request",&job.input).unwrap();
        connection.execute_batch(trigger).unwrap();
        if stage=="start" {
            assert!(proposals::start(&connection,&lease,&id).is_err(),"{stage}");
            let state:(String,String,String)=connection.query_row("SELECT p.state,a.state,r.status FROM agent_directive_proposals p JOIN agent_assignments a ON a.id=p.assignment_id JOIN agent_runs r ON r.id=p.child_run_id WHERE p.directive_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            assert_eq!(state,("prepared".into(),"leased".into(),"prepared".into()));
            connection.execute_batch("DROP TRIGGER ignore_checkpoint").unwrap();
            assert!(proposals::start(&connection,&lease,&id).unwrap());
        } else {
            assert!(proposals::start(&connection,&lease,&id).unwrap());
            let response=validated_human_proposal(valid_proposal_text(),false);
            let usage=serde_json::json!({"inputTokens":10,"cachedInputTokens":0,"outputTokens":10,"totalTokens":20,"modelRequests":1});
            assert!(proposals::record_response(&connection,&lease,&id,&response,&usage).is_err(),"{stage}");
            let state:(String,i64)=connection.query_row("SELECT p.state,(SELECT COUNT(*) FROM agent_events WHERE run_id=p.child_run_id AND event_type='model_round_completed') FROM agent_directive_proposals p WHERE directive_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
            assert_eq!(state,("executing".into(),0));
            assert!(!proposals::start(&connection,&lease,&id).unwrap(),"unknown provider outcome must not grant a second call");
            connection.execute_batch("DROP TRIGGER ignore_checkpoint").unwrap();
            proposals::record_response(&connection,&lease,&id,&response,&usage).unwrap();
        }
        let _=fs::remove_dir_all(root);
    }
}

#[test]
fn directive_proposal_atomic_assignment_snapshot_and_start_cas() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,context,id, _original_parent)=proposal_fixture("proposal-atomic","@mapper 请评估已有证据");
    let inbox=take_human_directives(&context).unwrap();
    let lease=inbox.lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let first=proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
    assert_eq!(first.directive_id,id);
    let second=proposals::prepare_next(&connection,&lease,&serde_json::json!({"changed":true})).unwrap().unwrap();
    assert_eq!(first.child,second.child);
    assert_eq!(first.input,second.input,"recovery must retain the original evidence");
    assert_eq!(first.input["constraints"]["targetRequests"],0);
    assert_eq!(first.input["constraints"]["tools"],serde_json::json!([]));
    assert!(proposals::start(&connection,&lease,&id).unwrap_err().contains("request_not_consumed"));
    consume_proposal_mailbox(&connection,&lease,&first.child.run_id,&first.request_message_id,"human_assessment_request",&first.input).unwrap();
    assert!(proposals::start(&connection,&lease,&id).unwrap());
    assert!(!proposals::start(&connection,&lease,&id).unwrap());
    assert!(proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().is_none(),"in-flight provider calls must never be reissued");
    let reserved:(i64,i64)=connection.query_row("SELECT reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",[&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(reserved,(4_000,1));
    assert!(proposals::complete(&connection,&lease,&id,"forged").unwrap_err().contains("result_missing"));
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_proposal_link_failure_rolls_back_assignment_budget_mailbox_and_status() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,context,id, _original_parent)=proposal_fixture("proposal-rollback","@investigator 请分析已有证据");
    let inbox=take_human_directives(&context).unwrap();
    let connection=db::open(&context.db_path).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_proposal BEFORE INSERT ON agent_directive_proposals BEGIN SELECT RAISE(ABORT,'injected_link_failure'); END;").unwrap();
    assert!(proposals::prepare_next(&connection,inbox.lease.as_ref().unwrap(),&context.evidence).unwrap_err().contains("injected_link_failure"));
    for table in ["agent_assignments","agent_lane_leases","agent_capability_leases","agent_messages","agent_directive_proposals","agent_budget_ledger"] {
        let count:i64=connection.query_row(&format!("SELECT COUNT(*) FROM {table}"),[],|r|r.get(0)).unwrap();
        assert_eq!(count,0,"orphaned {table}");
    }
    let status:String=connection.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[id],|r|r.get(0)).unwrap();
    assert_eq!(status,"accepted");
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_proposal_resume_revalidates_confirmed_source() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,context,id, _original_parent)=proposal_fixture("proposal-tamper","@mapper 请分析已有证据");
    let inbox=take_human_directives(&context).unwrap();
    let lease=inbox.lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
    connection.execute("UPDATE agent_user_directives SET text_redacted='tampered' WHERE id=?1",[&id]).unwrap();
    assert!(proposals::prepare_next(&connection,&lease,&context.evidence).unwrap_err().contains("source_invalid"));
    assert!(proposals::start(&connection,&lease,&id).unwrap_err().contains("source_invalid"));
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_proposal_concurrent_workers_only_one_can_start() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,context,id, _original_parent)=proposal_fixture("proposal-concurrent","@mapper 请分析已有证据");
    let inbox=take_human_directives(&context).unwrap();
    let lease=inbox.lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let job=proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
    consume_proposal_mailbox(&connection,&lease,&job.child.run_id,&job.request_message_id,"human_assessment_request",&job.input).unwrap();
    let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers=(0..2).map(|_| {
        let path=context.db_path.clone(); let lease=lease.clone(); let id=id.clone(); let barrier=barrier.clone();
        std::thread::spawn(move || { let c=db::open(&path).unwrap(); barrier.wait(); proposals::start(&c,&lease,&id).unwrap() })
    }).collect::<Vec<_>>();
    assert_eq!(workers.into_iter().map(|w|usize::from(w.join().unwrap())).sum::<usize>(),1);
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_proposal_mailbox_ack_failure_rolls_back_delivery() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,context,_, _original_parent)=proposal_fixture("proposal-mailbox-atomic","@mapper 请分析已有证据");
    let lease=take_human_directives(&context).unwrap().lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let job=proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
    connection.execute_batch("CREATE TRIGGER fail_proposal_ack BEFORE UPDATE OF acknowledged_at ON agent_messages WHEN NEW.acknowledged_at<>'' BEGIN SELECT RAISE(ABORT,'injected_ack_failure'); END;").unwrap();
    assert!(consume_proposal_mailbox(&connection,&lease,&job.child.run_id,&job.request_message_id,"human_assessment_request",&job.input).unwrap_err().contains("injected_ack_failure"));
    let state:(String,String,i64)=connection.query_row("SELECT delivered_at,acknowledged_at,delivery_attempts FROM agent_messages WHERE id=?1",[&job.request_message_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(state,(String::new(),String::new(),0),"delivery and acknowledgement must commit together");
    connection.execute_batch("DROP TRIGGER fail_proposal_ack").unwrap();
    for _ in 0..2 { consume_proposal_mailbox(&connection,&lease,&job.child.run_id,&job.request_message_id,"human_assessment_request",&job.input).unwrap(); }
    let attempts:i64=connection.query_row("SELECT delivery_attempts FROM agent_messages WHERE id=?1",[&job.request_message_id],|r|r.get(0)).unwrap();
    assert_eq!(attempts,1);
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_proposal_mailbox_replay_revalidates_fence_and_active_attempt() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,context,_, _original_parent)=proposal_fixture("proposal-mailbox-replay","@mapper 请分析已有证据");
    let lease=take_human_directives(&context).unwrap().lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let job=proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
    let consume=|fence|consume_proposal_mailbox(&connection,fence,&job.child.run_id,&job.request_message_id,"human_assessment_request",&job.input);
    consume(&lease).unwrap();
    let mut stale=lease.clone(); stale.fencing_token="not-current".into();
    assert!(consume(&stale).unwrap_err().contains("stale_coordinator_fencing_token"));
    connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1",[&lease.scan_id]).unwrap();
    assert!(consume(&lease).is_err(),"acknowledged messages do not bypass an inactive attempt");
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_proposal_mailbox_replay_rejects_changed_assignment_route() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,context,_, _original_parent)=proposal_fixture("proposal-route-replay","@mapper 请分析已有证据");
    let lease=take_human_directives(&context).unwrap().lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let job=proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
    let consume=||consume_proposal_mailbox(&connection,&lease,&job.child.run_id,&job.request_message_id,"human_assessment_request",&job.input);
    consume().unwrap();
    connection.execute("UPDATE agent_messages SET from_run_id='unrelated-run' WHERE id=?1",[&job.request_message_id]).unwrap();
    assert!(consume().is_err(),"acknowledgement cannot make a changed route trustworthy");
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_proposal_large_evidence_is_explicitly_bounded_not_silently_complete() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,mut context,_, _original_parent)=proposal_fixture("proposal-compaction","@mapper 请分析已有证据");
    context.evidence=serde_json::json!({"large":"中文证据与引号\"".repeat(4_000)});
    let inbox=take_human_directives(&context).unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let job=proposals::prepare_next(&connection,inbox.lease.as_ref().unwrap(),&context.evidence).unwrap().unwrap();
    assert!(job.input.to_string().len()<=2_600);
    assert_eq!(job.input["frozenEvidence"]["truncated"],true);
    assert_eq!(job.input["frozenEvidence"]["sourceSha256"],crate::agent_runtime::store::stable_hash(&context.evidence.to_string()));
    assert!(!job.input["frozenEvidence"]["excerpt"].as_str().unwrap().is_empty());
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_proposal_native_loop_receives_actual_specialist_result() {
    let _real = RealSpecialistTransport::enter();
    let mut harness = fresh_multi_production_harness("proposal-native");
    harness.context.execution_plan.max_turns = 2;
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        let response = if let Some(summary) = fresh_multi_root_wire_response(&request) {
            summary
        } else if request.contains("independent deep_investigator specialist") {
            proposal_model_response(valid_proposal_text())
        } else if fresh_multi_wire_system(&request).contains("SPA/API Mapper") {
            proposal_model_response(
                r#"{"summary":"actual local mapper","priorityContracts":[],"risks":[]}"#,
            )
        } else {
            model_round(&[], 10)
        };
        (200, "application/json", response)
    }));
    harness.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    freeze_fresh_multi_production_harness(&mut harness);
    let root_id = harness.context.run.as_ref().unwrap().run_id.clone();
    let mut session = multi_agent_prepare(&mut harness.context).unwrap();
    assert_eq!(
        harness.context.run.as_ref().unwrap().run_id,
        session.executor.run_id
    );
    let db = db::open(&harness.db_path).unwrap();
    let id = confirm_queue_directive(&db, &session.lease, "@investigator 请分析已有证据");
    let outcome = NativeAgentBackend.execute(&harness.context);
    let wire = seen.lock().unwrap().clone();
    let root_calls=wire.iter().filter(|r|fresh_multi_wire_system(r).contains("You are the Root Coordinator")).count();
    assert_eq!(root_calls,5,"existing business-logic assessment plus HumanDirective and feedback rounds");
    assert_eq!(wire.len(),root_calls+4,"one Mapper, one human specialist, two WebExecutor rounds: {outcome:?}");
    let executor = wire
        .iter()
        .filter(|r| {
            !fresh_multi_wire_system(r).contains("You are the Root Coordinator")
                && !fresh_multi_wire_system(r).contains("independent deep_investigator specialist")
                && !fresh_multi_wire_system(r).contains("SPA/API Mapper")
        })
        .collect::<Vec<_>>();
    assert_eq!(executor.len(), 2);
    for raw in executor {
        let request: JsonValue =
            serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
        let content = request["messages"].to_string();
        assert!(content.contains("独立评估已有路由"));
        assert!(content.contains("未执行任何建议"));
        assert!(!content.contains("角色请求仍需 Coordinator 派发"));
    }
    let status: String = db
        .query_row(
            "SELECT status FROM agent_user_directives WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "completed");
    let requests: i64 = db.query_row("SELECT COUNT(*) FROM agent_web_model_journal WHERE child_run_id=?1 AND phase='received'",
        [&session.executor.run_id], |r| r.get(0)).unwrap();
    assert_eq!(requests, 2);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &root_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        root_calls as i64
    );
    let root_calls: i64 = db.query_row("SELECT COUNT(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'", [&root_id], |r| r.get(0)).unwrap();
    assert_eq!(root_calls, 5);
    assert_eq!(
        wire.iter()
            .filter(|raw| fresh_multi_wire_system(raw).contains("You are the Root Coordinator"))
            .count(),
        5
    );
    multi_agent_finish_execution(&harness.context, &mut session, &outcome).unwrap();
    drop(session);
    drop(db);
    let _ = fs::remove_dir_all(harness.root);
}

