
// U130: A03 named try_produce wrappers must stay present (still not_ready).
#[test]
fn policy_source_pins_a03_named_try_produce() {
    let src = include_str!("agent_native/coordinator_rust_policy.rs");
    for name in [
        "native_try_produce_lease_expired_frame",
        "native_try_produce_new_evidence_frame",
        "native_try_produce_protection_changed_frame",
        "native_try_produce_human_recovery_frame",
    ] {
        assert!(src.contains(name), "missing {name}");
    }
}
