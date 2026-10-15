// §O0 — the real `AssetOwnershipGate.vue` is compiled and mounted here with only the IPC
// boundary faked. Every response is released by hand so the tests can reorder them: a late
// answer for the previous project, a save that lands after a switch, a queue reload while a
// draft is open. See docs/NEST_OWNERSHIP_GATE_SMART_GROUP_REVIEW_DESIGN_2026-10-08.md §4 末段、§11 第 1 步。

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const { renderToString } = require('@vue/server-renderer');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');

const GATE = 'src/features/assets/components/AssetOwnershipGate.vue';
const readSource = relative => fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');
const transpile = source => ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

/// Compile the real SFC. Each call evaluates the module again, so the module-scope draft
/// cache starts empty per test, exactly like a fresh app session.
function compileGate(api) {
  const requireModule = name => {
    if (name === 'vue') return vue;
    if (name === '../api') return { assetApi: api };
    if (name === '../ownershipDirectoryApi') return { ownershipAuthorizationApi: api };
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    if (name.endsWith('.css')) return {};
    // Child panels (e.g. the O1 project setup card) are outside this O0 gate test; they are
    // rendered as empty components so only the gate's own state machine is exercised here.
    if (/^\.\/[A-Za-z]+\.vue$/.test(name)) return { __esModule: true, default: { name: name.slice(2, -4), render: () => null } };
    throw Error(`Unexpected ownership gate import: ${name}`);
  };
  const filename = path.join(__dirname, '..', GATE);
  const id = path.basename(filename);
  const { descriptor } = parse(readSource(GATE), { filename });
  const script = compileScript(descriptor, { id });
  const template = compileTemplate({
    source: descriptor.template.content, filename, id,
    compilerOptions: { bindingMetadata: script.bindings },
  });
  assert.deepEqual(template.errors, [], '归属门禁模板必须能编译');
  const scriptExports = {}, templateExports = {};
  new Function('require', 'exports', transpile(script.content))(requireModule, scriptExports);
  new Function('require', 'exports', transpile(template.code))(requireModule, templateExports);
  return { withTemplate: { ...scriptExports.default, render: templateExports.render }, logicOnly: { ...scriptExports.default, render: () => null } };
}

/// Every IPC call returns a promise the test settles explicitly.
function controlledApi() {
  const calls = [];
  const call = (name, args) => {
    let resolve, reject;
    const promise = new Promise((ok, fail) => { resolve = ok; reject = fail; });
    const row = { name, args, settled: false };
    row.resolve = value => { row.settled = true; resolve(value); };
    row.reject = error => { row.settled = true; reject(error); };
    calls.push(row);
    return promise;
  };
  const api = {
    getAssetOwnershipProfile: projectId => call('profile', [projectId]),
    getAssetOwnershipSummary: projectId => call('summary', [projectId]),
    listAssets: query => call('list', [query]),
    saveAssetOwnershipProfile: (input, expectedVersion) => call('save', [input, expectedVersion]),
    assessAssetOwnership: projectId => call('assess', [projectId]),
    updateAssetOwnership: (...args) => call('update', args),
    confirmAssetOwnership: (...args) => call('confirm', args),
    authorizeAssetOwnership: (...args) => call('authorize', args),
  };
  const open = (name, projectId) => calls.find(row => row.name === name && !row.settled
    && (projectId === undefined || (name === 'list' ? row.args[0].projectId : name === 'save' ? row.args[0].projectId : row.args[0]) === projectId));
  const take = (name, projectId) => {
    const row = open(name, projectId);
    assert.ok(row, `expected an open ${name} call${projectId ? ` for project ${projectId}` : ''}; calls: ${JSON.stringify(calls.map(r => [r.name, r.settled]))}`);
    return row;
  };
  const count = name => calls.filter(row => row.name === name).length;
  return { api, calls, take, open, count };
}

