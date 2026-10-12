//! Creation-only protocol. Its presence conveys neither C nor target authority.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct NewEvidenceSupervision {
    schema_version: u8,
    source: String,
    deduplication: String,
    permitted_step: String,
    execution_grant: bool,
}
impl NewEvidenceSupervision {
    pub(super) fn creation_contract() -> Self {
        Self {
            schema_version: 1,
            source: "closed_web_executor_original_http_receipt".into(),
            deduplication: "original_node_physical_fact".into(),
            permitted_step: "assess:new_evidence".into(),
            execution_grant: false,
        }
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        if *self != Self::creation_contract() {
            return Err("root_new_evidence_supervision_contract_invalid".into());
        }
        Ok(())
    }
    pub(super) fn as_json(&self) -> Value {
        json!(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_evidence_protocol_rejects_grant_scope_step_and_dedup_mutations() {
        let original = NewEvidenceSupervision::creation_contract();
        original.validate().unwrap();
        let raw = original.as_json();
        for (key, value) in [
            ("schemaVersion", json!(2)),
            ("source", json!("graph_only")),
            ("deduplication", json!("revision_counter")),
            ("permittedStep", json!("dispatch:web_executor")),
            ("executionGrant", json!(true)),
        ] {
            let mut changed = raw.clone();
            changed[key] = value;
            let decoded: NewEvidenceSupervision = serde_json::from_value(changed).unwrap();
            assert_eq!(
                decoded.validate().unwrap_err(),
                "root_new_evidence_supervision_contract_invalid"
            );
        }
        let mut extra = raw;
        extra["capabilities"] = json!(["http_request"]);
        assert!(serde_json::from_value::<NewEvidenceSupervision>(extra).is_err());
    }
}
