// U57: wired supervision frame kinds map to explicit steps; unknown ≠ deep_investigator.
#[test]
fn coordinator_frame_step_inventory_is_explicit_for_wired_kinds() {
    let wired = [
        ("new-evidence", "assess:new_evidence"),
        ("lease-expired", "assess:lease_expired"),
        ("protection-changed", "assess:protection_changed"),
        ("human-directive", "assess:human_directive"),
        ("budget-allocation", "assess:budget_allocation"),
        ("budget-consumption", "assess:budget_consumption"),
        ("http-budget-consumption", "assess:http_budget_consumption"),
        ("capability-changed", "assess:capability_changed"),
        ("authorization-output", "assess:authorization_output"),
        ("external-surface-output", "assess:external_surface_output"),
        ("business-logic-output", "assess:business_logic_output"),
        ("business-logic-input", "dispatch:business_logic"),
        ("mapper-output", "dispatch:web_executor"),
        ("investigator-proposal", "assess:gap_proposal"),
        ("client-side-output", "assess:client_side_configuration"),
        (
            "identity-session-output",
            "assess:identity_session_metadata",
        ),
        ("reviewer-decision", "dispatch:deep_investigator"),
        // U176: the frozen InputParser trigger frame is wired with its reserved step.
        ("input-parser-output", "dispatch:input_parser"),
    ];
    assert_eq!(wired.len(), NATIVE_WIRED_SUPERVISION_FRAME_KINDS.len());
    for (kind, step) in wired {
        assert!(
            NATIVE_WIRED_SUPERVISION_FRAME_KINDS.contains(&kind),
            "{kind} missing from wired inventory"
        );
        let frame = NativeCoordinatorFrame {
            kind,
            semantic: json!({"display": kind}),
            rows: Vec::new(),
            review_refs: None,
            review_meta: None,
        };
        assert_eq!(frame.step(), step, "{kind}");
    }
    let unknown = NativeCoordinatorFrame {
        kind: "totally-unknown",
        semantic: json!({"display": "totally-unknown"}),
        rows: Vec::new(),
        review_refs: None,
        review_meta: None,
    };
    assert_eq!(unknown.step(), "assess:unmapped_frame_kind");
    assert_ne!(unknown.step(), "dispatch:deep_investigator");
}

#[test]
fn frame_step_helper_covers_every_wired_inventory_kind() {
    for kind in NATIVE_WIRED_SUPERVISION_FRAME_KINDS {
        let step = native_coordinator_frame_step_for_kind(kind);
        assert_ne!(
            step, "assess:unmapped_frame_kind",
            "{kind} wired but unmapped in step table"
        );
    }
    for kind in NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS {
        assert_eq!(
            native_coordinator_frame_step_for_kind(kind),
            "assess:unmapped_frame_kind",
            "{kind} not_ready must not advertise a live dispatch step"
        );
    }
}