const profile = (projectId, over = {}) => ({
  projectId, legalName: '', jurisdiction: '', jurisdictions: [], excludedJurisdictions: [], aliases: [],
  approvedDomains: [], sharedDomains: [], excludedNames: [], excludedDomains: [], notes: '',
  policyVersion: 1, updatedAt: '', ...over,
});
const summary = (over = {}) => ({ all: 0, unreviewed: 0, attributed: 0, confirmed: 0, related: 0, thirdParty: 0, excluded: 0, exposureEligible: 0, ...over });
const asset = (projectId, id, over = {}) => ({
  id, projectId, domain: `a${id}.example.cn`, host: '', link: '', ip: '', port: '', company: '', title: '',
  ownershipStatus: 'unreviewed', ownershipSource: '', ownershipConfidence: 0, authorizationStatus: 'unknown',
  exposureEligible: false, ownershipReason: '', ...over,
});
const pageOf = items => ({ items, total: items.length, page: 1, pageSize: 100, summary: {} });
const flush = async () => { for (let i = 0; i < 5; i++) await new Promise(resolve => setImmediate(resolve)); };

const PROJECTS = [{ id: 3, name: '中国移动历史资产', status: 'active' }, { id: 15, name: '香港移动', status: 'active' }];

function memoryRenderer() {
  return vue.createRenderer({
    createElement: tag => ({ tag, children: [], props: {} }),
    createText: text => ({ text }),
    createComment: text => ({ text }),
    setText: (node, text) => { node.text = text; },
    setElementText: (node, text) => { node.text = text; },
    patchProp: (el, key, _prev, value) => { el.props[key] = value; },
    insert: (child, parent) => { child.parent = parent; parent.children.push(child); },
    remove: node => { if (node.parent) node.parent.children = node.parent.children.filter(row => row !== node); },
    parentNode: node => node.parent ?? null,
    nextSibling: () => null,
  });
}

/// Mount the gate the way App.vue does: props come from the parent and change underneath it.
function mountGate(t, harness, projectId = 3) {
  const { logicOnly } = harness.compiled ??= compileGate(harness.api);
  const notes = [];
  const confirms = [];
  const host = vue.reactive({ projectId, mounted: true, confirmAnswer: true });
  globalThis.window = { confirm: message => { confirms.push(message); return host.confirmAnswer; } };
  const Root = { setup: () => () => (host.mounted ? vue.h(logicOnly, { projects: PROJECTS, selectedProjectId: host.projectId, onNotify: (type, text) => notes.push([type, text]) }) : null) };
  const app = memoryRenderer().createApp(Root);
  const root = app.mount({ tag: '#root', children: [], props: {} });
  t.after(() => { app.unmount(); delete globalThis.window; });
  const gate = () => root.$.subTree.component.setupState;
  return { host, gate, notes, confirms };
}

async function loadProject(api, projectId, { prof, sum = summary(), items = [] }) {
  api.take('profile', projectId).resolve(prof);
  api.take('summary', projectId).resolve(sum);
  api.take('list', projectId).resolve(pageOf(items));
  await flush();
}

test('a late response for the previous project never fills the next project page', async t => {
  const api = controlledApi();
  const { host, gate } = mountGate(t, api, 3);
  await flush();
  host.projectId = 15;
  await flush();
  // Project 15 answers first; then the stale project 3 answers arrive.
  await loadProject(api, 15, { prof: profile(15, { legalName: '香港移动通讯有限公司', policyVersion: 2, updatedAt: '2026-10-08 16:00:00' }), sum: summary({ all: 3960, unreviewed: 3960 }), items: [asset(15, 40)] });
  await loadProject(api, 3, { prof: profile(3, { legalName: '中国移动历史资产' }), sum: summary({ all: 14280, unreviewed: 14280 }), items: [asset(3, 1)] });
  const g = gate();
  assert.equal(g.form.legalName, '香港移动通讯有限公司', '旧项目档案不得写入新项目表单');
  assert.equal(g.baseProfile.projectId, 15);
  assert.equal(g.summary.all, 3960, '旧项目计数不得写入新项目');
  assert.deepEqual(g.assets.map(row => [row.projectId, row.id]), [[15, 40]], '旧项目复核行不得写入新项目');
  assert.equal(g.profileReady, true);
  assert.equal(g.queueReady, true);
});

