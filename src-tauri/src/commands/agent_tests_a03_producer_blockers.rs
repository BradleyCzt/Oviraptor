
// U143: A03 supervision producers stay not_ready until these blockers clear.
// The inventory must stay non-empty and every not_ready kind must still report
// producer_ready == false (no silent flip to a live producer).
#[test]
fn native_a03_producer_blockers_inventory_nonempty() {
    assert!(
        !NATIVE_A03_PRODUCER_BLOCKERS.is_empty(),
        "A03 producer blockers must not silently empty before producers ship"
    );
    for item in [
        "lease_scan_source",
        "evidence_revision_feed",
        "protection_watch_source",
        "human_recovery_directive_feed",
        "zero_write_proof_on_failure",
    ] {
        assert!(
            NATIVE_A03_PRODUCER_BLOCKERS.contains(&item),
            "missing A03 producer blocker {item}"
        );
    }
    // Honesty: while blockers remain, no not_ready kind may claim a live producer.
    for kind in NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS {
        assert!(
            !native_not_ready_supervision_producer_ready(kind),
            "A03 kind {kind} must stay producer_ready=false while blockers remain"
        );
    }
}
