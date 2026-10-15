//! Scanning business owner: admission, execution, retained evidence and recovery.
//! Source units below share this cohesive scope; other business owners stay explicit.
// Thin bridge for Ownership O3 regression fixtures; available only in test builds.
// The production function and AuthorizationControlSetup fields stay private.
#[cfg(test)]
pub(crate) mod ownership_test_support {
    pub(crate) fn authorization_control_setup_in(
        connection: &rusqlite::Connection,
        app_data_dir: &std::path::Path,
        scan_id: &str,
        target_url: &str,
    ) -> Result<super::AuthorizationControlSetup, String> {
        super::authorization_control_setup_in(connection, app_data_dir, scan_id, target_url)
    }
}

use crate::{db, llm_hook, AppState};
use crate::scanning_models::*;
#[cfg(test)]
use crate::asset_models::{
    AssetQuery, AssetSummary,
};
#[cfg(test)]
use crate::configuration_models::{
    ConfigProfileInput,
};
use crate::skills_catalog::model::{
    AgentKnowledgeEntry, AgentLearningCandidate,
};
use crate::model_configuration::{
    default_settings as sentinel_settings, openai_chat_completion_model,
};
#[cfg(test)]
use crate::model_configuration::model_parameter_billions;
use crate::worker_resources::{
    helper_node_candidates, resolve_frontend_recon_worker, resolve_helper_node,
    sentinel_runtime_path, verify_bundled_worker,
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rusqlite::{params, params_from_iter, types::Value as SqlValue, OptionalExtension, Row};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{mpsc, OnceLock},
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

#[cfg(test)]
use std::sync::Mutex;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(windows)]
fn configure_child_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_child_command(_command: &mut Command) {}

fn json(text: String) -> JsonValue {
    serde_json::from_str(&text).unwrap_or_else(|_| JsonValue::Object(Default::default()))
}

include!("commands/agent_instruction.rs");
// Scanning source units share this cohesive business owner.
// Other business commands compile under their explicit crate::commands owners.

// Scanning regression fixtures consume real Assets, project and configuration predicates.
#[cfg(test)]
use crate::commands::assets::test_support::asset_filter;
#[cfg(test)]
use crate::commands::assets::test_support::detect_target;
#[cfg(test)]
use crate::commands::workspace_projects::test_support::{delete_project_for_connection, project_impact_for_connection};
#[cfg(test)]
use crate::commands::workspace_config_profiles::test_support::{delete_config_profile_inner, persist_config_profile, read_config_profiles};

// Consume the Assets ownership domain through its explicit, read-only API.
use crate::commands::ownership::{domain_matches, normalize_ownership_domain};

#[cfg(test)]
use crate::commands::environment::test_support::ensure_environment_install_idle;