test('queue reloads never overwrite an unsaved draft, and drafts are kept per project', async t => {
  const api = controlledApi();
  const { host, gate } = mountGate(t, api, 3);
  await flush();
  await loadProject(api, 3, { prof: profile(3, { legalName: '中国移动历史资产' }), items: [asset(3, 1)] });
  const g = gate();
  g.form.legalName = '中国移动通信集团有限公司';
  g.form.aliasesText = '中国移动\nChina Mobile';
  assert.equal(g.profileDirty, true);
  // Search, view switch and paging reload the queue only.
  g.search = 'cmhk';
  g.setView('confirmed');
  await flush();
  api.take('summary', 3).resolve(summary());
  api.take('list', 3).resolve(pageOf([]));
  await flush();
  assert.equal(api.count('profile'), 1, '筛选重载不得重新读取并覆盖档案');
  assert.equal(gate().form.legalName, '中国移动通信集团有限公司');
  assert.equal(gate().form.aliasesText, '中国移动\nChina Mobile');
  // Switch away and back: the draft for project 3 comes back, project 15 never sees it.
  host.projectId = 15;
  await flush();
  await loadProject(api, 15, { prof: profile(15, { legalName: '香港移动' }) });
  assert.equal(gate().form.legalName, '香港移动');
  assert.equal(gate().profileDirty, false);
  host.projectId = 3;
  await flush();
  await loadProject(api, 3, { prof: profile(3, { legalName: '中国移动历史资产' }) });
  assert.equal(gate().form.legalName, '中国移动通信集团有限公司', '切回项目后应恢复该项目的草稿');
  assert.equal(gate().draftRestored, true);
  assert.equal(gate().profileDirty, true);
});

test('a draft survives leaving the ownership view and remounting', async t => {
  const api = controlledApi();
  const { host, gate } = mountGate(t, api, 3);
  await flush();
  await loadProject(api, 3, { prof: profile(3) });
  gate().form.legalName = '草稿主体';
  host.mounted = false;
  await flush();
  host.mounted = true;
  await flush();
  await loadProject(api, 3, { prof: profile(3) });
  assert.equal(gate().form.legalName, '草稿主体');
  assert.equal(gate().draftRestored, true);
});

