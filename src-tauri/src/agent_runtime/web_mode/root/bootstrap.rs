//! Original creation-only Mapper/Executor admission contract, never an existing-root upgrade.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BootstrapDispatch {
    schema_version: u8,
    permitted_step: String,
    reserved_model_tokens: i64,
    reserved_model_requests: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    allocation_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    executor_allocation_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_supervision_token_floor: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_supervision_request_floor: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model_cost_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    supervision_request_allocation_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    budget_consumption_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    http_budget_consumption_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    web_lease_expiry_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capability_transition_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    authorization_output_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    authorization_output_request_capacity_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_surface_response_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_surface_output_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    external_surface_output_request_capacity_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    business_logic_input_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    business_logic_output_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    business_logic_output_request_capacity_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    business_logic_dispatch_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mapper_loop_rule: Option<String>,
}
impl BootstrapDispatch {
    pub(super) fn creation_contract() -> Self {
        Self::web_expiry_creation_contract()
    }
    pub(super) fn original_v7_contract() -> Self {
        Self {
            schema_version: 7,
            permitted_step: "dispatch:spa_api_mapper".into(),
            reserved_model_tokens: 8_000,
            reserved_model_requests: 1,
            allocation_rule: Some("current_balance_after_reviewer_floor".into()),
            executor_allocation_rule: Some(
                "current_balance_after_reviewer_floor_and_ordered_proposals".into(),
            ),
            root_supervision_token_floor: Some(16_000),
            root_supervision_request_floor: Some(1),
            model_cost_rule: Some("finite_grant_share_native_unlimited_no_scarcity".into()),
            supervision_request_allocation_rule: Some(
                crate::agent_runtime::multi_agent::budget::supervision_capacity::ALLOCATION_RULE
                    .into(),
            ),
            budget_consumption_rule: Some("original_settled_web_call_advisory_v1".into()),
            http_budget_consumption_rule: None,
            web_lease_expiry_rule: None,
            capability_transition_rule: None,
            authorization_output_rule: None,
            authorization_output_request_capacity_rule: None,
            external_surface_response_rule: None,
            external_surface_output_rule: None,
            external_surface_output_request_capacity_rule: None,
            business_logic_input_rule: None,
            business_logic_output_rule: None,
            business_logic_output_request_capacity_rule: None,
            business_logic_dispatch_rule: None,
            mapper_loop_rule: None,
        }
    }
    pub(super) fn http_creation_contract() -> Self {
        let mut value = Self::original_v7_contract();
        value.schema_version = 8;
        value.http_budget_consumption_rule = Some(crate::agent_runtime::web_mode::http_budget_receipt::RULE.into());
        value
    }
    pub(super) fn web_expiry_creation_contract() -> Self {
        let mut value = Self::http_creation_contract();
        value.schema_version = 9;
        value.web_lease_expiry_rule = Some("original_expired_web_fee_delivery_advisory_v1".into());
        value
    }
    pub(super) fn capability_creation_contract() -> Self {
        let mut value = Self::web_expiry_creation_contract();
        value.schema_version = 10;
        value.capability_transition_rule = Some("original_closed_web_capability_transition_advisory_v1".into());
        value
    }
    pub(super) fn authorization_output_creation_contract() -> Self {
        let mut value = Self::capability_creation_contract();
        value.schema_version = 11;
        value.authorization_output_rule = Some("original_closed_three_side_authorization_output_advisory_v1".into());
        value.authorization_output_request_capacity_rule = Some("retain_one_original_authorization_output_assessment_per_worker_v1".into());
        value
    }
    pub(super) fn external_surface_output_creation_contract() -> Self {
        let mut value = Self::authorization_output_creation_contract();
        value.schema_version = 12;
        value.external_surface_response_rule = Some("original_received_anonymous_public_surface_response_v1".into());
        value.external_surface_output_rule = Some("original_closed_anonymous_entry_surface_output_advisory_v1".into());
        value.external_surface_output_request_capacity_rule = Some("retain_one_original_surface_output_assessment_per_worker_v1".into());
        value
    }
    pub(super) fn business_logic_output_creation_contract() -> Self {
        let mut value = Self::external_surface_output_creation_contract();
        value.schema_version = 13;
        value.business_logic_input_rule = Some("original_frozen_business_logic_input_v1".into());
        value.business_logic_output_rule = Some("original_closed_readonly_business_logic_output_advisory_v1".into());
        value.business_logic_output_request_capacity_rule = Some("retain_one_original_business_logic_output_assessment_per_worker_v1".into());
        value
    }
    pub(super) fn business_logic_dispatch_creation_contract() -> Self {
        let mut value = Self::business_logic_output_creation_contract();
        value.schema_version = 14;
        value.business_logic_dispatch_rule = Some("original_paid_frozen_business_logic_dispatch_v1".into());
        value
    }
    pub(super) fn mapper_loop_creation_contract() -> Self {
        let mut value = Self::business_logic_dispatch_creation_contract();
        value.schema_version = 15;
        value.reserved_model_requests = 2;
        value.mapper_loop_rule = Some("original_frozen_mapper_two_round_one_lookup_v1".into());
        value
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        let original_v6 = Self::original_v6_contract();
        let original_v5 = Self::original_v5_contract();
        let mut original_v4 = original_v5.clone();
        original_v4.schema_version = 4;
        original_v4.model_cost_rule = None;
        let mut original_v3 = original_v4.clone();
        original_v3.schema_version = 3;
        original_v3.root_supervision_token_floor = None;
        original_v3.root_supervision_request_floor = None;
        let mut original_v2 = original_v3.clone();
        original_v2.schema_version = 2;
        original_v2.executor_allocation_rule = None;
        let mut original_native = original_v2.clone();
        original_native.schema_version = 1;
        original_native.allocation_rule = None;
        if self != &Self::creation_contract()
            && self != &Self::original_v7_contract()
            && self != &Self::http_creation_contract()
            && self != &Self::web_expiry_creation_contract()
            && self != &Self::capability_creation_contract()
            && self != &Self::authorization_output_creation_contract()
            && self != &Self::external_surface_output_creation_contract()
            && self != &Self::business_logic_output_creation_contract()
            && self != &Self::business_logic_dispatch_creation_contract()
            && self != &Self::mapper_loop_creation_contract()
            && self != &original_v6
            && self != &original_v5
            && self != &original_v4
            && self != &original_v3
            && self != &original_v2
            && self != &original_native
        {
            return Err("root_bootstrap_dispatch_contract_invalid".into());
        }
        Ok(())
    }
    pub(super) fn original_v5_contract() -> Self {
        let mut value = Self::original_v6_contract();
        value.schema_version = 5;
        value.supervision_request_allocation_rule = None;
        value
    }
    pub(super) fn original_v6_contract() -> Self {
        let mut value = Self::original_v7_contract();
        value.schema_version = 6;
        value.budget_consumption_rule = None;
        value
    }
    pub(super) fn as_json(&self) -> Value {
        json!(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapper_v15_birth_requires_exact_new_contract_without_old_upgrade() {
        let old = BootstrapDispatch::business_logic_dispatch_creation_contract();
        let new = BootstrapDispatch::mapper_loop_creation_contract();
        new.validate().unwrap();
        assert_eq!(new.as_json()["schemaVersion"], 15);
        assert_eq!(new.as_json()["reservedModelRequests"], 2);
        assert_eq!(BootstrapDispatch::creation_contract().as_json()["schemaVersion"], 9);
        let original = serde_json::to_string(&old).unwrap();
        let decoded: BootstrapDispatch = serde_json::from_str(&original).unwrap();
        assert_eq!(serde_json::to_string(&decoded).unwrap(), original);
        assert!(old.as_json().get("mapperLoopRule").is_none());
        for field in ["mapperLoopRule", "businessLogicDispatchRule", "httpBudgetConsumptionRule"] {
            let mut missing = new.as_json();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<BootstrapDispatch>(missing).unwrap().validate().is_err());
        }
        let mut injected = old.as_json();
        injected["mapperLoopRule"] = new.as_json()["mapperLoopRule"].clone();
        assert!(serde_json::from_value::<BootstrapDispatch>(injected).unwrap().validate().is_err());
        let mut changed = new.as_json();
        changed["reservedModelRequests"] = json!(1);
        assert!(serde_json::from_value::<BootstrapDispatch>(changed).unwrap().validate().is_err());
        let mut inherited = new.as_json();
        inherited["schemaVersion"] = json!(14);
        inherited["reservedModelRequests"] = json!(1);
        inherited.as_object_mut().unwrap().remove("mapperLoopRule");
        assert_eq!(inherited, old.as_json());
    }
    #[test]
    fn business_logic_v14_dispatch_birth_requires_exact_original_rule() {
        let old = BootstrapDispatch::business_logic_output_creation_contract();
        let new = BootstrapDispatch::business_logic_dispatch_creation_contract();
        new.validate().unwrap();
        assert_eq!(new.as_json()["schemaVersion"],14);
        assert_eq!(new.as_json()["businessLogicDispatchRule"],"original_paid_frozen_business_logic_dispatch_v1");
        assert_eq!(BootstrapDispatch::creation_contract().as_json()["schemaVersion"],9);
        assert!(old.as_json().get("businessLogicDispatchRule").is_none());
        let raw=serde_json::to_string(&old).unwrap();
        let restored:BootstrapDispatch=serde_json::from_str(&raw).unwrap();
        restored.validate().unwrap();
        assert_eq!(serde_json::to_string(&restored).unwrap(),raw);
        let mut missing=new.as_json();
        missing.as_object_mut().unwrap().remove("businessLogicDispatchRule");
        assert!(serde_json::from_value::<BootstrapDispatch>(missing).unwrap().validate().is_err());
        let mut altered=new.as_json();
        altered["businessLogicDispatchRule"]=json!("foreign_rule");
        assert!(serde_json::from_value::<BootstrapDispatch>(altered).unwrap().validate().is_err());
        let mut injected=old.as_json();
        injected["businessLogicDispatchRule"]=new.as_json()["businessLogicDispatchRule"].clone();
        assert!(serde_json::from_value::<BootstrapDispatch>(injected).unwrap().validate().is_err());
        let mut inherited=new.as_json();
        inherited["schemaVersion"]=json!(13);
        inherited.as_object_mut().unwrap().remove("businessLogicDispatchRule");
        assert_eq!(inherited,old.as_json());
    }
    #[test]
    fn business_logic_v13_birth_requires_exact_rules_without_old_backfill() {
        let v12 = BootstrapDispatch::external_surface_output_creation_contract();
        let v13 = BootstrapDispatch::business_logic_output_creation_contract();
        v13.validate().unwrap();
        assert_eq!(v13.as_json()["schemaVersion"],13);
        let default = BootstrapDispatch::creation_contract();
        assert_eq!(default.as_json()["schemaVersion"],9);
        for key in ["businessLogicInputRule","businessLogicOutputRule","businessLogicOutputRequestCapacityRule"] {
            assert!(v12.as_json().get(key).is_none());
            assert!(default.as_json().get(key).is_none());
            let mut missing=v13.as_json();
            missing.as_object_mut().unwrap().remove(key);
            assert!(serde_json::from_value::<BootstrapDispatch>(missing).unwrap().validate().is_err());
            let mut changed=v13.as_json();
            changed[key]=json!("foreign_rule");
            assert!(serde_json::from_value::<BootstrapDispatch>(changed).unwrap().validate().is_err());
            for old in [&v12,&default] {
                let mut injected=old.as_json();
                injected[key]=v13.as_json()[key].clone();
                assert!(serde_json::from_value::<BootstrapDispatch>(injected).unwrap().validate().is_err());
            }
        }
        for key in ["externalSurfaceResponseRule","externalSurfaceOutputRule","externalSurfaceOutputRequestCapacityRule"] {
            assert_eq!(v13.as_json()[key],v12.as_json()[key]);
        }
        for original in [v12,default] {
            let raw=serde_json::to_string(&original).unwrap();
            let restored:BootstrapDispatch=serde_json::from_str(&raw).unwrap();
            restored.validate().unwrap();
            assert_eq!(serde_json::to_string(&restored).unwrap(),raw);
        }
    }
    #[test]
    fn external_surface_v12_birth_requires_all_original_rules_without_old_backfill() {
        let v11 = BootstrapDispatch::authorization_output_creation_contract();
        let v12 = BootstrapDispatch::external_surface_output_creation_contract();
        v12.validate().unwrap();
        assert_eq!(v12.as_json()["schemaVersion"], 12);
        assert_eq!(BootstrapDispatch::creation_contract().as_json()["schemaVersion"], 9);
        for key in ["externalSurfaceResponseRule", "externalSurfaceOutputRule", "externalSurfaceOutputRequestCapacityRule"] {
            assert!(v11.as_json().get(key).is_none());
            let mut missing = v12.as_json();
            missing.as_object_mut().unwrap().remove(key);
            assert!(serde_json::from_value::<BootstrapDispatch>(missing).unwrap().validate().is_err());
            let mut altered = v12.as_json();
            altered[key] = json!("foreign_rule");
            assert!(serde_json::from_value::<BootstrapDispatch>(altered).unwrap().validate().is_err());
            let mut injected = v11.as_json();
            injected[key] = v12.as_json()[key].clone();
            assert!(serde_json::from_value::<BootstrapDispatch>(injected).unwrap().validate().is_err());
        }
        for key in ["authorizationOutputRule", "authorizationOutputRequestCapacityRule"] {
            assert_eq!(v12.as_json()[key], v11.as_json()[key]);
        }
        let raw = serde_json::to_string(&v11).unwrap();
        assert_eq!(serde_json::to_string(&serde_json::from_str::<BootstrapDispatch>(&raw).unwrap()).unwrap(), raw);
    }
    #[test]
    fn mapper_creation_contract_keeps_original_native_json_and_rejects_rule_mutations() {
        let raw = r#"{"schemaVersion":1,"permittedStep":"dispatch:spa_api_mapper","reservedModelTokens":8000,"reservedModelRequests":1}"#;
        let original: BootstrapDispatch = serde_json::from_str(raw).unwrap();
        original.validate().unwrap();
        assert_eq!(serde_json::to_string(&original).unwrap(), raw);
        let raw_v2 = r#"{"schemaVersion":2,"permittedStep":"dispatch:spa_api_mapper","reservedModelTokens":8000,"reservedModelRequests":1,"allocationRule":"current_balance_after_reviewer_floor"}"#;
        let original_v2: BootstrapDispatch = serde_json::from_str(raw_v2).unwrap();
        original_v2.validate().unwrap();
        assert_eq!(serde_json::to_string(&original_v2).unwrap(), raw_v2);
        let raw_v3 = r#"{"schemaVersion":3,"permittedStep":"dispatch:spa_api_mapper","reservedModelTokens":8000,"reservedModelRequests":1,"allocationRule":"current_balance_after_reviewer_floor","executorAllocationRule":"current_balance_after_reviewer_floor_and_ordered_proposals"}"#;
        let original_v3: BootstrapDispatch = serde_json::from_str(raw_v3).unwrap();
        original_v3.validate().unwrap();
        assert_eq!(serde_json::to_string(&original_v3).unwrap(), raw_v3);
        let fresh = BootstrapDispatch::creation_contract();
        fresh.validate().unwrap();
        assert_eq!(fresh.as_json()["schemaVersion"], 9);
        assert_eq!(fresh.as_json()["webLeaseExpiryRule"], "original_expired_web_fee_delivery_advisory_v1");
        let capability = BootstrapDispatch::capability_creation_contract();
        capability.validate().unwrap();
        let authorization = BootstrapDispatch::authorization_output_creation_contract();
        authorization.validate().unwrap();
        assert_eq!(authorization.as_json()["schemaVersion"], 11);
        for key in ["authorizationOutputRule", "authorizationOutputRequestCapacityRule"] {
            let mut missing = authorization.as_json();
            missing.as_object_mut().unwrap().remove(key);
            assert!(serde_json::from_value::<BootstrapDispatch>(missing).unwrap().validate().is_err());
        }
        assert!(capability.as_json().get("authorizationOutputRule").is_none());
        let mut missing_capability = capability.as_json();
        missing_capability.as_object_mut().unwrap().remove("capabilityTransitionRule");
        assert!(serde_json::from_value::<BootstrapDispatch>(missing_capability).unwrap().validate().is_err());
        let mut missing_expiry = fresh.as_json();
        missing_expiry.as_object_mut().unwrap().remove("webLeaseExpiryRule");
        assert!(serde_json::from_value::<BootstrapDispatch>(missing_expiry).unwrap().validate().is_err());
        let http = BootstrapDispatch::http_creation_contract();
        http.validate().unwrap();
        let mut missing_http = http.as_json();
        missing_http.as_object_mut().unwrap().remove("httpBudgetConsumptionRule");
        assert!(serde_json::from_value::<BootstrapDispatch>(missing_http).unwrap().validate().is_err());
        let legacy = BootstrapDispatch::original_v6_contract();
        legacy.validate().unwrap();
        assert!(legacy.as_json().get("budgetConsumptionRule").is_none());
        let mut missing_consumption = fresh.as_json();
        missing_consumption.as_object_mut().unwrap().remove("budgetConsumptionRule");
        assert!(serde_json::from_value::<BootstrapDispatch>(missing_consumption).unwrap().validate().is_err());
        let mut injected_consumption = legacy.as_json();
        injected_consumption["budgetConsumptionRule"] = fresh.as_json()["budgetConsumptionRule"].clone();
        assert!(serde_json::from_value::<BootstrapDispatch>(injected_consumption).unwrap().validate().is_err());
        let raw_v5 = r#"{"schemaVersion":5,"permittedStep":"dispatch:spa_api_mapper","reservedModelTokens":8000,"reservedModelRequests":1,"allocationRule":"current_balance_after_reviewer_floor","executorAllocationRule":"current_balance_after_reviewer_floor_and_ordered_proposals","rootSupervisionTokenFloor":16000,"rootSupervisionRequestFloor":1,"modelCostRule":"finite_grant_share_native_unlimited_no_scarcity"}"#;
        let original_v5: BootstrapDispatch = serde_json::from_str(raw_v5).unwrap();
        original_v5.validate().unwrap();
        assert_eq!(serde_json::to_string(&original_v5).unwrap(), raw_v5);
        let mut missing_capacity = fresh.as_json();
        missing_capacity
            .as_object_mut()
            .unwrap()
            .remove("supervisionRequestAllocationRule");
        assert!(
            serde_json::from_value::<BootstrapDispatch>(missing_capacity)
                .unwrap()
                .validate()
                .is_err()
        );
        for old in [&original, &original_v2, &original_v3, &original_v5] {
            let mut injected = old.as_json();
            injected["supervisionRequestAllocationRule"] =
                fresh.as_json()["supervisionRequestAllocationRule"].clone();
            assert!(serde_json::from_value::<BootstrapDispatch>(injected)
                .unwrap()
                .validate()
                .is_err());
        }
        let raw_v4 = r#"{"schemaVersion":4,"permittedStep":"dispatch:spa_api_mapper","reservedModelTokens":8000,"reservedModelRequests":1,"allocationRule":"current_balance_after_reviewer_floor","executorAllocationRule":"current_balance_after_reviewer_floor_and_ordered_proposals","rootSupervisionTokenFloor":16000,"rootSupervisionRequestFloor":1}"#;
        let original_v4: BootstrapDispatch = serde_json::from_str(raw_v4).unwrap();
        original_v4.validate().unwrap();
        assert_eq!(serde_json::to_string(&original_v4).unwrap(), raw_v4);
        let mut missing_rule = fresh.as_json();
        missing_rule
            .as_object_mut()
            .unwrap()
            .remove("modelCostRule");
        assert!(serde_json::from_value::<BootstrapDispatch>(missing_rule)
            .unwrap()
            .validate()
            .is_err());
        for old in [&original, &original_v2, &original_v3, &original_v4] {
            let mut injected = old.as_json();
            injected["modelCostRule"] = fresh.as_json()["modelCostRule"].clone();
            assert!(serde_json::from_value::<BootstrapDispatch>(injected)
                .unwrap()
                .validate()
                .is_err());
        }
        for (key, value) in [
            ("schemaVersion", json!(10)),
            ("budgetConsumptionRule", json!("ignore_actual_cost")),
            ("supervisionRequestAllocationRule", json!("no_supervision")),
            ("modelCostRule", json!("free_model_calls")),
            ("allocationRule", json!("hard_ceiling")),
            ("reservedModelTokens", json!(9000)),
            ("reservedModelRequests", json!(2)),
        ] {
            let mut altered = fresh.as_json();
            altered[key] = value;
            assert!(serde_json::from_value::<BootstrapDispatch>(altered)
                .unwrap()
                .validate()
                .is_err());
        }
        for field in ["rootSupervisionTokenFloor", "rootSupervisionRequestFloor"] {
            let mut missing = fresh.as_json();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<BootstrapDispatch>(missing)
                .unwrap()
                .validate()
                .is_err());
            let mut injected = original_v3.as_json();
            injected[field] = fresh.as_json()[field].clone();
            assert!(serde_json::from_value::<BootstrapDispatch>(injected)
                .unwrap()
                .validate()
                .is_err());
        }
        let mut upgraded = original_v3.as_json();
        upgraded["rootSupervisionTokenFloor"] = json!(16_000);
        upgraded["rootSupervisionRequestFloor"] = json!(1);
        assert!(serde_json::from_value::<BootstrapDispatch>(upgraded)
            .unwrap()
            .validate()
            .is_err());
        let mut missing_executor = fresh.as_json();
        missing_executor
            .as_object_mut()
            .unwrap()
            .remove("executorAllocationRule");
        assert!(
            serde_json::from_value::<BootstrapDispatch>(missing_executor)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut old_v2 = original_v2.as_json();
        old_v2["executorAllocationRule"] = fresh.as_json()["executorAllocationRule"].clone();
        assert!(serde_json::from_value::<BootstrapDispatch>(old_v2)
            .unwrap()
            .validate()
            .is_err());
        let mut missing = fresh.as_json();
        missing.as_object_mut().unwrap().remove("allocationRule");
        assert!(serde_json::from_value::<BootstrapDispatch>(missing)
            .unwrap()
            .validate()
            .is_err());
        let mut old = original.as_json();
        old["allocationRule"] = fresh.as_json()["allocationRule"].clone();
        assert!(serde_json::from_value::<BootstrapDispatch>(old)
            .unwrap()
            .validate()
            .is_err());
    }
}
