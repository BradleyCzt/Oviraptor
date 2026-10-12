
// U92: producer_ready=false must keep try_produce Err for A03 + A04 reserved.
#[test]
fn producer_ready_false_keeps_try_produce_err_for_a03_and_a04() {
    for kind in NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS {
        assert!(!native_not_ready_supervision_producer_ready(kind));
        assert!(
            native_try_produce_not_ready_supervision_frame(kind, json!({"recovery": "not_ready"}))
                .is_err(),
            "{kind}"
        );
    }
    for kind in NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS {
        assert!(!native_reserved_role_output_producer_ready(kind));
        assert!(
            native_try_produce_reserved_role_output_frame(kind, json!({"recovery": "not_ready"}))
                .is_err(),
            "{kind}"
        );
    }
}
