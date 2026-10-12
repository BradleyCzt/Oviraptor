// §O2 — the real `OwnershipReviewGroups.vue` (script + template) is compiled and mounted on an in-memory
// renderer with only the IPC boundary faked; every response is released by hand so tests can reorder them.
// Covers docs/NEST_OWNERSHIP_GATE_SMART_GROUP_REVIEW_DESIGN_2026-10-08.md §7/§8/§10（决策组队列、cursor 分页、
// 组批量只提交 groupId、operationId 幂等重试、预览失效、取消、项目切换丢弃过期响应）.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');

const FILE = 'src/components/OwnershipReviewGroups.vue';
const readSource = relative => fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');
const transpile = source => ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } }).outputText;
const LANES = { conflict: '证据冲突', counter_evidence: '存在反证', consistent: '规则一致', insufficient: '证据不足', unknown: '未知', manual: '已有人工结论' };
const NAVS = { unknown: '未知', province: '省公司' };

function compileComponent(api) {
  const requireModule = name => {
    if (name === 'vue') return vue;
    if (name === '../features/assets/ownershipDirectoryApi') return { ownershipReviewApi: api, OWNERSHIP_LANE_LABELS: LANES, OWNERSHIP_NAV_LABELS: NAVS };
    throw Error(`Unexpected review groups import: ${name}`);
  };
  const filename = path.join(__dirname, '..', FILE);
  const id = path.basename(filename);
  const { descriptor } = parse(readSource(FILE), { filename });
  const script = compileScript(descriptor, { id });
  const template = compileTemplate({ source: descriptor.template.content, filename, id, scoped: true, compilerOptions: { bindingMetadata: script.bindings, scopeId: `data-v-${id}` } });
  assert.deepEqual(template.errors, [], '决策组队列模板必须能编译');
  const scriptExports = {}; const templateExports = {};
  new Function('require', 'exports', transpile(script.content))(requireModule, scriptExports);
  new Function('require', 'exports', transpile(template.code))(requireModule, templateExports);
  return { ...scriptExports.default, render: templateExports.render };
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
    startOwnershipAnalysis: projectId => call('start', [projectId]),
    readOwnershipAnalysis: projectId => call('read', [projectId]),
    cancelOwnershipAnalysis: (projectId, runId) => call('cancel', [projectId, runId]),
    pageOwnershipGroups: input => call('groups', [input]),
    pageOwnershipGroupMembers: input => call('members', [input]),
    previewOwnershipGroupDecision: input => call('preview', [input]),
    applyOwnershipGroupDecision: input => call('apply', [input]),
    readOwnershipOperation: (projectId, operationId) => call('operation', [projectId, operationId]),
    previewOwnershipGroupRevert: input => call('revertPreview', [input]),
    applyOwnershipGroupRevert: input => call('revert', [input]),
    revokeOwnershipGroupAuthorization: input => call('revoke', [input]),
  };
  const projectOf = row => (typeof row.args[0] === 'object' ? row.args[0].projectId : row.args[0]);
  const take = (name, projectId) => {
    const row = calls.find(item => item.name === name && !item.settled && (projectId === undefined || projectOf(item) === projectId));
    assert.ok(row, `expected an open ${name} call for project ${projectId}; calls: ${JSON.stringify(calls.map(item => [item.name, projectOf(item), item.settled]))}`);
    return row;
  };
  const count = name => calls.filter(row => row.name === name).length;
  const open = name => calls.filter(row => row.name === name && !row.settled).length;
  return { api, calls, take, count, open };
}

