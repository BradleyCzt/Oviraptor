const test = require('node:test');
const assert = require('node:assert/strict');
const { catalog, navigation } = require('./shell_module_harness.cjs');
const storage = seed => {
  const data = new Map(Object.entries(seed || {}));
  return { getItem: key => data.get(key) ?? null, setItem: (key, value) => data.set(key, value), removeItem: key => data.delete(key) };
};
test('fresh startup is independent Chat and malformed routes normalize safely', () => {
  assert.deepEqual(navigation.restoreRoute(storage()), { module: 'chat', view: 'chat' });
  for (const value of ['sandbox', 'agent-dialog', '__proto__', '', null])
    assert.deepEqual(navigation.normalizeRoute(value, value), { module: 'chat', view: 'chat' });
  assert.deepEqual(navigation.normalizeRoute('chat', 'logs'), { module: 'chat', view: 'chat' });
});
test('all saved business views keep their owner even when the module key is stale', () => {
  for (const module of catalog.MODULE_CATALOG) {
    for (const view of module.views)
      assert.deepEqual(navigation.normalizeRoute('chat', view), { module: module.id, view });
    for (const view of ['settings', 'projects'])
      assert.deepEqual(navigation.normalizeRoute(module.id, view), { module: module.id, view });
    if (module.hasLogs) assert.deepEqual(navigation.normalizeRoute(module.id, 'logs'), { module: module.id, view: 'logs' });
  }
  assert.deepEqual(navigation.normalizeRoute(null, 'logs'), { module: 'asset', view: 'logs' });
});
test('legacy navigation, project, theme and accent migrate idempotently without overwriting current settings', () => {
  const s = storage({ 'asset-atlas-module': 'sentinel', 'asset-atlas-view': 'sentinel', 'asset-atlas-project': '41', 'asset-atlas-theme': 'codex', 'asset-atlas-accent': '#112233', 'oviraptor-accent': '#abcdef' });
  const first = navigation.restoreRoute(s);
  assert.deepEqual(first, { module: 'sentinel', view: 'sentinel' });
  assert.deepEqual(navigation.restoreRoute(s), first);
  assert.equal(s.getItem('oviraptor-project'), '41');
  assert.equal(s.getItem('oviraptor-theme'), 'codex');
  assert.equal(s.getItem('oviraptor-accent'), '#abcdef');
  for (const suffix of ['module', 'view', 'project', 'theme', 'accent']) assert.equal(s.getItem(`asset-atlas-${suffix}`), null);
});
test('project IDs reject unsafe, negative and fractional selections', () => {
  for (const value of ['-1', 'NaN', 'Infinity', 1.5, 0, null, true, '9007199254740992']) assert.equal(navigation.parseProjectId(value), undefined);
  assert.equal(navigation.parseProjectId('41'), 41);
});
test('tray routes and scanning navigation preserve existing investigation entry points', () => {
  assert.deepEqual(navigation.trayRoute('assets'), { module: 'asset', view: 'assets' });
  assert.deepEqual(navigation.trayRoute('agent-tasks'), { module: 'sentinel', view: 'sentinel', section: 'queue' });
  assert.equal(navigation.trayRoute('unknown'), undefined);
  assert.deepEqual(catalog.workspaceNavigation('sentinel').map(item => item.key), ['dialog', 'urls', 'queue', 'skills']);
  assert.equal(catalog.workspaceNavigation('chat').length, 0);
  assert.ok(catalog.workspaceNavigation('asset').some(item => item.view === 'exposure'));
});
test('HackerOne remains available when configured and runtime services never become product routes', () => {
  assert.deepEqual(catalog.visibleModules(false).map(item => item.id), ['chat', 'asset', 'sentinel']);
  assert.deepEqual(catalog.visibleModules(true).map(item => item.id), ['chat', 'asset', 'sentinel', 'hackerone']);
  assert.equal(catalog.MODULE_CATALOG.find(item => item.id === 'sandbox'), undefined);
});
