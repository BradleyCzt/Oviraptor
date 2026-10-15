fn source_guidance_confirm(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    text: &str,
) -> String {
    use crate::agent_runtime::multi_agent::directive;
    let draft = directive::create_draft_in_thread(db, &lease.scan_id, lease.attempt_number,
        &lease.root_run_id, &lease.target_key, "coordinator", text,
        &format!("coordinator:{}",lease.root_run_id), lease.lease_epoch, &lease.fencing_token).unwrap();
    directive::confirm_bound_draft(db,&lease.scan_id,&draft.id,draft.revision,&draft.draft_hash).unwrap().id
}

#[test]
fn source_guidance_addressed_focus_drafts_preserve_actions_and_permissions() {
    use crate::agent_runtime::multi_agent::directive;
    let (root,db,_record,lease)=source_specialist_fixture();
    for (text,expected) in [
        ("@repo_mapper 请关注冻结源码的模块边界",Some("repo_mapper")),
        ("@源码分析 请解释冻结源码的鉴权条件",Some("source_analyst")),
        ("@source_analyst focus on input validation",Some("source_analyst")),
        ("@reviewer 请关注证据缺口",Some("evidence_reviewer")),
        ("@source_analyst 请暂停当前分析",None),
        ("@source_analyst 请增加预算",None),
        ("@source_analyst 请解释并修改这些文件",None),
        ("@source_analyst 请关注并扩大范围",None),
        ("@reviewer 请直接确认这些风险",None),
        ("@reviewer 跳过审查",None),
        ("@source_analyst @repo_mapper 请关注权限检查",None),
        ("@source_analyst_extra 请关注权限检查",None),
        ("@source_analyst 请创建独立分析任务",None),
    ] {
        let draft=directive::create_draft_in_thread(&db,&lease.scan_id,lease.attempt_number,
            &lease.root_run_id,&lease.target_key,"coordinator",text,"team",lease.lease_epoch,&lease.fencing_token).unwrap();
        if let Some(role)=expected {
            assert_eq!(draft.intent,"source_analysis_focus","{text}");
            assert_eq!(draft.requested_roles,[role]);
            assert_eq!(draft.estimated_requests,0);
            assert!(draft.confirmation_required);
            assert!(draft.safe_execution_text.contains("没有适用阶段则不送达"));
            assert!(draft.reason_codes.contains(&"source_focus_next_unfrozen_role_phase".into()));
        } else {
            assert_ne!(draft.intent,"source_analysis_focus","{text}");
            assert_ne!(draft.intent,"priority_adjustment","addressed actions must not become general focus: {text}");
        }
    }
    assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_user_directives",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    db.execute("INSERT INTO agent_messages(id,run_id,root_run_id,kind,correlation_id,dedup_key) VALUES('focus-private',?1,?1,'assessment','focus-private','focus-private')",[&lease.root_run_id]).unwrap();
    let private=directive::create_draft_in_thread(&db,&lease.scan_id,lease.attempt_number,
        &lease.root_run_id,&lease.target_key,"coordinator","@source_analyst 请关注权限检查","focus-private",
        lease.lease_epoch,&lease.fencing_token).unwrap();
    assert_eq!(private.intent,"agent_proposal_request","private threads cannot silently address another phase");
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_addressed_focus_waits_for_its_role_and_never_falls_back() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,directive::source_guidance as guidance}};
    let (root,db,_record,lease)=source_specialist_fixture();
    let analyst=source_guidance_confirm(&db,&lease,"@source_analyst 请关注冻结源码的权限检查");
    let reviewer=source_guidance_confirm(&db,&lease,"@reviewer 请关注冻结源码的证据缺口");
    guidance::freeze(&db,&lease,AgentRole::RepoMapper,false).unwrap();
    assert!(source::assessment_input(&db,&lease,AgentRole::RepoMapper).unwrap().get("operatorGuidance").is_none());
    for id in [&analyst,&reviewer] {
        assert_eq!(db.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[id],|r|r.get::<_,String>(0)).unwrap(),"accepted");
    }
    guidance::freeze(&db,&lease,AgentRole::SourceAnalyst,false).unwrap();
    let frozen=source::assessment_input(&db,&lease,AgentRole::SourceAnalyst).unwrap();
    assert_eq!(frozen["operatorGuidance"]["directives"].as_array().unwrap().len(),1);
    assert_eq!(frozen["operatorGuidance"]["directives"][0]["id"],analyst);
    let later=source_guidance_confirm(&db,&lease,"@source_analyst 请解释冻结源码的参数边界");
    guidance::freeze(&db,&lease,AgentRole::SourceAnalyst,false).unwrap();
    assert_eq!(source::assessment_input(&db,&lease,AgentRole::SourceAnalyst).unwrap(),frozen);
    guidance::freeze(&db,&lease,AgentRole::SourceAnalyst,true).unwrap();
    assert_eq!(guidance::attach(&db,&lease,AgentRole::SourceAnalyst,true,json!({})).unwrap()["operatorGuidance"]["directives"][0]["id"],later);
    let too_late=source_guidance_confirm(&db,&lease,"@source_analyst 请解释冻结源码的异常处理");
    guidance::freeze(&db,&lease,AgentRole::RepoMapper,true).unwrap();
    assert!(guidance::attach(&db,&lease,AgentRole::RepoMapper,true,json!({})).unwrap().get("operatorGuidance").is_none());
    // Neither a role absent from this v1 plan nor an exhausted role is rerouted.
    for id in [&reviewer,&too_late] {
        assert_eq!(db.query_row("SELECT count(*) FROM agent_source_guidance g,json_each(g.guidance_json,'$.directives') i WHERE json_extract(i.value,'$.id')=?1",[id],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
    let tx=rusqlite::Transaction::new_unchecked(&db,rusqlite::TransactionBehavior::Immediate).unwrap();
    close_human_directives_in_transaction(&tx,&lease,"test_end").unwrap();
    for id in [&reviewer,&too_late] {
        let payload:String=tx.query_row("SELECT payload_json FROM agent_user_directives WHERE id=?1",[id],|r|r.get(0)).unwrap();
        let payload:JsonValue=serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["taskClosure"]["disposition"],"not_applied");
        assert!(payload["sourceGuidance"].is_null());
    }
    tx.commit().unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM agent_capability_leases",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_addressed_snapshot_cannot_change_recipient_with_a_rehashed_copy() {
    use crate::agent_runtime::{contract::AgentRole,store::stable_hash,multi_agent::directive::source_guidance as guidance};
    let (root,db,_record,lease)=source_specialist_fixture();
    source_guidance_confirm(&db,&lease,"@source_analyst 请解释冻结源码的鉴权条件");
    guidance::freeze(&db,&lease,AgentRole::SourceAnalyst,false).unwrap();
    let saved:String=db.query_row("SELECT guidance_json FROM agent_source_guidance WHERE phase_key='assessment:source_analyst'",[],|r|r.get(0)).unwrap();
    let mut corrupt:JsonValue=serde_json::from_str(&saved).unwrap();
    corrupt["phase"]=json!("assessment:repo_mapper");
    corrupt["role"]=json!("repo_mapper");
    let text=corrupt.to_string();
    // Fault injection uses a new row, never disables the immutable protections.
    db.execute("INSERT INTO agent_source_guidance(root_run_id,phase_key,role,lease_epoch,fencing_token,guidance_json,guidance_hash) VALUES(?1,'assessment:repo_mapper','repo_mapper',?2,?3,?4,?5)",
        rusqlite::params![lease.root_run_id,lease.lease_epoch,lease.fencing_token,text,stable_hash(&text)]).unwrap();
    assert_eq!(guidance::attach(&db,&lease,AgentRole::RepoMapper,false,json!({})).unwrap_err(),"source_guidance_draft_invalid");
    assert_eq!(db.query_row("SELECT guidance_json FROM agent_source_guidance WHERE phase_key='assessment:repo_mapper'",[],|r|r.get::<_,String>(0)).unwrap(),text);
    assert!(guidance::attach(&db,&lease,AgentRole::SourceAnalyst,false,json!({})).is_ok());
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_freezes_each_phase_and_routes_late_messages_to_next_phase() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,directive::source_guidance as guidance}};
    let (root,db,_record,lease)=source_specialist_fixture();
    let first=source_guidance_confirm(&db,&lease,"请重点解释冻结源码里的权限检查");
    guidance::freeze(&db,&lease,AgentRole::RepoMapper,false).unwrap();
    let before=source::assessment_input(&db,&lease,AgentRole::RepoMapper).unwrap();
    assert_eq!(before["operatorGuidance"]["directives"][0]["id"],first);
    let late=source_guidance_confirm(&db,&lease,"请补充分析冻结源码里的输入校验");
    guidance::freeze(&db,&lease,AgentRole::RepoMapper,false).unwrap();
    assert_eq!(source::assessment_input(&db,&lease,AgentRole::RepoMapper).unwrap(),before);
    guidance::freeze(&db,&lease,AgentRole::SourceAnalyst,false).unwrap();
    let next=source::assessment_input(&db,&lease,AgentRole::SourceAnalyst).unwrap();
    assert_eq!(next["operatorGuidance"]["directives"].as_array().unwrap().len(),1);
    assert_eq!(next["operatorGuidance"]["directives"][0]["id"],late);
    let frozen:String=db.query_row("SELECT guidance_json FROM agent_source_guidance LIMIT 1",[],|r|r.get(0)).unwrap();
    assert!(!frozen.contains(&lease.fencing_token));
    assert!(db.execute("UPDATE agent_source_guidance SET guidance_json='{}'",[]).is_err());
    assert!(db.execute("DELETE FROM agent_source_guidance",[]).is_err());
    assert_eq!(db.query_row("SELECT count(*) FROM agent_specialist_calls",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    {
        let tx=rusqlite::Transaction::new_unchecked(&db,rusqlite::TransactionBehavior::Deferred).unwrap();
        assert!(guidance::project_delivery(&tx,&first).unwrap().is_none());
    }
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_empty_phase_and_legacy_assignment_never_absorb_late_chat() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler,directive::source_guidance as guidance}};
    let (root,db,_record,lease)=source_specialist_fixture();
    guidance::freeze(&db,&lease,AgentRole::RepoMapper,false).unwrap();
    let role=AgentRole::SourceAnalyst;
    let slice=source::task_slice(&db,&lease,role).unwrap();
    scheduler::prepare_readonly_child(&db,&lease,role,"source_results_ready",&slice,8_000).unwrap();
    source_guidance_confirm(&db,&lease,"请关注冻结源码里的边界条件");
    for role in [AgentRole::RepoMapper,AgentRole::SourceAnalyst] {
        guidance::freeze(&db,&lease,role,false).unwrap();
        assert!(source::assessment_input(&db,&lease,role).unwrap().get("operatorGuidance").is_none());
    }
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_guidance",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    guidance::freeze(&db,&lease,AgentRole::SourceAnalyst,true).unwrap();
    assert_eq!(guidance::attach(&db,&lease,role,true,json!({})).unwrap()["operatorGuidance"]["directives"].as_array().unwrap().len(),1);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_unknown_actions_are_deferred_without_new_permissions() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::directive::source_guidance as guidance};
    let (root,db,_record,lease)=source_specialist_fixture();
    for text in ["请暂停当前分析","@reviewer 请直接确认这些风险","请增加预算",
        "@source_analyst 请创建独立分析任务","@source_analyst 请暂停当前分析"] {
        source_guidance_confirm(&db,&lease,text);
    }
    guidance::freeze(&db,&lease,AgentRole::RepoMapper,false).unwrap();
    assert!(guidance::attach(&db,&lease,AgentRole::RepoMapper,false,json!({})).unwrap().get("operatorGuidance").is_none());
    assert_eq!(db.query_row("SELECT count(*) FROM agent_user_directives WHERE status='deferred' AND rejection_code='source_guidance_requires_dedicated_action'",[],|r|r.get::<_,i64>(0)).unwrap(),5);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_capability_leases",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_freeze_rolls_back_claim_and_acceptance_on_failure() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::directive::source_guidance as guidance};
    let (root,db,_record,lease)=source_specialist_fixture();
    let id=source_guidance_confirm(&db,&lease,"请关注冻结源码里的参数检查");
    db.execute_batch("CREATE TRIGGER fail_source_guidance BEFORE INSERT ON agent_source_guidance BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert_eq!(guidance::freeze(&db,&lease,AgentRole::RepoMapper,false).unwrap_err(),"source_guidance_freeze_unconfirmed");
    assert_eq!(db.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[&id],|r|r.get::<_,String>(0)).unwrap(),"pending");
    db.execute_batch("DROP TRIGGER fail_source_guidance").unwrap();
    guidance::freeze(&db,&lease,AgentRole::RepoMapper,false).unwrap();
    let mut stale=lease.clone();stale.fencing_token="stale".into();
    assert!(guidance::freeze(&db,&stale,AgentRole::SourceAnalyst,false).is_err());
    assert!(guidance::attach(&db,&stale,AgentRole::RepoMapper,false,json!({})).is_err());
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_response_delivery_is_atomic_and_replay_is_verified() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler,specialist,directive::source_guidance as guidance}};
    let (root,db,_record,lease)=source_specialist_fixture();
    let id=source_guidance_confirm(&db,&lease,"请解释冻结源码中的鉴权条件");
    let role=AgentRole::SourceAnalyst;
    guidance::freeze(&db,&lease,role,false).unwrap();
    let slice=source::task_slice(&db,&lease,role).unwrap();
    let child=scheduler::prepare_readonly_child(&db,&lease,role,"source_results_ready",&slice,12_000).unwrap();
    let request=source_specialist_request(&db,&lease,role);
    let specialist::Start::Dispatch(call)=specialist::start(&db,&lease,&child,&request).unwrap() else {panic!()};
    let usage=AgentTokenUsage{input_tokens:10,output_tokens:10,total_tokens:20,model_requests:1,..Default::default()};
    db.execute_batch("CREATE TRIGGER fail_guidance_delivery BEFORE UPDATE OF payload_json ON agent_user_directives BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(specialist::record_received(&db,&call,"Guided assessment",false,&usage).is_err());
    assert_eq!(db.query_row("SELECT state FROM agent_specialist_calls",[],|r|r.get::<_,String>(0)).unwrap(),"executing");
    assert_eq!(db.query_row("SELECT count(*) FROM agent_events WHERE event_type='model_round_completed'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    db.execute_batch("DROP TRIGGER fail_guidance_delivery").unwrap();
    specialist::record_received(&db,&call,"Guided assessment",false,&usage).unwrap();
    assert!(matches!(specialist::start(&db,&lease,&child,&request).unwrap(),specialist::Start::Received(_)));
    let payload:String=db.query_row("SELECT payload_json FROM agent_user_directives WHERE id=?1",[&id],|r|r.get(0)).unwrap();
    let payload:JsonValue=serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["sourceGuidance"]["assignmentId"],child.assignment_id);
    assert_eq!(payload["modelDelivery"]["runId"],child.run_id);
    let event:String=db.query_row("SELECT payload_json FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",[&child.run_id],|r|r.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<JsonValue>(&event).unwrap()["deliveredDirectiveIds"],json!([id]));
    {
        let tx=rusqlite::Transaction::new_unchecked(&db,rusqlite::TransactionBehavior::Immediate).unwrap();
        assert!(guidance::project_delivery(&tx,&id).unwrap().is_some());
        tx.execute("UPDATE agent_events SET payload_json=json_set(payload_json,'$.deliveredDirectiveIds',json('[]')) WHERE run_id=?1 AND event_type='model_round_completed'",[&child.run_id]).unwrap();
        assert!(guidance::project_delivery(&tx,&id).is_err());
    }
    {
        let tx=rusqlite::Transaction::new_unchecked(&db,rusqlite::TransactionBehavior::Immediate).unwrap();
        tx.execute_batch("CREATE TRIGGER damage_guidance_closure AFTER UPDATE OF status ON agent_user_directives WHEN NEW.status='completed' BEGIN UPDATE agent_user_directives SET status='accepted' WHERE id=NEW.id; END;").unwrap();
        assert_eq!(close_human_directives_in_transaction(&tx,&lease,"test_end").unwrap_err(),"directive_closure_postcondition_conflict");
    }
    assert_eq!(db.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[&id],|r|r.get::<_,String>(0)).unwrap(),"accepted");
    db.execute("UPDATE agent_user_directives SET payload_json=json_remove(payload_json,'$.sourceGuidance') WHERE id=?1",[&id]).unwrap();
    assert_eq!(specialist::start(&db,&lease,&child,&request).unwrap_err(),"source_guidance_delivery_invalid");
    {
        let tx=rusqlite::Transaction::new_unchecked(&db,rusqlite::TransactionBehavior::Immediate).unwrap();
        assert_eq!(guidance::project_delivery(&tx,&id).unwrap_err(),"source_guidance_delivery_missing");
        tx.execute("UPDATE agent_user_directives SET payload_json=json_remove(payload_json,'$.modelDelivery') WHERE id=?1",[&id]).unwrap();
        assert_eq!(guidance::project_delivery(&tx,&id).unwrap_err(),"source_guidance_delivery_missing");
        assert_eq!(close_human_directives_in_transaction(&tx,&lease,"test_end").unwrap_err(),"source_guidance_delivery_missing");
    }
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_real_transport_sends_frozen_input_not_later_messages() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,scheduler,directive::source_guidance as guidance}};
    let (port,seen,stop)=crate::scanning::agent_tests::spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",
        json!({"choices":[{"message":{"role":"assistant","content":"guided source assessment"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())));
    let environment=source_specialist_test_environment(port);
    let (root,db,record,lease)=source_specialist_true_born_model_fixture(&environment);
    let id=source_guidance_confirm(&db,&lease,"请解释冻结源码中的访问控制");
    let role=AgentRole::RepoMapper;
    guidance::freeze(&db,&lease,role,false).unwrap();
    let input=source::assessment_input(&db,&lease,role).unwrap();
    let slice=source::task_slice(&db,&lease,role).unwrap();
    let profile=agent_model_profile(&environment,None).unwrap();
    let (tokens,_)=source_assessment_budget(&source_assessment_messages(SOURCE_ASSESSMENT_SYSTEM,&input),&profile).unwrap();
    let child=scheduler::prepare_readonly_child(&db,&lease,role,"source_results_ready",&slice,tokens).unwrap();
    let late=source_guidance_confirm(&db,&lease,"另外请关注冻结源码中的空值处理");
    let path=root.join("oviraptor.sqlite3");
    let context=SpecialistTransportContext { supervision: None,db_path:&path,scan_id:&record.scan_id,attempt_number:1,target_key:&lease.target_key,
        run_id:&lease.root_run_id,environment:&environment,proxy:None,usage_dir:&root,deadline:None};
    let mut wrong=input.clone();wrong["operatorGuidance"]["directives"][0]["text"]=json!("unconfirmed override");
    assert!(specialist_round_transport(&context,&lease,&child,SOURCE_ASSESSMENT_SYSTEM,wrong).is_err());
    assert_eq!(seen.lock().unwrap().len(),0);
    specialist_round_transport(&context,&lease,&child,SOURCE_ASSESSMENT_SYSTEM,input.clone()).unwrap();
    specialist_round_transport(&context,&lease,&child,SOURCE_ASSESSMENT_SYSTEM,input).unwrap();
    let requests=seen.lock().unwrap();assert_eq!(requests.len(),1);
    assert!(requests[0].contains(&id));assert!(!requests[0].contains(&late));
    drop(requests);stop.store(true,std::sync::atomic::Ordering::Relaxed);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_tool_round_freezes_and_receipts_the_actual_input() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{source,scheduler,source_rounds as rounds,directive::source_guidance as guidance}};
    let (root,db,_record,lease)=source_specialist_fixture();
    db.execute("UPDATE agent_runs SET status='running',started_at=datetime('now','localtime') WHERE id=?1",[&lease.root_run_id]).unwrap();
    let role=AgentRole::SourceAnalyst;
    let id=source_guidance_confirm(&db,&lease,"请分析冻结源码中的鉴权遗漏");
    guidance::freeze(&db,&lease,role,true).unwrap();
    let slice=source::tool_task_slice(&db,&lease,role,1).unwrap();
    let capabilities=source::tool_capabilities(role).unwrap();
    let child=scheduler::schedule_child(&db,&lease,role,AgentLane::ReadOnlyAnalysis,"source_tools_ready",&slice,1,&capabilities,24_000,3).unwrap();
    scheduler::mark_child_running(&db,&lease,&child).unwrap();
    let input=guidance::attach(&db,&lease,role,true,json!({"sourceTask":slice})).unwrap();
    let tools=agent_tool_specs().iter().filter(|s|capabilities.iter().any(|n|n==s.name)).map(|s|s.as_function_spec()).collect::<Vec<_>>();
    let request=json!({"messages":[{"role":"user","content":input.to_string()}],"tools":tools});
    let mut wrong=request.clone();
    wrong["messages"][0]["content"]=json!({"sourceTask":slice,"operatorGuidance":{"directives":[]}}).to_string().into();
    assert_eq!(rounds::start_authorized(&db,&lease,&child,1,&wrong,8_000,|_|Ok(())).unwrap_err(),"source_guidance_input_mismatch");
    let rounds::Start::Dispatch(call)=rounds::start_authorized(&db,&lease,&child,1,&request,8_000,|_|Ok(())).unwrap() else {panic!()};
    let late=source_guidance_confirm(&db,&lease,"请另外分析冻结源码中的异常处理");
    assert!(!call.request().to_string().contains(&late));
    let response=source_round_result("inventory","repo.inventory",json!({}));
    rounds::record_received(&db,&call,&response).unwrap();
    assert!(matches!(rounds::start_authorized(&db,&lease,&child,1,&request,8_000,|_|Ok(())).unwrap(),rounds::Start::Received(..)));
    let payload:String=db.query_row("SELECT payload_json FROM agent_user_directives WHERE id=?1",[&id],|r|r.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<JsonValue>(&payload).unwrap()["sourceGuidance"]["phase"],"tools:source_analyst");
    let event:String=db.query_row("SELECT payload_json FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",[&child.run_id],|r|r.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<JsonValue>(&event).unwrap()["deliveredDirectiveIds"],json!([id]));
    assert_eq!(db.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[&late],|r|r.get::<_,String>(0)).unwrap(),"pending");
    // Complete the local tool before freezing the next round's operator input.
    assert!(rounds::prepare_next_authorized(&db,&lease,&child,2,&request,|_|Ok(())).is_err());
    rounds::execute_tool(&db,&call,0,|_|Ok(()),|_,_,_|Ok(json!({"inventory":[]}))).unwrap();
    let base=rounds::continuation(&db,&call).unwrap();
    let before=source_guidance_confirm(&db,&lease,"@source_analyst 请关注冻结源码的输入校验");
    let other=source_guidance_confirm(&db,&lease,"@repo_mapper 请关注冻结源码的依赖布局");
    let mut forged=base.clone();forged["messages"][0]["content"]="tampered".into();
    assert!(rounds::prepare_next_authorized(&db,&lease,&child,2,&forged,|_|Ok(())).unwrap_err().contains("transcript"));
    assert!(rounds::prepare_next_authorized(&db,&lease,&child,2,&base,|_|Err("revoked".into())).is_err());
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_guidance WHERE phase_key LIKE '%:round:%'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    let next=rounds::prepare_next_authorized(&db,&lease,&child,2,&base,|_|Ok(())).unwrap();
    let focus:JsonValue=serde_json::from_str(next["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap()).unwrap();
    let ids=focus["operatorGuidance"]["directives"].as_array().unwrap().iter().map(|v|v["id"].as_str().unwrap()).collect::<Vec<_>>();
    assert!(ids.contains(&late.as_str()));assert!(ids.contains(&before.as_str()));assert!(!ids.contains(&other.as_str()));
    let after=source_guidance_confirm(&db,&lease,"@source_analyst 请解释冻结源码的错误处理");
    assert_eq!(rounds::prepare_next_authorized(&db,&lease,&child,2,&base,|_|Ok(())).unwrap(),next);
    assert!(!next.to_string().contains(&after));
    assert_eq!(call.request(),&request);
    assert!(rounds::start_authorized(&db,&lease,&child,2,&base,8_000,|_|Ok(())).unwrap_err().contains("transcript"));
    let rounds::Start::Dispatch(second)=rounds::start_authorized(&db,&lease,&child,2,&next,8_000,|_|Ok(())).unwrap() else {panic!()};
    assert!(rounds::start_authorized(&db,&lease,&child,2,&next,8_000,|_|Ok(())).unwrap_err().contains("unknown"));
    let tx=rusqlite::Transaction::new_unchecked(&db,rusqlite::TransactionBehavior::Immediate).unwrap();
    assert!(guidance::project_delivery(&tx,&late).unwrap().is_none());tx.commit().unwrap();
    rounds::record_received(&db,&second,&source_round_result("finish","assignment.finish",json!({"summary":"done","gaps":[]}))).unwrap();
    let tx=rusqlite::Transaction::new_unchecked(&db,rusqlite::TransactionBehavior::Immediate).unwrap();
    let delivery=guidance::project_delivery(&tx,&late).unwrap().unwrap();
    assert_eq!(delivery["phase"],"tools:source_analyst:round:2");
    assert_eq!(delivery["childRunId"],child.run_id);
    tx.execute("UPDATE agent_user_directives SET payload_json=json_remove(payload_json,'$.sourceGuidance','$.modelDelivery') WHERE id=?1",[&late]).unwrap();
    assert_eq!(guidance::project_delivery(&tx,&late).unwrap_err(),"source_guidance_delivery_missing");
    tx.rollback().unwrap();
    assert!(rounds::continuation(&db,&second).unwrap_err().contains("no_continuation"));
    drop(db);
    let db=rusqlite::Connection::open(root.join("oviraptor.sqlite3")).unwrap();
    assert_eq!(rounds::prepare_next_authorized(&db,&lease,&child,2,&base,|_|Ok(())).unwrap(),next);
    assert!(matches!(rounds::start_authorized(&db,&lease,&child,2,&next,8_000,|_|Ok(())).unwrap(),rounds::Start::Received(..)));
    assert_eq!(db.query_row("SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",[&child.run_id],|r|r.get::<_,i64>(0)).unwrap(),2);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_freeze_checks_post_write_acceptance_and_deferral() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::directive::source_guidance as guidance};
    for (text,trigger,error) in [
        ("请解释冻结源码里的鉴权逻辑",
         "CREATE TRIGGER damage_guidance AFTER INSERT ON agent_source_guidance BEGIN UPDATE agent_user_directives SET status='deferred' WHERE status='accepted'; END;",
         "source_guidance_acceptance_unconfirmed"),
        ("请暂停当前分析",
         "CREATE TRIGGER damage_guidance AFTER INSERT ON agent_source_guidance BEGIN UPDATE agent_user_directives SET rejection_code='' WHERE status='deferred'; END;",
         "source_guidance_deferral_unconfirmed"),
    ] {
        let (root,db,_record,lease)=source_specialist_fixture();
        let id=source_guidance_confirm(&db,&lease,text);
        db.execute_batch(trigger).unwrap();
        assert_eq!(guidance::freeze(&db,&lease,AgentRole::RepoMapper,false).unwrap_err(),error);
        assert_eq!(db.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[&id],|r|r.get::<_,String>(0)).unwrap(),"pending");
        assert_eq!(db.query_row("SELECT count(*) FROM agent_source_guidance",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        db.execute_batch("DROP TRIGGER damage_guidance").unwrap();
        guidance::freeze(&db,&lease,AgentRole::RepoMapper,false).unwrap();
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

fn source_guidance_next_round_fixture() -> (PathBuf,rusqlite::Connection,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,JsonValue) {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{source,scheduler,source_rounds as rounds,directive::source_guidance as guidance}};
    let (root,db,_record,lease)=source_specialist_fixture();
    db.execute("UPDATE agent_runs SET status='running',started_at=datetime('now','localtime') WHERE id=?1",[&lease.root_run_id]).unwrap();
    let role=AgentRole::SourceAnalyst;
    guidance::freeze(&db,&lease,role,true).unwrap();
    let slice=source::tool_task_slice(&db,&lease,role,1).unwrap();
    let capabilities=source::tool_capabilities(role).unwrap();
    let child=scheduler::schedule_child(&db,&lease,role,AgentLane::ReadOnlyAnalysis,"source_tools_ready",&slice,1,&capabilities,24_000,3).unwrap();
    scheduler::mark_child_running(&db,&lease,&child).unwrap();
    let tools=agent_tool_specs().iter().filter(|s|capabilities.iter().any(|n|n==s.name)).map(|s|s.as_function_spec()).collect::<Vec<_>>();
    let request=json!({"messages":[{"role":"user","content":json!({"sourceTask":slice}).to_string()}],"tools":tools});
    let rounds::Start::Dispatch(first)=rounds::start_authorized(&db,&lease,&child,1,&request,8_000,|_|Ok(())).unwrap() else {panic!()};
    rounds::record_received(&db,&first,&source_round_result("inventory","repo.inventory",json!({}))).unwrap();
    rounds::execute_tool(&db,&first,0,|_|Ok(()),|_,_,_|Ok(json!({"inventory":[]}))).unwrap();
    let next=rounds::continuation(&db,&first).unwrap();
    (root,db,lease,child,next)
}

#[test]
fn source_guidance_next_round_freeze_rolls_back_on_fault_revocation_and_budget() {
    use crate::agent_runtime::multi_agent::source_rounds as rounds;
    for case in ["write_fault","post_revocation","budget"] {
        let (root,db,lease,child,base)=source_guidance_next_round_fixture();
        let id=source_guidance_confirm(&db,&lease,"@source_analyst 请关注冻结源码的鉴权");
        if case=="write_fault" {
            db.execute_batch("CREATE TRIGGER fail_round_focus AFTER INSERT ON agent_source_guidance WHEN NEW.phase_key LIKE '%:round:%' BEGIN UPDATE agent_user_directives SET status='deferred' WHERE status='accepted'; END;").unwrap();
        } else if case=="budget" {
            db.execute("UPDATE agent_assignments SET reserved_requests=1 WHERE id=?1",[&child.assignment_id]).unwrap();
        }
        let check=|db:&rusqlite::Connection| {
            let count:i64=db.query_row("SELECT count(*) FROM agent_source_guidance WHERE phase_key LIKE '%:round:%'",[],|r|r.get(0)).unwrap();
            if case=="post_revocation" && count>0 {Err("revoked_after_freeze".into())} else {Ok(())}
        };
        let error=rounds::prepare_next_authorized(&db,&lease,&child,2,&base,check).unwrap_err();
        assert!(error.contains(match case {"write_fault"=>"acceptance_unconfirmed","post_revocation"=>"revoked_after_freeze",_=>"budget"}),"{case}: {error}");
        assert_eq!(db.query_row("SELECT count(*) FROM agent_source_guidance WHERE phase_key LIKE '%:round:%'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(db.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[&id],|r|r.get::<_,String>(0)).unwrap(),"pending");
        assert_eq!(db.query_row("SELECT count(*) FROM agent_source_model_rounds",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_guidance_empty_round_snapshot_moves_late_chat_only_to_third_round() {
    use crate::agent_runtime::multi_agent::{source_rounds as rounds,directive::source_guidance as guidance};
    let (root,db,lease,child,base)=source_guidance_next_round_fixture();
    let next=rounds::prepare_next_authorized(&db,&lease,&child,2,&base,|_|Ok(())).unwrap();
    assert_eq!(next,base);
    let id=source_guidance_confirm(&db,&lease,"@source_analyst 请解释冻结源码的校验顺序");
    assert_eq!(rounds::prepare_next_authorized(&db,&lease,&child,2,&base,|_|Ok(())).unwrap(),next);
    let rounds::Start::Dispatch(second)=rounds::start_authorized(&db,&lease,&child,2,&next,8_000,|_|Ok(())).unwrap() else {panic!()};
    rounds::record_received(&db,&second,&source_round_result("inventory2","repo.inventory",json!({}))).unwrap();
    rounds::execute_tool(&db,&second,0,|_|Ok(()),|_,_,_|Ok(json!({"inventory":[]}))).unwrap();
    let base3=rounds::continuation(&db,&second).unwrap();
    let third_request=rounds::prepare_next_authorized(&db,&lease,&child,3,&base3,|_|Ok(())).unwrap();
    assert!(third_request.to_string().contains(&id));assert!(!next.to_string().contains(&id));
    let rounds::Start::Dispatch(third)=rounds::start_authorized(&db,&lease,&child,3,&third_request,8_000,|_|Ok(())).unwrap() else {panic!()};
    rounds::record_received(&db,&third,&source_round_result("done","assignment.finish",json!({"summary":"checked","gaps":[]}))).unwrap();
    let tx=rusqlite::Transaction::new_unchecked(&db,rusqlite::TransactionBehavior::Immediate).unwrap();
    assert_eq!(guidance::project_delivery(&tx,&id).unwrap().unwrap()["phase"],"tools:source_analyst:round:3");
    tx.commit().unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_model_rounds",[],|r|r.get::<_,i64>(0)).unwrap(),3);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_legacy_tool_assignment_does_not_acquire_round_focus() {
    use crate::agent_runtime::multi_agent::source_rounds as rounds;
    let (root,db,_context,lease,child,request)=source_round_fixture();
    let rounds::Start::Dispatch(first)=rounds::start_authorized(&db,&lease,&child,1,&request,8_000,|_|Ok(())).unwrap() else {panic!()};
    rounds::record_received(&db,&first,&source_round_result("inventory","repo.inventory",json!({}))).unwrap();
    rounds::execute_tool(&db,&first,0,|_|Ok(()),|_,_,_|Ok(json!({"inventory":[]}))).unwrap();
    let id=source_guidance_confirm(&db,&lease,"@source_analyst 请关注冻结源码的版本约束");
    let base=rounds::continuation(&db,&first).unwrap();
    assert_eq!(rounds::prepare_next_authorized(&db,&lease,&child,2,&base,|_|Ok(())).unwrap(),base);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_guidance",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(db.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[&id],|r|r.get::<_,String>(0)).unwrap(),"pending");
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_legacy_phase_only_draft_is_not_reinterpreted_at_a_round_boundary() {
    use crate::agent_runtime::{store::stable_hash,multi_agent::{directive,source_rounds as rounds}};
    let (root,db,lease,child,base)=source_guidance_next_round_fixture();
    let mut draft=directive::create_draft_in_thread(&db,&lease.scan_id,lease.attempt_number,
        &lease.root_run_id,&lease.target_key,"coordinator","@source_analyst 请关注冻结源码的错误处理","team",
        lease.lease_epoch,&lease.fencing_token).unwrap();
    assert!(draft.reason_codes.contains(&"source_guidance_tool_round_eligible".into()));
    // A valid pre-round-support draft, not a corrupted hash and not a disabled
    // integrity trigger. Its original phase-only semantics remain confirmed.
    draft.reason_codes.retain(|code|code!="source_guidance_tool_round_eligible");
    draft.safe_execution_text=draft.safe_execution_text.replace("（含工具阶段后续轮次）","");
    let mut material=serde_json::to_value(&draft).unwrap();
    for key in ["id","confirmationRequired","draftHash","status","confirmedDirectiveId"] {
        material.as_object_mut().unwrap().remove(key);
    }
    material["boundLeaseEpoch"]=json!(draft.bound_lease_epoch);
    material["boundFencingToken"]=json!(draft.bound_fencing_token);
    draft.draft_hash=stable_hash(&material.to_string());
    db.execute("UPDATE agent_directive_drafts SET reason_codes_json=?2,safe_execution_text=?3,draft_hash=?4 WHERE id=?1",
        rusqlite::params![draft.id,serde_json::to_string(&draft.reason_codes).unwrap(),draft.safe_execution_text,draft.draft_hash]).unwrap();
    let id=directive::confirm_bound_draft(&db,&lease.scan_id,&draft.id,draft.revision,&draft.draft_hash).unwrap().id;
    assert_eq!(rounds::prepare_next_authorized(&db,&lease,&child,2,&base,|_|Ok(())).unwrap(),base);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_guidance g,json_each(g.guidance_json,'$.directives') i WHERE json_extract(i.value,'$.id')=?1",[&id],|r|r.get::<_,i64>(0)).unwrap(),0);
    let saved:(String,String)=db.query_row("SELECT draft_hash,safe_execution_text FROM agent_directive_drafts WHERE id=?1",[&draft.id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(saved,(draft.draft_hash,draft.safe_execution_text));
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_immutable_snapshot_allows_parent_cascade_deletion() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::directive::source_guidance as guidance};
    let (root,db,_record,lease)=source_specialist_fixture();
    source_guidance_confirm(&db,&lease,"请解释冻结源码里的鉴权逻辑");
    guidance::freeze(&db,&lease,AgentRole::RepoMapper,false).unwrap();
    assert!(db.execute("DELETE FROM agent_source_guidance",[]).is_err());
    assert_eq!(db.execute("DELETE FROM agent_runs WHERE id=?1",[&lease.root_run_id]).unwrap(),1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_guidance",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_database_upgrade_and_reopen_preserve_existing_records() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::{source,directive::source_guidance as guidance}};
    let (root,connection,_record,lease)=source_specialist_fixture();
    let id=source_guidance_confirm(&connection,&lease,"请说明冻结源码的权限检查范围");
    let before:String=connection.query_row("SELECT payload_json FROM agent_user_directives WHERE id=?1",[&id],|r|r.get(0)).unwrap();
    let draft_count:i64=connection.query_row("SELECT count(*) FROM agent_directive_drafts",[],|r|r.get(0)).unwrap();
    let lease_state=|connection:&rusqlite::Connection| connection.query_row("SELECT json_object('epoch',lease_epoch,'fence',fencing_token,'expiry',lease_expires_at,'heartbeat',heartbeat_at) FROM agent_coordinator_leases WHERE root_run_id=?1",[&lease.root_run_id],|r|r.get::<_,String>(0)).unwrap();
    let original_lease=lease_state(&connection);
    // Simulate the immediately preceding schema in this disposable fixture.
    // There are no snapshots yet; only the new table and its triggers are absent.
    connection.execute_batch("DROP TABLE agent_source_guidance;").unwrap();
    drop(connection);
    let path=db::initialize(&root).unwrap();
    let connection=db::open(&path).unwrap();
    assert_eq!(connection.query_row("SELECT payload_json FROM agent_user_directives WHERE id=?1",[&id],|r|r.get::<_,String>(0)).unwrap(),before);
    assert_eq!(connection.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[&id],|r|r.get::<_,String>(0)).unwrap(),"pending");
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_directive_drafts",[],|r|r.get::<_,i64>(0)).unwrap(),draft_count);
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_source_guidance",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(lease_state(&connection),original_lease);
    guidance::freeze(&connection,&lease,AgentRole::RepoMapper,false).unwrap();
    let frozen=source::assessment_input(&connection,&lease,AgentRole::RepoMapper).unwrap();
    let snapshot:String=connection.query_row("SELECT guidance_json FROM agent_source_guidance",[],|r|r.get(0)).unwrap();
    drop(connection);
    db::initialize(&root).unwrap();
    let connection=db::open(&path).unwrap();
    assert_eq!(source::assessment_input(&connection,&lease,AgentRole::RepoMapper).unwrap(),frozen);
    assert_eq!(connection.query_row("SELECT guidance_json FROM agent_source_guidance",[],|r|r.get::<_,String>(0)).unwrap(),snapshot);
    assert_eq!(connection.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[&id],|r|r.get::<_,String>(0)).unwrap(),"accepted");
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_specialist_calls",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(lease_state(&connection),original_lease);
    assert!(connection.execute("UPDATE agent_source_guidance SET guidance_json='{}'",[]).is_err());
    assert!(connection.execute("DELETE FROM agent_source_guidance",[]).is_err());
    drop(connection);fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_guidance_production_entry_consumes_chat_at_next_real_phase() {
    use std::sync::{Arc,Mutex,atomic::{AtomicUsize,Ordering}};
    for addressed in [false,true] {
    let path=Arc::new(Mutex::new(None::<PathBuf>));
    let ids=Arc::new(Mutex::new(Vec::<String>::new()));
    let count=Arc::new(AtomicUsize::new(0));
    let (server_path,server_ids,server_count)=(path.clone(),ids.clone(),count.clone());
    let (port,seen,stop)=crate::scanning::agent_tests::spawn_endpoint(Arc::new(move |request| {
        let body:JsonValue=serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let number=server_count.fetch_add(1,Ordering::SeqCst);
        let input:JsonValue=body["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_str().and_then(|s|serde_json::from_str(s).ok()).unwrap_or(JsonValue::Null);
        if number<4 {
            let db=rusqlite::Connection::open(server_path.lock().unwrap().as_ref().unwrap()).unwrap();
            let (scan,target,root):(String,String,String)=db.query_row("SELECT scan_id,target_url,id FROM agent_runs WHERE role='coordinator'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            let lease=crate::agent_runtime::multi_agent::lease::acquire_coordinator_lease(&db,&scan,1,&target,&root,600).unwrap();
            let text=match (addressed,number) {
                (true,0)=>"@source_analyst 请关注冻结源码里的权限校验",
                (true,1)=>"@repo_mapper 请解释冻结源码里的异常处理",
                (_,2)=>"@repo_mapper 请关注冻结源码里的路由布局",
                (_,3)=>"@repo_mapper 请解释冻结源码里的工具结束情况",
                (false,0)=>"请关注冻结源码里的权限校验",
                (false,_)=>"请补充解释冻结源码里的异常处理",
                (true,_)=>unreachable!("guidance is injected only for the first four requests"),
            };
            server_ids.lock().unwrap().push(source_guidance_confirm(&db,&lease,text));
        }
        let message=if input["phase"]=="source_coverage_review" {
            json!({"role":"assistant","content":source_coverage_fixture_response(&input).to_string()})
        } else if body.get("tools").is_none() {
            json!({"role":"assistant","content":"{\"summary\":\"scoped assessment\"}"})
        } else {
            let finished=body["messages"].as_array().unwrap().iter().any(|m|m["role"]=="tool");
            let (name,args)=if finished {("assignment.finish",json!({"summary":"Scoped inventory checked","gaps":[]}))} else {("repo.inventory",json!({}))};
            json!({"role":"assistant","content":"Scoped inspection","tool_calls":[{"id":name,"type":"function","function":{"name":name,"arguments":args.to_string()}}]})
        };
        (200,"application/json",json!({"choices":[{"message":message,"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
    }));
    let environment=source_specialist_test_environment(port);
    let (root,db,record)=source_dispatch_fixture(&environment,None);
    *path.lock().unwrap()=Some(root.join("oviraptor.sqlite3"));
    let report=run_native_source_scan(&root.join("oviraptor.sqlite3"),&root,&record.scan_id,1,&root.join("attempt-0001"),&record.source_path,&record.scan_type,&record.diff_base).unwrap();
    assert_eq!(report["sourceMultiAgent"]["status"],"source_coverage_reviewed","{report}");
    let ids=ids.lock().unwrap();assert_eq!(ids.len(),4);
    let requests=seen.lock().unwrap();assert_eq!(requests.len(),7);
    assert!(!requests[0].contains(&ids[0]));
    assert!(requests[1].contains(&ids[0]));assert!(!requests[1].contains(&ids[1]));
    assert!(requests[2].contains(&ids[1]));assert!(!requests[2].contains(&ids[0]));
    assert!(!requests[2].contains(&ids[2]));assert!(requests[3].contains(&ids[2]));
    assert!(requests.iter().all(|request|!request.contains(&ids[3])));
    for (id,phase) in ids.iter().zip(["assessment:source_analyst","tools:repo_mapper","tools:repo_mapper:round:2"]) {
        let payload:String=db.query_row("SELECT payload_json FROM agent_user_directives WHERE id=?1",[id],|r|r.get(0)).unwrap();
        let payload:JsonValue=serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["sourceGuidance"]["phase"],phase);
        assert_eq!(payload["taskClosure"]["disposition"],"analysis_guidance_delivered");
        assert_eq!(db.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[id],|r|r.get::<_,String>(0)).unwrap(),"completed");
        let event:String=db.query_row("SELECT payload_json FROM agent_events WHERE run_id=?1 AND sequence=?2",rusqlite::params![payload["sourceGuidance"]["childRunId"].as_str(),payload["sourceGuidance"]["eventSequence"].as_i64()],|r|r.get(0)).unwrap();
        assert_eq!(serde_json::from_str::<JsonValue>(&event).unwrap()["deliveredDirectiveIds"],json!([id]));
    }
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_guidance",[],|r|r.get::<_,i64>(0)).unwrap(),7);
    let view=native_scan_status(&db,&record.scan_id).unwrap();
    for id in ids.iter().take(3) {
        let item=view["timeline"].as_array().unwrap().iter().find(|item|item["id"]==*id).unwrap();
        assert_eq!(item["sourceGuidance"]["state"],"model_received");
    }
    let undelivered=view["timeline"].as_array().unwrap().iter().find(|item|item["id"]==ids[3]).unwrap();
    assert!(undelivered["sourceGuidance"].is_null());
    assert_eq!(undelivered["taskClosure"]["disposition"],"not_applied");
    // Only delivered directives require delivery receipts. The fourth focus
    // arrived after the mapper's final round and legitimately has no receipt.
    let late_original:String=db.query_row("SELECT payload_json FROM agent_user_directives WHERE id=?1",[&ids[3]],|r|r.get(0)).unwrap();
    assert!(db.execute("UPDATE agent_user_directives SET payload_json=json_remove(payload_json,'$.taskClosure') WHERE id=?1",[&ids[3]]).is_err());
    db.execute("UPDATE agent_user_directives SET payload_json=json_set(payload_json,'$.sourceGuidance.childRunId','forged-child') WHERE id=?1",[&ids[3]]).unwrap();
    let forged=native_scan_status(&db,&record.scan_id).unwrap();
    let forged=forged["timeline"].as_array().unwrap().iter().find(|item|item["id"]==ids[3]).unwrap();
    assert_eq!(forged["deliveryState"],"receipt_unverified");
    db.execute("UPDATE agent_user_directives SET payload_json=?2 WHERE id=?1",rusqlite::params![ids[3],late_original]).unwrap();
    let restored=native_scan_status(&db,&record.scan_id).unwrap();
    let restored=restored["timeline"].as_array().unwrap().iter().find(|item|item["id"]==ids[3]).unwrap();
    assert_eq!(restored["deliveryState"],"persisted");
    assert!(restored["sourceGuidance"].is_null());
    assert_eq!(restored["taskClosure"]["disposition"],"not_applied");
    for id in ids.iter().take(3) {
        let original:String=db.query_row("SELECT payload_json FROM agent_user_directives WHERE id=?1",[id],|r|r.get(0)).unwrap();
        assert!(db.execute("UPDATE agent_user_directives SET payload_json=json_remove(payload_json,'$.taskClosure') WHERE id=?1",[id]).is_err());
        for expression in [
            "json_set(payload_json,'$.sourceGuidance.childRunId','forged-child')",
            "json_remove(payload_json,'$.sourceGuidance')",
            "json_set(payload_json,'$.sourceGuidance',NULL)",
            "json_remove(payload_json,'$.sourceGuidance','$.modelDelivery')",
        ] {
            db.execute(&format!("UPDATE agent_user_directives SET payload_json={expression} WHERE id=?1"),[id]).unwrap();
            let damaged:String=db.query_row("SELECT payload_json FROM agent_user_directives WHERE id=?1",[id],|r|r.get(0)).unwrap();
            let view=native_scan_status(&db,&record.scan_id).unwrap();
            let item=view["timeline"].as_array().unwrap().iter().find(|item|item["id"]==*id).unwrap();
            assert!(item["sourceGuidance"].is_null(),"{expression}: {item}");
            assert_eq!(item["deliveryState"],"receipt_unverified","{expression}");
            assert!(item["reasonCodes"].as_array().unwrap().contains(&json!("source_guidance_receipt_unverified")));
            assert_eq!(db.query_row("SELECT payload_json FROM agent_user_directives WHERE id=?1",[id],|r|r.get::<_,String>(0)).unwrap(),damaged);
            db.execute("UPDATE agent_user_directives SET payload_json=?2 WHERE id=?1",rusqlite::params![id,original]).unwrap();
        }
    }
    stop.store(true,Ordering::SeqCst);drop(requests);drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_guidance_reviewers_receive_only_their_frozen_focus_and_preserve_gaps() {
    use std::sync::{Arc,Mutex,atomic::{AtomicUsize,Ordering}};
    for addressed in [false,true] {
    for no_candidates in [false,true] {
        let path=Arc::new(Mutex::new(None::<PathBuf>));
        let ids=Arc::new(Mutex::new(Vec::<String>::new()));
        let count=Arc::new(AtomicUsize::new(0));
        let (server_path,server_ids,server_count)=(path.clone(),ids.clone(),count.clone());
        let (port,seen,stop)=crate::scanning::agent_tests::spawn_endpoint(Arc::new(move |request| {
            let body:JsonValue=serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            let number=server_count.fetch_add(1,Ordering::SeqCst);
            let last=body["messages"].as_array().unwrap().last().unwrap();
            let input:JsonValue=last["content"].as_str().and_then(|s|serde_json::from_str(s).ok()).unwrap_or(JsonValue::Null);
            // Submit while the prior provider call is in flight. The current
            // phase is frozen; only the next applicable phase can consume it.
            if number==5 || input["phase"]=="source_review" || input["phase"]=="source_coverage_review" {
                let db=rusqlite::Connection::open(server_path.lock().unwrap().as_ref().unwrap()).unwrap();
                let (scan,target,root):(String,String,String)=db.query_row("SELECT scan_id,target_url,id FROM agent_runs WHERE role='coordinator'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
                let lease=crate::agent_runtime::multi_agent::lease::acquire_coordinator_lease(&db,&scan,1,&target,&root,600).unwrap();
                let text=if number==5 {"请重点解释冻结源码的权限校验与证据缺口"}
                    else if input["phase"]=="source_review" {"请补充解释冻结源码的覆盖范围与未知项"}
                    else {"请最后解释冻结源码的异常处理"};
                let text=if addressed {format!("@reviewer 请关注以下分析重点：{text}")} else {text.into()};
                server_ids.lock().unwrap().push(source_guidance_confirm(&db,&lease,&text));
            }
            let message=if input["phase"]=="source_review" || input["phase"]=="source_coverage_review" {
                assert!(body.get("tools").is_none());
                assert_eq!(input["toolsGranted"],json!([]));
                assert_eq!(input["targetRequestsGranted"],0);
                assert_eq!(input["hostActionsGranted"],0);
                assert!(input["operatorGuidance"]["constraint"].as_str().unwrap().contains("not a verdict or evidence"));
                let response=if input["phase"]=="source_review" {
                    source_review_contract_response(&input["reviewMaterial"]["decisionContract"])
                } else {source_coverage_fixture_response(&input)};
                json!({"role":"assistant","content":response.to_string()})
            } else if body.get("tools").is_none() {
                json!({"role":"assistant","content":"Frozen source assessment"})
            } else {
                let analyst=body["tools"].as_array().unwrap().iter().any(|t|t["function"]["name"]=="evidence.submit_candidate");
                let (name,args)=if last["role"]=="tool" {
                    ("assignment.finish",json!({"summary":"Read-only inspection complete","gaps":["dependency_graph_incomplete"]}))
                } else if analyst && !no_candidates {
                    ("evidence.submit_candidate",json!({"title":"Needs review","rationale":"Needs corroboration","path":"app.py","line":1}))
                } else {("repo.inventory",json!({}))};
                json!({"role":"assistant","tool_calls":[{"id":name,"type":"function","function":{"name":name,"arguments":args.to_string()}}]})
            };
            (200,"application/json",json!({"choices":[{"message":message,"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
        }));
        let (root,db,record)=source_dispatch_fixture(&source_specialist_test_environment(port),None);
        *path.lock().unwrap()=Some(root.join("oviraptor.sqlite3"));
        analysis_view_run(&root,&record,|_,engine,_,_,scratch,_|Ok(if no_candidates {
            source_regression_outcome(engine,scratch)
        } else {source_result_outcome(engine,scratch,"app.py","review-guidance")})).unwrap();
        let report=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001")).unwrap();
        stop.store(true,Ordering::SeqCst);
        assert_eq!(report["modelRequests"],if no_candidates {7} else {8});
        assert_eq!(report["independentReviewCompleted"],true);
        assert_eq!(report["confirmedFindings"],0);
        assert_eq!(report["sourceCoverageDecision"]["decision"]["coverageSufficient"],false);
        assert_ne!(report["gate"]["status"],"passed");
        let ids=ids.lock().unwrap();
        let phases=if no_candidates {vec!["review:source_coverage"]} else {vec!["review:source_candidates","review:source_coverage"]};
        assert_eq!(ids.len(),phases.len()+1);
        let requests=seen.lock().unwrap();
        assert_eq!(requests.len(),if no_candidates {7} else {8});
        let mut children=std::collections::BTreeSet::new();
        for (offset,phase) in phases.iter().enumerate() {
            let id=&ids[offset];
            let body:JsonValue=serde_json::from_str(requests[6+offset].split("\r\n\r\n").nth(1).unwrap()).unwrap();
            let input:JsonValue=serde_json::from_str(body["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap()).unwrap();
            assert_eq!(input["operatorGuidance"]["phase"],*phase);
            assert_eq!(input["operatorGuidance"]["directives"].as_array().unwrap().len(),1);
            assert_eq!(input["operatorGuidance"]["directives"][0]["id"],*id);
            let original:String=db.query_row("SELECT payload_json FROM agent_user_directives WHERE id=?1",[id],|r|r.get(0)).unwrap();
            let payload:JsonValue=serde_json::from_str(&original).unwrap();
            assert_eq!(payload["sourceGuidance"]["phase"],*phase);
            assert_eq!(payload["sourceGuidance"]["advisoryOnly"],true);
            assert_eq!(payload["taskClosure"]["disposition"],"analysis_guidance_delivered");
            assert!(children.insert(payload["sourceGuidance"]["childRunId"].as_str().unwrap().to_owned()));
            let view=native_scan_status(&db,&record.scan_id).unwrap();
            let item=view["timeline"].as_array().unwrap().iter().find(|item|item["id"]==*id).unwrap();
            assert_eq!(item["sourceGuidance"],payload["sourceGuidance"]);
            let event:String=db.query_row("SELECT payload_json FROM agent_events WHERE run_id=?1 AND sequence=?2",
                rusqlite::params![payload["sourceGuidance"]["childRunId"].as_str(),payload["sourceGuidance"]["eventSequence"].as_i64()],|r|r.get(0)).unwrap();
            assert_eq!(serde_json::from_str::<JsonValue>(&event).unwrap()["deliveredDirectiveIds"],json!([id]));
            for expression in ["json_remove(payload_json,'$.sourceGuidance','$.modelDelivery')",
                "json_set(payload_json,'$.sourceGuidance.phase','review:wrong')",
                "json_set(payload_json,'$.sourceGuidance.childRunId','wrong-reviewer')"] {
                db.execute(&format!("UPDATE agent_user_directives SET payload_json={expression} WHERE id=?1"),[id]).unwrap();
                let damaged=source_reviewer_reentry_state(&db);
                let view=native_scan_status(&db,&record.scan_id).unwrap();
                let item=view["timeline"].as_array().unwrap().iter().find(|item|item["id"]==*id).unwrap();
                assert_eq!(item["deliveryState"],"receipt_unverified");
                assert!(item["sourceGuidance"].is_null());
                assert_eq!(source_reviewer_reentry_state(&db),damaged,"projection must not repair receipts");
                db.execute("UPDATE agent_user_directives SET payload_json=?2 WHERE id=?1",rusqlite::params![id,original]).unwrap();
            }
        }
        let late:String=db.query_row("SELECT payload_json FROM agent_user_directives WHERE id=?1",[ids.last().unwrap()],|r|r.get(0)).unwrap();
        let late:JsonValue=serde_json::from_str(&late).unwrap();
        assert!(late["sourceGuidance"].is_null());
        assert_ne!(late["taskClosure"]["disposition"],"analysis_guidance_delivered");
        assert_eq!(db.query_row("SELECT count(*) FROM agent_source_guidance WHERE role='evidence_reviewer'",[],|r|r.get::<_,i64>(0)).unwrap(),i64::try_from(phases.len()).unwrap());
        drop(requests);drop(db);fs::remove_dir_all(root).unwrap();
    }
    }
}

#[test]
fn source_guidance_review_freeze_is_atomic_and_legacy_assignment_keeps_original_input() {
    use crate::agent_runtime::multi_agent::{directive::source_guidance as guidance,source_reviewer,source_coverage_reviewer};
    for coverage in [false,true] {
        for legacy in [false,true] {
            let (root,db,lease,result)=source_reviewer_execution_fixture_using_calls("valid",None,4,None,8,|root,db,record,lease| {
                let checkpoint=if legacy {"undispatched"} else if coverage {"before_coverage"} else {"before_review"};
                if coverage {source_coverage_checkpoint_fixture(root,db,record,lease,checkpoint);}
                else {source_reviewer_checkpoint_fixture(root,db,record,lease,checkpoint);}
                let id=source_guidance_confirm(db,lease,"请关注冻结源码审查中的证据缺口");
                let slice=|db:&rusqlite::Connection| if coverage {source_coverage_reviewer::task_slice(db,lease)} else {source_reviewer::task_slice(db,lease)};
                let tx=db.unchecked_transaction().unwrap();
                let original=slice(&tx).unwrap().unwrap();
                tx.rollback().unwrap();
                if !legacy {
                    db.execute_batch("CREATE TRIGGER fail_review_guidance BEFORE INSERT ON agent_source_guidance BEGIN SELECT RAISE(IGNORE); END;").unwrap();
                    let before=source_reviewer_reentry_state(db);
                    let tx=db.unchecked_transaction().unwrap();
                    assert_eq!(guidance::freeze_review_in_transaction(&tx,lease,coverage).unwrap_err(),"source_guidance_freeze_unconfirmed");
                    tx.rollback().unwrap();
                    assert_eq!(source_reviewer_reentry_state(db),before);
                    db.execute_batch("DROP TRIGGER fail_review_guidance").unwrap();
                }
                let tx=db.unchecked_transaction().unwrap();
                guidance::freeze_review_in_transaction(&tx,lease,coverage).unwrap();
                let frozen=slice(&tx).unwrap().unwrap();
                if legacy {assert_eq!(frozen,original);}
                else {assert_eq!(frozen["operatorGuidance"]["directives"][0]["id"],id);}
                tx.commit().unwrap();
                source_guidance_confirm(db,lease,"请补充说明冻结源码的异常边界");
                let tx=db.unchecked_transaction().unwrap();
                guidance::freeze_review_in_transaction(&tx,lease,coverage).unwrap();
                assert_eq!(slice(&tx).unwrap().unwrap(),frozen,"late chat cannot rewrite review input");
                tx.commit().unwrap();
                run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"))
            });
            assert_eq!(result.unwrap()["modelRequests"],8);
            let phase=if coverage {"review:source_coverage"} else {"review:source_candidates"};
            assert_eq!(db.query_row("SELECT count(*) FROM agent_source_guidance WHERE phase_key=?1 AND root_run_id=?2",rusqlite::params![phase,lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),i64::from(!legacy));
            drop(db);fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn source_guidance_received_review_resumes_without_resending_or_absorbing_late_chat() {
    use crate::agent_runtime::multi_agent::{directive::source_guidance as guidance,source_reviewer,source_coverage_reviewer};
    for coverage in [false,true] {
        let (root,db,lease,result)=source_reviewer_execution_fixture_using_calls("valid",None,4,None,8,|root,db,record,lease| {
            if coverage {source_coverage_checkpoint_fixture(root,db,record,lease,"before_coverage");}
            else {source_reviewer_checkpoint_fixture(root,db,record,lease,"before_review");}
            let id=source_guidance_confirm(db,lease,"请解释冻结源码审查的反证与未知项");
            let tx=db.unchecked_transaction().unwrap();
            guidance::freeze_review_in_transaction(&tx,lease,coverage).unwrap();
            let slice=if coverage {source_coverage_reviewer::task_slice(&tx,lease)} else {source_reviewer::task_slice(&tx,lease)}.unwrap().unwrap();
            assert!(guidance::project_delivery(&tx,&id).unwrap().is_none());
            tx.commit().unwrap();
            let database=root.join("oviraptor.sqlite3");
            let work=root.join("attempt-0001");
            let (model,runtime,_)=verify_source_runtime_contract(db,&record.scan_id,1,&work).unwrap();
            let proxy=source_runtime_proxy(&runtime).unwrap();
            let profile=agent_model_profile(&model,proxy).unwrap();
            let context=SpecialistTransportContext { supervision: None,db_path:&database,scan_id:&record.scan_id,attempt_number:1,
                target_key:&lease.target_key,run_id:&lease.root_run_id,environment:&model,proxy,usage_dir:&work,
                deadline:Some(std::time::Instant::now()+Duration::from_secs(180))};
            let system=if coverage {source_coverage_reviewer::SYSTEM} else {source_reviewer::SYSTEM};
            let (tokens,_)=source_assessment_budget(&source_assessment_messages(system,&slice),&profile).unwrap();
            let child=if coverage {source_coverage_reviewer::prepare(db,lease,&slice,tokens)} else {source_reviewer::prepare(db,lease,&slice,tokens)}.unwrap();
            specialist_round_transport(&context,lease,&child,system,slice.clone()).unwrap();
            let receipt:String=db.query_row("SELECT payload_json FROM agent_user_directives WHERE id=?1",[&id],|r|r.get(0)).unwrap();
            source_guidance_confirm(db,lease,"请补充解释冻结源码的参数边界");
            let tx=db.unchecked_transaction().unwrap();
            let actual=if coverage {source_coverage_reviewer::task_slice(&tx,lease)} else {source_reviewer::task_slice(&tx,lease)}.unwrap().unwrap();
            assert_eq!(actual,slice);
            assert_eq!(guidance::project_delivery(&tx,&id).unwrap().unwrap()["childRunId"],child.run_id);
            tx.rollback().unwrap();
            // A lost receipt before closure cannot degrade into ordinary
            // unapplied advice or trigger another provider request on resume.
            db.execute("UPDATE agent_user_directives SET payload_json=json_remove(payload_json,'$.sourceGuidance','$.modelDelivery') WHERE id=?1",[&id]).unwrap();
            let before=source_exit_snapshot(db);
            assert!(run_native_source_assessments(&database,&record.scan_id,1,&work).is_err());
            source_exit_assert_snapshot(db, lease, &before);
            db.execute("UPDATE agent_user_directives SET payload_json=?2 WHERE id=?1",rusqlite::params![id,receipt]).unwrap();
            run_native_source_assessments(&database,&record.scan_id,1,&work)
        });
        assert_eq!(result.unwrap()["modelRequests"],8);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1 AND role='evidence_reviewer'",[&lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),2);
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}
