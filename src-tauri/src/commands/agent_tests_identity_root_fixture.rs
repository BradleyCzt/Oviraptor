// Genuine new creator/Root/model at birth; role replies remain scripted.
type IdentityRootBoundary = std::sync::Arc<std::sync::Mutex<Option<Box<dyn FnOnce(&rusqlite::Connection) + Send>>>>;
fn identity_root_fixture(tag: &str, forbidden: bool, invalid: bool, boundary: IdentityRootBoundary)
    -> (RootTickFixture, Seen, std::sync::Arc<std::sync::atomic::AtomicBool>) {
    let database=std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let original=database.clone();
    let (port,seen,stop)=spawn_endpoint(std::sync::Arc::new(move |wire| {
        let text=if wire.contains("You are the Root Coordinator") {
            let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            if input["basis"]["phase"] == "lease-expired" {
                json!({"schemaVersion":1,"observed":["original signed expired identity metadata"],"missing":[],"suggestions":["assess:lease_expired"],"costNotes":[],"risks":[]}).to_string()
            } else if input["basis"]["phase"] == "identity-session-output" {
                if let Some(apply)=boundary.lock().unwrap().take() {
                    let db=db::open(original.lock().unwrap().as_ref().unwrap()).unwrap(); apply(&db);
                }
                json!({"schemaVersion":1,"observed":["paid identity metadata assessment"],"missing":["no actual identity comparison"],
                    "suggestions":if forbidden {vec!["dispatch:web_executor"]}else{vec!["assess:identity_session_metadata"]},"costNotes":[],"risks":[]}).to_string()
            } else {
                let response:JsonValue=serde_json::from_str(&fresh_multi_root_wire_response(&wire).unwrap()).unwrap();
                response["choices"][0]["message"]["content"].as_str().unwrap().into()
            }
        } else if wire.contains("SPA/API Mapper") {
            r#"{"summary":"frozen authenticated hints only","priorityContracts":[],"risks":[]}"#.into()
        } else if wire.contains("IdentitySession") {
            json!({"summary":"validated metadata only; no target effect proven","observedAuthentication":["one validated handle"],
                "evidenceGaps":["missing actual identity comparison"],"authorizationProven":invalid}).to_string()
        } else {
            return (200,"application/json",model_round(&[],10));
        };
        (200,"application/json",proposal_model_response(&text))
    }));
    let fixture=web_mode_fixture("multi","https://authorized.example.test");
    let mut f=root_tick_fixture_from_web_mode_turns(tag,&format!("http://127.0.0.1:{port}/v1"),true,(60_000,20),None,fixture,Some(1));
    *database.lock().unwrap()=Some(f.context.db_path.clone());
    drop(f.parent.take());
    let db=db::open(&f.context.db_path).unwrap();
    let document=json!({"scopeHosts":["authorized.example.test"],"cookies":[{"name":"session","value":"fixture-identity-secret"}]});
    db.execute("INSERT INTO browser_auth_sessions(id,project_id,owner_scan_id,name,entry_url,status,session_json,expires_at)
        SELECT 'identity-one',project_id,id,'Identity One',?2,'valid',?3,?4 FROM sentinel_scans WHERE id=?1",
        params![f.context.scan_id,f.context.target_url,document.to_string(),(chrono::Utc::now()+chrono::Duration::hours(8)).to_rfc3339()]).unwrap();
    assert_eq!(db.execute("UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,'$.authSessionIds',json(?2)) WHERE scan_id=?1",params![f.context.scan_id,json!(["identity-one"]).to_string()]).unwrap(),1);
    (f,seen,stop)
}
fn identity_root_child(db:&rusqlite::Connection, root:&str)->crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
    let (assignment_id,run_id)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='identity_session'",[root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild {assignment_id,run_id,role:crate::agent_runtime::contract::AgentRole::IdentitySession}
}
fn identity_root_mapper(db:&rusqlite::Connection,root:&str)->crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
    let (assignment_id,run_id)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='spa_api_mapper'",[root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild {assignment_id,run_id,role:crate::agent_runtime::contract::AgentRole::SpaApiMapper}
}
