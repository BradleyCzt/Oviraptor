// Shared Reviewer floor must stay 15_000 so proposal/ordered/surface gates cannot drift.
#[test]
fn reviewer_floor_tokens_constant_is_15000_and_surface_min_includes_grant() {
    use crate::agent_runtime::multi_agent::client_side::{
        REVIEWER_FLOOR_REQUESTS, REVIEWER_FLOOR_TOKENS,
    };
    assert_eq!(REVIEWER_FLOOR_TOKENS, 15_000);
    assert_eq!(REVIEWER_FLOOR_REQUESTS, 1);
    let surface_grant = 2_000;
    assert_eq!(surface_grant + REVIEWER_FLOOR_TOKENS, 17_000);
}
