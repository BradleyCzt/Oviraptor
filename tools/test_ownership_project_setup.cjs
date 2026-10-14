// §O1 — the real `OwnershipProjectSetup.vue` is compiled and mounted with only the IPC boundary
// faked; every response is released by hand so tests can reorder them. Covers
// docs/NEST_OWNERSHIP_GATE_SMART_GROUP_REVIEW_DESIGN_2026-10-08.md §4（预填优先级、草稿、CAS、项目切换）.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');

const SETUP = 'src/features/assets/components/OwnershipProjectSetup.vue';
const readSource = relative => fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');
const transpile = source => ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } }).outputText;

function compileSetup(api) {
  const requireModule = name => {
    if (name === 'vue') return vue;
    if (name === '../ownershipDirectoryApi') return { ownershipDirectoryApi: api, ORG_ENTITY_KIND_LABELS: { group: '集团', province_company: '省公司', professional_subsidiary: '专业子公司', unknown: '类型未知' }, SUGGESTION_STATE_LABELS: {} };
    throw Error(`Unexpected project setup import: ${name}`);
  };
  const filename = path.join(__dirname, '..', SETUP);
  const id = path.basename(filename);
  const { descriptor } = parse(readSource(SETUP), { filename });
  const script = compileScript(descriptor, { id });
  const template = compileTemplate({ source: descriptor.template.content, filename, id, scoped: true, compilerOptions: { bindingMetadata: script.bindings, scopeId: `data-v-${id}` } });
  assert.deepEqual(template.errors, [], '项目归属设置模板必须能编译');
  const scriptExports = {};
  new Function('require', 'exports', transpile(script.content))(requireModule, scriptExports);
  return { ...scriptExports.default, render: () => null };
}

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
    getProjectOwnershipBootstrap: projectId => call('bootstrap', [projectId]),
    previewProjectOwnershipSuggestions: projectId => call('preview', [projectId]),
    saveProjectOwnershipDirectory: input => call('save', [input]),
  };
  const projectOf = row => (row.name === 'save' ? row.args[0].projectId : row.args[0]);
  const take = (name, projectId) => {
    const row = calls.find(item => item.name === name && !item.settled && (projectId === undefined || projectOf(item) === projectId));
    assert.ok(row, `expected an open ${name} call for project ${projectId}; calls: ${JSON.stringify(calls.map(item => [item.name, projectOf(item), item.settled]))}`);
    return row;
  };
  const count = name => calls.filter(row => row.name === name).length;
  return { api, calls, take, count };
}

const flush = async () => { for (let i = 0; i < 5; i++) await new Promise(resolve => setImmediate(resolve)); };
const NAMES = { 3: '中国移动历史资产', 15: '香港移动' };
const bootstrap = (projectId, over = {}) => ({
  projectId, projectName: NAMES[projectId], profile: {}, profileSaved: false, profilePrefillSources: {}, binding: null,
  directoryVersion: 0, entities: [], entitiesTruncated: false, scopeEntityIds: [], authorization: { allowed: 0, exposureEligible: 0 },
  dataRevision: 'rows=1', readiness: ['rules_only', 'model_not_ready', 'directory_unbound'], ...over,
});
const suggestion = (over) => ({
  entityId: null, entityKind: '', parentEntityId: null, parentHint: '', regionClue: '', locationClue: '', source: 'company_label',
  supportRows: 1, conflictRows: 0, examplesRef: [1], reason: '', directoryVersion: 0, policyVersion: 0, dataRevision: 'rows=1', ...over,
});
const PREVIEW = {
  projectId: 3, directoryVersion: 0, policyVersion: 0, dataRevision: 'rows=4', scopeRows: 4, unlabeledRows: 0, labelCount: 4, truncatedLabels: 0, readiness: [],
  suggestions: [
    suggestion({ key: 'legal-name:title', field: 'legal_name', value: '中国移动历史资产', state: 'pending' }),
    suggestion({ key: 'group-prefix:中国移动通信集团', field: 'group', value: '中国移动通信集团', entityKind: 'group', state: 'pending', supportRows: 3 }),
    suggestion({ key: 'label:中国移动通信集团浙江有限公司', field: 'member_entity', value: '中国移动通信集团浙江有限公司', entityKind: 'province_company', regionClue: '浙江', state: 'pending' }),
    suggestion({ key: 'label:中移(苏州)软件技术有限公司', field: 'member_entity', value: '中移（苏州）软件技术有限公司', entityKind: 'professional_subsidiary', locationClue: '苏州', state: 'pending' }),
    suggestion({ key: 'label:161.81.120.0/22', field: 'network_clue', value: '161.81.120.0/22', state: 'unknown' }),
    suggestion({ key: 'label:conflict', field: 'member_entity', value: '同名公司', state: 'conflict' }),
    suggestion({ key: 'label:cmi.chinamobile.com', field: 'domain_clue', value: 'cmi.chinamobile.com', state: 'unknown' }),
  ],
};

