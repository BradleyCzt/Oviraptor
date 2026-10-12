//! Fresh ordinary Root contract; no execution or renewal authority.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct LeaseExpirySupervision {
    schema_version: u8,
    source: String,
    closure: String,
    deduplication: String,
    permitted_step: String,
    execution_grant: bool,
}
impl LeaseExpirySupervision {
    pub(super) fn creation_contract() -> Self {
        Self {
            schema_version: 1,
            source: "original_signed_worker_expiry_receipt".into(),
            closure: "original_cost_settled_saved_and_acknowledged".into(),
            deduplication: "original_receipt_physical_fact".into(),
            permitted_step: "assess:lease_expired".into(),
            execution_grant: false,
        }
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        if *self != Self::creation_contract() {
            return Err("root_lease_expiry_supervision_contract_invalid".into());
        }
        Ok(())
    }
    pub(super) fn as_json(&self) -> Value { json!(self) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lease_expiry_protocol_requires_signed_closed_physical_fact_without_grant() {
        let original = LeaseExpirySupervision::creation_contract();
        original.validate().unwrap();
        for (field, value) in [
            ("schemaVersion", json!(2)),
            ("source", json!("unsigned_worker_state")),
            ("closure", json!("received_only")),
            ("deduplication", json!("revision")),
            ("permittedStep", json!("renew:worker")),
            ("executionGrant", json!(true)),
        ] {
            let mut raw = original.as_json();
            raw[field] = value;
            let changed: LeaseExpirySupervision = serde_json::from_value(raw).unwrap();
            assert_eq!(changed.validate().unwrap_err(), "root_lease_expiry_supervision_contract_invalid");
        }
        let mut raw = original.as_json();
        raw["targetRequestsGranted"] = json!(1);
        assert!(serde_json::from_value::<LeaseExpirySupervision>(raw).is_err());
    }
}
