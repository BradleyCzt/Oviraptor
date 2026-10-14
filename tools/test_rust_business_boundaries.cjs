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
};
for (const [owner, commands] of Object.entries(owners)) {
  test(`${owner} has an explicit Rust boundary and stable IPC registration`, () => {
    const source = read(`src-tauri/src/commands/${owner}.rs`);
    assert.match(legacy, new RegExp(`mod ${owner};`));
    assert.ok(!legacy.includes(`include!("commands/${owner}.rs")`));
    // A wildcard of the parent silently restores all cross-business coupling.
    assert.doesNotMatch(source, /use\s+(?:super|crate::commands)::\s*\*/);
    assert.doesNotMatch(source, /include!\s*\(/);
    assert.doesNotMatch(source, /(?:use\s+(?:crate|super)::|crate::)(?:conversations|skill_packages|agent_runtime|commands::(?:agent|scan))/);
    const exported = [...source.matchAll(/#\[tauri::command\]\s*pub (?:async )?fn (\w+)/g)].map(match => match[1]);
    assert.deepEqual(exported.sort(), [...commands].sort());
    for (const command of commands) {
      const routes = [...registration.matchAll(new RegExp(`commands::(?:[a-z_]+::)?${command}(?=,)`, 'g'))];
      assert.deepEqual(routes.map(match => match[0]), [`commands::${owner}::${command}`]);
    }
  });
}
test('asset exports and ownership share the asset selection predicate explicitly', () => {
  assert.match(read('src-tauri/src/commands/asset_export.rs'), /use super::assets::\{asset_filter, asset_from_row, ASSET_SELECT\}/);
  assert.match(legacy, /use assets::asset_filter;/);
  assert.match(read('src-tauri/src/commands/asset_ownership.rs'), /asset_filter\(&query, true\)/);
});
