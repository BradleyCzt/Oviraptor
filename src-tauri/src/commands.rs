use crate::{db, llm_hook, models::*, AppState};
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
// Business modules compile with explicit imports instead of inheriting this
// legacy scan/agent include! scope. IPC registration uses their actual owners.
#[path = "commands/workspace_projects.rs"]
pub(crate) mod workspace_projects;
#[path = "commands/application_overview.rs"]
pub(crate) mod application_overview;
#[path = "commands/application_settings.rs"]
pub(crate) mod application_settings;
#[path = "commands/workspace_config_profiles.rs"]
pub(crate) mod workspace_config_profiles;
#[path = "commands/assets.rs"]
pub(crate) mod assets;
#[path = "commands/asset_export.rs"]
pub(crate) mod asset_export;

// Ownership regression fixtures compare against the asset selection predicate.
#[cfg(test)]
use assets::asset_filter;
#[cfg(test)]
use assets::detect_target;
#[cfg(test)]
use workspace_projects::{delete_project_for_connection, project_impact_for_connection};
#[cfg(test)]
use workspace_config_profiles::{delete_config_profile_inner, persist_config_profile, read_config_profiles};
include!("commands/asset_ownership.rs");
include!("commands/ownership_directory.rs");
include!("commands/ownership_public_suffix.rs");
include!("commands/ownership_review.rs");
include!("commands/ownership_operations.rs");
include!("commands/ownership_authorization.rs");
include!("commands/ownership_unit_projection.rs");
include!("commands/ownership_o3_tests.rs");
include!("commands/exposure.rs");
include!("commands/hackerone.rs");
#[path = "commands/environment.rs"]
pub(crate) mod environment;
#[path = "commands/model_profiles.rs"]
pub(crate) mod model_profiles;
#[cfg(test)]
use environment::ensure_environment_install_idle;
#[path = "commands/tool_administrator.rs"]
pub(crate) mod tool_administrator;
include!("commands/src_assurance.rs");
include!("commands/web_investigation_policy.rs");
include!("commands/native_agent_trace.rs");
include!("commands/historical_agent_trace.rs");
include!("commands/trace_display_privacy.rs");
include!("commands/scan_lifecycle.rs");
include!("commands/knowledge_learning.rs");
include!("commands/knowledge_learning_skills.rs");
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