function memoryRenderer() {
  return vue.createRenderer({
    createElement: tag => ({ tag, children: [], props: {} }), createText: text => ({ text }), createComment: text => ({ text }),
    setText: (node, text) => { node.text = text; }, setElementText: (node, text) => { node.text = text; },
    patchProp: (el, key, _prev, value) => { el.props[key] = value; },
    insert: (child, parent) => { child.parent = parent; parent.children.push(child); },
    remove: node => { if (node.parent) node.parent.children = node.parent.children.filter(row => row !== node); },
    parentNode: node => node.parent ?? null, nextSibling: () => null,
  });
}

function mountSetup(t, harness, projectId = 3) {
  const component = harness.compiled ??= compileSetup(harness.api);
  const notes = []; const saved = []; const prompts = [];
  const host = vue.reactive({ projectId, refreshToken: 0, mounted: true, promptAnswer: '中国移动通信集团有限公司', confirmAnswer: true });
  globalThis.window = { prompt: (message, value) => { prompts.push([message, value]); return host.promptAnswer; }, confirm: () => host.confirmAnswer };
  const Root = { setup: () => () => (host.mounted ? vue.h(component, { projectId: host.projectId, projectName: NAMES[host.projectId], refreshToken: host.refreshToken, onNotify: (type, text) => notes.push([type, text]), onDirectorySaved: id => saved.push(id) }) : null) };
  const app = memoryRenderer().createApp(Root);
  const root = app.mount({ tag: '#root', children: [], props: {} });
  t.after(() => { app.unmount(); delete globalThis.window; });
  return { host, state: () => root.$.subTree.component.setupState, notes, saved, prompts };
}

async function openSuggestions(api, state, projectId = 3, preview = PREVIEW) {
  api.take('bootstrap', projectId).resolve(bootstrap(projectId));
  await flush();
  state().toggleSuggestions();
  api.take('preview', projectId).resolve({ ...preview, projectId });
  await flush();
}
const item = (state, key) => state().preview.suggestions.find(row => row.key === key);

test('a late bootstrap for the previous project never fills the next project card', async t => {
  const api = controlledApi();
  const { host, state } = mountSetup(t, api, 3);
  await flush();
  host.projectId = 15;
  await flush();
  api.take('bootstrap', 15).resolve(bootstrap(15));
  api.take('bootstrap', 3).resolve(bootstrap(3, { directoryVersion: 9 }));
  await flush();
  assert.equal(state().bootstrap.projectId, 15);
  assert.equal(state().bootstrap.directoryVersion, 0);
});

test('accepting suggestions only builds a draft; title, network clues and conflicts are not acceptable', async t => {
  const api = controlledApi();
  const { state, prompts } = mountSetup(t, api, 3);
  await openSuggestions(api, state);
  for (const key of ['legal-name:title', 'label:161.81.120.0/22', 'label:conflict', 'label:cmi.chinamobile.com']) assert.equal(state().canAccept(item(state, key)), false, key);
  assert.match(state().acceptBlockedReason(item(state, 'legal-name:title')), /不能直接作为法人名称/);
  assert.match(state().acceptBlockedReason(item(state, 'label:中国移动通信集团浙江有限公司')), /先确认集团/);
  state().accept(item(state, 'group-prefix:中国移动通信集团'));
  assert.equal(prompts.length, 1, 'a label prefix is not a legal name: the user types the official name');
  state().accept(item(state, 'label:中国移动通信集团浙江有限公司'));
  state().accept(item(state, 'label:中移(苏州)软件技术有限公司'));
  await flush();
  const draft = state().draft;
  assert.equal(api.count('save'), 0, 'accepting never writes');
  assert.deepEqual(draft.entities.map(row => [row.legalName, row.entityKind, row.parentKey]), [
    ['中国移动通信集团有限公司', 'group', ''],
    ['中国移动通信集团浙江有限公司', 'province_company', `draft:${draft.entities[0].draftKey}`],
    ['中移（苏州）软件技术有限公司', 'professional_subsidiary', `draft:${draft.entities[0].draftKey}`],
  ], 'the Suzhou subsidiary hangs under the group, never under a province company');
  assert.equal(draft.binding.groupKey, `draft:${draft.entities[0].draftKey}`);
  assert.equal(state().canAccept(item(state, 'label:中国移动通信集团浙江有限公司')), false, 'already in draft');
});

