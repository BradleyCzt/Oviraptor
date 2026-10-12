// U61: reserved role-output frames have steps reserved and admission not_ready.
#[test]
fn reserved_role_output_frames_map_steps_and_stay_not_ready() {
    assert!(NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS.is_empty());
    for (kind,step,name) in [("input-parser-output","dispatch:input_parser","input_parser_trigger"),("business-logic-output","assess:business_logic_output","child_output_envelope"),("authorization-output","assess:authorization_output","child_output_envelope"),("external-surface-output","assess:external_surface_output","child_output_envelope")] {
        assert!(NATIVE_WIRED_SUPERVISION_FRAME_KINDS.contains(&kind));
        assert_eq!(native_coordinator_frame_step_for_kind(kind),step);
        assert_eq!(native_supervision_frame_admission(kind),Ok(()));
        let frame=NativeCoordinatorFrame {kind,semantic:json!({"display":kind,"recovery":"not_ready"}),rows:Vec::new(),review_refs:None,review_meta:None};
        assert_eq!(NativeCoordinatorTrigger::classify(&frame).unwrap().name(),name);
        // Classification is not source verification or a dispatch grant.
        assert!(frame.rows.is_empty());
    }
}

#[test]
fn reserved_role_output_frame_skeletons_cover_inventory() {
    assert!(NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS.is_empty());
    for kind in ["business-logic-output","input-parser-output","authorization-output","external-surface-output","mapper-output","lease-expired","totally-unknown"] {
        assert_eq!(native_reserved_role_output_frame_skeleton(kind,json!({})).err().as_deref(),Some("root_trigger_skeleton_kind_invalid"));
    }
}

#[test]
fn named_reserved_role_output_frame_skeletons_stay_not_ready() {
    use crate::agent_runtime::contract::AgentRole;
    for role in [AgentRole::InputParser,AgentRole::BusinessLogic,AgentRole::Authorization,AgentRole::ExternalSurface] {
        assert_eq!(native_reserved_role_output_frame_skeleton_for_role(role,"display").err().as_deref(),Some("root_trigger_skeleton_kind_invalid"));
    }
}

#[test]
fn reserved_role_output_frames_align_schedule_not_ready_gates() {
    use crate::agent_runtime::contract::AgentRole;
    for (role,kind,step,code) in [(AgentRole::InputParser,"input-parser-output","dispatch:input_parser","input_parser_dispatch_not_ready"),(AgentRole::BusinessLogic,"business-logic-output","assess:business_logic_output","business_logic_dispatch_not_ready"),(AgentRole::Authorization,"authorization-output","assess:authorization_output","authorization_dispatch_not_ready"),(AgentRole::ExternalSurface,"external-surface-output","assess:external_surface_output","external_surface_dispatch_not_ready")] {
        assert_eq!(native_root_schedule_decision_role_gate(role).unwrap_err(),code);
        assert_eq!(native_supervision_frame_admission(kind),Ok(()));
        assert_eq!(native_coordinator_frame_step_for_kind(kind),step);
        assert_eq!(native_reserved_role_output_frame_skeleton_for_role(role,"display").err().as_deref(),Some("root_trigger_skeleton_kind_invalid"));
    }
}

#[test]
fn reserved_role_output_kind_for_role_covers_a04_gates() {
    use crate::agent_runtime::contract::AgentRole;

    let mapped = [
        (AgentRole::InputParser, "input-parser-output"),
        (AgentRole::BusinessLogic, "business-logic-output"),
        (AgentRole::Authorization, "authorization-output"),
        (AgentRole::ExternalSurface, "external-surface-output"),
    ];
    // U176: InputParser's kind moved from reserved to wired; the role mapping stays.
    assert_eq!(
        mapped.len(),
        NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS.len() + 4
    );
    for (role, kind) in mapped {
        assert_eq!(native_reserved_role_output_kind_for_role(role), Some(kind));
        // schedule stays not_ready for all four (no mint yet).
        assert!(native_root_schedule_decision_role_gate(role).is_err());
        if matches!(
            role,
            AgentRole::InputParser | AgentRole::BusinessLogic | AgentRole::Authorization | AgentRole::ExternalSurface
        ) {
            assert!(NATIVE_WIRED_SUPERVISION_FRAME_KINDS.contains(&kind));
            assert!(!NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS.contains(&kind));
            assert_eq!(native_supervision_frame_admission(kind), Ok(()));
        } else {
            assert!(NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS.contains(&kind));
            assert!(native_supervision_frame_admission(kind).is_err());
        }
        let step = match role {
            AgentRole::Authorization => "assess:authorization_output",
            AgentRole::ExternalSurface => "assess:external_surface_output",
            AgentRole::InputParser => "dispatch:input_parser",
            AgentRole::BusinessLogic => "assess:business_logic_output",
            _ => unreachable!(),
        };
        assert_eq!(native_coordinator_frame_step_for_kind(kind), step);
    }
    for role in AgentRole::ALL {
        let expect = matches!(
            *role,
            AgentRole::InputParser
                | AgentRole::BusinessLogic
                | AgentRole::Authorization
                | AgentRole::ExternalSurface
        );
        assert_eq!(
            native_reserved_role_output_kind_for_role(*role).is_some(),
            expect,
            "{role:?}"
        );
    }
}

