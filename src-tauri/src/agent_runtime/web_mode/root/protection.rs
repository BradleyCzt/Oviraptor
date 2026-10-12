//! Birth-only contract; no protection receipt, target authority, or execution grant.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProtectionSupervision {
    schema_version: u8,
    source: String,
    closure: String,
    deduplication: String,
    permitted_step: String,
    execution_grant: bool,
}
impl ProtectionSupervision {
    pub(super) fn creation_contract() -> Self {
        Self {
            schema_version: 1,
            source: "original_signed_protection_response_receipt".into(),
            closure: "original_cost_settled_saved_and_acknowledged".into(),
            deduplication: "original_response_physical_fact".into(),
            permitted_step: "assess:protection_changed".into(),
            execution_grant: false,
        }
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        if *self != Self::creation_contract() {
            return Err("root_protection_supervision_contract_invalid".into());
        }
        Ok(())
    }
    pub(super) fn as_json(&self) -> Value {
        json!(self)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ModeFact, NewRootModeDeclaration, VerifiedMode, WebMode};
    use super::*;

    fn declaration(mode: WebMode) -> NewRootModeDeclaration {
        let fact = ModeFact {
            schema_version: 1,
            receipt_id: uuid::Uuid::new_v4().to_string(),
            draft_id: uuid::Uuid::new_v4().to_string(),
            scan_id: "protection-contract-test".into(),
            attempt_number: 1,
            execution_mode: "initial".into(),
            parent_receipt_id: None,
            creation_action_hash: None,
            mode,
            work_dir: "/private/protection-contract-test".into(),
            task_hash: "1".repeat(64),
            policy_hash: "2".repeat(64),
            runtime_hash: "3".repeat(64),
            startup_tag: "4".repeat(64),
            targets: vec!["https://example.test".into()],
        };
        let key = [7_u8; 32];
        let signature = aws_lc_rs::hmac::sign(
            &aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256, &key),
            &fact.message().unwrap(),
        );
        let proof = VerifiedMode::verify(fact, &key, signature.as_ref()).unwrap();
        NewRootModeDeclaration::from_verified(&proof, "https://example.test", &"5".repeat(64))
            .unwrap()
    }

    fn validate(declaration: &NewRootModeDeclaration) -> Result<(), String> {
        declaration.validate_scope(
            &declaration.fact.scan_id,
            declaration.fact.attempt_number,
            &declaration.target_url,
            &declaration.plan_hash,
        )
    }

    #[test]
    fn protection_protocol_rejects_mutations_unknown_and_missing_fields() {
        let original = ProtectionSupervision::creation_contract();
        original.validate().unwrap();
        for (field, value) in [
            ("schemaVersion", json!(2)),
            ("source", json!("transport_observed_unsealed")),
            ("closure", json!("received_only")),
            ("deduplication", json!("response_body_hash")),
            ("permittedStep", json!("dispatch:web_executor")),
            ("executionGrant", json!(true)),
        ] {
            let mut raw = original.as_json();
            raw[field] = value;
            let changed: ProtectionSupervision = serde_json::from_value(raw).unwrap();
            assert_eq!(
                changed.validate().unwrap_err(),
                "root_protection_supervision_contract_invalid"
            );
            let mut missing = original.as_json();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<ProtectionSupervision>(missing).is_err());
        }
        let mut extra = original.as_json();
        extra["targetRequestsGranted"] = json!(1);
        assert!(serde_json::from_value::<ProtectionSupervision>(extra).is_err());
    }

    #[test]
    fn protection_birth_preserves_old_serialization_and_changes_binding() {
        let old = declaration(WebMode::Multi)
            .with_new_evidence_supervision()
            .unwrap();
        let old_text = serde_json::to_string(&old).unwrap();
        assert!(!old_text.contains("protectionSupervision"));
        let decoded: NewRootModeDeclaration = serde_json::from_str(&old_text).unwrap();
        assert_eq!(serde_json::to_string(&decoded).unwrap(), old_text);
        assert!(old.same_binding(&decoded));
        validate(&decoded).unwrap();
        assert_eq!(
            decoded.with_protection_supervision().unwrap_err(),
            "web_mode_private_creation_missing"
        );

        let new = old.clone().with_protection_supervision().unwrap();
        validate(&new).unwrap();
        assert!(!old.same_binding(&new));
        assert_eq!(
            new.protection_supervision().unwrap(),
            ProtectionSupervision::creation_contract().as_json()
        );
        let new_text = serde_json::to_string(&new).unwrap();
        let decoded: NewRootModeDeclaration = serde_json::from_str(&new_text).unwrap();
        assert!(new.same_binding(&decoded));
        assert_eq!(serde_json::to_string(&decoded).unwrap(), new_text);
        validate(&decoded).unwrap();
        assert_eq!(
            decoded.with_protection_supervision().unwrap_err(),
            "web_mode_private_creation_missing"
        );
    }

    #[test]
    fn protection_birth_requires_multi_new_evidence_and_private_authorization() {
        let plain = declaration(WebMode::Multi);
        assert_eq!(
            plain.clone().with_protection_supervision().unwrap_err(),
            "root_protection_supervision_mode_invalid"
        );
        let mut forged_plain = plain;
        forged_plain.protection_supervision = Some(ProtectionSupervision::creation_contract());
        assert_eq!(
            validate(&forged_plain).unwrap_err(),
            "root_protection_supervision_mode_invalid"
        );

        let single = declaration(WebMode::Single);
        let original = serde_json::to_string(&single).unwrap();
        let unchanged = single.with_protection_supervision().unwrap();
        assert_eq!(serde_json::to_string(&unchanged).unwrap(), original);
        assert!(unchanged.protection_supervision().is_none());
        let mut forged_single = unchanged;
        forged_single.protection_supervision = Some(ProtectionSupervision::creation_contract());
        assert_eq!(
            validate(&forged_single).unwrap_err(),
            "root_protection_supervision_mode_invalid"
        );
    }
}
