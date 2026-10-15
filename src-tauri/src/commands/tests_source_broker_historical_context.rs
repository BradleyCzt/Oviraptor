fn source_broker_context(root: &Path, connection: &rusqlite::Connection, record: &WorkbenchStartRecord) -> AgentRunContext {
    source_broker_context_for_attempt(root,connection,record,1)
}

fn source_broker_context_for_attempt(root: &Path, connection: &rusqlite::Connection, record: &WorkbenchStartRecord, attempt:i64) -> AgentRunContext {
    use crate::agent_runtime::{contract::{AgentBackendKind,AgentRole},store::{self,AgentRunRow}};
    let mut context=crate::scanning::agent_tests::test_context(&root.join("oviraptor.sqlite3"),"https://source.example.invalid",Vec::new());
    context.scan_id=record.scan_id.clone();
    context.attempt_number=attempt;
    context.execution_plan.attempt_number=attempt;
    let run_id=format!("source-root-{}-{attempt}",record.scan_id);
    let mut row=AgentRunRow::new(&run_id,&record.scan_id,attempt,&context.target_url,AgentBackendKind::Native,
        AgentRole::Coordinator,context.execution_plan.hash(),"source-fixture");
    row.root_run_id=run_id.clone();
    store::create_run(connection,&row).unwrap();
    store::record_attempt_plan(connection,&record.scan_id,attempt,&context.target_url,AgentBackendKind::Native,
        &context.execution_plan.hash(),&context.execution_plan.as_json()).unwrap();
    context.run=Some(AgentRunLedger{db_path:context.db_path.clone(),run_id});
    context
}
