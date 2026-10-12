// U59/U60: not-ready supervision frame skeletons are pure + admission named.
#[test]
fn lease_expired_frame_skeleton_requires_original_closed_source() {
    let frame = native_lease_expired_frame_skeleton("asg-1", "run-1");
    assert_eq!(frame.kind, "lease-expired");
    assert_eq!(frame.semantic["recovery"], "original_closed_source_required");
    assert!(frame.rows.is_empty());
    assert_eq!(frame.step(), "assess:lease_expired");
    let f = root_tick_fixture("lease-display-rejected", "http://127.0.0.1:9/v1");
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(frame.verify(&db, &f.context, &f.actor).unwrap_err(), "root_lease_closed_binding_invalid");
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(native_try_produce_lease_expired_frame("asg-1", "run-1").err().unwrap(), "root_lease_expiry_original_fact_required");
}

#[test]
fn not_ready_supervision_frame_skeletons_cover_inventory() {
    let cases = [
        (
            "human-recovery",
            json!({"directiveId": "d1", "recovery": "not_ready"}),
            "root_trigger_human_recovery_not_ready",
        ),
    ];
    assert_eq!(cases.len(), NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS.len());
    for (kind, semantic, code) in cases {
        let frame = native_not_ready_supervision_frame_skeleton(kind, semantic).unwrap();
        assert_eq!(frame.kind, kind);
        assert_eq!(frame.semantic["recovery"], "not_ready");
        assert_eq!(frame.step(), "assess:unmapped_frame_kind");
        assert_eq!(native_supervision_frame_admission(kind).unwrap_err(), code);
        assert_eq!(NativeCoordinatorTrigger::classify(&frame).unwrap_err(), code);
    }
    match native_not_ready_supervision_frame_skeleton("mapper-output", json!({})) {
        Ok(_) => panic!("wired kind must not mint a not_ready skeleton"),
        Err(code) => assert_eq!(code, "root_trigger_skeleton_kind_invalid"),
    }
}

#[test]
fn named_not_ready_supervision_frame_skeletons_stay_not_ready() {
    let cases = [
        (
            native_human_recovery_frame_skeleton("d-9"),
            "human-recovery",
            "root_trigger_human_recovery_not_ready",
        ),
    ];
    assert_eq!(cases.len(), NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS.len());
    for (frame, kind, code) in cases {
        assert_eq!(frame.kind, kind);
        assert_eq!(frame.semantic["recovery"], "not_ready");
        assert!(frame.rows.is_empty());
        assert_eq!(frame.step(), "assess:unmapped_frame_kind");
        assert_eq!(native_supervision_frame_admission(kind).unwrap_err(), code);
        assert_eq!(NativeCoordinatorTrigger::classify(&frame).unwrap_err(), code);
    }
}

#[test]
fn not_ready_supervision_producers_remain_unready() {
    assert_eq!(NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS.len(), 1);
    for kind in NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS {
        assert!(
            !native_not_ready_supervision_producer_ready(kind),
            "{kind} must not claim a live producer yet"
        );
        assert!(native_supervision_frame_admission(kind).is_err());
    }
    assert!(!native_not_ready_supervision_producer_ready("mapper-output"));
    assert!(!native_not_ready_supervision_producer_ready("input-parser-output"));
    assert!(!native_not_ready_supervision_producer_ready("totally-unknown"));
}

#[test]
fn try_produce_not_ready_supervision_frame_stays_not_ready() {
    for kind in NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS {
        let code = format!("root_trigger_{}_not_ready", kind.replace('-', "_"));
        match native_try_produce_not_ready_supervision_frame(kind, json!({"recovery": "not_ready"})) {
            Ok(_) => panic!("{kind} must not produce while producer_ready is false"),
            Err(err) => assert_eq!(err, code),
        }
    }
    match native_try_produce_not_ready_supervision_frame("mapper-output", json!({})) {
        Ok(_) => panic!("wired kind must not enter not_ready try_produce"),
        Err(code) => assert_eq!(code, "root_trigger_skeleton_kind_invalid"),
    }
}

#[test]
fn named_try_produce_not_ready_supervision_stays_not_ready() {
    let cases = [
        (
            native_try_produce_human_recovery_frame("d-1"),
            "root_trigger_human_recovery_not_ready",
        ),
    ];
    assert_eq!(cases.len(), NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS.len());
    for (result, code) in cases {
        match result {
            Ok(_) => panic!("named try_produce must stay not_ready"),
            Err(err) => assert_eq!(err, code),
        }
    }
}

// Revision-only skeletons remain rejected after the physical producer is wired.
#[test]
fn new_evidence_legacy_revision_only_producers_cannot_mint_paid_fact() {
    for result in [native_new_evidence_frame_skeleton(3), native_try_produce_new_evidence_frame(4)] {
        match result {
            Ok(_) => panic!("revision-only caller cannot mint closed original fact"),
            Err(error) => assert_eq!(error, "root_new_evidence_original_fact_required"),
        }
    }
    assert_eq!(native_not_ready_supervision_frame_skeleton("new-evidence", json!({})).err().unwrap(),
        "root_trigger_skeleton_kind_invalid");
}

#[test]
fn protection_changed_legacy_display_and_strings_cannot_mint_paid_source() {
    for result in [native_protection_changed_frame_skeleton("waf"), native_try_produce_protection_changed_frame("captcha")] {
        assert_eq!(result.err().unwrap(), "root_protection_original_fact_required");
    }
    assert_eq!(native_not_ready_supervision_frame_skeleton("protection-changed", json!({})).err().unwrap(), "root_trigger_skeleton_kind_invalid");
}
