// Architectural regression guard: business commands must compile in their own
// Rust namespace and retain their existing IPC names after ownership changes.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const read = (relative) => fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');
const registration = read('src-tauri/src/lib.rs');
const legacy = read('src-tauri/src/commands.rs');
const owners = {
  workspace_projects: ['list_projects', 'save_project', 'project_impact', 'archive_project', 'delete_project'],
  application_overview: ['dashboard_stats'],
  application_settings: ['get_app_settings', 'get_app_icon_data_url', 'save_app_settings', 'save_app_icon', 'reset_app_icon'],
  workspace_config_profiles: ['list_config_profiles', 'save_config_profile', 'delete_config_profile'],
  assets: ['import_targets', 'list_targets', 'remove_target', 'list_assets', 'mark_asset_entry_opened',
    'add_content_rule', 'update_decision', 'update_asset_decisions', 'soft_delete_assets',
    'soft_delete_asset_selections', 'list_runs', 'list_logs', 'list_asset_events', 'startup_status',
    'acknowledge_interrupted_run'],
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
    assert.doesNotMatch(source, /(?:use\s+(?:crate|super)::|crate::)(?:conversations|skill_packages|commands::(?:agent|scan))/);
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
test('asset exports share the inventory predicate and ownership fixtures verify it', () => {
  assert.match(read('src-tauri/src/commands/asset_export.rs'), /use super::assets::\{asset_filter, asset_from_row, ASSET_SELECT\}/);
  assert.match(legacy, /#\[cfg\(test\)\]\s*use assets::asset_filter;/);
  assert.match(read('src-tauri/src/commands/asset_ownership.rs'), /asset_filter\(&query, true\)/);
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
  assert.doesNotMatch(legacy, /fofa::|request_fofa_account|test_fofa_api/);
  assert.match(jobs, /crate::asset_fofa::request_fofa_account\(fofa_key\.trim\(\), fofa_proxy\.trim\(\)\)/);
  assert.doesNotMatch(jobs, /commands::|\bcommands\s*,/);
});

test('asset ownership compiles in its domain scope with explicit read consumers', () => {
  const source = read('src-tauri/src/commands/ownership.rs');
  assert.match(legacy, /pub\(crate\) mod ownership;/);
  assert.doesNotMatch(legacy, /include!\("commands\/(?:asset_ownership|ownership_\w+)\.rs"\)/);
  assert.doesNotMatch(source, /use\s+(?:super|crate::commands)::\s*\*/);
  assert.doesNotMatch(source, /crate::(?:conversations|skill_packages|skills_catalog|agent_runtime|scanning)::/);
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
  assert.doesNotMatch(source, /crate::commands::/);
  assert.doesNotMatch(legacy, /fn conversation_model_profile\(/);
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
