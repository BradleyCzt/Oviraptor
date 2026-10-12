#[test]
fn budget_specialist_missing_provider_usage_keeps_receipt_but_rejects_result() {
    // All faults arrive through the real gateway. Root, Mapper and Reviewer
    // are independently paid; only the Investigator invoice lacks provenance.
    for fault in ["missing", "partial", "inconsistent"] {
        let (port,seen,stop)=specialist_original_endpoint(std::sync::Arc::new(move |request| {
            assert!(request.contains("Deep Investigator"));
            let mut body:JsonValue=serde_json::from_str(&changed_fact_reviewer_response(&request,true)).unwrap();
            match fault {
                "missing" => {body.as_object_mut().unwrap().remove("usage");},
                "partial" => {body["usage"]=json!({"total_tokens":20});},
                _ => {body["usage"]=json!({"prompt_tokens":10,"completion_tokens":10,"total_tokens":25});},
            }
            (200,"application/json",body.to_string())
        }));
        let (root,context,session)=specialist_original_gap_fixture(&format!("http://127.0.0.1:{port}/v1"));
        let original_web=specialist_original_take_web_seen(&session.lease.root_run_id);
        assert_eq!(original_web.count(),1);
        let db=db::open(&context.db_path).unwrap();
        assert_eq!(root_tick_count(&db,&session.lease.root_run_id,"publication"),4,
            "bootstrap, Mapper feedback, actual Web exit and existing business-logic assessment each have an original paid Root receipt");
        let _real=RealSpecialistTransport::enter();
        let error=multi_agent_investigate_review_gap(&context,&session,"candidate-gap",2,&json!(["missing control"])).unwrap_err();
        assert_eq!(error,"deep_investigator_model_usage_requires_reconciliation","{fault}");
        let db=db::open(&context.db_path).unwrap();
        let (assignment,rejection,reported):(String,String,bool)=db.query_row("SELECT assignment_id,json_extract(response_json,'$.rejection'),json_extract(response_json,'$.usageReported')
            FROM agent_specialist_calls WHERE role='deep_investigator' AND state='received'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(rejection,"model_usage_requires_reconciliation");assert!(!reported);
        for dimension in crate::agent_runtime::multi_agent::budget::DIMENSIONS.into_iter().take(3) {
            let b=crate::agent_runtime::multi_agent::budget::balance(&db,&session.lease.root_run_id,Some(&assignment),dimension).unwrap();
            assert_eq!((b.reserved,b.consumed,b.indeterminate),(0,0,4_000),"{fault}/{dimension}");
        }
        let requests=crate::agent_runtime::multi_agent::budget::balance(&db,&session.lease.root_run_id,Some(&assignment),"model_requests").unwrap();
        assert_eq!((requests.reserved,requests.consumed,requests.indeterminate),(0,1,0));
        assert_specialist_usage_pending(&db,"deep_investigator",4_000);
        assert_eq!(specialist_child_call_count(&seen),3);
        assert_eq!(specialist_handler_call_count(&seen),1);
        assert_eq!(seen.lock().unwrap().len(),3,"Root and Web requests use the separate frozen provider");
        assert_eq!(root_tick_count(&db,&session.lease.root_run_id,"publication"),5,
            "the gap assessment spends one further Root call before the unknown Investigator invoice");
        assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE kind IN ('gap_proposed','proposal_assessed')",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        let rows=web_mode_test_rows(&db);
        assert!(multi_agent_investigate_review_gap(&context,&session,"candidate-gap",2,&json!(["missing control"])).is_err());
        web_mode_assert_rows(&db,&rows);assert_eq!(seen.lock().unwrap().len(),3);
        assert_eq!(original_web.count(),1,"unknown usage must never retry the original Web SDK");
        assert_eq!(root_tick_count(&db,&session.lease.root_run_id,"publication"),5,"retry must not pay Root again");
        drop(db);drop(session);fs::remove_dir_all(root).unwrap();stop.store(true,std::sync::atomic::Ordering::SeqCst);
    }
}
