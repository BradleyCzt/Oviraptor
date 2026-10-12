// Test subprocess only. Called after persist_received_invoice COMMIT and before
// the terminal/decision transaction. No release hook, database write or invoice
// fabrication; the normal SDK and original journal writer already ran.
#[cfg(test)]
fn native_financial_continuation_received_test_checkpoint(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    let Ok(root) = std::env::var("OVIRAPTOR_TEST_FINANCIAL_EXIT_AFTER_RECEIVED_INVOICE") else {
        return;
    };
    let Some(path) = std::env::var_os("OVIRAPTOR_TEST_FINANCIAL_RECEIVED_DRIVER_DB") else {
        return;
    };
    // Match both the exact new Root and disposable driver database. An ambient
    // test variable cannot crash another fixture or the old paid Root.
    if root == actor.root_run_id && Path::new(&path) == context.db_path {
        std::process::exit(87);
    }
}
