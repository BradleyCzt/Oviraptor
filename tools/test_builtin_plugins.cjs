const test = require('node:test');
const assert = require('node:assert/strict');
const plugin = require('./plugin_registry_harness.cjs');
const store = (seed = {}) => {
  const values = new Map(Object.entries(seed));
  const writes = [];
  return { values, writes, getItem: key => values.get(key) ?? null,
    setItem(key, value) { writes.push(key); values.set(key, value); } };
};
function fixture(overrides = {}) {
  const state = { enabled: plugin.defaultPluginEnabledState(), project: 1, projects: [{ id: 1, status: 'active' }, { id: 2, status: 'active' }] };
  const calls = [], navigation = [];
  const host = plugin.createBuiltinPluginHost({
    enabled: () => state.enabled, projectId: () => state.project, projects: () => state.projects,
    navigate: intent => navigation.push(intent),
    api: {
      async listAssets(query) { calls.push(['assets', query]); return assetPage([asset()], query); },
      async searchSentinelScanPage(...args) { calls.push(['scans', ...args]); return [scan()]; },
      ...overrides,
    },
  });
  return { host, state, calls, navigation };
}
const asset = (change = {}) => ({ id: 7, projectId: 1, isDeleted: false, title: 'Workspace app', host: 'example.test', link: 'https://example.test', assetKey: 'key', extra: { secret: 'do not project' }, ...change });
const scan = (change = {}) => ({ id: 'scan-1', projectId: 1, taskName: 'Source review', scanType: 'code', status: 'completed', updatedAt: '2026-10-13 00:00:00', taskPath: '/private/evidence', ...change });
const assetPage = (items, query = { page: 1, pageSize: 50 }, total = items.length) => ({ items, total, page: query.page, pageSize: query.pageSize, summary: {} });
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const assetsInput = { pluginId: 'asset', projectId: 1 };
const scansInput = { pluginId: 'sentinel', projectId: 1 };

