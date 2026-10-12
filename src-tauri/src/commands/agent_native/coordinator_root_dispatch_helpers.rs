// Shared Root paid-suggestion selection for not-ready / not-implemented gates.
fn native_root_paid_suggestion_selected(
    decision: &NativeCoordinatorTickReceipt,
    step: &str,
) -> bool {
    decision.summary.as_json()["suggestions"]
        .as_array()
        .is_some_and(|items| items.iter().any(|v| v.as_str() == Some(step)))
}
