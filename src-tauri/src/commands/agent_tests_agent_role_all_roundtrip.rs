// U47: AgentRole::ALL round-trips through as_str/try_parse and stays exhaustive.
#[test]
fn agent_role_all_roundtrips_and_stays_exhaustive() {
    use crate::agent_runtime::contract::AgentRole;
    use std::collections::BTreeSet;

    assert_eq!(AgentRole::ALL.len(), 15);
    let mut seen = BTreeSet::new();
    for role in AgentRole::ALL {
        let s = role.as_str();
        assert!(!s.is_empty(), "{role:?}");
        assert!(seen.insert(s), "duplicate as_str {s}");
        assert_eq!(AgentRole::try_parse(s), Some(*role), "try_parse({s})");
        assert_eq!(AgentRole::parse(s), *role);
    }
    // Unknown words must not silently become a real specialist.
    assert_eq!(AgentRole::try_parse("not_a_real_role"), None);
    assert_eq!(AgentRole::parse("not_a_real_role"), AgentRole::Coordinator);
}
