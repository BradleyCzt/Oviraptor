// All three production target brokers must use the same remaining root time.
fn budget_clock_target_transport_contract(kind:&str) {
    let entered=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));let calls=entered.clone();
    let (port,_seen,_stop)=spawn_endpoint(std::sync::Arc::new(move |_| {
        calls.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(Duration::from_secs(4));
        (200,"application/json",r#"{"order":{"id":"owner"}}"#.into())
    }));
    let target=format!("http://127.0.0.1:{port}");
    let (root,context,mut runtime,request,lease,child,control)=match kind {
        "http"=>{let (root,context,runtime,request)=real_http_transport_fixture(&target,0);
            (root,context,Some(runtime),Some(request),None,None,None)},
        "public"=>{let (root,mut context,lease)=public_surface_fixture(&target);let child=public_surface_child(&mut context,&lease);
            (root,context,None,None,Some(lease),Some(child),None)},
        _=>{let (root,context,child,control)=budget_authorization_fixture_for_target(Some(&target));
            (root,context,None,None,None,Some(child),Some(control))},
    };
    let db=db::open(&context.db_path).unwrap();
    let root_id:String=db.query_row("SELECT root_run_id FROM agent_runs WHERE id=?1",[&context.run.as_ref().unwrap().run_id],|r|r.get(0)).unwrap();
    db.execute_batch("DROP TRIGGER budget_limit_no_update").unwrap();
    db.execute("UPDATE agent_budget_limits SET hard_limit=2000 WHERE root_run_id=?1 AND dimension='wall_time_ms'",[&root_id]).unwrap();
    let started=std::time::Instant::now();
    let result=match kind {
        "http"=>agent_http_exchange(&context,runtime.as_mut().unwrap(),request.as_ref().unwrap()).map(|_|()).map_err(|e|e.to_string()),
        "public"=>capture_public_surface(&context,lease.as_ref().unwrap(),child.as_ref().unwrap()).map(|_|()),
        _=>{let control=control.as_ref().unwrap();execute_authorization_side(&context,child.as_ref().unwrap(),control,"owner",&control.owner_identity,&control.owner_object_url).map(|_|())},
    };
    assert!(result.is_err(),"{kind}: root expired before result, got {result:?}");
    assert!(started.elapsed()<Duration::from_millis(3300),"{kind}: did not close at root deadline: {:?}",started.elapsed());
    assert_eq!(entered.load(std::sync::atomic::Ordering::SeqCst),1,"{kind}: one actual request, no retry");
    let cost=crate::agent_runtime::multi_agent::budget::balance(&db,&root_id,None,"target_requests").unwrap();
    assert_eq!((cost.reserved,cost.consumed,cost.indeterminate),(0,0,1),"{kind}: sent cost cannot refund");
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_clock_target_http_transport_obeys_shared_deadline() {budget_clock_target_transport_contract("http");}
#[test]
fn budget_clock_target_public_transport_obeys_shared_deadline() {budget_clock_target_transport_contract("public");}
#[test]
fn budget_clock_target_authorization_transport_obeys_shared_deadline() {budget_clock_target_transport_contract("authorization");}