include!("commands/src_assurance.rs");
include!("commands/web_investigation_policy.rs");
include!("commands/native_agent_trace.rs");
include!("commands/historical_agent_trace.rs");
include!("commands/trace_display_privacy.rs");
include!("commands/scan_lifecycle.rs");
include!("commands/knowledge_learning.rs");
// Scanning supplies retained evidence; Skills owns the saved instruction and
// knowledge lifecycle. Only the structured learning model service is shared.
#[cfg(test)]
use crate::skills_catalog::{
    instructions::{import_agent_instructions_path,upsert_sec_skill_package},
    knowledge::import_agent_knowledge_path,
    patch::apply_skill_patch,
    sources::sec_skill_line_is_unsafe,
};
include!("commands/knowledge_learning_traces.rs");
include!("commands/knowledge_learning_candidates.rs");
include!("commands/knowledge_learning_quality.rs");
include!("commands/rule_packs.rs");
include!("commands/runtime_config.rs");
include!("commands/frontend_recon.rs");
include!("commands/native_frontend_recon.rs");
include!("commands/native_helpers.rs");
include!("commands/native_helper_process.rs");
include!("commands/native_helper_log_owner.rs");
include!("commands/native_helper_observer.rs");
include!("commands/native_recon_contract.rs");
#[cfg(test)]
include!("commands/native_src_transport.rs");
#[path = "commands/native_sensitive.rs"]
mod native_sensitive;
include!("commands/runtime_environment.rs");
include!("commands/agent_contract.rs");
include!("commands/role_config.rs");
include!("commands/agent_authorization_control.rs");
#[path = "commands/agent_authorization_broker.rs"]
pub(crate) mod authorization_side;
use authorization_side::execute_authorization_side;
#[cfg(test)]
use authorization_side::{authorization_identity_binding, claim_authorization_probe, authorization_probe_authority};
include!("commands/agent_runtime.rs");
include!("commands/agent_model_client.rs");
#[path = "commands/agent_native/http_budget_receipt.rs"]
pub(crate) mod http_budget_receipt;
include!("commands/agent_http_sealed_transport.rs");
include!("commands/agent_tools.rs");
include!("commands/agent_tools_specs.rs");
include!("commands/agent_tools_schema.rs");
include!("commands/agent_tools_scope.rs");
include!("commands/agent_tools_http.rs");
#[path = "commands/agent_tools_http/exchange.rs"]
pub(crate) mod http_exchange;
use http_exchange::agent_http_exchange;
include!("commands/agent_tools_replay.rs");
include!("commands/agent_tools_identity_diff.rs");
include!("commands/agent_tools_discovery.rs");
include!("commands/agent_tools_browser.rs");
include!("commands/agent_tools_verdict.rs");
include!("commands/agent_tools_budget_preflight.rs");
include!("commands/agent_tools_dispatch.rs");
include!("commands/agent_tools_source.rs");
include!("commands/agent_native.rs");
include!("commands/agent_backend.rs");
include!("commands/agent_backend_storage.rs");
include!("commands/agent_request_accounting.rs");
include!("commands/agent_request_review.rs");
include!("commands/agent_http_journal.rs");
include!("commands/agent_protection_stop.rs");
include!("commands/multi_agent_runtime.rs");
include!("commands/multi_agent_external_surface.rs");
include!("commands/multi_agent_business_logic.rs");
include!("commands/multi_agent_child_delivery.rs");
include!("commands/agent_gap_followup.rs");
include!("commands/agent_gap_submission.rs");
include!("commands/agent_gap_review.rs");
include!("commands/multi_agent_review_recovery.rs");
include!("commands/multi_agent_gap_recovery.rs");
include!("commands/agent_directive_proposals.rs");
include!("commands/agent_directive_closure.rs");
include!("commands/scan_execution.rs");
include!("commands/scan_execution_frontend.rs");
include!("commands/scan_execution_runtime.rs");
include!("commands/runner_log_display.rs");
include!("commands/native_process_log_replay.rs");
include!("commands/native_sdk_log.rs");
include!("commands/scan_execution_metrics.rs");
include!("commands/scan_execution_routing.rs");
include!("commands/native_source_scan.rs");
include!("commands/native_source_analysis.rs");
include!("commands/native_source_ci_policy.rs");
include!("commands/native_source_scope.rs");
include!("commands/native_source_runtime.rs");
include!("commands/native_source_initial.rs");
include!("commands/native_source_coordinator.rs");
include!("commands/native_source_heartbeat.rs");
include!("commands/native_source_tools.rs");
include!("commands/native_source_controlled_adapter.rs");
include!("commands/native_source_tool_reentry.rs");
include!("commands/native_source_completion.rs");
include!("commands/native_source_review_accounting.rs");
include!("commands/native_source_reviewer.rs");
include!("commands/native_source_expired_review.rs");
include!("commands/native_source_coverage_reviewer.rs");
include!("commands/native_scan_branches.rs");
include!("commands/native_protection_recovery_contract.rs");
include!("commands/native_protection_recovery_scope_budget.rs");
include!("commands/native_protection_recovery_sources.rs");
include!("commands/native_protection_recovery_approval.rs");
include!("commands/native_single_continuation_preview.rs");
include!("commands/native_single_continuation_model_contract.rs");
include!("commands/native_protection_recovery_priced.rs");
include!("commands/native_protection_recovery_priced_cancel.rs");
include!("commands/native_protection_recovery_draft.rs");
include!("commands/native_protection_recovery_cancel.rs");
#[path = "commands/native_protection_recovery_signature.rs"]
pub(crate) mod recovery_approval_signature;
include!("commands/agent_dialog_view.rs");
include!("commands/agent_dialog_selection.rs");
include!("commands/native_execution_history.rs");
include!("commands/native_source_findings.rs");
include!("commands/native_branch_dispatch.rs");
include!("commands/native_invocation.rs");
include!("commands/scan_quiescence.rs");
include!("commands/retired_data_guards.rs");
include!("commands/scan_deletion.rs");
include!("commands/source_inventory.rs");
include!("commands/code_analysis.rs");
include!("commands/workbench_startup.rs");
include!("commands/workbench_auth.rs");
include!("commands/workbench_retry.rs");
include!("commands/scan_control.rs");
include!("commands/scan_startup.rs");
include!("commands/web_dispatch_binding.rs");
include!("commands/native_web_mode_binding.rs");
include!("commands/native_tool_selection_policy_source.rs");
include!("commands/native_web_mode_plan.rs");
include!("commands/native_tool_supply_reentry.rs");
include!("commands/web_draft_id.rs");
include!("commands/web_dispatch_recovery.rs");
include!("commands/web_dispatch_closure.rs");
include!("commands/web_administrative_closure.rs");
include!("commands/web_closure_handoff.rs");
include!("commands/web_toolchain_binding.rs");
include!("commands/appsec_validation.rs");
#[cfg(test)]
include!("commands/tests_overview_provenance.rs");
include!("commands/investigation.rs");
include!("commands/result_ingestion.rs");
include!("commands/artifact_import_status.rs");
include!("commands/historical_import_views.rs");
include!("commands/result_ingestion_recon.rs");
include!("commands/result_ingestion_runs.rs");
include!("commands/result_ingestion_engines.rs");
#[cfg(test)]
include!("commands/web_mode_test_support.rs");
include!("commands/tests.rs");
include!("commands/agent_tests.rs");
include!("commands/agent_acceptance.rs");

