// U33: known-but-unwired REM-A03 supervision triggers get stable not-ready codes.
#[test]
fn coordinator_named_supervision_triggers_return_stable_not_ready_codes() {
    let cases = [("human-recovery", "root_trigger_human_recovery_not_ready")];
    for (kind, code) in cases {
        let frame = NativeCoordinatorFrame {
            kind,
            semantic: json!({"display": kind}),
            rows: Vec::new(),
            review_refs: None,
            review_meta: None,
        };
        assert_eq!(
            NativeCoordinatorTrigger::classify(&frame).unwrap_err(),
            code,
            "{kind}"
        );
    }
    // Unknown kinds still use the generic code.
    let unknown = NativeCoordinatorFrame {
        kind: "unproved-human-directive",
        semantic: json!({"display": "continue"}),
        rows: Vec::new(),
        review_refs: None,
        review_meta: None,
    };
    assert_eq!(
        NativeCoordinatorTrigger::classify(&unknown).unwrap_err(),
        "root_trigger_not_enabled"
    );
}

#[test]
fn coordinator_wired_supervision_triggers_still_classify() {
    let wired = [
        ("new-evidence", "new_evidence"),
        ("lease-expired", "lease_expired"),
        ("protection-changed", "protection_changed"),
        ("input-parser-output", "input_parser_trigger"),
        ("human-directive", "human_directive"),
        ("budget-allocation", "capability_budget_state_changed"),
        ("budget-consumption", "capability_budget_state_changed"),
        ("http-budget-consumption", "capability_budget_state_changed"),
        ("capability-changed", "capability_budget_state_changed"),
        ("authorization-output", "child_output_envelope"),
        ("external-surface-output", "child_output_envelope"),
        ("business-logic-output", "child_output_envelope"),
        ("business-logic-input", "business_logic_input"),
        ("mapper-output", "child_output_envelope"),
        ("client-side-output", "child_output_envelope"),
        ("identity-session-output", "child_output_envelope"),
        ("reviewer-decision", "reviewer_decision"),
        ("investigator-proposal", "child_output_envelope"),
    ];
    for (kind, name) in wired {
        let frame = NativeCoordinatorFrame {
            kind,
            semantic: json!({"display": kind}),
            rows: Vec::new(),
            review_refs: None,
            review_meta: None,
        };
        let trigger = NativeCoordinatorTrigger::classify(&frame).unwrap();
        assert_eq!(trigger.name(), name, "{kind}");
        if kind == "business-logic-input" {
            assert!(matches!(trigger, NativeCoordinatorTrigger::BusinessLogicInput));
            assert_eq!(frame.step(), "dispatch:business_logic");
        }
    }
}

#[test]
fn supervision_trigger_inventory_covers_wired_and_not_ready_kinds() {
    assert_eq!(NATIVE_WIRED_SUPERVISION_FRAME_KINDS, &[
        "new-evidence", "lease-expired", "protection-changed", "human-directive",
        "budget-allocation", "budget-consumption", "http-budget-consumption",
        "capability-changed", "authorization-output", "external-surface-output",
        "business-logic-output", "business-logic-input", "mapper-output",
        "client-side-output", "identity-session-output", "reviewer-decision",
        "investigator-proposal", "input-parser-output",
    ]);
    assert_eq!(NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS, &["human-recovery"]);
    // U52: inventories must stay disjoint so admission cannot be ambiguous.
    for kind in NATIVE_WIRED_SUPERVISION_FRAME_KINDS {
        assert!(
            !NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS.contains(kind),
            "{kind} listed as both wired and not_ready"
        );
    }
    for kind in NATIVE_WIRED_SUPERVISION_FRAME_KINDS {
        let frame = NativeCoordinatorFrame {
            kind,
            semantic: json!({"display": kind}),
            rows: Vec::new(),
            review_refs: None,
            review_meta: None,
        };
        assert!(
            NativeCoordinatorTrigger::classify(&frame).is_ok(),
            "{kind} must stay wired"
        );
    }
    for kind in NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS {
        let frame = NativeCoordinatorFrame {
            kind,
            semantic: json!({"display": kind}),
            rows: Vec::new(),
            review_refs: None,
            review_meta: None,
        };
        let err = NativeCoordinatorTrigger::classify(&frame).unwrap_err();
        assert!(
            err.starts_with("root_trigger_") && err.ends_with("_not_ready"),
            "{kind} -> {err}"
        );
        assert_ne!(err, "root_trigger_not_enabled", "{kind}");
    }
}

#[test]
fn supervision_frame_admission_matches_wired_and_not_ready_inventories() {
    for kind in [
        "lease-expired",
        "protection-changed",
        "input-parser-output",
        "new-evidence",
        "human-directive",
        "budget-allocation",
        "budget-consumption",
        "http-budget-consumption",
        "capability-changed",
        "authorization-output",
        "external-surface-output",
        "business-logic-output",
        "business-logic-input",
        "mapper-output",
        "client-side-output",
        "identity-session-output",
        "reviewer-decision",
        "investigator-proposal",
    ] {
        assert!(
            native_supervision_frame_admission(kind).is_ok(),
            "{kind} must be wired"
        );
    }
    let (kind, code) = ("human-recovery", "root_trigger_human_recovery_not_ready");
    assert_eq!(
        native_supervision_frame_admission(kind).unwrap_err(),
        code,
        "{kind}"
    );
    assert_eq!(
        native_supervision_frame_admission("totally-unknown").unwrap_err(),
        "root_trigger_not_enabled"
    );
}

#[test]
fn reserved_role_output_kinds_are_disjoint_from_wired_and_supervision_not_ready() {
    // Every role output now has a dedicated source consumer; display producers remain refused.
    assert!(NATIVE_WIRED_SUPERVISION_FRAME_KINDS.contains(&"business-logic-output"));
    assert_eq!(NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS.len(), 0);
    assert!(NATIVE_WIRED_SUPERVISION_FRAME_KINDS.contains(&"input-parser-output"));
    assert!(NATIVE_WIRED_SUPERVISION_FRAME_KINDS.contains(&"authorization-output"));
    assert!(NATIVE_WIRED_SUPERVISION_FRAME_KINDS.contains(&"external-surface-output"));
    assert_eq!(
        NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS,
        &[] as &[&str]
    );
    for kind in NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS {
        assert!(
            !NATIVE_WIRED_SUPERVISION_FRAME_KINDS.contains(kind),
            "{kind} must not be wired yet"
        );
        assert!(
            !NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS.contains(kind),
            "{kind} is role-output reserved, not A03 supervision not_ready"
        );
        assert!(
            native_supervision_frame_admission(kind).is_err(),
            "{kind} must stay not_ready at admission"
        );
    }
}