const flush = async () => { for (let i = 0; i < 8; i++) await new Promise(resolve => setImmediate(resolve)); };
const VERSIONS = { policyVersion: 2, directoryVersion: 3, bindingVersion: 1, scopeVersion: 'scope-x', algorithmVersion: 'o2-review-groups-v1' };
const run = (id, projectId, over = {}) => ({
  id, projectId, status: 'completed', versions: VERSIONS, dataRevision: 'r', dataRevisionAtFinish: 'r', totalAssets: 30, processedAssets: 30,
  groupCount: 3, memberCount: 30, cancelRequested: false, membersPruned: false, error: '', createdAt: 't0', startedAt: 't0', finishedAt: '2026-10-08 20:00:00', ...over,
});
const queue = over => ({ pendingAssets: 27, pendingGroups: 3, conflictGroups: 1, counterEvidenceGroups: 1, completedGroups: 1, manualAssets: 3, newSinceSnapshot: 4, changedSinceSnapshot: 2, memberTotal: 30, groupMemberSum: 30, groupTotal: 4, ...over });
const overview = (projectId, over = {}) => ({
  projectId, activeRun: null, latestRun: run(7, projectId), snapshot: run(7, projectId), current: VERSIONS, currentDataRevision: 'r', staleReasons: [],
  queue: queue(), readiness: ['rules_only', 'model_not_ready'], ...over,
});
const sample = (assetId, host) => ({ assetId, host, registrableDomain: host, suffixSection: 'icann', company: '甲公司', link: `https://${host}/`, title: '', changedSinceSnapshot: false,
  current: { linked: true, inScope: true, status: 'pending', source: '', authorizationStatus: 'unreviewed', exposureEligible: false, reason: '' } });
const group = (id, lane, over = {}) => ({
  id, runId: 7, lane, navBucket: 'unknown', candidateEntityId: null, candidateLabel: `候选${id}`, candidateSource: 'directory', relationKind: 'self', regionClue: '',
  boundaryKind: 'registrable_domain', boundaryValue: `g${id}.example.com.cn`, supportKind: 'strong', counterKind: '', decisionStatus: 'auto', decisionSource: 'auto',
  authorizationStatus: 'unreviewed', exposureEligible: false, memberCount: 10, distinctHosts: 2, samples: [sample(id * 100, `a.g${id}.example.com.cn`)], sampleRule: 'distinct hosts',
  reason: `理由${id}`, priority: 30, sortRank: id, groupVersion: 1, appliedOperations: 0,
  allowedActions: lane === 'consistent' ? ['confirm_ownership', 'related', 'third_party', 'excluded'] : lane === 'manual' ? [] : ['third_party', 'excluded'], ...over,
});
const groupPage = (projectId, items, over = {}) => ({ projectId, snapshot: run(7, projectId), staleReasons: [], total: 3, items, nextCursor: '', ...over });
const previewOf = (groupId, over = {}) => ({
  projectId: 3, runId: 7, groupId, groupVersion: 1, policyVersion: 2, directoryVersion: 3, bindingVersion: 1, action: 'confirm_ownership', note: '',
  expectedActionHash: 'hash-1', memberCount: 10, applicable: 9, skipped: 1, conflicted: 0, reasons: { manual_decision_locked: 1 }, excludedSamples: [{ assetId: 5, outcome: 'skipped', reason: 'manual_decision_locked' }],
  currentAuthorization: { unreviewed: 10 }, proposed: { status: 'confirmed', source: 'manual', authorization: 'unchanged', exposureEligible: 'unchanged', grantsAuthorization: false, setsExposureEligible: false },
  notes: ['只写归属事实，不授予扫描或暴露面权限'], ...over,
});

function memoryRenderer() {
  const element = tag => ({ tag, children: [], props: {}, options: [], value: '', addEventListener() {}, removeEventListener() {} });
  return vue.createRenderer({
    createElement: element, createText: text => ({ text, children: [] }), createComment: text => ({ comment: text, children: [] }),
    setText: (node, text) => { node.text = text; }, setElementText: (node, text) => { node.children = []; node.text = text; },
    patchProp: (el, key, _prev, value) => { el.props[key] = value; },
    insert: (child, parent, anchor) => { child.parent = parent; const at = anchor ? parent.children.indexOf(anchor) : -1; if (at >= 0) parent.children.splice(at, 0, child); else parent.children.push(child); },
    remove: node => { if (node.parent) node.parent.children = node.parent.children.filter(row => row !== node); },
    parentNode: node => node.parent ?? null,
    nextSibling: node => { if (!node.parent) return null; const list = node.parent.children; return list[list.indexOf(node) + 1] ?? null; },
  });
}
const textOf = node => (node.comment !== undefined ? '' : `${node.text ?? ''}${(node.children ?? []).map(textOf).join('')}`);
const findAll = (node, predicate, out = []) => { if (predicate(node)) out.push(node); for (const child of node.children ?? []) findAll(child, predicate, out); return out; };