test('save is a CAS on the loaded version and a conflict keeps the draft', async t => {
  const api = controlledApi();
  const { gate, notes, host } = mountGate(t, api, 3);
  await flush();
  await loadProject(api, 3, { prof: profile(3, { legalName: '中国移动历史资产' }) });
  gate().form.legalName = '中国移动通信集团有限公司';
  void gate().saveProfile();
  await flush();
  const first = api.take('save', 3);
  assert.equal(first.args[1], 0, '尚未保存的预填必须以 expectedVersion=0 保存');
  first.reject('OWNERSHIP_PROFILE_VERSION_CONFLICT: 归属档案已被其他操作更新（当前 v1，页面基于 未保存的预填），本次未写入');
  await flush();
  assert.equal(gate().saveConflict, true);
  assert.equal(gate().form.legalName, '中国移动通信集团有限公司', '冲突不得覆盖草稿');
  assert.match(notes.at(-1)[1], /OWNERSHIP_PROFILE_VERSION_CONFLICT/);
  // The page refreshes only the "server is ahead" hint.
  api.take('profile', 3).resolve(profile(3, { legalName: '另一窗口保存的主体', policyVersion: 1, updatedAt: '2026-10-08 17:00:00' }));
  await flush();
  assert.equal(gate().serverAhead, true);
  assert.equal(gate().form.legalName, '中国移动通信集团有限公司');
  // Declining either resolution changes nothing.
  host.confirmAnswer = false;
  gate().discardDraft();
  gate().rebaseDraftOnLatest();
  assert.equal(gate().form.legalName, '中国移动通信集团有限公司');
  assert.equal(gate().serverAhead, true);
  // Keeping the draft is an explicit choice; the next save carries the version now shown.
  host.confirmAnswer = true;
  gate().rebaseDraftOnLatest();
  assert.equal(gate().serverAhead, false);
  gate().form.aliasesText = 'CMHK';
  void gate().saveProfile();
  await flush();
  const second = api.take('save', 3);
  assert.equal(second.args[1], 1);
  assert.equal(second.args[0].legalName, '中国移动通信集团有限公司');
  // The backend normalises lists (lower-case, de-duplicated); the form must show what was stored.
  second.resolve(profile(3, { legalName: '中国移动通信集团有限公司', aliases: ['cmhk'], policyVersion: 2, updatedAt: '2026-10-08 17:01:00' }));
  await flush();
  assert.equal(gate().baseProfile.policyVersion, 2);
  assert.equal(gate().form.aliasesText, 'cmhk');
  assert.equal(gate().profileDirty, false);
  // Discarding loads the latest server profile into the form.
  gate().form.notes = 'x';
  gate().serverProfile = profile(3, { legalName: '服务器最新', policyVersion: 3, updatedAt: '2026-10-08 17:02:00' });
  gate().discardDraft();
  assert.equal(gate().form.legalName, '服务器最新');
  assert.equal(gate().baseProfile.policyVersion, 3);
});

test('a save that lands after switching projects stays with its own project', async t => {
  const api = controlledApi();
  const { host, gate, notes } = mountGate(t, api, 3);
  await flush();
  await loadProject(api, 3, { prof: profile(3, { legalName: '中国移动历史资产' }) });
  gate().form.legalName = '中国移动通信集团有限公司';
  void gate().saveProfile();
  await flush();
  const pendingSave = api.take('save', 3);
  host.projectId = 15;
  await flush();
  await loadProject(api, 15, { prof: profile(15, { legalName: '香港移动通讯有限公司', updatedAt: '2026-10-08 16:00:00' }) });
  pendingSave.resolve(profile(3, { legalName: '中国移动通信集团有限公司', policyVersion: 1, updatedAt: '2026-10-08 17:05:00' }));
  await flush();
  assert.equal(gate().form.legalName, '香港移动通讯有限公司', '旧项目的保存结果不得写入新项目表单');
  assert.equal(gate().baseProfile.projectId, 15);
  assert.ok(notes.some(([type, text]) => type === 'success' && text.includes('「中国移动历史资产」归属档案已保存为 v1')));
  // Back on project 3 the saved version is shown, not a stale draft based on the prefill.
  host.projectId = 3;
  await flush();
  await loadProject(api, 3, { prof: profile(3, { legalName: '中国移动通信集团有限公司', policyVersion: 1, updatedAt: '2026-10-08 17:05:00' }) });
  assert.equal(gate().draftRestored, false);
  assert.equal(gate().profileDirty, false);
  assert.equal(gate().serverAhead, false);
});

test('switching back before an in-flight save returns rebases the restored draft', async t => {
  const api = controlledApi();
  const { host, gate } = mountGate(t, api, 3);
  await flush();
  await loadProject(api, 3, { prof: profile(3) });
  gate().form.legalName = '中国移动通信集团有限公司';
  void gate().saveProfile();
  await flush();
  const pendingSave = api.take('save', 3);
  host.projectId = 15;
  await flush();
  host.projectId = 3;
  await flush();
  // The reload read the profile before the save committed.
  api.take('profile', 3).resolve(profile(3));
  await flush();
  assert.equal(gate().draftRestored, true);
  pendingSave.resolve(profile(3, { legalName: '中国移动通信集团有限公司', policyVersion: 1, updatedAt: '2026-10-08 17:06:00' }));
  await flush();
  assert.equal(gate().baseProfile.policyVersion, 1);
  assert.equal(gate().baseProfile.updatedAt, '2026-10-08 17:06:00', '恢复的草稿必须改为基于刚保存的版本，否则下次保存必然冲突');
  assert.equal(gate().profileDirty, false);
});

