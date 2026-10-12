
// U140: prepare_implemented must not gain a true arm until deliberate (a) work.
// U181 (pin updated, reason: U181 按计划只翻 InputParser): the deliberate (a) work for
// InputParser is complete (empty blocker inventory + U180 end-to-end), so the body now
// has exactly ONE true arm, `InputParser => true`; every other arm still returns false
// and no other `=> true` / `return true` may appear.
#[test]
fn policy_source_pins_prepare_implemented_has_no_true_arm() {
    let src = include_str!("agent_native/coordinator_rust_policy.rs");
    let start = src
        .find("fn native_root_role_prepare_implemented")
        .expect("prepare_implemented");
    let rest = &src[start..];
    let end = rest.find("\n}\n").expect("fn end") + 2;
    let body = &rest[..end];
    assert_eq!(body.matches("=> true").count(), 1, "exactly one true arm: {body}");
    assert!(body.contains("InputParser => true,"), "the only true arm is InputParser: {body}");
    assert!(!body.contains("return true"), "{body}");
    assert!(
        body.contains("BusinessLogic | Authorization | ExternalSurface => false,"),
        "BL/AZ/ES stay false: {body}"
    );
    assert!(body.contains("_ => false,"), "every other role stays false: {body}");
    assert!(!body.contains("InputParser |") && !body.contains("| InputParser"), "InputParser not grouped: {body}");
}
