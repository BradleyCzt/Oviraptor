// Execute App.vue setup and render its real template/InlineConfirm. Desktop
// startup, unrelated panels, IPC and timers are isolated; no external IO occurs.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const vue = require('vue');
const { catalog, navigation, plugins } = require('./shell_module_harness.cjs');
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
function mount(overrides = {}, storageSeed = {}) {
  const calls = [];
  const defaults = { projectImpact: async () => impact(), deleteProject: async () => {},
    archiveProject: async () => {}, listProjects: async () => [project()], listProfiles: async () => [],
    dashboardStats: async () => ({}), getAppSettings: async () => ({}),
    listRuns: async () => [], listEvents: async () => [], startupStatus:async()=>({reminderDays:7,interruptedJobs:[],staleProjects:[]}) };
  const api = new Proxy({}, { get(_, name) {
    const fn = overrides[name] || defaults[name];
    if (!fn) throw new Error(`Unexpected IPC ${String(name)}`);
    return (...args) => { calls.push([name, ...args]); return fn(...args); };
  } });
  const stored=new Map(Object.entries(storageSeed));
  const storage = { getItem: key => stored.get(key)??null, setItem(key,value){stored.set(key,value);}, removeItem(key){stored.delete(key);} };
  const i18n = {};
  new Function('require', 'exports', 'localStorage', transpile(
    fs.readFileSync(path.join(__dirname, '../src/i18n.ts'), 'utf8'),
  ))(require, i18n, storage);
  const exports = {};
  new Function('require', 'exports', 'localStorage', 'setTimeout', appSource.script)((name) => {
    if (name === 'vue') return { ...vue, onMounted() {} };
    if (name === './api') return { api };
    if (name === './plugins') return plugins;
    if (name === './shell/builtinWorkspaces') return {BUILTIN_WORKSPACES:{asset:()=>null,sentinel:()=>null,hackerone:()=>null}};
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
  const restored={module:bindings.activeModule.value,view:bindings.activeView.value};
  bindings.loading.value = false;
  bindings.projects.value = [project(), project(92)];
  bindings.activeView.value = 'projects';
  return { b: bindings, calls, storage, restored, unmount: () => app.unmount(),
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
test('actual shell routes asset actions from Chat and preserves scanning tray targets', async () => {
  const m=mount();
  try {
    m.b.activeModule.value='chat';m.b.activeView.value='chat';
    m.b.navigate('tasks');await flushShell();
    assert.equal(m.b.activeModule.value,'asset');assert.equal(m.b.activeView.value,'tasks');
    m.b.navigateFromTray('agent-tasks');
    assert.equal(m.b.activeModule.value,'sentinel');assert.equal(m.b.activeView.value,'sentinel');assert.equal(m.b.pluginNavigation.value.section,'queue');
    m.b.navigate('settings');assert.equal(m.b.activeModule.value,'sentinel');
    m.b.switchModule('chat');assert.equal(m.b.activeView.value,'chat');
    m.b.navigateFromTray('assets');await flushShell();assert.equal(m.b.activeModule.value,'asset');assert.equal(m.b.activeView.value,'assets');
  } finally { m.unmount(); }
});
test('actual shell validates selected projects without owning plugin statistics or result reads', async () => {
  const m=mount();
  try {
    m.b.selectProject(91);m.b.selectProject(92);await flushShell();
    assert.equal(m.b.selectedProjectId.value,92);
    assert.equal(m.b.stats,undefined);assert.equal(m.b.runs,undefined);
    assert.ok(!m.calls.some(([name])=>['dashboardStats','listRuns','listEvents','listSentinelScans'].includes(name)));
    m.b.selectProject(999);assert.equal(m.b.selectedProjectId.value,undefined);
  } finally {m.unmount();}
});
test('actual shell navigation never starts hidden plugin reads and clears stale startup intents', async () => {
  const m=mount();
  try {
    m.b.quickStartProject(91);assert.equal(m.b.pendingWorkspaceIntent.value.kind,'start');
    m.b.switchModule('chat');await flushShell();
    assert.equal(m.b.pendingWorkspaceIntent.value,undefined);assert.equal(m.b.activeView.value,'chat');
    assert.ok(!m.calls.some(([name])=>['listRuns','startJob'].includes(name)));
  } finally {m.unmount();}
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
test('disabled stored module restores to Chat without touching profile or legacy settings',()=>{
 const profileBlob=JSON.stringify({model:'configured',apiKey:'fixture'});
 const m=mount({}, {'oviraptor-module':'asset','oviraptor-view':'assets','oviraptor-builtin-plugins-v1':JSON.stringify({version:1,enabled:{asset:false}}),'profiles':profileBlob,'oviraptor-accent':'#abcd12'});
 try{assert.deepEqual(m.restored,{module:'chat',view:'chat'});assert.equal(m.storage.getItem('profiles'),profileBlob);assert.equal(m.storage.getItem('oviraptor-accent'),'#abcd12');assert.equal(m.calls.length,0);}finally{m.unmount();}
});
test('disabled tray and workspace entry reject atomically without silently re-enabling',()=>{
 const m=mount();try{m.b.switchModule('chat');m.b.togglePlugin('sentinel',false);m.b.navigateFromTray('agent-tasks');assert.equal(m.b.activeModule.value,'chat');assert.equal(m.b.activeView.value,'chat');assert.equal(m.b.pluginEnabled.value.sentinel,false);assert.ok(m.b.navigationError.value);m.b.openProjectWorkspace(project(),'sentinel');assert.equal(m.b.activeModule.value,'chat');assert.equal(m.b.selectedProjectId.value,undefined);}finally{m.unmount();}
});
test('disabling mounted plugin returns to Chat and discards unconsumed intents without cancelling work',()=>{
 const m=mount();try{m.b.quickStartProject(91);assert.ok(m.b.pendingWorkspaceIntent.value);m.b.togglePlugin('asset',false);assert.equal(m.b.activeModule.value,'chat');assert.equal(m.b.activeView.value,'chat');assert.equal(m.b.pendingWorkspaceIntent.value,undefined);m.b.togglePlugin('asset',true);assert.equal(m.b.pendingWorkspaceIntent.value,undefined);assert.equal(m.calls.filter(([name])=>/cancel|delete|archive|startJob/.test(name)).length,0);}finally{m.unmount();}
});
test('shared Settings and Projects remain available after HackerOne credentials become unavailable',async()=>{
 const m=mount({}, {'oviraptor-module':'hackerone','oviraptor-view':'hackerone'});
 try{assert.equal(m.b.activeWorkspace.value,undefined);m.b.activeModule.value='hackerone';m.b.navigate('settings');assert.equal(m.b.activeView.value,'settings');assert.equal(m.b.activeModule.value,'chat');m.b.activeModule.value='hackerone';m.b.navigate('projects');assert.equal(m.b.activeView.value,'projects');await m.b.refresh();assert.equal(m.b.activeModule.value,'chat');}finally{m.unmount();}
});
test('profile settings survive plugin changes and corrupt preference notice stays inline',async()=>{
 const raw='{"version":99,"enabled":{"asset":false}}';const m=mount({}, {'oviraptor-builtin-plugins-v1':raw});
 try{m.b.profiles.value=[{id:1,isDefault:true,settings:{model:'kept',fofaKey:'fixture'}}];const before=JSON.stringify(m.b.profiles.value);m.b.managePlugins();assert.ok(m.b.pluginPreferenceError.value);m.b.togglePlugin('asset',false);assert.equal(m.b.pluginEnabled.value.asset,true);assert.equal(m.storage.getItem('oviraptor-builtin-plugins-v1'),raw);assert.equal(JSON.stringify(m.b.profiles.value),before);assert.ok((await m.render()).includes('version'));}finally{m.unmount();}
});
test('Chat plugin handoff is current-project navigation only and refuses scope mismatch',()=>{
 const m=mount();try{m.b.selectProject(91);m.b.switchModule('chat');m.b.pluginHost.openWorkspace({pluginId:'asset',projectId:91,page:'assets'});assert.equal(m.b.activeView.value,'assets');assert.throws(()=>m.b.pluginHost.openWorkspace({pluginId:'sentinel',projectId:92,page:'sentinel',section:'dialog'}));assert.equal(m.b.activeView.value,'assets');assert.equal(m.calls.filter(([name])=>/create|start|send/.test(name)).length,0);}finally{m.unmount();}
});
test('failed plugin persistence restores native checkbox to authoritative availability',()=>{
 const m=mount({}, {'oviraptor-builtin-plugins-v1':'{broken'});try{const target={checked:false};m.b.onPluginToggle('asset',{target});assert.equal(target.checked,true);assert.equal(m.b.pluginEnabled.value.asset,true);assert.equal(m.storage.getItem('oviraptor-builtin-plugins-v1'),'{broken');}finally{m.unmount();}
});
test('host resume receipt survives plugin unmount, blocks duplicates and reconciles original reminder',async()=>{
 const {mountWorkspace,deferred}=require('./plugin_workspace_harness.cjs');const ack=deferred();const job={runId:17,projectId:91,projectName:'Project 91',name:'Old run',pipeline:'full',profileId:1};
 const m=mount();let plugin;
 try{
  m.b.startup.value={reminderDays:7,interruptedJobs:[job],staleProjects:[]};m.b.resumeInterrupted(job);const intent=m.b.pendingWorkspaceIntent.value;
  plugin=mountWorkspace(null,'asset',{acknowledgeInterruptedRun:()=>ack.promise},{view:'tasks',intent,resumeWorkflow:m.b.resumeWorkflow});
  m.b.consumeWorkspaceIntent(intent.id);await flushShell();assert.equal(m.b.resumeReceipts.value.get(17).state,'admitted');plugin.unmount();m.b.switchModule('chat');
  m.b.resumeInterrupted(job);assert.equal(m.b.pendingWorkspaceIntent.value,undefined);assert.equal(plugin.calls.filter(([name])=>name==='startJob').length,0);
  ack.resolve();await flushShell();assert.deepEqual(plugin.calls.filter(([name])=>name==='startJob'),[['startJob',91,1,'Old run · Resume','full']]);assert.equal(m.b.startup.value.interruptedJobs.length,0);assert.equal(m.b.resumeReceipts.value.has(17),false);assert.equal(m.b.activeView.value,'chat');assert.equal(plugin.events.filter(([name])=>name==='navigate').length,0);m.b.resumeInterrupted(job);assert.equal(m.b.pendingWorkspaceIntent.value,undefined);
 }finally{ack.resolve();plugin?.unmount();m.unmount();}
});
test('failed resume reconciliation remains blocked until explicit read-only status retry',async()=>{
 let unavailable=true;const job={runId:18,projectId:91,projectName:'Project 91',name:'Old run',pipeline:'full'};
 const m=mount({startupStatus:async()=>{if(unavailable)throw Error('status unavailable');return{reminderDays:7,interruptedJobs:[job],staleProjects:[]};}});
 try{m.b.resumeWorkflow.begin(job);m.b.resumeWorkflow.settle(job,{started:false,acknowledged:false,error:'start failed'});await flushShell();assert.equal(m.b.resumeReceipts.value.has(18),true);m.b.resumeInterrupted(job);assert.equal(m.b.pendingWorkspaceIntent.value,undefined);assert.match(m.b.startupReadError.value,/unverified|核实/);assert.match(m.b.startupActionError.value,/Project 91.*#18/);unavailable=false;await m.b.refreshStartupStatus();assert.equal(m.b.resumeReceipts.value.has(18),false);assert.equal(m.b.startup.value.interruptedJobs[0].runId,18);assert.equal(m.b.startupReadError.value,'');assert.match(m.b.startupActionError.value,/start failed/);assert.equal(m.calls.filter(([name])=>/startJob|resumeJob/.test(name)).length,0);}finally{m.unmount();}
});
