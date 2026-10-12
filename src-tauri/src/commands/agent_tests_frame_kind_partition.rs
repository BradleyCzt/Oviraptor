
// U69: wired / A03 not_ready / reserved role-output inventories partition cleanly.
#[test]
fn supervision_and_reserved_frame_kind_inventories_partition() {
    let wired: Vec<&str> = NATIVE_WIRED_SUPERVISION_FRAME_KINDS.to_vec();
    let not_ready: Vec<&str> = NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS.to_vec();
    let reserved: Vec<&str> = NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS.to_vec();

    // Original eleven plus SDK/HTTP consumption, capability, Authorization and Surface sources.
    assert_eq!(wired.len(), 18);
    assert_eq!(not_ready.len(), 1);
    assert_eq!(reserved.len(), 0);
    assert!(reserved.is_empty());
    assert!(wired.contains(&"business-logic-output"));
    assert!(wired.contains(&"business-logic-input"));
    assert!(wired.contains(&"external-surface-output"));
    assert!(wired.contains(&"authorization-output"));
    assert!(wired.contains(&"protection-changed"));
    assert!(!not_ready.contains(&"protection-changed"));
    assert_eq!(not_ready, vec!["human-recovery"]);

    let mut all = Vec::new();
    all.extend(wired.iter().copied());
    all.extend(not_ready.iter().copied());
    all.extend(reserved.iter().copied());
    all.sort_unstable();
    let mut uniq = all.clone();
    uniq.dedup();
    assert_eq!(all.len(), uniq.len(), "inventories must be pairwise disjoint: {all:?}");

    for kind in &wired {
        assert!(native_supervision_frame_admission(kind).is_ok(), "{kind}");
        assert_ne!(
            native_coordinator_frame_step_for_kind(kind),
            "assess:unmapped_frame_kind",
            "{kind}"
        );
    }
    for kind in &not_ready {
        let code = format!("root_trigger_{}_not_ready", kind.replace('-', "_"));
        assert_eq!(native_supervision_frame_admission(kind).unwrap_err(), code);
        assert_eq!(
            native_coordinator_frame_step_for_kind(kind),
            "assess:unmapped_frame_kind",
            "{kind}"
        );
    }
    for kind in &reserved {
        let code = format!("root_trigger_{}_not_ready", kind.replace('-', "_"));
        assert_eq!(native_supervision_frame_admission(kind).unwrap_err(), code);
        assert!(
            native_coordinator_frame_step_for_kind(kind).starts_with("dispatch:"),
            "{kind}"
        );
    }
    assert_eq!(
        native_supervision_frame_admission("totally-unknown").unwrap_err(),
        "root_trigger_not_enabled"
    );
    assert_eq!(
        native_coordinator_frame_step_for_kind("totally-unknown"),
        "assess:unmapped_frame_kind"
    );
}
