
// U134: Upload/Concurrency Root stubs keep distinct not_implemented codes.
#[test]
fn stub_role_gate_source_pins_distinct_not_implemented() {
    let src = include_str!("agent_native/coordinator_stub_role_gate.rs");
    assert!(src.contains("upload_role_not_implemented"));
    assert!(src.contains("concurrency_role_not_implemented"));
    assert!(src.contains("native_coordinator_prepare_upload"));
    assert!(src.contains("native_coordinator_prepare_concurrency"));
    assert!(src.contains("dispatch:upload"));
    assert!(src.contains("dispatch:concurrency"));
}