test('trusted bundled registry keeps Chat in host and declares only implemented read/navigation commands', () => {
  assert.deepEqual(plugin.BUILTIN_PLUGINS.map(item => item.id), ['asset', 'sentinel', 'hackerone']);
  assert.ok(!plugin.isBuiltinPluginId('chat'));
  for (const id of ['computer', 'sandbox', 'https://example.test/plugin', '__proto__', null]) assert.equal(plugin.isBuiltinPluginId(id), false);
  for (const item of plugin.BUILTIN_PLUGINS) {
    assert.equal(item.label.length, 2); assert.equal(item.description.length, 2);
    assert.ok(Object.isFrozen(item)); assert.ok(Object.isFrozen(item.pages));
    assert.ok(item.views.includes(item.defaultView));
    for (const command of item.commands) {
      assert.ok(['read', 'navigate'].includes(command.kind));
      assert.equal(command.requiresModel, false); assert.equal(command.requiresNetwork, false);
      assert.equal(command.scope, 'selected-project');
    }
    for (const capability of item.capabilities) {
      assert.equal(capability.hostCallable, false);
      assert.equal(capability.authority, 'existing-domain-gates');
    }
  }
  assert.equal(plugin.getBuiltinPlugin('sentinel').capabilities.find(item => item.id === 'scanning.native-investigation').requiresModel, true);
  assert.equal(plugin.getBuiltinPlugin('asset').capabilities.find(item => item.id === 'assets.fofa-collection').requiresModel, false);
  assert.deepEqual(plugin.getBuiltinPlugin('hackerone').requirements, ['hackerone-profile']);
});
test('all Assets and Scanning legacy pages have explicit contributions', () => {
  assert.deepEqual(plugin.getBuiltinPlugin('asset').pages.map(item => item.view), ['dashboard', 'tasks', 'assets', 'ownership', 'exposure', 'changes', 'quarantine', 'query', 'logs']);
  assert.deepEqual(plugin.getBuiltinPlugin('sentinel').pages.map(item => item.id), ['dialog', 'urls', 'queue', 'skills', 'logs']);
  assert.throws(() => plugin.getBuiltinPlugin('unknown'), /plugin_unknown/);
});
test('preferences default on and load never writes or changes application profiles', () => {
  const storage = store({ 'oviraptor-project': '12', 'oviraptor-view': 'assets', 'config-profiles': 'unchanged' });
  assert.deepEqual(plugin.loadPluginEnabledState(storage), { asset: true, sentinel: true, hackerone: true });
  assert.deepEqual(storage.writes, []);
  plugin.savePluginEnabledState(storage, { asset: false, sentinel: true, hackerone: false });
  assert.deepEqual(storage.writes, [plugin.PLUGIN_PREFERENCE_KEY]);
  assert.equal(storage.values.get('config-profiles'), 'unchanged');
  assert.equal(storage.values.get('oviraptor-project'), '12');
  assert.equal(storage.values.get('oviraptor-view'), 'assets');
  assert.deepEqual(plugin.loadPluginEnabledState(storage), { asset: false, sentinel: true, hackerone: false });
});
test('preference edits preserve unknown inert values without accepting them as plugins', () => {
  const storage = store({ [plugin.PLUGIN_PREFERENCE_KEY]: JSON.stringify({ version: 1, futureMetadata: 'keep', enabled: { asset: false, sentinel: true, hackerone: true, futurePlugin: false } }) });
  plugin.savePluginEnabledState(storage, { asset: true, sentinel: false, hackerone: true });
  const result = JSON.parse(storage.getItem(plugin.PLUGIN_PREFERENCE_KEY));
  assert.equal(result.futureMetadata, 'keep'); assert.equal(result.enabled.futurePlugin, false);
  assert.equal(plugin.isPluginEnabled(result.enabled, 'futurePlugin'), false);
});
test('corrupt and future preference records have visible notices and cannot be silently overwritten', () => {
  for (const value of ['{broken', '[]', JSON.stringify({ version: 2, enabled: { asset: false } })]) {
    const storage = store({ [plugin.PLUGIN_PREFERENCE_KEY]: value });
    assert.deepEqual(plugin.loadPluginEnabledState(storage), plugin.defaultPluginEnabledState());
    assert.match(plugin.getPluginPreferenceNotice(storage), /damaged|unsupported/);
    assert.throws(() => plugin.savePluginEnabledState(storage, plugin.defaultPluginEnabledState()), /preserved/);
    assert.equal(storage.getItem(plugin.PLUGIN_PREFERENCE_KEY), value); assert.equal(storage.writes.length, 0);
  }
});
test('invalid individual preference values fall back without becoming execution authority', () => {
  const storage = store({ [plugin.PLUGIN_PREFERENCE_KEY]: JSON.stringify({ version: 1, enabled: { asset: false, sentinel: 'approved', hackerone: true } }) });
  assert.deepEqual(plugin.loadPluginEnabledState(storage), { asset: false, sentinel: true, hackerone: true });
  assert.match(plugin.getPluginPreferenceNotice(storage), /invalid/);
  assert.throws(() => plugin.savePluginEnabledState(storage, { asset: false, sentinel: 'yes', hackerone: true }), /invalid/);
});
test('asset candidate reads have explicit bounded Project scope and minimal DTO projection', async () => {
  const f = fixture();
  const result = await f.host.listLinkCandidates({ ...assetsInput, search: ' test ' });
  assert.deepEqual(f.calls, [['assets', { projectId: 1, search: 'test', conditions: [], page: 1, pageSize: 50, includeDeleted: false, deletedView: 'active' }]]);
  assert.deepEqual(result, { items: [{ kind: 'asset', targetId: '7', projectId: 1, label: 'Workspace app', detail: 'example.test' }] });
  assert.equal(JSON.stringify(result).includes('secret'), false);
  assert.deepEqual(f.navigation, []);
});
test('scan candidate search uses existing scoped query and never returns private evidence paths', async () => {
  const f = fixture();
  const result = await f.host.listLinkCandidates({ ...scansInput, search: 'source' });
  assert.deepEqual(f.calls, [['scans', 1, 'source', 'all', 50, undefined]]);
  assert.deepEqual(result.items, [{ kind: 'scan', targetId: 'scan-1', projectId: 1, label: 'Source review', detail: 'code · completed' }]);
  assert.equal(JSON.stringify(result).includes('/private'), false);
});
test('missing, wrong, unsafe and disabled Project scope causes zero provider reads', async () => {
  for (const projectId of [undefined, 0, -1, 1.5, 2, Number.MAX_SAFE_INTEGER + 1]) {
    const f = fixture();
    await assert.rejects(f.host.listLinkCandidates({ ...assetsInput, projectId }), /plugin_/);
    assert.equal(f.calls.length, 0);
  }
  const f = fixture(); f.state.enabled = { ...f.state.enabled, asset: false };
  await assert.rejects(f.host.listLinkCandidates(assetsInput), /plugin_disabled/);
  f.state.enabled = plugin.defaultPluginEnabledState(); f.state.projects = [];
  await assert.rejects(f.host.listLinkCandidates(assetsInput), /plugin_project_unavailable/);
  assert.equal(f.calls.length, 0);
});
test('unknown command-shaped inputs and arbitrary navigation payloads have no call path', async () => {
  const f = fixture();
  assert.equal(f.host.invoke, undefined); assert.equal(f.host.execute, undefined);
  for (const input of [{ ...assetsInput, command: 'start_sentinel_scan' }, { ...assetsInput, pluginId: 'hackerone' }, { ...assetsInput, search: 'x'.repeat(501) }]) await assert.rejects(f.host.listLinkCandidates(input), /plugin_input_invalid/);
  for (const intent of [{ pluginId: 'asset', projectId: 1, page: 'assets', assetId: 7 }, { pluginId: 'sentinel', projectId: 1, page: 'sentinel', section: 'execute' }, { pluginId: 'computer', projectId: 1, page: 'terminal' }]) assert.throws(() => f.host.openWorkspace(intent), /plugin_navigation_invalid/);
  assert.equal(f.calls.length, 0); assert.equal(f.navigation.length, 0);
});
test('candidate DTOs reject cross-project, deleted, missing or duplicate ownership facts', async () => {
  for (const rows of [[asset({ projectId: 2 })], [asset({ projectId: undefined })], [asset({ isDeleted: true })], [asset({ id: -1 })], [asset(), asset()]]) {
    const f = fixture({ listAssets: async query => assetPage(rows, query) });
    await assert.rejects(f.host.listLinkCandidates(assetsInput), /plugin_response_scope_invalid/);
  }
  for (const rows of [[scan({ projectId: 2 })], [scan({ projectId: undefined })], [scan({ updatedAt: '' })], [scan(), scan()]]) {
    const f = fixture({ searchSentinelScanPage: async () => rows });
    await assert.rejects(f.host.listLinkCandidates(scansInput), /plugin_response_scope_invalid/);
  }
});
test('malformed or excessive read pages fail rather than claiming full coverage', async () => {
  for (const value of [null, assetPage([], { page: 2, pageSize: 50 }), assetPage([], { page: 1, pageSize: 300 }), { ...assetPage([]), total: -1 }, assetPage(Array.from({ length: 51 }, (_, i) => asset({ id: i + 1 })))]) {
    const f = fixture({ listAssets: async () => value });
    await assert.rejects(f.host.listLinkCandidates(assetsInput), /plugin_response_invalid/);
  }
  const f = fixture({ searchSentinelScanPage: async () => Array(51).fill(scan()) });
  await assert.rejects(f.host.listLinkCandidates(scansInput), /plugin_response_invalid/);
});
test('pagination cursors bind plugin, Project and normalized search and stay bounded', async () => {
  const f = fixture({ listAssets: async query => assetPage([asset()], query, 150) });
  const first = await f.host.listLinkCandidates({ ...assetsInput, search: 'x' });
  assert.deepEqual(first.nextCursor, { pluginId: 'asset', projectId: 1, search: 'x', page: 2 });
  const next = await f.host.listLinkCandidates({ ...assetsInput, search: ' x ', cursor: first.nextCursor });
  assert.equal(next.nextCursor.page, 3);
  for (const cursor of [{ ...first.nextCursor, projectId: 2 }, { ...first.nextCursor, search: 'y' }, { ...first.nextCursor, page: 0 }, { ...first.nextCursor, page: 1000001 }]) await assert.rejects(f.host.listLinkCandidates({ ...assetsInput, search: 'x', cursor }), /plugin_cursor/);
});
test('scan cursor preserves exact existing backend tuple and rejects lack of progress', async () => {
  const rows = Array.from({ length: 50 }, (_, i) => scan({ id: `scan-${i}` }));
  const f = fixture({ searchSentinelScanPage: async () => rows });
  const first = await f.host.listLinkCandidates(scansInput);
  assert.deepEqual(first.nextCursor, { pluginId: 'sentinel', projectId: 1, search: '', before: { id: 'scan-49', updatedAt: rows[49].updatedAt } });
  await assert.rejects(f.host.listLinkCandidates({ ...scansInput, cursor: first.nextCursor }), /plugin_cursor_not_advanced/);
});
test('Project switch rejects late read results; invalidation also rejects A to B to A', async () => {
  for (const returnToA of [false, true]) {
    const d = deferred(), f = fixture({ listAssets: () => d.promise });
    const pending = f.host.listLinkCandidates(assetsInput);
    f.state.project = 2; f.host.invalidate();
    if (returnToA) { f.state.project = 1; f.host.invalidate(); }
    d.resolve(assetPage([asset()]));
    await assert.rejects(pending, /plugin_scope_changed/);
  }
});
test('disable/re-enable invalidation cannot revive a pending candidate read', async () => {
  const d = deferred(), f = fixture({ searchSentinelScanPage: () => d.promise });
  const pending = f.host.listLinkCandidates(scansInput);
  f.state.enabled = { ...f.state.enabled, sentinel: false }; f.host.invalidate();
  f.state.enabled = { ...f.state.enabled, sentinel: true }; f.host.invalidate();
  d.resolve([scan()]);
  await assert.rejects(pending, /plugin_scope_changed/);
  assert.equal(f.navigation.length, 0);
});
test('mutable caller input cannot retarget an in-flight read after dispatch', async () => {
  const d = deferred(), f = fixture({ listAssets: () => d.promise });
  const input = { ...assetsInput, search: 'original' };
  const pending = f.host.listLinkCandidates(input);
  input.projectId = 2; input.search = 'changed'; input.pluginId = 'sentinel';
  d.resolve(assetPage([asset()]));
  assert.equal((await pending).items[0].projectId, 1);
});
test('archived Project history remains readable but still has no execution adapter', async () => {
  const f = fixture(); f.state.projects[0].status = 'archived';
  assert.equal((await f.host.listLinkCandidates(assetsInput)).items.length, 1);
  f.host.openWorkspace({ pluginId: 'sentinel', projectId: 1, page: 'sentinel', section: 'results' });
  assert.equal(f.navigation.length, 1); assert.equal(f.host.startScan, undefined);
});
test('explicit workspace transitions are synchronous, scoped, and never queue for re-enable', () => {
  const f = fixture(); const intent = { pluginId: 'asset', projectId: 1, page: 'assets' };
  f.host.openWorkspace(intent); intent.projectId = 2;
  assert.deepEqual(f.navigation, [{ pluginId: 'asset', projectId: 1, page: 'assets' }]);
  f.state.enabled = { ...f.state.enabled, asset: false }; f.host.invalidate();
  assert.throws(() => f.host.openWorkspace({ ...intent, projectId: 1 }), /plugin_disabled/);
  f.state.enabled = { ...f.state.enabled, asset: true }; f.host.invalidate();
  assert.equal(f.navigation.length, 1);
});
test('backend error detail is not projected into plugin candidates or UI errors', async () => {
  const f = fixture({ listAssets: async () => { throw Error('secret provider token'); } });
  await assert.rejects(f.host.listLinkCandidates(assetsInput), error => error.message === 'plugin_read_failed');
});
