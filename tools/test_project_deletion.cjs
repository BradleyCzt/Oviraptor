// Execute App.vue setup and render its real template/InlineConfirm. Desktop
// startup, unrelated panels, IPC and timers are isolated; no external IO occurs.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const vue = require('vue');
const { catalog, navigation } = require('./shell_module_harness.cjs');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { renderToString } = require('@vue/server-renderer');
const transpile = (content) => ts.transpileModule(content, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
function compile(relative, id) {
  const filename = path.join(__dirname, relative);
  const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
  const script = compileScript(descriptor, { id });
  const template = compileTemplate({ source: descriptor.template.content, filename, id,
    compilerOptions: { bindingMetadata: script.bindings } });
  assert.deepEqual(template.errors, []);
  const exports = {};
  new Function('require', 'exports', transpile(template.code))(require, exports);
  return { script: transpile(script.content), render: exports.render };
}
const appSource = compile('../src/App.vue', 'project-delete-app');
const inlineSource = compile('../src/components/InlineConfirm.vue', 'project-delete-confirm');
const inlineExports = {};
new Function('require', 'exports', inlineSource.script)(
  (name) => name === 'vue' ? vue : { AlertTriangle: () => null }, inlineExports,
);
const InlineConfirm = { ...inlineExports.default, render: inlineSource.render };
const renderer = vue.createRenderer({
  createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
  insert() {}, remove() {}, setText() {}, setElementText() {}, patchProp() {},
  parentNode: () => null, nextSibling: () => null,
});
const project = (id = 91) => ({ id, name: `Workspace ${id}`, description: '', status: 'active',
  assetCount: 0, scanCount: 0, vulnerabilityCount: 0, activeFuseCount: 0 });
const impact = (overrides = {}) => ({ assetCount: 0, assetEventCount: 0, targetCount: 0,
  assetRunCount: 0, savedViewCount: 0, sentinelScanCount: 0, sentinelTargetCount: 0,
  findingCount: 0, validationCount: 0, opportunityCount: 0, fuseCount: 0,
  appsecVulnerabilityCount: 0, knowledgeCount: 0, learningCandidateCount: 0,
  browserAuthSessionCount: 0, otherRecordCount: 0, totalRecords: 0, ...overrides });
function mount(overrides = {}) {
  const calls = [];
  const defaults = { projectImpact: async () => impact(), deleteProject: async () => {},
    archiveProject: async () => {}, listProjects: async () => [project()], listProfiles: async () => [],
    dashboardStats: async () => ({}), getAppSettings: async () => ({}),
    listRuns: async () => [], listEvents: async () => [] };
  const api = new Proxy({}, { get(_, name) {
    const fn = overrides[name] || defaults[name];
    if (!fn) throw new Error(`Unexpected IPC ${String(name)}`);
    return (...args) => { calls.push([name, ...args]); return fn(...args); };
  } });
  const storage = { getItem: () => null, setItem() {}, removeItem() {} };
  const i18n = {};
  new Function('require', 'exports', 'localStorage', transpile(
    fs.readFileSync(path.join(__dirname, '../src/i18n.ts'), 'utf8'),
  ))(require, i18n, storage);
  const exports = {};
  new Function('require', 'exports', 'localStorage', 'setTimeout', appSource.script)((name) => {
    if (name === 'vue') return { ...vue, onMounted() {} };
    if (name === './api') return { api };
    if (name === './shell/moduleCatalog') return catalog;
    if (name === './shell/navigation') return navigation;
    if (name === './i18n') return i18n;
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    if (name === '@tauri-apps/api/event') return { listen: async () => () => {} };
    if (name === './features/sentinel/presentation') return { humanizeScanCheckpoint: (v) => v };
    // Log lifecycle is covered by test_runner_log_panel; keep deletion isolated.
    if (name === './features/sentinel/execution/useRunnerLogPanel') return { useRunnerLogPanel: () => ({
      snapshot: vue.ref(), loading: vue.ref(false), readFailed: vue.ref(false),
      connectionUnavailable: vue.ref(false), refresh: async () => {},
    }) };
    if (name === './features/assets/useAssetLogPanel') return { useAssetLogPanel: () => ({
      logs: vue.ref([]), loading: vue.ref(false), readFailed: vue.ref(false),
      connectionUnavailable: vue.ref(false), refresh: async () => {},
    }) };
    if (name === './features/environment/useInstallLogPanel') return { useInstallLogPanel: () => ({
      logs: vue.ref([]), evicted: vue.ref(0), connectionUnavailable: vue.ref(false),
      gapPossible: vue.ref(false), clear() {}, append() {},
    }) };
    if (name === './components/InlineConfirm.vue') return { default: InlineConfirm };
    if (name.endsWith('.vue')) return { default: () => null };
    if (name.endsWith('.css')) return {};
    if (name.endsWith('.png')) return { default: '' };
    if (name === '../package.json') return { default: { version: 'fixture' } };
    throw new Error(`Unexpected import ${name}`);
  }, exports, storage, () => 1);
  let bindings;
  const setup = exports.default.setup;
  const component = { ...exports.default, setup(props, ctx) { bindings = setup(props, ctx); return () => null; } };
  const app = renderer.createApp(component);
  app.mount({});
  bindings.loading.value = false;
  bindings.projects.value = [project(), project(92)];
  bindings.activeView.value = 'projects';
  return { b: bindings, calls, unmount: () => app.unmount(),
    render: () => renderToString(vue.createSSRApp({ render: () => appSource.render({}, [], {}, vue.proxyRefs(bindings), {}, {}) })),
  };
}

test('actual workspace confirmation shows supplementary records and only archives', async () => {
  const m = mount({ projectImpact: async () => impact({ otherRecordCount: 2,
    sentinelTargetCount: 1, opportunityCount: 3, totalRecords: 6 }) });
  try {
    await m.b.removeProject(project());
    assert.equal(m.b.pendingProjectAction.value.mode, 'archive');
    const html = await m.render();
    for (const text of ['归档并保留历史', '2 条其他关联记录', '1 个 Nest 目标', '3 条测试机会'])
      assert.ok(html.includes(text), text);
    await m.b.confirmProjectAction();
    assert.deepEqual(m.calls.filter(([n]) => ['deleteProject', 'archiveProject'].includes(n)), [['archiveProject', 91, true]]);
  } finally { m.unmount(); }
});

test('empty preview may request deletion but backend rejection retains the confirmation and data', async () => {
  const m = mount({ deleteProject: async () => { throw Error('关联记录已变化，请归档'); } });
  try {
    await m.b.removeProject(project());
    assert.equal(m.b.pendingProjectAction.value.mode, 'delete');
    assert.ok((await m.render()).includes('删除这个空工作空间'));
    await m.b.confirmProjectAction();
    assert.equal(m.b.pendingProjectAction.value.project.id, 91);
    assert.equal(m.b.projects.value.length, 2);
    assert.equal(m.b.deletingProjectId.value, undefined);
    assert.ok(m.b.toasts.value.some((item) => item.type === 'error' && item.text.includes('请归档')));
    assert.ok(!m.b.toasts.value.some((item) => item.type === 'success'));
  } finally { m.unmount(); }
});

test('impact preview in flight cannot switch target or confirm an older selection', async () => {
  let resolve;
  const m = mount({ projectImpact: () => new Promise((done) => { resolve = done; }) });
  try {
    m.b.pendingProjectAction.value = { project: project(92), impact: impact(), mode: 'delete' };
    const operation = m.b.removeProject(project());
    assert.equal(m.b.projectImpactLoadingId.value, 91);
    assert.equal(m.b.pendingProjectAction.value, undefined);
    await m.b.removeProject(project(92));
    await m.b.confirmProjectAction();
    assert.deepEqual(m.calls, [['projectImpact', 91]]);
    resolve(impact());
    await operation;
    assert.equal(m.b.pendingProjectAction.value.project.id, 91);
    assert.equal(m.b.projectImpactLoadingId.value, undefined);
  } finally { resolve?.(impact()); m.unmount(); }
});

test('in-flight deletion cannot be cancelled, duplicated or retargeted', async () => {
  let resolve;
  const m = mount({ deleteProject: () => new Promise((done) => { resolve = done; }) });
  try {
    await m.b.removeProject(project());
    const operation = m.b.confirmProjectAction();
    m.b.cancelProjectAction();
    await m.b.removeProject(project(92));
    await m.b.confirmProjectAction();
    assert.equal(m.b.pendingProjectAction.value.project.id, 91);
    assert.deepEqual(m.calls.filter(([n]) => n === 'deleteProject'), [['deleteProject', 91]]);
    resolve();
    await operation;
    assert.equal(m.b.pendingProjectAction.value, undefined);
    assert.equal(m.b.deletingProjectId.value, undefined);
    assert.ok(m.b.toasts.value.some((item) => item.text.includes('空工作空间已删除')));
  } finally { resolve?.(); m.unmount(); }
});

test('failed impact lookup clears stale confirmation and permits a fresh lookup', async () => {
  let fail = true;
  const m = mount({ projectImpact: async () => { if (fail) throw Error('read failed'); return impact(); } });
  try {
    m.b.pendingProjectAction.value = { project: project(92), impact: impact(), mode: 'delete' };
    await m.b.removeProject(project());
    assert.equal(m.b.pendingProjectAction.value, undefined);
    assert.equal(m.b.projectImpactLoadingId.value, undefined);
    await m.b.confirmProjectAction();
    assert.ok(!m.calls.some(([n]) => n === 'deleteProject'));
    fail = false;
    await m.b.removeProject(project());
    assert.equal(m.b.pendingProjectAction.value.project.id, 91);
    m.b.cancelProjectAction();
    assert.equal(m.b.pendingProjectAction.value, undefined);
  } finally { m.unmount(); }
});

// Shell extraction regressions execute the real App setup as well as the pure
// route tests; business views, tray targets and project refreshes stay wired.
const flushShell = async () => { for (let i=0;i<30;i++) await vue.nextTick(); };
const shellDeferred = () => { let resolve; const promise = new Promise(done => { resolve=done; }); return {promise,resolve}; };
test('actual shell routes asset actions from Chat and preserves scanning tray targets', async () => {
  const m=mount();
  try {
    m.b.activeModule.value='chat';m.b.activeView.value='chat';
    m.b.navigate('tasks');await flushShell();
    assert.equal(m.b.activeModule.value,'asset');assert.equal(m.b.activeView.value,'tasks');
    m.b.navigateFromTray('agent-tasks');
    assert.equal(m.b.activeModule.value,'sentinel');assert.equal(m.b.activeView.value,'sentinel');assert.equal(m.b.sentinelSection.value,'queue');
    m.b.navigate('settings');assert.equal(m.b.activeModule.value,'sentinel');
    m.b.switchModule('chat');assert.equal(m.b.activeView.value,'chat');
    m.b.navigateFromTray('assets');await flushShell();assert.equal(m.b.activeModule.value,'asset');assert.equal(m.b.activeView.value,'assets');
  } finally { m.unmount(); }
});
test('actual shell cannot replace current project statistics with an old async response', async () => {
  const older=shellDeferred(),newer=shellDeferred();
  const m=mount({dashboardStats:id=>id===91?older.promise:id===92?newer.promise:Promise.resolve({assetCount:0})});
  try {
    m.b.selectProject(91);m.b.selectProject(92);
    newer.resolve({assetCount:92});await flushShell();older.resolve({assetCount:91});await flushShell();
    assert.equal(m.b.selectedProjectId.value,92);assert.equal(m.b.stats.value.assetCount,92);
    m.b.selectProject(999);await flushShell();assert.equal(m.b.selectedProjectId.value,undefined);
  } finally {older.resolve({});newer.resolve({});m.unmount();}
});
test('actual shell refuses stale secondary data after switching project and module', async () => {
  const old=shellDeferred();
  const m=mount({listRuns:()=>old.promise});
  try {
    m.b.navigate('tasks');m.b.switchModule('chat');old.resolve([{id:777,projectId:91}]);await flushShell();
    assert.deepEqual(m.b.runs.value,[]);assert.equal(m.b.activeView.value,'chat');
  } finally {old.resolve([]);m.unmount();}
});
test('actual shell preserves HackerOne gating and configured entry', async () => {
  const m=mount();
  try {
    m.b.switchModule('hackerone');assert.notEqual(m.b.activeModule.value,'hackerone');
    m.b.profiles.value=[{id:1,isDefault:true,settings:{hackerOneUsername:'user',hackerOneToken:'fixture'}}];
    m.b.switchModule('hackerone');assert.equal(m.b.activeModule.value,'hackerone');assert.equal(m.b.activeView.value,'hackerone');
    m.b.navigate('exposure');assert.equal(m.b.activeModule.value,'asset');assert.equal(m.b.activeView.value,'exposure');
  } finally {m.unmount();}
});