test('confirming ownership is its own step: it carries per-row versions, grants nothing and says so', async t => {
  const api = controlledApi();
  const { gate, host, confirms } = mountGate(t, api, 3);
  await flush();
  const seen = asset(3, 1, { ownershipStatus: 'attributed', ownershipSource: 'auto', ownershipConfidence: 92, ownershipReason: '属于确认根域名 example.cn' });
  await loadProject(api, 3, { prof: profile(3), items: [seen, asset(3, 2)] });
  gate().toggle(gate().assets[0], true);
  host.confirmAnswer = false;
  await gate().confirmOwnership(false);
  assert.equal(api.count('confirm') + api.count('authorize') + api.count('update'), 0, '取消确认框不得写入');
  assert.match(confirms[0], /只写入归属事实/);
  assert.match(confirms[0], /授权状态、暴露面准入 = 保持各资产现状（不授权）/);
  assert.match(confirms[0], /确认归属本身不创建任何执行授权/);
  assert.doesNotMatch(confirms[0], /授权状态 = 允许/);
  host.confirmAnswer = true;
  void gate().confirmOwnership(false);
  await flush();
  const confirm = api.take('confirm');
  const [input, expectedProjectId, expectations] = confirm.args;
  assert.deepEqual(input, { selections: [{ projectId: 3, assetId: 1 }], note: '', domainRuleScope: 'none' });
  assert.equal(expectedProjectId, 3);
  assert.deepEqual(expectations, [{ assetId: 1, status: 'attributed', source: 'auto', confidence: 92, authorizationStatus: 'unknown', exposureEligible: false, reason: '属于确认根域名 example.cn' }]);
  confirm.resolve({ changed: 1, expectations: [], ruleScope: 'none', learnedRules: [] });
  await flush();
  assert.equal(api.count('authorize'), 0, '只确认归属不会调用授权');
  assert.equal(api.count('update'), 0, '确认归属不再走旧的“确认并授权”合同');
});

test('authorization is a separate step that only accepts manually confirmed rows', async t => {
  const api = controlledApi();
  const { gate, confirms, notes } = mountGate(t, api, 3);
  await flush();
  const confirmed = asset(3, 1, { ownershipStatus: 'confirmed', ownershipSource: 'manual', ownershipConfidence: 100, ownershipReason: '人工确认' });
  await loadProject(api, 3, { prof: profile(3), items: [confirmed, asset(3, 2, { ownershipStatus: 'attributed', ownershipSource: 'auto' })] });
  gate().toggle(gate().assets[1], true);
  await gate().authorizeOwnership();
  assert.equal(api.count('authorize'), 0, '未人工确认归属的行不得授权');
  assert.equal(confirms.length, 0);
  assert.match(notes.at(-1)[1], /尚未人工确认归属，未授权/);
  gate().toggle(gate().assets[1], false);
  gate().toggle(gate().assets[0], true);
  void gate().authorizeOwnership();
  await flush();
  assert.match(confirms[0], /授权状态 = 允许/);
  assert.match(confirms[0], /暴露面准入 = 是/);
  assert.match(confirms[0], /这不是主动扫描授权/);
  const authorize = api.take('authorize');
  assert.deepEqual(authorize.args[0], { selections: [{ projectId: 3, assetId: 1 }], note: '' });
  assert.equal(authorize.args[1], 3);
  assert.deepEqual(authorize.args[2], [{ assetId: 1, status: 'confirmed', source: 'manual', confidence: 100, authorizationStatus: 'unknown', exposureEligible: false, reason: '人工确认' }]);
});