#[path="commands/agent_homogeneous_execution.rs"]
pub(crate) mod homogeneous_execution;

// Explicit Scanning port; original authority types/fields remain unchanged.
pub(crate) mod worker_bridge {
    pub(crate) use super::{list_sentinel_scans_inner, sentinel_project_bundle, pause_sentinel_scan, resume_sentinel_scan, cancel_sentinel_scan, import_sentinel_snapshot_bytes};
}

// Explicit Scanning port; original authority types/fields remain unchanged.
pub(crate) mod scope_contract {
    pub(crate) use super::{capture_original_tool_selection_policy_on, original_root_capture_mode};
}

// Explicit Scanning port; original authority types/fields remain unchanged.
pub(crate) mod source_deletion {
    pub(crate) use super::{verify_closed_source_for_deletion};
}

// Explicit Scanning port; original authority types/fields remain unchanged.
pub(crate) mod financial_continuation {
    pub(crate) use super::{FinancialContinuationAuthorization, verify_financial_continuation_birth_on, native_coordinator_require_original_model_on};
}

// Explicit Scanning port; original authority types/fields remain unchanged.
pub(crate) mod publication_replay {
    pub(crate) use super::{native_coordinator_authorization_output_publication_replay_on, native_coordinator_business_logic_output_publication_replay_on, native_coordinator_external_surface_output_publication_replay_on, native_coordinator_capability_transition_publication_replay_on, native_coordinator_web_expiry_publication_replay_on};
}

// Explicit Scanning port; original authority types/fields remain unchanged.
pub(crate) mod evidence_bridge {
    pub(crate) use super::{resolve_agent_evidence_location, native_business_logic_input_collection_on, native_business_logic_output_collection_on, BUSINESS_LOGIC_READONLY_SYSTEM};
}

// Explicit Scanning port; original authority types/fields remain unchanged.
pub(crate) mod recovery_draft {
    pub(crate) use super::{native_protection_recovery_draft_action_on};
}

// Explicit Scanning port; original authority types/fields remain unchanged.
pub(crate) mod log_events {
    pub(crate) use super::{notify_native_sdk_log, notify_native_process_log, install_nest_runner_log_bus};
}

// Explicit Scanning port; original authority types/fields remain unchanged.
#[cfg(test)]
pub(crate) mod runtime_test_support {
    pub(crate) use super::{native_homogeneous_apply_after_paid_tick};
}

// Explicit Scanning port; original authority types/fields remain unchanged.
#[cfg(test)]
pub(crate) mod process_log_test_support {
    pub(crate) use super::{read_native_process_log_snapshot};
}