test('save is a CAS on the versions the draft was based on, and a conflict keeps the draft', async t => {
  const api = controlledApi();
  const { state, saved, notes } = mountSetup(t, api, 3);
  await openSuggestions(api, state);
  state().setBinding({ projectKind: 'group_ledger' });
  state().accept(item(state, 'group-prefix:中国移动通信集团'));
  state().accept(item(state, 'label:中国移动通信集团浙江有限公司'));
  await flush();
  void state().saveDirectory();
  await flush();
  const first = api.take('save', 3);
  const input = first.args[0];
  assert.equal(input.expectedDirectoryVersion, 0);
  assert.equal(input.expectedBindingVersion, 0);
  assert.equal(input.entities[0].entityKind, 'group');
  assert.deepEqual(input.entities[1].parent, { draftKey: input.entities[0].draftKey });
  assert.deepEqual(input.binding, { projectKind: 'group_ledger', group: { draftKey: input.entities[0].draftKey }, unit: undefined });
  first.reject('ORG_DIRECTORY_VERSION_CONFLICT: 组织目录已被其他操作更新（当前 v1，页面基于 v0），本次未写入');
  await flush();
  assert.equal(state().draft.entities.length, 2, '冲突不得丢弃草稿');
  assert.match(state().conflict, /VERSION_CONFLICT/);
  api.take('bootstrap', 3).resolve(bootstrap(3, { directoryVersion: 1 }));
  await flush();
  assert.equal(state().draft.baseDirectoryVersion, 0, 'reloading the latest directory does not silently rebase the draft');
  // Saving again without an explicit rebase still sends the version the draft was built on, not the latest one.
  void state().saveDirectory();
  await flush();
  const retry = api.take('save', 3);
  assert.equal(retry.args[0].expectedDirectoryVersion, 0, 'the CAS base is the draft version, never the freshly loaded one');
  assert.equal(retry.args[0].expectedBindingVersion, 0);
  retry.reject('ORG_DIRECTORY_VERSION_CONFLICT: 组织目录已被其他操作更新（当前 v1，页面基于 v0），本次未写入');
  await flush();
  assert.equal(state().draft.entities.length, 2);
  api.take('bootstrap', 3).resolve(bootstrap(3, { directoryVersion: 1 }));
  await flush();
  state().rebaseDraft();
  assert.equal(state().draft.baseDirectoryVersion, 1);
  void state().saveDirectory();
  await flush();
  const second = api.take('save', 3);
  assert.equal(second.args[0].expectedDirectoryVersion, 1);
  second.resolve({ projectId: 3, directoryVersion: 2, bindingVersion: 1, createdEntities: [], createdIdentifiers: 0 });
  await flush();
  assert.equal(state().draft, null);
  assert.deepEqual(saved, [3]);
  assert.ok(notes.some(([type, text]) => type === 'success' && /不授予任何扫描或暴露面权限/.test(text)));
  assert.ok(api.take('bootstrap', 3), 'the card reloads the saved directory');
});

