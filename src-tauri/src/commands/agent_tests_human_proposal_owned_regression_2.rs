// Actual live SDK regressions use new original Root/parent fixtures.
#[test]
fn directive_proposal_concurrent_full_apply_is_idempotent() {
    for iteration in 0..8 {
        let (root,mut context,id,_parent)=live_proposal_fixture("proposal-apply-race","@mapper 请分析已有证据");
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(valid_proposal_text()))));
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        let barrier=std::sync::Arc::new(std::sync::Barrier::new(4));
        let workers=(0..4).map(|_| {
            let context=context.clone(); let barrier=barrier.clone();
            std::thread::spawn(move || {
                let mut inbox=take_human_directives(&context)?;
                barrier.wait();
                apply_human_proposal_actions(&context,&mut inbox)
            })
        }).collect::<Vec<_>>();
        for worker in workers { worker.join().unwrap().unwrap_or_else(|e|panic!("iteration {iteration}: {e}")); }
        assert_eq!(seen.lock().unwrap().len(),1,"one real HTTP request across complete concurrent entrypoints");
        let connection=db::open(&context.db_path).unwrap();
        let state:String=connection.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[id],|r|r.get(0)).unwrap();
        assert_eq!(state,"completed");
        let counts:(i64,i64,i64)=connection.query_row("SELECT (SELECT COUNT(*) FROM agent_assignments),(SELECT spent_requests FROM agent_budget_ledger),(SELECT SUM(delivery_attempts) FROM agent_messages)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(counts,(1,1,2),"one assignment, one settlement, one consumption per message");
        let _=fs::remove_dir_all(root);
    }
}

#[test]
fn directive_proposal_completion_replay_rejects_substituted_result_id() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,mut context,id,_parent)=live_proposal_fixture("proposal-receipt-replay","@mapper 请分析已有证据");
    let (port,_seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(valid_proposal_text()))));
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let mut inbox=take_human_directives(&context).unwrap();
    apply_human_proposal_actions(&context,&mut inbox).unwrap();
    let connection=db::open(&context.db_path).unwrap();
    assert!(proposals::complete(&connection,inbox.lease.as_ref().unwrap(),&id,"forged-result").is_err(),"completed is not permission to accept a different receipt");
    let message:String=connection.query_row("SELECT result_message_id FROM agent_directive_proposals WHERE directive_id=?1",[&id],|r|r.get(0)).unwrap();
    proposals::complete(&connection,inbox.lease.as_ref().unwrap(),&id,&message).unwrap();
    connection.execute("UPDATE agent_messages SET payload_json=json_set(payload_json,'$.summary','substituted summary') WHERE id=?1",[&message]).unwrap();
    assert!(proposals::complete(&connection,inbox.lease.as_ref().unwrap(),&id,&message).unwrap_err().contains("payload_conflict"));
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_proposal_concurrent_received_recovery_never_reissues_http() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    for valid in [true,false] {
        let (root,mut context,id,_parent)=live_proposal_fixture("proposal-received-race","@mapper 请分析已有证据");
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(500,"text/plain","must not be called".into())));
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        let lease=take_human_directives(&context).unwrap().lease.unwrap();
        let connection=db::open(&context.db_path).unwrap();
        let job=proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
        consume_proposal_mailbox(&connection,&lease,&job.child.run_id,&job.request_message_id,"human_assessment_request",&job.input).unwrap();
        assert!(proposals::start(&connection,&lease,&id).unwrap());
        let response=validated_human_proposal(if valid { valid_proposal_text() } else { "invalid" },false);
        proposals::record_response(&connection,&lease,&id,&response,&serde_json::json!({"inputTokens":10,"cachedInputTokens":0,"outputTokens":10,"totalTokens":20,"modelRequests":1})).unwrap();
        let barrier=std::sync::Arc::new(std::sync::Barrier::new(4));
        let workers=(0..4).map(|_| {
            let context=context.clone(); let barrier=barrier.clone(); let lease=lease.clone();
            std::thread::spawn(move || {
                let mut inbox=HumanDirectiveContext{lease:Some(lease),items:vec![]};
                barrier.wait();
                apply_human_proposal_actions(&context,&mut inbox)
            })
        }).collect::<Vec<_>>();
        for worker in workers { worker.join().unwrap().unwrap(); }
        assert!(seen.lock().unwrap().is_empty());
        let result:(String,i64,i64)=connection.query_row("SELECT p.state,(SELECT spent_requests FROM agent_budget_ledger),(SELECT COUNT(*) FROM agent_messages WHERE kind='human_assessment_result') FROM agent_directive_proposals p WHERE directive_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(result,((if valid {"completed"} else {"failed"}).into(),1,1));
        let _=fs::remove_dir_all(root);
    }
}