function mount(t, harness, projectId = 3) {
  const component = harness.compiled ??= compileComponent(harness.api);
  const notes = []; const applied = [];
  const host = vue.reactive({ projectId, refreshToken: 0 });
  const Root = { setup: () => () => vue.h(component, { projectId: host.projectId, refreshToken: host.refreshToken, onNotify: (type, text) => notes.push([type, text]), onApplied: id => applied.push(id) }) };
  const app = memoryRenderer().createApp(Root);
  const container = { tag: '#root', children: [], props: {} };
  const root = app.mount(container);
  t.after(() => app.unmount());
  const buttons = () => findAll(container, node => node.tag === 'button').map(node => textOf(node));
  return { host, state: () => root.$.subTree.component.setupState, notes, applied, text: () => textOf(container), buttons };
}

async function loaded(api, projectId = 3, items = [group(1, 'conflict', { counterKind: 'entity_mismatch', priority: 10 }), group(2, 'consistent'), group(3, 'manual', { decisionSource: 'manual', decisionStatus: 'confirmed' })], over = {}) {
  api.take('read', projectId).resolve(overview(projectId, over));
  await flush();
  api.take('groups', projectId).resolve(groupPage(projectId, items));
  await flush();
}

test('a late overview or group page for the previous project never fills the next project queue', async t => {
  const api = controlledApi();
  const { host, state } = mount(t, api, 3);
  await flush();
  host.projectId = 15;
  await flush();
  api.take('read', 3).resolve(overview(3, { queue: queue({ pendingAssets: 999 }) }));
  await flush();
  assert.equal(state().overview, null, 'the project 3 overview arrived after the switch and is dropped');
  assert.equal(api.open('groups'), 0, 'no group page is requested for a project no longer shown');
  api.take('read', 15).resolve(overview(15));
  await flush();
  const page15 = api.take('groups', 15);
  host.projectId = 3;
  await flush();
  page15.resolve(groupPage(15, [group(9, 'consistent')]));
  await flush();
  assert.deepEqual(state().groups, [], 'the project 15 page landed after switching back and is dropped');
  api.take('read', 3).resolve(overview(3));
  await flush();
  api.take('groups', 3).resolve(groupPage(3, [group(1, 'consistent')]));
  await flush();
  assert.deepEqual(state().groups.map(row => row.id), [1]);
});

