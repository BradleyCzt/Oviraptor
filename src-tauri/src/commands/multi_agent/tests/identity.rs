#[test]
fn scan_identity_mode_requires_task_bound_valid_distinct_target_scoped_sessions() {
    use crate::auth_session::{validated_scan_identities, ScanIdentityMode};

    let (root, db_path, _, lease) = multi_agent_test_root("identity-mode", 20_000, 10);
    let connection = db::open(&db_path).unwrap();
    let scan_id = lease.scan_id.as_str();
    let target = lease.target_key.as_str();
    assert_eq!(
        validated_scan_identities(&connection, scan_id, target).unwrap(),
        (ScanIdentityMode::AnonymousOnly, vec![])
    );
    let project_id: i64 = connection.query_row(
        "SELECT project_id FROM sentinel_scans WHERE id=?1", [scan_id], |row| row.get(0)
    ).unwrap();
    let add = |id: &str, token: &str, host: &str| {
        let document = serde_json::json!({
            "name": id, "scopeHosts": [host],
            "cookies": [{"name":"session", "value":token}]
        });
        connection.execute(
            "INSERT INTO browser_auth_sessions(id,project_id,owner_scan_id,name,entry_url,status,session_json,expires_at) \
             VALUES(?1,?2,?3,?1,?4,'valid',?5,?6)",
            rusqlite::params![id, project_id, scan_id, target, document.to_string(),
                (chrono::Utc::now() + chrono::Duration::hours(8)).to_rfc3339()],
        ).unwrap();
    };
    let policy = |ids: &[&str]| {
        connection.execute(
            "INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES(?1,?2) \
             ON CONFLICT(scan_id) DO UPDATE SET policy_json=excluded.policy_json",
            rusqlite::params![scan_id, serde_json::json!({"authSessionIds":ids}).to_string()],
        ).unwrap();
    };
    add("a", "one", "authorized.example.test");
    add("b", "two", "authorized.example.test");
    policy(&["a"]);
    assert_eq!(
        validated_scan_identities(&connection, scan_id, target).unwrap(),
        (ScanIdentityMode::SingleIdentity, vec!["a".into()])
    );
    policy(&["a", "b"]);
    assert_eq!(
        validated_scan_identities(&connection, scan_id, target).unwrap(),
        (ScanIdentityMode::IdentitySet, vec!["a".into(), "b".into()])
    );
    connection.execute("UPDATE browser_auth_sessions SET session_json=(SELECT session_json FROM browser_auth_sessions WHERE id='a') WHERE id='b'", []).unwrap();
    assert!(validated_scan_identities(&connection, scan_id, target).unwrap_err().contains("相同认证材料"));
    connection.execute("UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.cookies[0].value','two') WHERE id='b'", []).unwrap();
    connection.execute("UPDATE browser_auth_sessions SET owner_scan_id='another-task' WHERE id='b'", []).unwrap();
    assert!(validated_scan_identities(&connection, scan_id, target).unwrap_err().contains("未绑定"));
    connection.execute("UPDATE browser_auth_sessions SET owner_scan_id=?1,status='invalid' WHERE id='b'", [scan_id]).unwrap();
    assert!(validated_scan_identities(&connection, scan_id, target).unwrap_err().contains("有效状态"));
    connection.execute("UPDATE browser_auth_sessions SET status='valid',expires_at='expired' WHERE id='b'", []).unwrap();
    assert!(validated_scan_identities(&connection, scan_id, target).unwrap_err().contains("安全期限"));
    connection.execute("UPDATE browser_auth_sessions SET expires_at=?1,session_json=json_set(session_json,'$.scopeHosts[0]','other.example.test') WHERE id='b'",
        [(chrono::Utc::now() + chrono::Duration::hours(8)).to_rfc3339()]).unwrap();
    assert!(validated_scan_identities(&connection, scan_id, target).unwrap_err().contains("共同作用域"));
    policy(&["b"]);
    assert!(validated_scan_identities(&connection, scan_id, target).unwrap_err().contains("目标主机"));
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn identity_specialist_is_a_separate_read_only_run_with_a_delivered_message() {
    let _real=RealSpecialistTransport::enter();
    let boundary=std::sync::Arc::new(std::sync::Mutex::new(None));
    let (mut f,seen,stop)=identity_root_fixture("identity-child",false,false,boundary);
    let mut session=multi_agent_prepare(&mut f.context).unwrap();
    let connection=db::open(&f.context.db_path).unwrap();
    // prepare.rs: the validated account set, plus exactly one fixed
    // credential-free control (not another account and not an identity grant;
    // the authorization broker binds the same set).
    assert_eq!(f.context.identities,vec![AgentIdentity::scoped("identity-one"),AgentIdentity::anonymous()]);
    let identity=identity_root_child(&connection,&session.lease.root_run_id);
    assert_ne!(identity.run_id,session.mapper.run_id);assert_ne!(identity.run_id,session.executor.run_id);
    let (lane,status,capability):(String,String,String)=connection.query_row("SELECT a.lane,r.status,a.capability_lease_json FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1",[&identity.assignment_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(lane,"read_only_analysis");assert_eq!(status,"terminal");assert!(!capability.contains("replay_http"));
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_messages WHERE from_run_id=?1 AND kind='identity_assessment' AND acknowledged_at<>''",[&identity.run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
    let paid=crate::agent_runtime::multi_agent::specialist::received_for_reconciliation(&tx,&session.lease,&identity).unwrap();
    assert_eq!(paid.usage.model_requests,1);assert_eq!(paid.usage.total_tokens,20);tx.commit().unwrap();
    assert_eq!(seen.lock().unwrap().iter().filter(|w|w.contains("IdentitySession 分析专家")).count(),1);
    assert!(f.context.evidence.get("multiAgentIdentitySession").is_some());
    assert!(!f.context.evidence.to_string().contains("fixture-identity-secret"));
    let outcome=run_native_agent(&f.context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{}",outcome.detail());
    multi_agent_finish_execution(&f.context,&mut session,&outcome).unwrap();
    drop(session);drop(connection);drop(f);stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn identity_specialist_cannot_be_scheduled_for_an_anonymous_task() {
    use crate::agent_runtime::{contract::{AgentLane, AgentRole}, multi_agent::scheduler};
    let (root, db_path, _, lease) = multi_agent_test_root("identity-anon", 20_000, 10);
    let connection = db::open(&db_path).unwrap();
    assert_eq!(scheduler::schedule_child(
        &connection, &lease, AgentRole::IdentitySession, AgentLane::ReadOnlyAnalysis,
        "fake_identity", &serde_json::json!({}), 1,
        &["evidence.read".into()], 4_000, 1,
    ).unwrap_err(), "identity_session_requires_bound_identity");
    let assignments: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_assignments WHERE coordinator_run_id=?1",
        [&lease.root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(assignments, 0);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