#[test]
fn reserved_role_output_kind_role_roundtrip() {
    use crate::agent_runtime::contract::AgentRole;

    for kind in NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS {
        let role = native_agent_role_for_reserved_role_output_kind(kind)
            .expect("reserved kind maps to role");
        assert_eq!(native_reserved_role_output_kind_for_role(role), Some(*kind));
    }
    for role in [
        AgentRole::InputParser,
        AgentRole::BusinessLogic,
        AgentRole::Authorization,
        AgentRole::ExternalSurface,
    ] {
        let kind = native_reserved_role_output_kind_for_role(role).unwrap();
        assert_eq!(
            native_agent_role_for_reserved_role_output_kind(kind),
            Some(role)
        );
    }
    assert_eq!(
        native_agent_role_for_reserved_role_output_kind("mapper-output"),
        None
    );
    assert_eq!(
        native_agent_role_for_reserved_role_output_kind("lease-expired"),
        None
    );
}

#[test]
fn reserved_role_output_skeleton_for_role_stays_not_ready() {
    use crate::agent_runtime::contract::AgentRole;
    for role in [AgentRole::InputParser,AgentRole::BusinessLogic,AgentRole::Authorization,AgentRole::ExternalSurface] {
        assert_eq!(native_reserved_role_output_frame_skeleton_for_role(role,"d").err().as_deref(),Some("root_trigger_skeleton_kind_invalid"));
    }
    assert_eq!(native_reserved_role_output_frame_skeleton_for_role(AgentRole::WebExecutor,"x").err().as_deref(),Some("root_trigger_skeleton_kind_invalid"));
}

#[test]
fn reserved_role_output_producers_remain_unready() {
    // U176: 4 -> 3 (input-parser-output wired); it still claims no reserved producer.
    assert_eq!(NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS.len(), 0);
    for kind in ["input-parser-output","business-logic-output","authorization-output","external-surface-output"] {
        assert!(!native_reserved_role_output_producer_ready(kind));
        assert_eq!(native_try_produce_reserved_role_output_frame(kind,json!({"recovery":"not_ready"})).err().as_deref(),Some("root_trigger_skeleton_kind_invalid"));
    }
    assert!(!native_reserved_role_output_producer_ready(
        "input-parser-output"
    ));
    for kind in NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS {
        assert!(
            !native_reserved_role_output_producer_ready(kind),
            "{kind} must not claim a live producer yet"
        );
        assert!(native_supervision_frame_admission(kind).is_err());
    }
    assert!(!native_reserved_role_output_producer_ready("mapper-output"));
    assert!(!native_reserved_role_output_producer_ready("lease-expired"));
    assert!(!native_reserved_role_output_producer_ready(
        "totally-unknown"
    ));
}

#[test]
fn try_produce_reserved_role_output_frame_stays_not_ready() {
    for kind in NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS {
        let code = format!("root_trigger_{}_not_ready", kind.replace('-', "_"));
        match native_try_produce_reserved_role_output_frame(kind, json!({"recovery": "not_ready"}))
        {
            Ok(_) => panic!("{kind} must not produce while producer_ready is false"),
            Err(err) => assert_eq!(err, code),
        }
    }
    match native_try_produce_reserved_role_output_frame("lease-expired", json!({})) {
        Ok(_) => panic!("A03 kind must not enter reserved try_produce"),
        Err(code) => assert_eq!(code, "root_trigger_skeleton_kind_invalid"),
    }
    // U176: the wired InputParser kind has no display-skeleton producer either.
    match native_try_produce_reserved_role_output_frame(
        "input-parser-output",
        json!({"recovery": "not_ready"}),
    ) {
        Ok(_) => panic!("wired InputParser kind must only come from its frozen producer"),
        Err(code) => assert_eq!(code, "root_trigger_skeleton_kind_invalid"),
    }
}

#[test]
fn try_produce_reserved_role_output_for_role_stays_not_ready() {
    use crate::agent_runtime::contract::AgentRole;

    for role in [
        AgentRole::InputParser,
        AgentRole::BusinessLogic,
        AgentRole::Authorization,
        AgentRole::ExternalSurface,
    ] {
        let kind = native_reserved_role_output_kind_for_role(role).unwrap();
        // U176: InputParser's kind is wired; its display skeleton is refused instead.
        let code = if matches!(
            role,
            AgentRole::InputParser | AgentRole::BusinessLogic | AgentRole::Authorization | AgentRole::ExternalSurface
        ) {
            "root_trigger_skeleton_kind_invalid".to_string()
        } else {
            format!("root_trigger_{}_not_ready", kind.replace('-', "_"))
        };
        match native_try_produce_reserved_role_output_for_role(role, "prep") {
            Ok(_) => panic!("{role:?} must stay not_ready"),
            Err(err) => assert_eq!(err, code),
        }
    }
    match native_try_produce_reserved_role_output_for_role(AgentRole::WebExecutor, "x") {
        Ok(_) => panic!("WebExecutor must not try_produce reserved output"),
        Err(code) => assert_eq!(code, "root_trigger_skeleton_kind_invalid"),
    }
}

#[test]
fn named_try_produce_reserved_role_output_stays_not_ready() {
    let cases = [
        // U176: wired kind, display-skeleton producer refused.
        (
            native_try_produce_input_parser_output_frame("ip"),
            "root_trigger_skeleton_kind_invalid",
        ),
        (
            native_try_produce_business_logic_output_frame("bl"),
            "root_trigger_skeleton_kind_invalid",
        ),
        (
            native_try_produce_authorization_output_frame("az"),
            "root_trigger_skeleton_kind_invalid",
        ),
        (
            native_try_produce_external_surface_output_frame("es"),
            "root_trigger_skeleton_kind_invalid",
        ),
    ];
    assert_eq!(
        cases.len(),
        NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS.len() + 4
    );
    for (result, code) in cases {
        match result {
            Ok(_) => panic!("named reserved try_produce must stay not_ready"),
            Err(err) => assert_eq!(err, code),
        }
    }
}