#[test]
fn directive_proposal_recovers_local_commit_failures_without_duplicate_model_calls() {
    for (name,trigger) in [
        ("settlement", "CREATE TRIGGER fail_proposal_commit BEFORE UPDATE OF budget_settled_at ON agent_assignments WHEN NEW.budget_settled_at<>'' BEGIN SELECT RAISE(ABORT,'injected_proposal_commit'); END;"),
        ("finish", "CREATE TRIGGER fail_proposal_commit BEFORE UPDATE OF state ON agent_assignments WHEN NEW.state='completed' BEGIN SELECT RAISE(ABORT,'injected_proposal_commit'); END;"),
        ("mailbox", "CREATE TRIGGER fail_proposal_commit BEFORE INSERT ON agent_messages WHEN NEW.kind='human_assessment_result' BEGIN SELECT RAISE(ABORT,'injected_proposal_commit'); END;"),
        ("receipt", "CREATE TRIGGER fail_proposal_commit BEFORE UPDATE OF status ON agent_user_directives WHEN NEW.status='completed' BEGIN SELECT RAISE(ABORT,'injected_proposal_commit'); END;"),
    ] {
        let (root,mut context,id,_parent)=live_proposal_fixture(name,"@mapper 请分析已有证据");
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(valid_proposal_text()))));
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        let connection=db::open(&context.db_path).unwrap();
        connection.execute_batch(trigger).unwrap();
        let mut inbox=take_human_directives(&context).unwrap();
        assert!(apply_human_proposal_actions(&context,&mut inbox).unwrap_err().contains("injected_proposal_commit"));
        let state:String=connection.query_row("SELECT state FROM agent_directive_proposals WHERE directive_id=?1",[&id],|r|r.get(0)).unwrap();
        assert_eq!(state,"received");
        let receipt:(String,String,i64,i64)=connection.query_row("SELECT a.state,a.budget_settled_at,(SELECT spent_requests FROM agent_budget_ledger),(SELECT COUNT(*) FROM agent_messages WHERE kind='human_assessment_result') FROM agent_assignments a JOIN agent_directive_proposals p ON p.assignment_id=a.id WHERE p.directive_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(receipt,("running".into(),String::new(),0,0),"{name}: all local receipt writes must roll back together");
        connection.execute_batch("DROP TRIGGER fail_proposal_commit").unwrap();
        assert_eq!(apply_human_proposal_actions(&context,&mut inbox).unwrap().len(),1,"{name}");
        assert_eq!(seen.lock().unwrap().len(),1,"{name}: duplicate billed child request");
        let state:String=connection.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[&id],|r|r.get(0)).unwrap();
        assert_eq!(state,"completed");
        let requests:i64=connection.query_row("SELECT spent_requests FROM agent_budget_ledger",[],|r|r.get(0)).unwrap();
        assert_eq!(requests,1,"{name}: duplicate settlement");
        let _=fs::remove_dir_all(root);
    }
}

#[test]
fn directive_proposal_real_executor_child_preserves_reviewer_budget() {
    use crate::agent_runtime::multi_agent::scheduler;
    // Test the actual scheduled executor reservation; it performs no target I/O.
    // 27k with an 8k held executor leaves ~19k free: one 4k proposal + 15k Reviewer floor, not a second.
    let f = root_tick_fixture_protocol_limits("proposal-executor", "http://127.0.0.1:9/v1", true, (27_000,20));
    let mut context=f.context.clone();
    let connection=db::open(&context.db_path).unwrap();
    let executor=scheduler::schedule_child(&connection,&f.actor,crate::agent_runtime::contract::AgentRole::WebExecutor,crate::agent_runtime::contract::AgentLane::TargetTouching,
        "human-reviewer-floor",&context.execution_plan.as_json(),1,&[],8_000,1).unwrap();
    scheduler::mark_child_running(&connection,&f.actor,&executor).unwrap();
    let id=confirm_queue_directive(&connection,&f.actor,"@mapper 请评估已有证据");
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(valid_proposal_text()))));
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let mut inbox=take_human_directives(&context).unwrap();
    assert_eq!(apply_human_proposal_actions(&context,&mut inbox).unwrap().len(),1);
    assert_eq!(seen.lock().unwrap().len(),1);
    let (tokens,requests):(i64,i64)=connection.query_row("SELECT total_tokens-spent_tokens-reserved_tokens,total_requests-spent_requests-reserved_requests FROM agent_budget_ledger",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert!(tokens>=15_000 && requests>=1,"review reservation consumed: {tokens}/{requests}");
    let status:String=connection.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[id],|r|r.get(0)).unwrap();
    assert_eq!(status,"completed");
    let second=confirm_queue_directive(&connection,&f.actor,"@investigator 请分析已有证据");
    let mut inbox=take_human_directives(&context).unwrap();
    apply_human_proposal_actions(&context,&mut inbox).unwrap();
    let reason:String=connection.query_row("SELECT rejection_code FROM agent_user_directives WHERE id=?1",[second],|r|r.get(0)).unwrap();
    assert_eq!(reason,"proposal_budget_reserved_for_review");
    assert_eq!(seen.lock().unwrap().len(),1,"cannot steal Reviewer's floor for another proposal");
    scheduler::finish_child(&connection,&f.actor,&executor,true,"fixture done").unwrap();
    let _=fs::remove_dir_all(&f.f.root);
}
