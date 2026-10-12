// U62: reserved role-output steps stay aligned with Root dispatch gate step constants.
#[test]
fn reserved_role_output_steps_align_with_dispatch_gate_constants() {
    assert_eq!(
        native_coordinator_frame_step_for_kind("input-parser-output"),
        NATIVE_INPUT_PARSER_DISPATCH_STEP
    );
    assert_eq!(
        native_coordinator_frame_step_for_kind("business-logic-output"),
        "assess:business_logic_output"
    );
    assert_eq!(
        native_coordinator_frame_step_for_kind("authorization-output"),
        "assess:authorization_output"
    );
    assert_eq!(
        native_coordinator_frame_step_for_kind("external-surface-output"),
        "assess:external_surface_output"
    );
}