test('groups and members page by backend cursor with the real total, filters restart from the first page', async t => {
  const api = controlledApi();
  const { state, text } = mount(t, api, 3);
  api.take('read', 3).resolve(overview(3));
  await flush();
  const first = api.take('groups', 3);
  assert.deepEqual(first.args[0], { projectId: 3, runId: 7, lane: '', navBucket: '', region: '', pageSize: 20, cursor: '' });
  first.resolve(groupPage(3, [group(1, 'consistent')], { total: 41, nextCursor: 'g7:1' }));
  await flush();
  assert.match(text(), /共 41 组，已加载 1/);
  void state().loadGroups(false);
  await flush();
  const second = api.take('groups', 3);
  assert.equal(second.args[0].cursor, 'g7:1');
  second.resolve(groupPage(3, [group(2, 'consistent')], { total: 41, nextCursor: '' }));
  await flush();
  assert.deepEqual(state().groups.map(row => row.id), [1, 2]);
  assert.equal(state().groupsCursor, '');
  void state().loadMembers(state().groups[0]);
  await flush();
  const members = api.take('members', 3);
  assert.deepEqual(members.args[0], { projectId: 3, runId: 7, groupId: 1, pageSize: 50, cursor: '' });
  members.resolve({ projectId: 3, runId: 7, groupId: 1, groupVersion: 1, total: 10, items: [sample(100, 'a.g1.example.com.cn')], nextCursor: 'm7:1:100' });
  await flush();
  void state().loadMembers(state().groups[0], true);
  await flush();
  const more = api.take('members', 3);
  assert.equal(more.args[0].cursor, 'm7:1:100');
  more.resolve({ projectId: 3, runId: 7, groupId: 1, groupVersion: 1, total: 10, items: [sample(101, 'b.g1.example.com.cn')], nextCursor: '' });
  await flush();
  assert.deepEqual(state().expanded[1].items.map(row => row.assetId), [100, 101]);
  assert.match(text(), /已显示全部 10 条/);
  state().laneFilter = 'conflict';
  state().applyFilters();
  await flush();
  const filtered = api.take('groups', 3);
  assert.equal(filtered.args[0].lane, 'conflict');
  assert.equal(filtered.args[0].cursor, '', 'a new filter restarts from the first page');
});

test('the queue shows counter-evidence, conflicts and the unprocessed increment, and conflict groups offer no confirm', async t => {
  const api = controlledApi();
  const { text, buttons } = mount(t, api, 3);
  await loaded(api);
  const page = text();
  assert.match(page, /待复核资产27/);
  assert.match(page, /冲突组1反证组 1/);
  assert.match(page, /未处理增量6新增 4 · 变化 2/);
  assert.match(page, /反证：entity_mismatch/);
  assert.match(page, /组确认只写归属事实，不授予扫描或暴露面权限/);
  const confirms = buttons().filter(label => /预览确认归属/.test(label));
  assert.equal(confirms.length, 1, 'only the consistent group offers a confirm preview');
  assert.ok(buttons().filter(label => /预览第三方服务/.test(label)).length === 2, 'conflict groups still allow narrowing actions');
  assert.match(page, /已有人工结论：请在下方资产明细中逐条修改/);
});

test('apply submits only groupId with the preview versions and hash, and a network retry reuses the operationId', async t => {
  const api = controlledApi();
  const { state, notes, applied, text } = mount(t, api, 3);
  await loaded(api);
  void state().previewDecision(state().groups[1], 'confirm_ownership');
  await flush();
  const previewCall = api.take('preview', 3);
  assert.deepEqual(previewCall.args[0], { projectId: 3, runId: 7, groupId: 2, action: 'confirm_ownership', note: '' });
  previewCall.resolve(previewOf(2));
  await flush();
  assert.match(text(), /授权 = 保持各资产现状；暴露面准入 = 保持各资产现状/);
  const operationId = state().operationId;
  assert.ok(operationId.length >= 8);
  void state().applyDecision();
  await flush();
  const firstApply = api.take('apply', 3);
  assert.deepEqual(Object.keys(firstApply.args[0]).sort(), ['action', 'bindingVersion', 'directoryVersion', 'expectedActionHash', 'groupId', 'groupVersion', 'note', 'operationId', 'policyVersion', 'projectId', 'runId']);
  assert.ok(!JSON.stringify(firstApply.args[0]).includes('assetId'), 'no asset id list is ever sent');
  assert.deepEqual({ ...firstApply.args[0], operationId: 'x' }, { operationId: 'x', projectId: 3, runId: 7, groupId: 2, groupVersion: 1, policyVersion: 2, directoryVersion: 3, bindingVersion: 1, action: 'confirm_ownership', note: '', expectedActionHash: 'hash-1' });
  firstApply.reject('IPC timeout');
  await flush();
  assert.ok(state().preview, 'a network failure keeps the preview so the same operation can be retried');
  assert.match(notes.at(-1)[1], /同一操作编号重复提交不会重复写入/);
  void state().applyDecision();
  await flush();
  const retry = api.take('apply', 3);
  assert.equal(retry.args[0].operationId, operationId, 'retry reuses the operationId');
  retry.resolve({ operationId, projectId: 3, runId: 7, groupId: 2, action: 'confirm_ownership', applied: 9, skipped: 1, conflicted: 0, failed: 0, reasons: { manual_decision_locked: 1 }, groupVersion: 2, replayed: true });
  await flush();
  assert.equal(state().preview, null);
  assert.deepEqual(applied, [3]);
  assert.match(notes.at(-1)[1], /写入 9，跳过 1，冲突 0，失败 0（重复提交，未再次写入）/);
  assert.ok(api.take('read', 3), 'the queue reloads the snapshot after an apply');
});

