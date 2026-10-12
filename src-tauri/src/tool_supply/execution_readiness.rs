//! Read-only supply/platform admission. This does not grant a role, claim a
//! process, execute a package, or manufacture an execution receipt.
use super::{approval, frozen_plan, ToolAdministratorConfiguration, ToolCapabilityManifest};
use rusqlite::Transaction;
use serde_json::json;

/// No JSON constructor, caller-selected runner, or support boolean exists.
/// A Broker must still prove its original worker capability and input scope.
pub(crate) struct VerifiedToolExecutionReadiness {
    current: frozen_plan::VerifiedCurrentToolPlan,
    approval: approval::VerifiedToolApproval,
    container: Option<super::container_parser::Runtime>,
}
impl VerifiedToolExecutionReadiness {
    pub(crate) fn original_plan(&self) -> &frozen_plan::VerifiedCurrentToolPlan {
        &self.current
    }
    pub(crate) fn original_approval(&self) -> &approval::VerifiedToolApproval {
        &self.approval
    }
    pub(crate) fn container(&self) -> Option<&super::container_parser::Runtime> {
        self.container.as_ref()
    }
}

fn platform() -> String {
    let os = std::env::consts::OS;
    format!("{os}-{}", std::env::consts::ARCH)
}

fn require_fixed_parser_contract(
    manifest: &ToolCapabilityManifest,
    runtime: &str,
) -> Result<(), String> {
    if manifest.platform != platform() {
        return Err("tool_execution_platform_mismatch".into());
    }
    // This names the sole prospective fixed parser adapter, not arbitrary
    // package entrypoints or an assertion that this adapter already executes.
    if runtime != "offline-parser-v1" && runtime != super::container_parser::ADAPTER {
        return Err("tool_execution_runtime_unsupported".into());
    }
    if manifest.network_policy != "none"
        || manifest.filesystem_policy != "read-only-input-and-private-temp-output"
        || manifest.process_limit != 1
        || manifest.cpu_limit != 1
        || manifest.memory_mib == 0
        || manifest.timeout_seconds == 0
        || manifest.output_bytes == 0
        || manifest.input_schema_version != 1
        || manifest.output_schema_version != 1
    {
        return Err("tool_execution_parser_policy_unsupported".into());
    }
    Ok(())
}

fn require_actual_supported_parser() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        super::macos_static_parser::MacosStaticParserReadiness::inspect()
            .require_supported()
            .map_err(str::to_owned)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("unsupported_sandbox:fixed_parser_platform_adapter_unavailable".into())
    }
}

pub(crate) fn require_current_on(
    tx: &Transaction<'_>,
    configuration: &ToolAdministratorConfiguration,
    root: &str,
    approval_id: &str,
) -> Result<VerifiedToolExecutionReadiness, String> {
    let current = frozen_plan::require_current_on(tx, configuration, root)?;
    if !uuid::Uuid::parse_str(approval_id).is_ok_and(|id| id.to_string() == approval_id) {
        return Err("tool_execution_approval_id_invalid".into());
    }
    let tools = current.fact()["tools"]
        .as_array()
        .ok_or("tool_execution_original_inventory_changed")?;
    let selected: Vec<_> = tools
        .iter()
        .filter(|item| item["approvalId"].as_str() == Some(approval_id))
        .collect();
    if selected.len() != 1 {
        return Err("tool_execution_approval_not_frozen".into());
    }
    let approved = approval::read_original_on(tx, configuration, approval_id)?
        .ok_or("tool_execution_original_approval_missing")?;
    let expected = json!({
        "approvalId": approved.id(), "approvalHash": approved.envelope_hash(),
        "manifest": approved.manifest(), "runtimeAdapter": approved.runtime_adapter(),
        "runtimeDigest": approved.runtime_digest(),
    });
    if selected[0] != &expected {
        return Err("tool_execution_original_inventory_changed".into());
    }
    require_fixed_parser_contract(approved.manifest(), approved.runtime_adapter())?;
    // The original host parser remains unsupported. The explicit container
    // adapter binds its complete fixed mechanism to the original administrator
    // digest; each execution additionally verifies its actual cgroup policy.
    // Neither this inventory proof nor container availability grants a Worker.
    let container = if approved.runtime_adapter() == super::container_parser::ADAPTER {
        let runtime = super::container_parser::Runtime::inspect()?;
        if runtime.digest() != approved.runtime_digest() {
            return Err("tool_execution_runtime_digest_mismatch".into());
        }
        Some(runtime)
    } else {
        require_actual_supported_parser()?;
        None
    };
    Ok(VerifiedToolExecutionReadiness {
        current,
        approval: approved,
        container,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> ToolCapabilityManifest {
        ToolCapabilityManifest {
            schema_version: 1,
            id: "offline-parser".into(),
            version: "1.0".into(),
            platform: platform(),
            source: "fixed-release".into(),
            digest: format!("sha256:{}", "a".repeat(64)),
            license: "MIT".into(),
            entrypoint: "bin/parser".into(),
            broker_capabilities: vec!["source_analysis".into()],
            network_policy: "none".into(),
            filesystem_policy: "read-only-input-and-private-temp-output".into(),
            timeout_seconds: 10,
            memory_mib: 64,
            cpu_limit: 1,
            process_limit: 1,
            output_bytes: 4096,
            input_schema_version: 1,
            output_schema_version: 1,
        }
    }
    #[test]
    fn execution_readiness_contract_never_promotes_signed_inventory_to_supported_runner() {
        require_fixed_parser_contract(&manifest(), "offline-parser-v1").unwrap();
        let error = require_actual_supported_parser().unwrap_err();
        #[cfg(target_os = "macos")]
        assert!(matches!(
            error.as_str(),
            "unsupported_sandbox:macos_hard_memory_containment_unverified"
                | "unsupported_sandbox:macos_seatbelt_unavailable"
        ));
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            error,
            "unsupported_sandbox:fixed_parser_platform_adapter_unavailable"
        );
    }
    #[test]
    fn execution_readiness_fixed_contract_rejects_platform_runtime_and_expanded_policy() {
        let mut changed = manifest();
        changed.platform = "caller-platform".into();
        assert_eq!(
            require_fixed_parser_contract(&changed, "offline-parser-v1").unwrap_err(),
            "tool_execution_platform_mismatch"
        );
        assert_eq!(
            require_fixed_parser_contract(&manifest(), "arbitrary-shell").unwrap_err(),
            "tool_execution_runtime_unsupported"
        );
        for dimension in 0..7 {
            let mut changed = manifest();
            match dimension {
                0 => changed.network_policy = "broker-only".into(),
                1 => changed.filesystem_policy = "host".into(),
                2 => changed.process_limit = 2,
                3 => changed.cpu_limit = 2,
                4 => changed.memory_mib = 0,
                5 => changed.input_schema_version = 2,
                _ => changed.output_schema_version = 2,
            }
            assert_eq!(
                require_fixed_parser_contract(&changed, "offline-parser-v1").unwrap_err(),
                "tool_execution_parser_policy_unsupported"
            );
        }
    }
}
