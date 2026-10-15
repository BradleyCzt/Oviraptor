const test = require('node:test');
const assert = require('node:assert/strict');
const { catalog, navigation, transpile } = require('./shell_module_harness.cjs');
const fs = require('node:fs');
const path = require('node:path');
const vue = require('vue');
const { parse, compileScript } = require('@vue/compiler-sfc');
const { renderToString } = require('@vue/server-renderer');
const source = relative => fs.readFileSync(path.join(__dirname,'..',relative),'utf8');
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
  assert.deepEqual(first, { module: 'chat', view: 'chat' });
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


test('every fresh launch returns to Chat regardless of previously selected plugin or shared view', () => {
  for (const module of catalog.MODULE_CATALOG) {
    for (const view of [...module.views, 'settings', 'projects', 'logs']) {
      const s = storage({'oviraptor-module':module.id,'oviraptor-view':view,'oviraptor-project':'41','oviraptor-theme':'codex','oviraptor-accent':'#112233','conversation-history':'keep'});
      assert.deepEqual(navigation.restoreRoute(s),{module:'chat',view:'chat'});
      assert.equal(s.getItem('oviraptor-module'),'chat');
      assert.equal(s.getItem('oviraptor-view'),'chat');
      assert.equal(s.getItem('oviraptor-project'),'41');
      assert.equal(s.getItem('oviraptor-theme'),'codex');
      assert.equal(s.getItem('oviraptor-accent'),'#112233');
      assert.equal(s.getItem('conversation-history'),'keep');
    }
  }
});

test('explicit plugin selection remains routable after a Chat-only startup', () => {
  const s=storage({'oviraptor-module':'sentinel','oviraptor-view':'sentinel'});
  navigation.restoreRoute(s);
  const route=navigation.normalizeRoute('sentinel','sentinel');
  navigation.persistRoute(s,route);
  assert.deepEqual(route,{module:'sentinel',view:'sentinel'});
  assert.equal(s.getItem('oviraptor-module'),'sentinel');
  assert.deepEqual(navigation.restoreRoute(s),{module:'chat',view:'chat'});
});


async function renderNavigation(props={}) {
  const filename=path.join(__dirname,'../src/shell/WorkspaceNavigation.vue');
  const {descriptor}=parse(source('src/shell/WorkspaceNavigation.vue'),{filename});
  const script=compileScript(descriptor,{id:'workspace-navigation-test',inlineTemplate:true});
  const exports={};
  new Function('require','exports',transpile(script.content))(name=>{
    if(name==='vue')return vue;
    if(name==='@lucide/vue')return new Proxy({},{get:()=>()=>null});
    if(name==='../i18n')return {useI18n:()=>({tr:(_,en)=>en})};
    if(name==='./moduleCatalog')return catalog;
    throw Error(`Unexpected navigation import ${name}`);
  },exports);
  return renderToString(vue.createSSRApp({render:()=>vue.h(exports.default,{module:'chat',view:'chat',sentinelMenu:'dialog',hackerOneEnabled:true,...props},{history:()=>vue.h('div',{id:'history-fixture'},'Saved conversation'),workspace:()=>vue.h('div',{id:'workspace-fixture'},'Workspace picker')})}));
}

test('one navigation places history directly after Chat and all catalog plugins above workspace',async()=>{
  const html=await renderNavigation();
  const chat=html.indexOf('workspace-chat-module');
  const history=html.indexOf('id="history-fixture"');
  const plugins=html.indexOf('workspace-plugin-group');
  const workspace=html.indexOf('id="workspace-fixture"');
  assert.ok(chat>=0&&chat<history&&history<plugins&&plugins<workspace);
  const chatSection=html.slice(chat,history);
  assert.match(chatSection,/aria-label="Chat"/);
  for(const item of catalog.visibleModules(true).filter(item=>item.id!=='chat')){
    assert.doesNotMatch(chatSection,new RegExp(`aria-label="${item.label[1]}"`));
    assert.ok(html.indexOf(`aria-label="${item.label[1]}"`,plugins)<workspace);
  }
});

test('collapsed navigation keeps history target mounted and plugins in the bottom group',async()=>{
  const html=await renderNavigation({collapsed:true});
  assert.match(html,/class="workspace-chat-history" style="display:none;"/);
  assert.match(html,/id="history-fixture"/);
  assert.match(html,/workspace-plugin-group/);
  assert.ok(html.indexOf('workspace-chat-history')<html.indexOf('workspace-plugin-group'));
});

test('startup only mounts the selected async workspace and fences optional plugin activity',()=>{
  const app=source('src/App.vue');
  const registry=source('src/shell/builtinWorkspaces.ts');
  assert.match(app,/const initialRoute = restoreRoute\(localStorage\)/);
  assert.match(app,/v-if="!loading&&activeWorkspace" :is="activeWorkspace"/);
  assert.match(registry,/asset: defineAsyncComponent\(\(\) => import/);
  assert.match(registry,/sentinel: defineAsyncComponent\(\(\) => import/);
  assert.match(app,/if \(activeModule.value !== "hackerone" \|\| !hackerOneEnabled.value \|\| !pluginEnabled.value.hackerone\) return;/);
  assert.equal((app.match(/<WorkspaceNavigation /g)||[]).length,1);
  assert.equal((app.match(/id="conversation-history"/g)||[]).length,1);
});


test('fresh Chat home never starts remote worker reads or imports and explicit Assets can resume configured sync',()=>{
  const app=source('src/App.vue');
  const sync=app.slice(app.indexOf('async function autoSyncWorkers()'),app.indexOf('onMounted(async()=>'));
  assert.match(sync,/if\(workerAutoSyncRunning\|\|!assetWorkspaceActive\(\)\)return;/);
  assert.ok(sync.indexOf('!assetWorkspaceActive()')<sync.indexOf('runtimeApi.listWorkerNodes()'));
  assert.match(sync,/if\(!assetWorkspaceActive\(\)\)break;/);
  assert.match(app,/if \(module === "asset"\) void autoSyncWorkers\(\)/);
  assert.doesNotMatch(sync,/saveWorker|deleteWorker|enabled\s*=/);
});


test('Skills is a host-owned route independent of every plugin and fresh launch still opens Chat',async()=>{
  for(const module of catalog.MODULE_CATALOG){
    assert.deepEqual(navigation.normalizeRoute(module.id,'skills'),{module:'chat',view:'skills'});
  }
  const s=storage({'oviraptor-module':'chat','oviraptor-view':'skills'});
  assert.deepEqual(navigation.restoreRoute(s),{module:'chat',view:'chat'});
  const html=await renderNavigation({view:'skills',enabledPlugins:{asset:false,sentinel:false,hackerone:false}});
  assert.match(html,/aria-label="Skills"/);
  assert.doesNotMatch(html,/aria-label="Assets"|aria-label="Scanning"/);
  assert.equal((html.match(/aria-current="page"/g)||[]).length,1);
});