test('the wizard states both impacts and authorizes with the versions the first step wrote', async t => {
  const api = controlledApi();
  const { gate, confirms } = mountGate(t, api, 3);
  await flush();
  await loadProject(api, 3, { prof: profile(3), items: [asset(3, 1)] });
  gate().toggle(gate().assets[0], true);
  gate().learnDomainRule = true;
  assert.equal(gate().ruleScope, 'exact_host', '学习规则默认精确主机');
  void gate().confirmOwnership(true);
  await flush();
  assert.match(confirms[0], /第 1 步 确认归属/);
  assert.match(confirms[0], /第 2 步 授权（既有受控授权合同）/);
  assert.match(confirms[0], /授权状态 = 允许/);
  assert.match(confirms[0], /只匹配与这些资产完全相同的主机，不包含子域/);
  const confirm = api.take('confirm');
  assert.equal(confirm.args[0].domainRuleScope, 'exact_host');
  const written = [{ assetId: 1, status: 'confirmed', source: 'manual', confidence: 100, authorizationStatus: 'unknown', exposureEligible: false, reason: '人工确认：属于当前主体（仅确认归属，未授权）' }];
  confirm.resolve({ changed: 1, expectations: written, ruleScope: 'exact_host', learnedRules: ['host:a1.example.cn'] });
  await flush();
  const authorize = api.take('authorize');
  assert.deepEqual(authorize.args[2], written, '第 2 步使用第 1 步写入后的结论版本');
  // Subdomain scope must be an explicit choice and is spelled out.
  authorize.resolve(1);
  await flush();
  api.take('summary', 3).resolve(summary());
  api.take('list', 3).resolve(pageOf([asset(3, 2)]));
  await flush();
  gate().toggle(gate().assets[0], true);
  gate().learnDomainRule = true;
  gate().ruleScope = 'subdomains';
  void gate().confirmOwnership(false);
  await flush();
  assert.match(confirms.at(-1), /主机及其子域/);
  assert.match(confirms.at(-1), /共享域名、公共后缀\/托管平台根和 IP 会被拒绝/);
  assert.equal(api.take('confirm').args[0].domainRuleScope, 'subdomains');
});

test('non-granting decisions keep the old contract without any grant prompt', async t => {
  const api = controlledApi();
  const { gate, confirms } = mountGate(t, api, 3);
  await flush();
  await loadProject(api, 3, { prof: profile(3), items: [asset(3, 2)] });
  gate().toggle(gate().assets[0], true);
  void gate().decide('excluded');
  await flush();
  assert.equal(confirms.length, 0);
  const update = api.take('update');
  assert.equal(update.args[1], 'excluded');
  assert.equal(update.args[4], 3);
});

test('switching projects clears the selection and blocks decisions until the new queue loads', async t => {
  const api = controlledApi();
  const { host, gate } = mountGate(t, api, 3);
  await flush();
  await loadProject(api, 3, { prof: profile(3), items: [asset(3, 1)] });
  gate().toggle(gate().assets[0], true);
  assert.equal(gate().selectedRows.length, 1);
  host.projectId = 15;
  await flush();
  assert.equal(gate().selectedRows.length, 0, '切换项目必须清空旧选择');
  assert.equal(gate().queueReady, false);
  // Even a forged selection of an old row is refused before the new queue is ready.
  gate().toggle(asset(3, 1), true);
  await gate().decide('excluded');
  assert.equal(api.count('update'), 0);
  // And once project 15 is loaded, an old-project row in the selection is still refused.
  await loadProject(api, 15, { prof: profile(15), items: [asset(15, 40)] });
  gate().toggle(asset(3, 1), true);
  await gate().decide('excluded');
  assert.equal(api.count('update'), 0, '其他项目的选择不得提交');
  assert.equal(gate().selectedRows.length, 0);
});

