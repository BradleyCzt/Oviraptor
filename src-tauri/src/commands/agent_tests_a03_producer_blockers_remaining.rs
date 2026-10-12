
// U162: per-kind A03 producer-blocker view mirrors the A04 per-role accessor and
// stays honest — the four not_ready supervision kinds share the full blocker list,
// every wired/reserved/unknown kind reports None, and no kind claims a live producer.
#[test]
fn native_a03_producer_blockers_remaining_matches_not_ready_kinds() {
    for kind in NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS {
        let remaining = native_a03_producer_blockers_remaining(kind)
            .unwrap_or_else(|| panic!("{kind} must expose A03 producer blockers"));
        assert_eq!(
            remaining, NATIVE_A03_PRODUCER_BLOCKERS,
            "{kind} must share the full A03 blocker inventory"
        );
        assert!(!remaining.is_empty());
        // Honesty: blockers remain ⇒ producer stays not_ready.
        assert!(!native_not_ready_supervision_producer_ready(kind));
    }
    // Wired supervision kinds have no pending A03 producer work.
    for kind in NATIVE_WIRED_SUPERVISION_FRAME_KINDS {
        assert!(
            native_a03_producer_blockers_remaining(kind).is_none(),
            "wired kind {kind} must not expose A03 producer blockers"
        );
    }
    // Reserved role-output kinds and unknown kinds also return None.
    for kind in NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS {
        assert!(native_a03_producer_blockers_remaining(kind).is_none());
    }
    assert!(native_a03_producer_blockers_remaining("totally-unknown-kind").is_none());
}