test('a stale preview is cleared and the next preview gets a new operationId', async t => {
  const api = controlledApi();
  const { state, notes } = mount(t, api, 3);
  await loaded(api);
  void state().previewDecision(state().groups[1], 'related');
  await flush();
  api.take('preview', 3).resolve(previewOf(2, { action: 'related' }));
  await flush();
  const firstId = state().operationId;
  void state().applyDecision();
  await flush();
  api.take('apply', 3).reject('OWNERSHIP_PREVIEW_STALE: 组成员或其当前状态已在预览后变化，请重新预览');
  await flush();
  assert.equal(state().preview, null, 'the stale preview is dropped');
  assert.equal(state().operationId, '');
  assert.match(notes.at(-1)[1], /请重新预览/);
  api.take('read', 3).resolve(overview(3));
  await flush();
  api.take('groups', 3).resolve(groupPage(3, [group(2, 'consistent', { groupVersion: 2 })]));
  await flush();
  void state().previewDecision(state().groups[0], 'related');
  await flush();
  api.take('preview', 3).resolve(previewOf(2, { action: 'related', groupVersion: 2, expectedActionHash: 'hash-2' }));
  await flush();
  assert.notEqual(state().operationId, firstId, 'a fresh preview is a new operation');
  void state().applyDecision();
  await flush();
  const apply = api.take('apply', 3);
  assert.equal(apply.args[0].groupVersion, 2);
  assert.equal(apply.args[0].expectedActionHash, 'hash-2');
});

test('a late apply result for the previous project neither updates nor reloads the shown project', async t => {
  const api = controlledApi();
  const { host, state, applied } = mount(t, api, 3);
  await loaded(api);
  void state().previewDecision(state().groups[1], 'excluded');
  await flush();
  api.take('preview', 3).resolve(previewOf(2, { action: 'excluded' }));
  await flush();
  void state().applyDecision();
  await flush();
  const pending = api.take('apply', 3);
  host.projectId = 15;
  await flush();
  assert.equal(state().preview, null, 'switching projects clears the preview');
  pending.resolve({ operationId: 'x', projectId: 3, runId: 7, groupId: 2, action: 'excluded', applied: 9, skipped: 1, conflicted: 0, failed: 0, reasons: {}, groupVersion: 2, replayed: false });
  await flush();
  assert.deepEqual(applied, [], 'the gate is not told to refresh a project it no longer shows');
  assert.equal(api.count('read'), 2, 'only the project 15 overview was requested');
  assert.equal(state().lastResult, null);
});

