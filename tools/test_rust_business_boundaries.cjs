// Architectural regression guard: business commands must compile in their own
// Rust namespace and retain their existing IPC names after ownership changes.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const read = (relative) => fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');
const registration = read('src-tauri/src/lib.rs');
const legacy = read('src-tauri/src/commands.rs');
const scanning = read('src-tauri/src/scanning.rs');
const owners = {
  workspace_projects: ['list_projects', 'save_project', 'project_impact', 'archive_project', 'delete_project'],
  application_overview: ['dashboard_stats'],
  application_settings: ['get_app_settings', 'get_app_icon_data_url', 'save_app_settings', 'save_app_icon', 'reset_app_icon'],
  workspace_config_profiles: ['list_config_profiles', 'save_config_profile', 'delete_config_profile'],
  assets: ['import_targets', 'list_targets', 'remove_target', 'list_assets', 'mark_asset_entry_opened',
    'add_content_rule', 'update_decision', 'update_asset_decisions', 'soft_delete_assets',
    'soft_delete_asset_selections', 'list_runs', 'list_logs', 'list_asset_events', 'startup_status',
    'acknowledge_interrupted_run'],
  asset_rule_catalog: ['get_asset_rule_catalog'],
  page_isolation: ['preview_page_rule', 'apply_page_rule', 'list_page_rules', 'disable_page_rule'],
  asset_export: ['export_assets'],
  tool_administrator: ['approve_tool_inventory', 'revoke_tool_inventory', 'read_tool_approval_inventory'],
  environment: ['check_environment', 'get_environment_preparation_status', 'list_environment_install_logs',
    'recover_environment_preparation', 'install_environment_dependencies'],
  model_profiles: ['test_model_profile'],
  hackerone: ['list_hackerone_programs', 'get_hackerone_detail', 'set_hackerone_bookmark',
    'list_hackerone_events', 'sync_hackerone', 'add_hackerone_scopes_to_project'],
};
for (const [owner, commands] of Object.entries(owners)) {
  test(`${owner} has an explicit Rust boundary and stable IPC registration`, () => {
    const source = read(`src-tauri/src/commands/${owner}.rs`);
    assert.match(legacy, new RegExp(`mod ${owner};`));
    assert.ok(!legacy.includes(`include!("commands/${owner}.rs")`));
    // A wildcard of the parent silently restores all cross-business coupling.
    assert.doesNotMatch(source, /use\s+(?:super|crate::commands)::\s*\*/);
    assert.doesNotMatch(source, /include!\s*\(/);
    if (owner === 'hackerone') {
      assert.match(source, /hackerone_models::\{/);
      assert.doesNotMatch(source, /crate::models::|(?:^|[\s,{])models::\{/m);
      assert.doesNotMatch(source, /crate::(?:commands|scanning|skills_catalog)::/);
    }
    assert.doesNotMatch(source, /(?:use\s+(?:crate|super)::|crate::)(?:conversations|skill_packages|scanning|commands::(?:agent|scan))/);
    if (owner === 'model_profiles') {
      // A connectivity check uses the shared gateway's refusal classifier;
      // it must not import any scan/Agent runtime or grant machinery.
      assert.doesNotMatch(source, /crate::agent_runtime::(?!model::gateway::looks_like_unsupported_tools)/);
    } else {
      assert.doesNotMatch(source, /(?:use\s+(?:crate|super)::|crate::)agent_runtime/);
    }
    const exported = [...source.matchAll(/#\[tauri::command\]\s*pub (?:async )?fn (\w+)/g)].map(match => match[1]);
    assert.deepEqual(exported.sort(), [...commands].sort());
    for (const command of commands) {
      const routes = [...registration.matchAll(new RegExp(`commands::(?:[a-z_]+::)?${command}(?=,)`, 'g'))];
      assert.deepEqual(routes.map(match => match[0]), [`commands::${owner}::${command}`]);
    }
  });
}

test('Scanning keeps the actual owner scope and explicit DTO edges', () => {
  assert.match(registration, /mod scanning;/);
  const indexNames = [...legacy.matchAll(/^\s*pub\(crate\) mod (\w+);/gm)].map(match => match[1]);
  const expectedIndex = ["workspace_projects", "application_overview", "application_settings", "workspace_config_profiles", "assets", "asset_rule_catalog", "page_isolation", "asset_export", "tool_administrator", "environment", "model_profiles", "ownership", "exposure", "hackerone"];
  assert.deepEqual(indexNames.sort(), [...expectedIndex].sort());
  const indexBody = legacy.replace(/\/\/[^\n]*/g, '')
    .replace(/#\[path = "[^"]+"\]\s*/g, '')
    .replace(/pub\(crate\) mod \w+;/g, '').trim();
  assert.equal(indexBody, '', 'index contains only the 14 reviewed module declarations');
  assert.match(legacy, /^pub\(crate\) mod exposure;/m);
  assert.doesNotMatch(legacy, /#\[path = "commands\/exposure\.rs"\]/);
  assert.doesNotMatch(legacy, /include!\s*\(/);
  assert.doesNotMatch(scanning, /pub(?:\([^)]*\))?\s+use[^;]*\*/);
  assert.match(scanning, /use crate::scanning_models::\*;/);
  assert.doesNotMatch(scanning, /(?:crate::|[\s,{])models::\*/);
  assert.match(scanning, /use crate::skills_catalog::model::\{\s*AgentKnowledgeEntry, AgentLearningCandidate,/);
  assert.match(scanning, /#\[cfg\(test\)\]\s*use crate::asset_models::\{\s*AssetQuery, AssetSummary,/);
  assert.match(scanning, /#\[cfg\(test\)\]\s*use crate::configuration_models::\{\s*ConfigProfileInput,/);
  assert.doesNotMatch(scanning, /use crate::(?:environment_models|hackerone_models|workspace_models|application_models)::/);
  const handler = registration.split('tauri::generate_handler![')[1]?.split('])')[0];
  assert.ok(handler, 'live IPC handler input exists');
  const routes = [...handler.matchAll(/^\s*((?:[a-z_]\w*::)+\w+),?\s*$/gm)].map(match => match[1]);
  assert.equal(routes.length, 294);
  assert.equal(new Set(routes.map(route => route.split('::').at(-1))).size, 294);
  assert.deepEqual(routes.filter(route => route.startsWith('scanning::')).map(route => route.split('::').at(-1)), ["create_sentinel_scan", "create_sentinel_url_scan", "create_sentinel_homogeneous_url_scan", "create_sentinel_url_scan_with_tool_selection", "list_capability_bundles", "list_role_config_drafts", "save_role_config_draft", "role_config_draft_status_label", "save_authorization_control", "get_authorization_control_setup", "list_agent_traces", "get_agent_trace", "generate_agent_learning_candidate", "analyze_agent_trace", "aggregate_agent_knowledge", "list_security_rule_packs", "save_security_rule_pack", "delete_security_rule_pack", "sync_security_rule_pack", "start_workbench_scan", "rescan_workbench_scan", "rescan_sentinel_scan", "confirm_sentinel_scan", "pause_sentinel_scan", "recover_native_source_pause_result", "resume_sentinel_scan", "cancel_sentinel_scan", "delete_sentinel_scan", "list_sentinel_scans", "search_sentinel_scan_page", "archive_sentinel_scan", "list_sentinel_scan_attempts", "list_sentinel_vulnerability_scan_ids", "get_sentinel_runner_log", "read_sentinel_runner_log", "read_native_process_log", "read_native_sdk_log", "read_native_mapper_prepared_history", "search_sentinel_scan_ids", "list_sentinel_targets", "list_sentinel_fuse_zone", "save_sentinel_fuse_review", "remove_sentinel_fuse_entry", "list_sentinel_checkpoints", "list_sentinel_findings", "list_historical_import_previews", "list_historical_import_runs", "list_historical_bundle_previews", "get_agent_target_execution", "get_agent_request_reviews", "record_agent_request_review", "get_native_scan_status", "get_native_budget_diagnostics", "get_native_root_cost_reconciliations", "confirm_native_root_cost_reconciliation", "preview_native_root_financial_continuation", "preview_native_single_continuation", "preview_native_protection_recovery", "discover_native_protection_recovery_sources", "preview_native_protection_recovery_proposal", "approve_native_protection_recovery", "preview_native_priced_protection_recovery", "approve_native_priced_protection_recovery", "get_native_priced_protection_recovery_approval", "find_native_priced_protection_recovery_approval", "cancel_native_priced_protection_recovery_approval", "get_native_priced_protection_recovery_cancellation", "get_native_protection_recovery_approval", "create_native_protection_recovery_draft", "get_native_protection_recovery_draft", "get_native_protection_recovery_cancellation", "get_native_protection_recovery_cancellation_by_request", "approve_native_root_financial_continuation", "get_native_root_financial_continuation", "discover_native_root_financial_continuation", "get_native_scan_timeline_page", "get_agent_dialog_view", "save_agent_dialog_view", "get_agent_dialog_selection", "save_agent_dialog_selection", "get_agent_dialog_task", "recover_never_dispatched_web_attempt", "close_never_dispatched_web_attempt", "preview_web_administrative_closure", "close_web_task_administratively", "preview_web_closure_handoff", "create_web_closure_handoff", "preview_agent_gap_followup", "create_agent_gap_followup", "get_agent_gap_followup_submission", "release_agent_gap_followup_submission", "get_native_attempt_mailbox_history", "get_native_attempt_tool_history", "get_native_attempt_execution_history", "get_native_source_findings", "export_native_source_findings", "draft_scan_directive", "post_scan_directive", "confirm_scan_directive", "revise_scan_directive", "reject_scan_directive", "reconcile_scan_directive_receipt", "reconcile_historical_scan_directive_receipt", "cancel_scan_directive", "list_sentinel_opportunities", "update_sentinel_opportunity_status", "get_investigation_graph", "list_investigation_hypotheses", "update_investigation_hypothesis", "set_investigation_mutation_approval", "replay_investigation_request", "save_investigation_validation", "list_investigation_validations", "investigation_overview", "list_appsec_scan_result", "sentinel_overview_stats", "list_sentinel_validations", "list_all_sentinel_validations", "list_sentinel_validation_work_items", "save_sentinel_validation", "export_sentinel_results", "import_sentinel_results", "export_sentinel_project", "import_sentinel_project", "get_artifact_import_status"]);
  for (const name of ["worker_bridge", "scope_contract", "source_deletion", "financial_continuation", "publication_replay", "evidence_bridge", "recovery_draft", "log_events"]) {
    assert.ok(scanning.includes(`pub(crate) mod ${name} {`), name);
    assert.ok(!scanning.includes(`#[cfg(test)]\npub(crate) mod ${name} {`), name);
  }
  for (const name of ["runtime_test_support", "process_log_test_support"])
    assert.ok(scanning.includes(`#[cfg(test)]\npub(crate) mod ${name} {`), name);
  const typedScope = scanning + '\n' + read('src-tauri/src/commands/agent_native.rs')
    + '\n' + read('src-tauri/src/commands/multi_agent_runtime.rs');
  for (const name of ["authorization_output_receipt", "authorization_side", "business_logic_dispatch_receipt", "business_logic_output_receipt", "external_surface_output_receipt", "external_surface_response_receipt", "homogeneous_execution", "http_budget_receipt", "http_exchange", "mapper_evidence_lookup", "mapper_loop_request", "recovery_approval_signature"])
    assert.equal(typedScope.split(`pub(crate) mod ${name};`).length - 1, 1, name);
  const modelLines = read('src-tauri/src/models.rs').split('\n')
    .map(line => line.trim()).filter(line => line && !line.startsWith('//'));
  assert.deepEqual(modelLines, ["pub use crate::asset_models::*;", "pub use crate::scanning_models::*;", "pub use crate::workspace_models::*;", "pub use crate::configuration_models::*;", "pub use crate::application_models::*;", "pub use crate::environment_models::*;", "pub use crate::hackerone_models::*;", "pub use crate::skills_catalog::model::*;"]);
});

test('asset exports share the inventory predicate and ownership fixtures verify it', () => {
  const exported = read('src-tauri/src/commands/asset_export.rs');
  const assets = read('src-tauri/src/commands/assets.rs');
  const ownerImports = [...exported.matchAll(/^\s*use\s+super::assets::([^;]+);/gm)];
  assert.equal(ownerImports.length, 1, 'export has one narrow Assets owner import');
  const clause = /^\{([^{}]+)\}$/.exec(ownerImports[0][1].trim());
  assert.ok(clause, 'Assets owner import is a plain named symbol set');
  const helpers = clause[1].split(',').map(name => name.trim()).filter(Boolean);
  assert.ok(helpers.every(name => /^[A-Za-z_]\w*$/.test(name)), 'no aliases, glob or nested import');
  assert.deepEqual(helpers.sort(), ['asset_filter', 'asset_from_row', 'validate_asset_query', 'ASSET_SELECT'].sort());
  assert.doesNotMatch(exported, /^\s*use\s+(?:super|crate::commands|crate::scanning)::\s*\*/m);
  assert.doesNotMatch(exported, /\bfn\s+(?:asset_filter|asset_from_row|validate_asset_query)\s*\(|\bconst\s+ASSET_SELECT\b/);
  assert.match(assets, /pub\(super\)\s+fn\s+validate_asset_query\(/);
  const exportEntry = exported.slice(exported.indexOf('pub fn export_assets('), exported.indexOf('\n#[cfg(test)]'));
  const exportValidation = [...exportEntry.matchAll(/\bvalidate_asset_query\s*\(\s*&request\.query\s*\)\s*\?\s*;/g)];
  assert.equal(exportValidation.length, 1, 'export uses the shared fail-closed query validator');
  for (const effect of [/\bdb::open\s*\(/g, /\bfs::create_dir_all\s*\(/g, /\bstd::fs::OpenOptions::new\s*\(/g, /\b(?:file|writer)\s*\.\s*(?:write_all|write_record|flush)\s*\(/g]) {
    const matches = [...exportEntry.matchAll(effect)];
    assert.ok(matches.length > 0, `actual export boundary is present: ${effect}`);
    assert.ok(matches.every(match => match.index > exportValidation[0].index), `validate before database/output: ${effect}`);
  }
  assert.match(exportEntry, /\basset_filter\s*\(\s*&request\.query\s*,\s*true\s*\)/);
  assert.match(exportEntry, /format!\(\s*"\{ASSET_SELECT\} WHERE \{filter\}/);
  assert.match(exportEntry, /\.query_map\s*\(\s*params_from_iter\s*\(\s*values\.iter\s*\(\s*\)\s*\)\s*,\s*asset_from_row\s*\)/);
  const inventoryEntry = assets.slice(assets.indexOf('fn list_assets_on('), assets.indexOf('\nfn list_assets_on_read_snapshot('));
  assert.match(inventoryEntry, /->\s*Result<AssetPage,\s*String>\s*\{\s*validate_asset_query\s*\(\s*query\s*\)\s*\?\s*;/);
  const inventoryValidation = [...inventoryEntry.matchAll(/\bvalidate_asset_query\s*\(\s*query\s*\)\s*\?\s*;/g)];
  assert.equal(inventoryValidation.length, 1, 'inventory reads use the same owner validator');
  for (const readBoundary of [/\.unchecked_transaction\s*\(/g, /\blist_assets_on_read_snapshot\s*\(/g]) {
    const matches = [...inventoryEntry.matchAll(readBoundary)];
    assert.ok(matches.length > 0, `actual inventory read boundary is present: ${readBoundary}`);
    assert.ok(matches.every(match => match.index > inventoryValidation[0].index), 'inventory validates before transaction/snapshot reads');
  }
  assert.match(scanning, /#\[cfg\(test\)\]\s*use crate::commands::assets::test_support::asset_filter;/);
  assert.match(read('src-tauri/src/commands/asset_ownership.rs'), /asset_filter\(&query, true\)/);
});

test('Scanning fixtures use test-only owner adapters without widening production helpers', () => {
  const adapters = {
    assets: ['asset_filter', 'detect_target'],
    workspace_projects: ['delete_project_for_connection', 'project_impact_for_connection'],
    workspace_config_profiles: ['read_config_profiles', 'persist_config_profile', 'delete_config_profile_inner'],
    environment: ['ensure_environment_install_idle'],
  };
  for (const [owner, helpers] of Object.entries(adapters)) {
    const source = read(`src-tauri/src/commands/${owner}.rs`);
    const split = source.split('#[cfg(test)]\npub(crate) mod test_support {');
    assert.equal(split.length, 2, `${owner} has exactly one cfg(test) adapter module`);
    const [production, support] = split;
    assert.doesNotMatch(support, /(?:SELECT|INSERT|UPDATE|DELETE)\s/);
    assert.doesNotMatch(support, /crate::(?:models|scanning|agent_runtime)::|super::\*/);
    assert.deepEqual([...support.matchAll(/pub\(crate\) fn (\w+)\(/g)].map(match => match[1]).sort(), [...helpers].sort());
    assert.match(scanning, new RegExp(`#\\[cfg\\(test\\)\\]\\s*use crate::commands::${owner}::test_support::`));
    for (const helper of helpers) {
      assert.match(production, new RegExp(`pub\\(super\\) fn ${helper}\\(`));
      assert.match(support, new RegExp(`super::${helper}\\(`));
      assert.doesNotMatch(scanning, new RegExp(`use crate::commands::${owner}::(?:\\{[^}]*\\b)?${helper}\\b`));
    }
  }
});

test('FOFA account validation belongs to Assets with direct jobs and IPC consumers', () => {
  const source = read('src-tauri/src/assets/fofa.rs');
  const jobs = read('src-tauri/src/jobs.rs');
  assert.match(registration, /#\[path = "assets\/fofa\.rs"\]\s*mod asset_fofa;/);
  assert.match(source, /use crate::asset_models::\{FofaApiTestInput, FofaApiTestResult\};/);
  assert.doesNotMatch(source, /crate::(?:commands|scanning|agent_runtime|conversations|skill_packages|skills_catalog|models)::/);
  assert.doesNotMatch(source, /use\s+(?:super|crate::commands)::\s*\*/);
  assert.doesNotMatch(source, /include!\s*\(/);
  assert.deepEqual([...source.matchAll(/#\[tauri::command\]\s*pub (?:async )?fn (\w+)/g)]
    .map(match => match[1]), ['test_fofa_api']);
  const routes = [...registration.matchAll(/\b(?:[a-z_]+::)*test_fofa_api(?=,)/g)].map(match => match[0]);
  assert.deepEqual(routes, ['asset_fofa::test_fofa_api']);
  assert.doesNotMatch(read('src-tauri/src/commands/runtime_environment.rs'),
    /fn (?:fofa_transport_error|request_fofa_account|test_fofa_api)\(/);
  assert.doesNotMatch(legacy + '\n' + scanning, /fofa::|request_fofa_account|test_fofa_api/);
  assert.match(jobs, /crate::asset_fofa::request_fofa_account\(fofa_key\.trim\(\), fofa_proxy\.trim\(\)\)/);
  assert.doesNotMatch(jobs, /commands::|\bcommands\s*,/);
});

test('asset ownership compiles in its domain scope with explicit read consumers', () => {
  const source = read('src-tauri/src/commands/ownership.rs');
  assert.match(legacy, /pub\(crate\) mod ownership;/);
  assert.doesNotMatch(legacy, /include!\("commands\/(?:asset_ownership|ownership_\w+)\.rs"\)/);
  assert.doesNotMatch(source, /use\s+(?:super|crate::commands)::\s*\*/);
  const allowedScanTestImport = "#[cfg(test)]\nuse crate::scanning::ownership_test_support::authorization_control_setup_in;";
  assert.equal(source.split(allowedScanTestImport).length - 1, 1,
    'only the exact cfg(test) Ownership bridge import is allowed');
  const productionSource = source.replace(allowedScanTestImport, '');
  assert.doesNotMatch(productionSource, /crate::(?:conversations|skill_packages|skills_catalog|agent_runtime|scanning)::/);
  assert.match(scanning, /#\[cfg\(test\)\]\s*pub\(crate\) mod ownership_test_support/);
  const originalAuthorization = read('src-tauri/src/commands/agent_authorization_control.rs');
  assert.doesNotMatch(originalAuthorization, /pub(?:\([^)]*\))?\s+fn authorization_control_setup_in\(/);
  assert.match(source, /asset_models::\{/);
  assert.deepEqual([...source.matchAll(/include!\("([^"]+)"\)/g)].map(match => match[1]), [
    'asset_ownership.rs', 'ownership_directory.rs', 'ownership_public_suffix.rs',
    'ownership_review.rs', 'ownership_operations.rs', 'ownership_authorization.rs',
    'ownership_unit_projection.rs', 'ownership_o3_tests.rs',
  ]);
  const commands = [
    'get_asset_ownership_profile', 'save_asset_ownership_profile', 'get_asset_ownership_summary',
    'assess_asset_ownership', 'update_asset_ownership', 'get_project_ownership_bootstrap',
    'preview_project_ownership_suggestions', 'save_project_ownership_directory',
    'start_ownership_analysis', 'read_ownership_analysis', 'cancel_ownership_analysis',
    'page_ownership_groups', 'page_ownership_group_members', 'preview_ownership_group_decision',
    'apply_ownership_group_decision', 'read_ownership_operation', 'preview_ownership_group_revert',
    'apply_ownership_group_revert', 'revoke_ownership_group_authorization',
    'confirm_asset_ownership', 'authorize_asset_ownership',
  ];
  for (const command of commands) {
    assert.match(registration, new RegExp(`commands::ownership::${command},`));
    assert.doesNotMatch(registration, new RegExp(`commands::${command},`));
  }
  const projection = read('src-tauri/src/commands/exposure/units.rs');
  assert.match(projection, /crate::commands::ownership::ownership_asset_unit_projection/);
});

test('Chat resolves its model through snapshot-preserving infrastructure', () => {
  const source = read('src-tauri/src/conversations/reply.rs');
  assert.match(source, /crate::model_configuration::gateway_profile_on\(db, None\)/);
  assert.match(source, /map_err\(\|_\| "conversation_model_unavailable"\.to_string\(\)\)/);
  assert.doesNotMatch(source, /crate::(?:commands|scanning)::/);
  assert.doesNotMatch(legacy + '\n' + scanning, /fn conversation_model_profile\(/);
});

test('model configuration owns the only application resolver without business dependencies', () => {
  const source = read('src-tauri/src/model_configuration.rs');
  assert.match(registration, /mod model_configuration;/);
  assert.match(source, /ORDER BY is_default DESC,id LIMIT 1/);
  assert.match(source, /let environment = model_connection\(&settings\)\?;/);
  assert.match(source, /db::normalize_settings\(settings\)/);
  assert.doesNotMatch(source, /crate::(?:commands|conversations|skill_packages|scanning|assets)::/);
  assert.doesNotMatch(source, /ModelRuntimeEnv|full_power|agentLocalFullPower|prompt_audit_mode|agentPromptAuditMode|frontend_packet_budget_bytes|startup_idle_seconds|startup_hard_seconds/);
  assert.match(source, /Self\(db::normalize_settings\(settings\)\)/);
  assert.doesNotMatch(read('src-tauri/src/commands/runtime_environment.rs'), /fn model_connection\(/);
  assert.doesNotMatch(read('src-tauri/src/commands/scan_lifecycle.rs'), /fn sentinel_settings\(/);
  assert.match(read('src-tauri/src/commands/agent_model_client.rs'), /crate::model_configuration::gateway_profile\(&environment\.as_connection\(\), proxy\)/);
});

test('environment preparation shares pinned worker mechanisms without scan command imports', () => {
  const source = read('src-tauri/src/worker_resources.rs');
  assert.match(registration, /mod worker_resources;/);
  assert.match(source, /include_bytes!\("\.\.\/resources\/workers\/9_frontend_runtime_probe\.cjs"\)/);
  assert.doesNotMatch(source, /crate::(?:commands|conversations|skill_packages|scanning|assets)::/);
  assert.match(read('src-tauri/src/commands/environment.rs'), /worker_resources::\{resolve_frontend_recon_worker, verify_bundled_worker\}/);
  assert.match(read('src-tauri/src/commands/environment.rs'), /db::begin_environment_preparation\(&state.db_path\)/);
  assert.match(read('src-tauri/src/worker.rs'), /commands::environment::check_environment\(/);
});


test('exposure facade and collection kernel share one explicit Assets owner', () => {
  const source = read('src-tauri/src/commands/exposure.rs');
  assert.match(legacy, /^pub\(crate\) mod exposure;$/m);
  // Its children use exposure/, which an explicit path on this module changes.
  assert.doesNotMatch(legacy, /#\[path = "commands\/exposure\.rs"\]\s*pub\(crate\) mod exposure;/);
  assert.doesNotMatch(legacy, /include!\("commands\/exposure\.rs"\)/);
  assert.doesNotMatch(source, /pub mod exposure\s*\{/);
  assert.doesNotMatch(source, /use\s+(?:super|crate::commands)::\s*\*/);
  assert.doesNotMatch(source, /crate::models::|(?:^|[\s,{])models::\{/m);
  assert.match(source, /asset_models::\{/);
  assert.doesNotMatch(source, /include!\s*\(/);
  for (const core of ['scope', 'state', 'orchestrator', 'recovery', 'repository', 'units']) {
    assert.match(source, new RegExp(`^pub mod ${core};$`, 'm'));
  }
  const commands = [
    "page_exposure_findings",
    "page_exposure_runs",
    "list_exposure_source_results_for_project",
    "read_exposure_finding",
    "exposure_scope_summary",
    "exposure_known_limits",
    "exposure_summary",
    "list_exposure_findings",
    "save_exposure_finding",
    "list_exposure_runs",
    "list_exposure_source_results",
    "cancel_exposure_scan",
    "export_exposure_findings",
    "page_exposure_incidents",
    "read_exposure_incident",
    "page_exposure_incident_locations",
    "exposure_coverage_drilldown",
    "act_on_exposure_incident",
    "propose_exposure_incidents",
    "verify_exposure_export",
    "review_exposure_finding_scoped",
    "list_exposure_review_trail",
    "save_exposure_remediation",
    "page_exposure_remediations",
    "start_exposure_retest",
    "read_exposure_retest_outcome",
    "mark_exposure_remediation",
    "exposure_review_taxonomy",
    "start_exposure_scan",
    "resume_exposure_scan",
    "get_exposure_settings",
    "save_exposure_settings",
    "list_exposure_setting_versions",
    "page_exposure_pending_discoveries",
    "page_exposure_clusters",
    "page_exposure_cluster_members",
    "read_exposure_evidence",
    "import_exposure_documents",
    "reclassify_exposure_finding",
    "grade_exposure_intel",
  ];
  // Attributes on the existing coverage command must not hide it from this count.
  const exported = [...source.matchAll(/#\[tauri::command\]\s*(?:#\[[^\]]*\]\s*)*pub fn (\w+)/g)].map(match => match[1]);
  assert.equal(commands.length, 40);
  assert.deepEqual(exported.sort(), [...commands].sort());
  for (const command of commands) {
    const routes = [...registration.matchAll(new RegExp(`commands::(?:[a-z_]+::)?${command}(?=,)`, 'g'))];
    assert.deepEqual(routes.map(match => match[0]), [`commands::exposure::${command}`]);
  }
  for (const core of ['repository', 'classify']) {
    assert.doesNotMatch(read(`src-tauri/src/commands/exposure/${core}.rs`), /crate::commands::json\(/);
  }
});
