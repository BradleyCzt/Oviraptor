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
};
for (const [owner, commands] of Object.entries(owners)) {
  test(`${owner} has an explicit Rust boundary and stable IPC registration`, () => {
    const source = read(`src-tauri/src/commands/${owner}.rs`);
    assert.match(legacy, new RegExp(`mod ${owner};`));
    assert.ok(!legacy.includes(`include!("commands/${owner}.rs")`));
    // A wildcard of the parent silently restores all cross-business coupling.
    assert.doesNotMatch(source, /use\s+(?:super|crate::commands)::\s*\*/);
    assert.doesNotMatch(source, /include!\s*\(/);
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