test('start and cancel go through the backend run; an active run polls and shows the old snapshot', async t => {
  const api = controlledApi();
  const { state, text, buttons } = mount(t, api, 3);
  api.take('read', 3).resolve(overview(3, { snapshot: null, latestRun: null, queue: null, readiness: ['no_snapshot'] }));
  await flush();
  assert.match(text(), /没有快照不代表没有待办/);
  void state().startAnalysis();
  await flush();
  api.take('start', 3).resolve(run(8, 3, { status: 'running', processedAssets: 0 }));
  await flush();
  api.take('read', 3).resolve(overview(3, { activeRun: run(8, 3, { status: 'running', processedAssets: 500, totalAssets: 4000 }) }));
  await flush();
  api.take('groups', 3).resolve(groupPage(3, [group(2, 'consistent')]));
  await flush();
  assert.match(text(), /已处理 500 \/ 4,000 条 · 全量；下方仍为旧快照 #7/);
  assert.ok(buttons().includes('取消分析'));
  void state().cancelAnalysis();
  await flush();
  const cancel = api.take('cancel', 3);
  assert.deepEqual(cancel.args, [3, 8]);
  cancel.resolve(run(8, 3, { status: 'cancelled' }));
  await flush();
  api.take('read', 3).resolve(overview(3, { latestRun: run(8, 3, { status: 'cancelled', processedAssets: 500 }) }));
  await flush();
  assert.match(text(), /最近一次分析 #8 已取消（已处理 500 条，未发布快照）/);
});

// ---------------------------------------------------------------- O3
test('polling backs off without progress, resets on progress and backs off harder on errors', async t => {
  const api = controlledApi();
  const { state } = mount(t, api, 3);
  const { nextPollDelay } = state();
  assert.equal(nextPollDelay(1000, false, false), 1600);
  assert.equal(nextPollDelay(8000, false, false), 10000, 'idle backoff caps at 10s');
  assert.equal(nextPollDelay(6400, true, false), 1000, 'progress resets to 1s');
  assert.equal(nextPollDelay(1000, false, true), 2000);
  assert.equal(nextPollDelay(20000, false, true), 30000, 'error backoff caps at 30s');
  const active = processedAssets => overview(3, { activeRun: run(8, 3, { status: 'running', processedAssets, totalAssets: 4000 }) });
  api.take('read', 3).resolve(active(500));
  await flush();
  api.take('groups', 3).resolve(groupPage(3, [group(2, 'consistent')]));
  await flush();
  assert.equal(state().pollDelay, 1000, 'first observation is progress');
  for (const expected of [1600, 2560, 4096]) {
    void state().loadOverview();
    await flush();
    api.take('read', 3).resolve(active(500));
    await flush();
    assert.equal(state().pollDelay, expected, 'no progress: back off');
  }
  void state().loadOverview();
  await flush();
  api.take('read', 3).resolve(active(1000));
  await flush();
  assert.equal(state().pollDelay, 1000, 'progress resets the delay');
  void state().loadOverview();
  await flush();
  api.take('read', 3).reject('IPC timeout');
  await flush();
  assert.equal(state().pollDelay, 2000, 'a failed read backs off instead of hammering');
});

test('a large group apply shows the real chunk progress and publishes the result only when the operation completes', async t => {
  const api = controlledApi();
  const { state, text, applied } = mount(t, api, 3);
  await loaded(api);
  void state().previewDecision(state().groups[1], 'confirm_ownership');
  await flush();
  api.take('preview', 3).resolve(previewOf(2, { memberCount: 4000, applicable: 4000, skipped: 0, reasons: {}, excludedSamples: [] }));
  await flush();
  const operationId = state().operationId;
  void state().applyDecision();
  await flush();
  api.take('apply', 3).resolve({ operationId, projectId: 3, runId: 7, groupId: 2, action: 'confirm_ownership', applied: 500, skipped: 0, conflicted: 0, failed: 0, reasons: {}, groupVersion: 1, replayed: false, status: 'applying', totalItems: 4000, doneItems: 500 });
  await flush();
  assert.match(text(), /分块写入中：500 \/ 4,000/);
  assert.match(text(), /完成前不发布整组结果/);
  assert.deepEqual(applied, [], 'nothing is announced as applied while chunks remain');
  assert.equal(state().lastResult, null);
  void state().pollOperation();
  await flush();
  const poll = api.take('operation', 3);
  assert.deepEqual(poll.args, [3, operationId]);
  poll.resolve({ operationId, projectId: 3, runId: 7, groupId: 2, kind: 'group_decision', action: 'confirm_ownership', status: 'completed', targetOperationId: '', totalItems: 4000, doneItems: 4000, applied: 3998, skipped: 0, conflicted: 2, failed: 0, reasons: { decision_changed: 2 }, resumeCount: 1, error: '', createdAt: '', updatedAt: '', ownedByLiveWorker: false });
  await flush();
  assert.equal(state().pendingOperation, null);
  assert.equal(state().lastResult.applied, 3998);
  assert.deepEqual(applied, [3]);
  assert.ok(api.take('read', 3), 'the queue reloads once the operation completes');
});

test('a group operation can be reverted after a preview that says permissions are not restored', async t => {
  const api = controlledApi();
  const { state, text } = mount(t, api, 3);
  await loaded(api);
  state().lastResult = { operationId: 'op-1', projectId: 3, runId: 7, groupId: 2, action: 'excluded', applied: 9, skipped: 1, conflicted: 0, failed: 0, reasons: {}, groupVersion: 2, replayed: false, status: 'completed' };
  await flush();
  void state().previewRevert();
  await flush();
  const preview = api.take('revertPreview', 3);
  assert.deepEqual(preview.args[0], { projectId: 3, targetOperationId: 'op-1', note: '' });
  preview.resolve({ projectId: 3, targetOperationId: 'op-1', targetAction: 'excluded', runId: 7, groupId: 2, note: '组复核：撤销此前的组操作', appliedInTarget: 9, revertible: 8, conflicted: 1, reasons: { changed_after_operation: 1 }, authorizationNotRestored: 1, expectedRevertHash: 'rh-1', notes: [] });
  await flush();
  assert.match(text(), /可恢复 8 条；之后已被修改、保留现状 1 条（操作之后已被修改（保留现状） 1）/);
  assert.match(text(), /撤销不会恢复任何“允许\/准入”/);
  const revertId = state().revertOperationId;
  assert.ok(revertId.length >= 8);
  void state().applyRevert();
  await flush();
  const apply = api.take('revert', 3);
  assert.deepEqual(apply.args[0], { operationId: revertId, projectId: 3, targetOperationId: 'op-1', note: '组复核：撤销此前的组操作', expectedRevertHash: 'rh-1' });
});

test('revoking a group authorization asks first, explains block-then-display, and sends only the group', async t => {
  const api = controlledApi();
  const { state, text, buttons, notes } = mount(t, api, 3);
  await loaded(api, 3, [group(4, 'manual', { decisionSource: 'manual', decisionStatus: 'confirmed', authorizationStatus: 'allowed', exposureEligible: true }), group(5, 'consistent')]);
  assert.equal(buttons().filter(label => label === '撤销授权').length, 1, 'only groups that carry an authorization offer revoke');
  state().askRevoke(state().groups[0]);
  await flush();
  assert.match(text(), /先阻断，再更新本页显示/);
  const revokeId = state().revokeOperationId;
  void state().applyRevoke();
  await flush();
  const revoke = api.take('revoke', 3);
  assert.deepEqual(revoke.args[0], { operationId: revokeId, projectId: 3, runId: 7, groupId: 4, note: '' });
  revoke.resolve({ operationId: revokeId, projectId: 3, runId: 7, groupId: 4, status: 'completed', revoked: 3, alreadyRevoked: 0, withoutDecision: 0, groupVersion: 2, replayed: false });
  await flush();
  assert.match(notes.at(-1)[1], /已撤销授权 3 条（已阻断下游暴露面）/);
});

test('the snapshot line tells incremental from full runs and shows crash takeovers', async t => {
  const api = controlledApi();
  const { text } = mount(t, api, 3);
  await loaded(api, 3, [group(2, 'consistent')], { snapshot: run(7, 3, { runKind: 'incremental', baseRunId: 6, modeReason: 'base_run:6', reclassifiedAssets: 3, carriedAssets: 27, resumeCount: 1 }) });
  assert.match(text(), /增量（基于快照 #6，重新分类 3 · 沿用 27） · 崩溃\/重启后接管 1 次/);
});