test('a rejected precondition reloads the queue instead of retrying the write', async t => {
  const api = controlledApi();
  const { gate, notes } = mountGate(t, api, 3);
  await flush();
  await loadProject(api, 3, { prof: profile(3), items: [asset(3, 1)] });
  gate().toggle(gate().assets[0], true);
  void gate().decide('related');
  await flush();
  api.take('update').reject('OWNERSHIP_DECISION_PRECONDITION_FAILED: 批量操作已拒绝，未写入任何结论：0 条已不属于当前工作空间或已删除；0 条已不在归属复核范围；1 条归属结论在页面加载后已变化；0 条缺少页面看到的结论版本。');
  await flush();
  assert.ok(api.open('list', 3), '前置条件失败后应重新加载复核队列');
  assert.equal(api.count('update'), 1, '不得自动重试写入');
  assert.match(notes.at(-1)[1], /页面加载后已变化/);
  assert.equal(gate().selectedRows.length, 0);
});

test('the rendered page is honest about unsaved prefill and the granting button', async () => {
  const api = controlledApi();
  const { withTemplate } = compileGate(api.api);
  const html = await renderToString(vue.createSSRApp(withTemplate, { projects: PROJECTS, selectedProjectId: 3 }));
  assert.match(html, /档案未加载/, '档案未加载完成时必须明说');
  assert.match(html, /disabled[^>]*>[^<]*<!---->[^<]*保存归属档案|保存归属档案/);
  assert.doesNotMatch(html, /undefined|NaN/);
  assert.equal(api.calls.length, 0, 'SSR 渲染不得发出 IPC');
  const source = readSource(GATE);
  assert.doesNotMatch(source, /确认归属并授权/, '逐条“确认归属并授权”已拆分');
  assert.match(source, /title="只写入：确认归属（人工锁定）；授权与暴露面准入保持现状"/);
  assert.match(source, /title="只对已人工确认归属的资产：授权=允许 \+ 暴露面准入=是（不是主动扫描授权）"/);
  // Unsaved prefill must be labelled as unsaved and must not present the project title as a legal name.
  assert.match(source, /<template v-else>尚未保存（[^<]*(不一定是法人名称|项目标题不作为法人名称)[^<]*）<\/template>/);
  assert.doesNotMatch(source, /saveAssetOwnershipProfile\([^)]*\)\s*;?\s*$/m, '保存必须携带 expectedVersion');
});

test('a successful save or decision asks the project setup card to refresh; a rejected one does not', async t => {
  const api = controlledApi();
  const { gate } = mountGate(t, api, 3);
  await flush();
  await loadProject(api, 3, { prof: profile(3), items: [asset(3, 1)] });
  assert.equal(gate().setupRefresh, 0);
  gate().form.legalName = '中国移动通信集团有限公司';
  void gate().saveProfile();
  await flush();
  api.take('save', 3).reject('OWNERSHIP_PROFILE_VERSION_CONFLICT: 归属档案已被其他操作更新');
  await flush();
  assert.equal(gate().setupRefresh, 0, 'a rejected save changed nothing on the server');
  api.take('profile', 3).resolve(profile(3));
  await flush();
  void gate().saveProfile();
  await flush();
  api.take('save', 3).resolve(profile(3, { legalName: '中国移动通信集团有限公司', updatedAt: '2026-10-08 18:00:00' }));
  await flush();
  assert.equal(gate().setupRefresh, 1);
  gate().toggle(gate().assets[0], true);
  void gate().decide('related');
  await flush();
  api.take('update').reject('OWNERSHIP_DECISION_PRECONDITION_FAILED: x');
  await flush();
  assert.equal(gate().setupRefresh, 1, 'a rejected decision changed no grant');
  api.take('summary', 3).resolve(summary());
  api.take('list', 3).resolve(pageOf([asset(3, 1)]));
  await flush();
  gate().toggle(gate().assets[0], true);
  void gate().decide('related');
  await flush();
  api.take('update').resolve(1);
  await flush();
  assert.equal(gate().setupRefresh, 2);
});
