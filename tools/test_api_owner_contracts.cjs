const test = require('node:test');
const assert = require('node:assert/strict');
const { loadApiOwner } = require('./api_owner_harness.cjs');

test('environment owns exact install log cursors, profile scope and explicit recovery acknowledgement', async () => {
  const calls = [], marker = { synthetic: true };
  const { environmentApi } = loadApiOwner('features/environment/api.ts', async (...args) => { calls.push(args); return marker; });
  assert.equal(await environmentApi.checkEnvironment(17), marker);
  await environmentApi.installEnvironmentDependencies(17);
  await environmentApi.listEnvironmentInstallLogs(91, 40);
  await environmentApi.getEnvironmentPreparationStatus();
  await environmentApi.recoverEnvironmentPreparation('original-owner', 'explicit-review');
  assert.deepEqual(calls, [
    ['check_environment', { profileId: 17 }],
    ['install_environment_dependencies', { profileId: 17 }],
    ['list_environment_install_logs', { afterId: 91, limit: 40 }],
    ['get_environment_preparation_status'],
    ['recover_environment_preparation', { expectedOwner: 'original-owner', acknowledgement: 'explicit-review' }],
  ]);
});

test('remote scan adapter retains node and scan identity without changing control or history-sync IPC', async () => {
  const calls = [];
  const { remoteScanningApi } = loadApiOwner('features/sentinel/remoteWorkerApi.ts', async (...args) => calls.push(args));
  const input = { nodeId: 7, scanId: 'original-scan', action: 'resume' };
  await remoteScanningApi.listRemoteWorkerScans(7);
  await remoteScanningApi.controlRemoteWorkerScan(input);
  await remoteScanningApi.syncWorkerNode(7);
  assert.deepEqual(calls, [
    ['list_remote_worker_scans', { nodeId: 7 }],
    ['control_remote_worker_scan', { input }],
    ['sync_worker_node', { nodeId: 7 }],
  ]);
});

test('shell aggregate and startup reads remain separate from project lifecycle and asset recovery', async () => {
  const calls = [], invoke = async (...args) => calls.push(args);
  const { shellApi } = loadApiOwner('shell/api.ts', invoke);
  const { workspaceApi } = loadApiOwner('features/workspaces/api.ts', invoke);
  const { assetApi } = loadApiOwner('features/assets/api.ts', invoke);
  await shellApi.dashboardStats(4); await shellApi.startupStatus();
  await workspaceApi.projectImpact(4); await workspaceApi.archiveProject(4, true);
  await assetApi.acknowledgeInterruptedRun(81);
  assert.deepEqual(calls, [
    ['dashboard_stats', { projectId: 4 }], ['startup_status'],
    ['project_impact', { projectId: 4 }], ['archive_project', { projectId: 4, archived: true }],
    ['acknowledge_interrupted_run', { runId: 81 }],
  ]);
});

test('configuration round trips opaque existing fields and preserves provider-test arguments', async () => {
  const calls = [];
  const { configurationApi } = loadApiOwner('features/configuration/api.ts', async (...args) => calls.push(args));
  const input = { id: 9, name: 'Existing profile', description: '', isDefault: true,
    settings: { opaqueLegacyField: { keep: true }, agentQuickTimeout: 913, modelProfiles: [] } };
  const model = { llm: 'synthetic', deployment: 'cloud', apiBase: 'https://provider.invalid/v1', apiKey: 'synthetic-only' };
  await configurationApi.saveProfile(input); await configurationApi.testModelProfile(model);
  await configurationApi.testFofaApi({ key: 'synthetic-only', proxyUrl: 'https://proxy.invalid' });
  assert.deepEqual(calls, [
    ['save_config_profile', { input }], ['test_model_profile', { input: model }],
    ['test_fofa_api', { input: { key: 'synthetic-only', proxyUrl: 'https://proxy.invalid' } }],
  ]);
});

test('public API compatibility still exposes commands moved to their owners', async () => {
  const calls = [];
  const { api } = loadApiOwner('api.ts', async (...args) => calls.push(args));
  await api.dashboardStats(2); await api.listEnvironmentInstallLogs(undefined, 300);
  await api.listRemoteWorkerScans(3); await api.listProfiles();
  await api.acknowledgeInterruptedRun(8); await api.generateAgentLearningCandidate('scan-a');
  await api.listSecurityRulePacks();
  assert.deepEqual(calls, [
    ['dashboard_stats', { projectId: 2 }], ['list_environment_install_logs', { afterId: undefined, limit: 300 }],
    ['list_remote_worker_scans', { nodeId: 3 }], ['list_config_profiles'],
    ['acknowledge_interrupted_run', { runId: 8 }], ['generate_agent_learning_candidate', { scanId: 'scan-a' }],
    ['list_security_rule_packs'],
  ]);
});