test('drafts are kept per project and a save that lands after switching stays with its project', async t => {
  const api = controlledApi();
  const { host, state, saved } = mountSetup(t, api, 3);
  await openSuggestions(api, state);
  state().accept(item(state, 'group-prefix:中国移动通信集团'));
  await flush();
  host.projectId = 15;
  await flush();
  assert.equal(state().draft, null, '项目 15 不得看到项目 3 的草稿');
  api.take('bootstrap', 15).resolve(bootstrap(15));
  await flush();
  host.projectId = 3;
  await flush();
  assert.equal(state().draft.entities[0].legalName, '中国移动通信集团有限公司', '切回项目 3 恢复其草稿');
  api.take('bootstrap', 3).resolve(bootstrap(3));
  await flush();
  void state().saveDirectory();
  await flush();
  const pending = api.take('save', 3);
  host.projectId = 15;
  await flush();
  api.take('bootstrap', 15).resolve(bootstrap(15, { directoryVersion: 0 }));
  await flush();
  pending.resolve({ projectId: 3, directoryVersion: 1, bindingVersion: 1, createdEntities: [], createdIdentifiers: 1 });
  await flush();
  assert.equal(state().bootstrap.projectId, 15, 'the late save result does not touch the project 15 card');
  assert.equal(api.count('bootstrap'), 4, 'no reload is issued for a project that is no longer shown');
  assert.deepEqual(saved, [3]);
  host.projectId = 3;
  await flush();
  assert.equal(state().draft, null, 'the saved draft of project 3 is cleared');
});

test('batch accept drafts only rows that single accept would allow, and never writes', async t => {
  const api = controlledApi();
  const { state } = mountSetup(t, api, 3);
  const preview = { ...PREVIEW, suggestions: [...PREVIEW.suggestions,
    suggestion({ key: 'label:浙江移动', field: 'alias', value: '浙江移动', state: 'pending' }),
    suggestion({ key: 'label:已确认单位', field: 'member_entity', value: '已确认单位', entityId: 7, entityKind: 'province_company', state: 'directory_confirmed' }),
  ] };
  await openSuggestions(api, state, 3, preview);
  state().acceptAll('member_entity');
  await flush();
  assert.equal(state().draft, null, 'without a confirmed group no member unit can be drafted');
  for (const field of ['legal_name', 'network_clue', 'domain_clue', 'group']) state().acceptAll(field);
  await flush();
  assert.equal(state().draft, null, 'title, network/domain clues and group names are never batch-accepted');
  state().accept(item(state, 'group-prefix:中国移动通信集团'));
  state().acceptAll('member_entity');
  state().acceptAll('member_entity');
  state().acceptAll('alias');
  await flush();
  const draft = state().draft;
  assert.deepEqual(draft.entities.map(row => row.legalName), ['中国移动通信集团有限公司', '中国移动通信集团浙江有限公司', '中移（苏州）软件技术有限公司'],
    'conflict and already-confirmed directory rows are skipped; repeating the batch adds nothing');
  assert.deepEqual(draft.identifiers.map(row => [row.kind, row.value, row.targetKey]), [['brand', '浙江移动', '']], 'an alias still needs the user to pick its unit');
  assert.ok(draft.entities.slice(1).every(row => row.parentKey === `draft:${draft.entities[0].draftKey}`), 'members hang under the group, never under a place-name province');
  assert.equal(api.count('save'), 0, 'batch accept only builds the draft');
});

test('a refresh from the gate reloads the card summary but keeps the directory draft', async t => {
  const api = controlledApi();
  const { host, state } = mountSetup(t, api, 3);
  await openSuggestions(api, state);
  state().accept(item(state, 'group-prefix:中国移动通信集团'));
  await flush();
  host.refreshToken++;
  await flush();
  const entity = (id, identifiers) => ({ id, groupId: 1, parentId: 1, entityKind: 'province_company', legalName: `单位${id}`, regionCode: '', unifiedCode: '', source: 'manual', sourceRef: '', directoryVersion: 1, identifiers: identifiers.map((kind, index) => ({ id: id * 10 + index, kind, value: `${kind}.example.cn`, source: 'manual', sourceRef: '' })) });
  api.take('bootstrap', 3).resolve(bootstrap(3, {
    profileSaved: true, readiness: ['rules_only', 'model_not_ready'], authorization: { allowed: 2, exposureEligible: 2 },
    entities: [entity(2, ['official_domain', 'brand']), entity(3, ['shared_domain'])], scopeEntityIds: [2],
  }));
  await flush();
  assert.equal(state().bootstrap.authorization.allowed, 2, 'the card shows the current project grant counts after a decision');
  assert.equal(state().scopeDomainCount, 1, 'only directory domains of entities in this project scope are counted');
  assert.equal(state().draft.entities.length, 1, 'refreshing never drops the directory draft');
  assert.equal(api.count('bootstrap'), 2);
  host.projectId = 15;
  await flush();
  host.refreshToken++;
  await flush();
  assert.equal(api.count('bootstrap'), 3, 'no refresh is issued before the new project card is loaded');
});
