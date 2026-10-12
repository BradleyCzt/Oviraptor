#[cfg(test)]
mod protection_witness_scope_tests {
    use super::*;

    #[test]
    fn protection_witness_rejects_scope_changes_and_mutated_authorization() {
        // Pure envelope checks only; the actual fresh Root/send fixture belongs
        // to the production transport test, not this synthetic binding test.
        let binding = json!({"scanId":"scan","attemptNumber":1,"targetUrl":"https://example.test",
            "dbPath":"/private/test.db","targetDir":"/private/target","runId":"child",
            "invocationId":"invoke","requestIndex":1,"budgetAttempt":1,"ordinal":1,"requestHash":"hash"});
        let canonical =
            json!({"claim":binding,"originalFacts":["original-physical-hash"]}).to_string();
        let mut witness = ProtectionPreSendWitness {
            claim_binding: binding.to_string(),
            authorization_sha256: format!("{:x}", Sha256::digest(canonical.as_bytes())),
            canonical_authorization: canonical,
            actor: crate::agent_runtime::multi_agent::lease::CoordinatorLease {
                scan_id: "scan".into(),
                attempt_number: 1,
                target_key: "https://example.test".into(),
                root_run_id: "root".into(),
                lease_epoch: 1,
                fencing_token: "fence".into(),
                lease_expires_at: "old".into(),
            },
            historical: vec![],
        };
        witness.require_binding(&binding.to_string()).unwrap();
        for field in binding.as_object().unwrap().keys() {
            let mut changed = binding.clone();
            changed[field] = json!("different");
            assert_eq!(
                witness.require_binding(&changed.to_string()).unwrap_err(),
                "protection_pre_send_binding_changed"
            );
        }
        witness.canonical_authorization.push(' ');
        assert_eq!(
            witness.require_binding(&binding.to_string()).unwrap_err(),
            "protection_pre_send_binding_changed"
        );
    }
}
