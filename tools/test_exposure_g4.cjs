// G4 — frontend honesty tests for incidents, coverage, live progress and exports: a coverage
// gap is never green, a missing projection reads 未报告 rather than 0, live progress has no
// fake percent, the incident panel says integration_pending (IR-15) instead of "no incidents"
// while the commands are unregistered, every human action carries revision + operation id +
// reason, a retry reuses its operation id, handling is recorded only, and the export line
// reports the digest read back from the file. Pure modules are transpiled and run.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');

const readSource = relative => fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');
const transpile = source => ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
const loadModule = (relative, requireModule) => {
  const exports = {};
  new Function('require', 'exports', transpile(readSource(relative)))(requireModule, exports);
  return exports;
};
const E = 'src/features/assets/exposure/';
const contracts = () => loadModule(E + 'contracts.ts', () => { throw Error('unexpected'); });
const incidents = () => loadModule(E + 'incidents.ts', () => { throw Error('unexpected'); });
const composable = () => {
  const c = contracts();
  const i = incidents();
  return loadModule(E + 'useExposureIncidents.ts', name => ({ vue, './contracts': c, './incidents': i })[name] ?? (() => { throw Error(name); })());
};
const flush = () => new Promise(resolve => setImmediate(resolve));

test('g4 a coverage cell is green only for executed_no_hit and unknown states stay grey', () => {
  const m = incidents();
  assert.equal(m.coverageCellView({ infoCategory: 'credential', state: 'executed_no_hit', tone: 'green' }).tone, 'green');
  for (const state of ['not_run', 'unconfigured', 'not_implemented', 'failed', 'partial', 'cancelled_interrupted', 'pending_authorization', 'planned', 'not_applicable', 'weird']) {
    const view = m.coverageCellView({ infoCategory: 'credential', state, tone: 'green' });
    assert.notEqual(view.tone, 'green', `${state} 即使后端误给 green 也不能显示绿色`);
  }
  assert.equal(m.coverageCellView(null).label, '未报告');
  assert.match(m.coverageCellView({ state: 'brand_new' }).label, /未知状态/);
  const rows = m.coverageRows({
    rows: [{ capability: 'archive_index', state: 'partial', reason: 'x', integration: '', denominator: { unit: '资产', planned: 10, executed: 6, failed: 2, blockedQuota: 1, outsidePlan: 4 }, cells: [{ infoCategory: 'document', state: 'partial', tone: 'amber' }] },
      { capability: 'file_import', state: 'not_implemented', integration: 'integration_pending', denominator: {}, cells: [] }],
  });
  assert.match(rows[0].denominator, /计划 10 资产 · 已执行 6 · 未覆盖 4 · 失败 2 · 配额阻塞 1 · 另有 4 个超出本轮上限未计划/);
  assert.equal(rows[1].integrationPending, true);
  assert.match(m.coverageNotice(null, 'boom'), /不是“全部覆盖”/);
  assert.match(m.coverageNotice(null, ''), /integration_pending/);
  assert.match(m.coverageNotice({ unitAxis: { state: 'not_reported' } }, ''), /单位维度未报告/);
});

test('g4 unit statistics keep group-unique apart from the per-unit sum and say 未报告 for a missing projection', () => {
  const m = incidents();
  const lines = m.unitStatisticsLines({
    groupUnique: { open: 1 }, sumOfUnitInvolvement: 2,
    provinces: [{ legalName: '甲省公司', uniqueIncidents: 1, confirmed: 1, candidate: 0, conflict: 0 }],
    professionalSubsidiaries: [{ legalName: '专业公司', confirmed: 0, candidate: 1 }],
    unknownUnitIncidents: 0, dedup: { riskLeads: 3, openIncidents: 1, activeLocations: 2 }, assets: { state: 'not_reported' },
  }, '');
  assert.match(lines[0], /集团唯一事件 1 起.*单位涉及数之和 2.*各计一次/);
  assert.ok(lines.some(line => /专业公司单列/.test(line)));
  assert.ok(lines.includes('资产数：未报告'), '没有投影时写未报告而不是 0');
  assert.ok(!lines.some(line => /资产数：0/.test(line)));
  assert.deepEqual(m.unitStatisticsLines(null, ''), ['后端未报告单位统计（integration_pending）']);
  assert.match(m.unitStatisticsLines(null, 'x')[0], /不可用/);
});

test('g4 live progress gives a percent only with its basis and keeps cancel and gaps visible', () => {
  const m = incidents();
  const live = m.liveRunLines({ runId: 7, status: 'running', stage: 'collecting', currentSource: 'passive_web', cancelRequested: true, resumable: false,
    progress: { done: 2, total: 4, percent: 50, note: '按工作项计' }, sources: [{ sourceKey: 'web_archive', completed: 0, total: 2 }],
    usage: { requests: 6, budgetRequests: 100, bytes: 300, budgetBytes: 1000 }, lastPersistedAt: 't', gapsTotal: 25, gapsTruncated: true }, '');
  assert.match(live.join('\n'), /工作项 2 \/ 4（50%，按工作项计）/);
  assert.match(live.join('\n'), /已请求取消/);
  assert.match(live.join('\n'), /未闭合工作项 25 个（只列出前 20 个）/);
  assert.match(live.join('\n'), /请求 6 \/ 预算 100/);
  const empty = m.liveRunLines({ runId: 8, status: 'queued', progress: { done: 0, total: 0, percent: null } }, '');
  assert.match(empty.join('\n'), /尚无可计算的进度/);
  assert.ok(!/%/.test(empty.join('\n')), '没有工作项时不显示百分比');
  assert.match(m.liveRunLines({ runId: 9, status: 'cancelled', resumable: true, progress: {} }, '').join('\n'), /按冻结计划恢复/);
  assert.deepEqual(m.liveRunLines(null, ''), []);
});

test('g4 the summary reader passes the G4 blocks through and never invents them', () => {
  const c = contracts();
  const base = { projectId: 3, projectActive: true, eligibleAssetsTotal: 2, eligibleAssetsPlanned: 2, assetsBeyondPlan: 0, findingsTotal: 0, findingsNew: 0,
    findingsReviewing: 0, findingsConfirmed: 0, findingsDismissed: 0, highRiskOpen: 0, knownLimits: [] };
  assert.equal(typeof c.readScopeSummary, 'function');
  const read = c.readScopeSummary({ ...base, liveRun: { runId: 1 }, coverage: { rows: [] }, coverageError: '', unitStatistics: null, unitStatisticsError: 'e' });
  {
    assert.deepEqual(read.liveRun, { runId: 1 });
    assert.deepEqual(read.coverage, { rows: [] });
    assert.equal(read.unitStatistics, null);
    assert.equal(read.unitStatisticsError, 'e');
    const old = c.readScopeSummary(base);
    assert.equal(old.coverage, null, '旧后端没给覆盖矩阵就是 null，不是空矩阵');
  }
});

test('g4 every incident action carries revision, operation id and a reason, and notification needs a channel', () => {
  const m = incidents();
  const inc = { id: 5, revision: 3 };
  assert.equal(m.buildIncidentAction(inc, { action: 'review', status: 'verified', actor: 'a', reason: '短' }, 'op-12345678').ok, false);
  assert.equal(m.buildIncidentAction(inc, { action: 'review', status: 'merged', actor: 'a', reason: '理由充分足够' }, 'op-12345678').ok, false, '合并不能当复核结论');
  assert.equal(m.buildIncidentAction(inc, { action: 'review', status: 'verified', actor: '', reason: '理由充分足够' }, 'op-12345678').ok, false);
  assert.equal(m.buildIncidentAction(inc, { action: 'review', status: 'verified', actor: 'a', reason: '理由充分足够' }, 'bad id').ok, false);
  const ok = m.buildIncidentAction(inc, { action: 'review', status: 'verified', actor: ' a ', reason: ' 已人工核对原文 ' }, 'op-12345678');
  assert.deepEqual(ok.payload, { operationId: 'op-12345678', action: 'review', incidentId: 5, expectedRevision: 3, actor: 'a', reason: '已人工核对原文', status: 'verified' });
  const noChannel = m.buildIncidentAction(inc, { action: 'record_action', handling: 'notification_recorded', actor: 'a', reason: '已电话通知' }, 'op-12345678');
  assert.equal(noChannel.ok, false);
  assert.match(noChannel.error, /应用不会代为联系/);
  assert.equal(m.buildIncidentAction(inc, { action: 'record_action', handling: 'retest_linked', actor: 'a', reason: '复测关联' }, 'op-12345678').ok, false);
  assert.equal(m.buildIncidentAction(inc, { action: 'link_entity', entityId: 9, linkLevel: 'confirmed_link', evidenceObservationIds: [], actor: 'a', reason: '域名归属' }, 'op-12345678').ok, false, '确认关联要证据');
  const link = m.buildIncidentAction(inc, { action: 'link_entity', entityId: 9, linkLevel: 'candidate_link', actor: 'a', reason: '疑似归属' }, 'op-12345678');
  assert.deepEqual(link.payload.evidenceObservationIds, []);
  for (const row of m.HANDLING_ACTIONS) assert.ok(/登记|记录|关联|打开/.test(row.label), `${row.value} 的文案是登记而不是发送`);
  assert.match(m.HANDLING_NOTICE, /不会联系任何人或平台/);
  assert.match(m.newOperationId(() => 'ab/cd-ef'), /^ui-abcd-ef$/);
});

test('g4 the export line reports the digest read back and admits when it could not verify', () => {
  const m = contracts();
  assert.match(m.exportLine({ path: '/x/a.csv', rows: 3 }, { rows: 3, sha256: 'a'.repeat(64), format: 'ledger_v1' }), /已导出 3 行（ledger_v1）· SHA-256 aaaaaaaaaaaaaaaa…/);
  assert.match(m.exportLine({ path: '/x/a.csv', rows: 3 }, null), /未能读回核验/);
});

function harness(t, api, ids) {
  const projectRef = vue.ref(10);
  const notes = [];
  const state = composable().useExposureIncidents({
    projectId: () => projectRef.value,
    api,
    notify: (kind, text) => notes.push([kind, text]),
    operationId: ids ? () => ids.shift() : undefined,
  });
  return { state, notes, projectRef };
}
const pageOf = (ids, hasMore, next = '') => ({ items: ids.map(id => ({ id, revision: 1, status: 'candidate', title: `t${id}`, activeLocations: 2, units: { unknown: true }, handlingState: 'none' })), total: 3, hasMore, nextCursor: next });

test('g4 the incident panel says integration_pending while the commands are unregistered', async (t) => {
  const h = harness(t, { pageExposureIncidents: async () => { throw new Error('command page_exposure_incidents not found'); } });
  assert.equal(await h.state.load(), false);
  assert.equal(h.state.available.value, false);
  assert.match(h.state.notice.value, /integration_pending，IR-15/);
  assert.match(h.state.notice.value, /不是“没有事件”/);
  assert.equal(await h.state.act({ action: 'review', status: 'verified', actor: 'a', reason: '理由足够长' }), false, '未接入时不提供动作');
  const none = harness(t, {});
  await none.state.load();
  assert.equal(none.state.available.value, false);
});

test('g4 incident pages append without duplicates and a stale project answer is dropped', async (t) => {
  const calls = [];
  const h = harness(t, {
    pageExposureIncidents: async (pid, status, only, size, cursor) => { calls.push([pid, status, only, size, cursor]); return cursor ? pageOf([2, 1], false) : pageOf([3, 2], true, 'v1:abc:2'); },
  });
  await h.state.load();
  await h.state.loadMore();
  assert.deepEqual(h.state.items.value.map(row => row.id), [3, 2, 1]);
  assert.deepEqual(calls.map(row => row.slice(1)), [['open', false, 20, ''], ['open', false, 20, 'v1:abc:2']]);
  assert.match(h.state.items.value[0].line, /单位未知/);
  // a page claiming more without a cursor is an error, not a silent end
  const bad = harness(t, { pageExposureIncidents: async () => ({ items: [], total: 1, hasMore: true, nextCursor: '' }) });
  assert.equal(await bad.state.load(), false);
  assert.match(bad.state.notice.value, /事件读取失败/);
  // project switch while a read is in flight
  let release;
  const slow = harness(t, { pageExposureIncidents: () => new Promise(resolve => { release = () => resolve(pageOf([9], false)); }) });
  const pending = slow.state.load();
  slow.projectRef.value = 11;
  release();
  assert.equal(await pending, false);
  assert.deepEqual(slow.state.items.value, []);
});

test('g4 an action reuses its operation id on retry and refreshes on a revision conflict', async (t) => {
  const sent = [];
  let mode = 'fail';
  let revision = 1;
  const api = {
    pageExposureIncidents: async () => pageOf([5], false),
    readExposureIncident: async (pid, id) => ({ id, projectId: pid, revision, status: 'candidate' }),
    actOnExposureIncident: async (pid, action) => {
      sent.push(action);
      if (mode === 'fail') throw new Error('network down');
      if (mode === 'conflict') { revision = 2; throw new Error('[exposure_incident_conflict] 当前版本 2'); }
      return { replayed: mode === 'replay' };
    },
  };
  const h = harness(t, api, ['op-aaaaaaaa', 'op-bbbbbbbb', 'op-cccccccc']);
  await h.state.load();
  await h.state.open(5);
  const form = { action: 'review', status: 'reviewing', actor: 'a', reason: '开始复核' };
  assert.equal(await h.state.act(form), false);
  mode = 'replay';
  assert.equal(await h.state.act(form), true);
  assert.equal(sent[0].operationId, sent[1].operationId, '重试沿用同一操作编号');
  assert.ok(h.notes.some(([, text]) => /此前已提交/.test(text)));
  mode = 'conflict';
  assert.equal(await h.state.act(form), false);
  assert.notEqual(sent[2].operationId, sent[1].operationId, '成功后的新提交用新编号');
  assert.equal(h.state.detail.value.revision, 2, '冲突后刷新到最新版本');
  mode = 'ok';
  await h.state.act(form);
  assert.equal(sent[3].expectedRevision, 2);
  assert.notEqual(sent[3].operationId, sent[2].operationId);
  assert.ok(sent.every(row => row.reason && row.actor && row.expectedRevision >= 1));
});

test('g4 the detail must belong to the selected project and incident', async (t) => {
  const h = harness(t, { pageExposureIncidents: async () => pageOf([5], false), readExposureIncident: async () => ({ id: 5, projectId: 99, revision: 1 }) });
  await h.state.open(5);
  assert.equal(h.state.detail.value, null);
  assert.ok(h.notes.some(([kind, text]) => kind === 'error' && /不一致/.test(text)));
});

test('g4 the surface wires the ledger tab, the two exports, live lines and resume, and never auto-contacts', () => {
  const surface = readSource('src/features/assets/exposure/ExposureSurface.vue');
  assert.match(surface, /事件台账/);
  assert.match(surface, /exportAll\('ledger'\)/);
  assert.match(surface, /exportAll\('findings'\)/);
  assert.match(surface, /liveLines/);
  assert.match(surface, /resumeRun\(resumableLive\)/);
  const api = readSource(E + 'api.ts');
  assert.match(api, /IR-15/);
  const panel = readSource(E + 'ExposureIncidentsPanel.vue');
  assert.match(panel, /HANDLING_NOTICE/);
  for (const source of [panel, readSource(E + 'incidents.ts'), readSource(E + 'useExposureIncidents.ts')]) {
    assert.ok(!/mailto:|sendMail|smtp|webhook|fetch\(/i.test(source), '事件处置代码里没有任何对外联系的路径');
  }
});

// ───────────── P1 UI 闭环（2026-10-09）：合并/拆分、第 51 条以后的位置、覆盖钻取 ─────────────
// 组件交互测试：真实编译 SFC，用一个最小的自定义渲染器挂载，按 data-* 找到按钮/输入框，
// 触发它们自己的事件处理器，断言发出的 IPC 载荷。不是只看源码字符串。

const { parse: parseSfc, compileScript: compileSfcScript, compileTemplate: compileSfcTemplate } = require('@vue/compiler-sfc');

function fakeDom() {
  // vModelText 在更新时问“是不是当前焦点”；测试环境没有 DOM，这里给出“永远不是焦点”的最小答案。
  globalThis.Document ??= class Document {};
  globalThis.ShadowRoot ??= class ShadowRoot {};
  const make = (type) => ({ getRootNode() { return {}; }, type, props: {}, children: [], parent: null, listeners: {}, value: '', checked: false, options: [], selectedIndex: -1,
    addEventListener(name, fn) { (this.listeners[name] ??= []).push(fn); },
    removeEventListener(name, fn) { this.listeners[name] = (this.listeners[name] ?? []).filter(f => f !== fn); },
    setAttribute(key, value) { this.props[key] = value; }, removeAttribute(key) { delete this.props[key]; },
  });
  const ops = {
    createElement: make,
    createText: text => ({ ...make('#text'), text }),
    createComment: text => ({ ...make('#comment'), text }),
    setText: (node, text) => { node.text = text; },
    setElementText: (el, text) => { el.children = [{ ...make('#text'), text, parent: el }]; },
    insert: (child, parent, anchor) => {
      if (child.parent) child.parent.children = child.parent.children.filter(n => n !== child);
      const index = anchor ? parent.children.indexOf(anchor) : -1;
      if (index >= 0) parent.children.splice(index, 0, child); else parent.children.push(child);
      child.parent = parent;
    },
    remove: child => { if (child.parent) child.parent.children = child.parent.children.filter(n => n !== child); child.parent = null; },
    parentNode: node => node.parent,
    nextSibling: node => { if (!node.parent) return null; const s = node.parent.children; return s[s.indexOf(node) + 1] ?? null; },
    patchProp: (el, key, _prev, next) => {
      if (key === 'value') el.value = next;
      else if (key === 'checked') el.checked = next;
      el.props[key] = next;
    },
  };
  return { ops, root: make('root') };
}
const textOf = node => node.type === '#text' ? node.text : node.type === '#comment' ? '' : node.children.map(textOf).join('');
const findAll = (node, pred, out = []) => { if (pred(node)) out.push(node); node.children.forEach(child => findAll(child, pred, out)); return out; };
const byAttr = (root, key, value) => findAll(root, n => n.props && (value === undefined ? key in n.props : String(n.props[key]) === String(value)))[0];
const fire = async (node, prop, event = {}) => {
  assert.ok(node, `找不到要操作的元素（${prop}）`);
  const handler = node.props[prop];
  const target = { value: node.value, checked: node.checked, ...event.target };
  for (const fn of [].concat(handler ?? [])) await fn({ target, ...event });
  for (const fn of node.listeners[prop.slice(2).toLowerCase()] ?? []) await fn({ target: node, ...event });
  await flushMany();
};
const typeInto = async (node, value) => { node.value = String(value); for (const fn of node.listeners.input ?? []) fn({ target: node }); for (const fn of node.listeners.change ?? []) fn({ target: node }); await flushMany(); };
const flushMany = async () => { for (let i = 0; i < 20; i++) { await vue.nextTick(); await new Promise(resolve => setImmediate(resolve)); } };

function mountSfc(t, file, props, requireMap) {
  const filename = path.join(__dirname, '..', file);
  const { descriptor } = parseSfc(readSource(file), { filename });
  const script = compileSfcScript(descriptor, { id: file });
  const compiled = compileSfcTemplate({ source: descriptor.template.content, filename, id: file, compilerOptions: { bindingMetadata: script.bindings } });
  assert.deepEqual(compiled.errors, [], `${file} 模板必须能编译`);
  const requireModule = name => { if (name === 'vue') return vue; if (name in requireMap) return requireMap[name]; throw Error(`unexpected import ${name}`); };
  const exports = {}, templateExports = {};
  new Function('require', 'exports', transpile(script.content))(requireModule, exports);
  new Function('require', 'exports', transpile(compiled.code))(requireModule, templateExports);
  const dom = fakeDom();
  const app = vue.createRenderer(dom.ops).createApp({ ...exports.default, render: templateExports.render }, props);
  app.mount(dom.root);
  t.after(() => app.unmount());
  return dom.root;
}

const locationRows = (from, to) => Array.from({ length: to - from + 1 }, (_, i) => ({ id: from + i, locationKind: 'first_party', collectionMode: 'passive', canonicalUrl: `https://a.example.test/${from + i}`, runId: 1, observationId: 1000 + from + i, observedAt: 't' }));

function incidentsHarness(t, over = {}) {
  const calls = [];
  const api = {
    pageExposureIncidents: async () => ({ items: [{ id: 11, title: '泄漏', status: 'verified', revision: 3, activeLocations: 120, units: {} }], total: 1, hasMore: false, nextCursor: '' }),
    readExposureIncident: async (projectId, id) => {
      calls.push(['read', id]);
      if (id === 11) return { id: 11, projectId, title: '泄漏', status: 'verified', revision: 3, entities: [], events: [], locations: { items: locationRows(1, 50), total: 120, hasMore: true, nextCursor: 'v1:h:50' } };
      if (id === 12) return { id: 12, projectId, title: '同一份文件', status: 'reviewing', revision: 5, locations: { items: [], total: 0, hasMore: false, nextCursor: '' } };
      if (id === 13) return { id: 13, projectId, title: '已合并', status: 'merged', revision: 2, locations: { items: [], total: 0, hasMore: false, nextCursor: '' } };
      throw Error('[exposure_incident_missing]');
    },
    pageExposureIncidentLocations: async (projectId, id, size, cursor) => {
      calls.push(['locations', id, size, cursor]);
      return cursor === 'v1:h:50'
        ? { items: locationRows(51, 100), total: 120, hasMore: true, nextCursor: 'v1:h:100' }
        : { items: locationRows(101, 120), total: 120, hasMore: false, nextCursor: '' };
    },
    actOnExposureIncident: async (projectId, action) => { calls.push(['act', action]); return { replayed: false }; },
    proposeExposureIncidents: async () => ({}),
    ...over,
  };
  const c = contracts();
  const i = incidents();
  const comp = loadModule(E + 'useExposureIncidents.ts', name => ({ vue, './contracts': c, './incidents': i })[name] ?? (() => { throw Error(name); })());
  const root = mountSfc(t, E + 'ExposureIncidentsPanel.vue', { projectId: 7 }, { './api': { exposureIncidentApi: api }, './useExposureIncidents': comp, './incidents': i, './contracts': c });
  return { root, calls };
}

async function openIncident(h) {
  await flushMany();
  const view = findAll(h.root, n => n.type === 'button' && textOf(n) === '查看')[0];
  await fire(view, 'onClick');
}

async function fillActor(root) {
  const inputs = findAll(root, n => n.type === 'input' && !n.props.type && !n.props['data-field']);
  await typeInto(inputs[0], '李四');
  await typeInto(inputs[1], '人工判断后的理由');
}

test('P1 UI propagation locations beyond the 51st are paged in by cursor from the incident panel', async (t) => {
  const h = incidentsHarness(t);
  await openIncident(h);
  assert.match(textOf(h.root), /已显示 50 \/ 共 120/);
  await fire(byAttr(h.root, 'data-action', 'more-locations'), 'onClick');
  assert.match(textOf(h.root), /已显示 100 \/ 共 120/);
  assert.match(textOf(h.root), /https:\/\/a\.example\.test\/51\b/, '第 51 条位置出现在页面上');
  await fire(byAttr(h.root, 'data-action', 'more-locations'), 'onClick');
  assert.match(textOf(h.root), /已显示 120 \/ 共 120/);
  assert.equal(byAttr(h.root, 'data-action', 'more-locations'), undefined, '读完后不再显示加载更多');
  assert.deepEqual(h.calls.filter(c => c[0] === 'locations'), [['locations', 11, 50, 'v1:h:50'], ['locations', 11, 50, 'v1:h:100']]);
  const ids = findAll(h.root, n => n.props && 'data-location' in n.props).map(n => n.props['data-location']);
  assert.equal(new Set(ids).size, 120, '位置不重复、不遗漏');
});

test('P1 UI a split sends exactly the ticked locations with the revision that was read', async (t) => {
  const h = incidentsHarness(t);
  await openIncident(h);
  await fire(byAttr(h.root, 'data-action', 'more-locations'), 'onClick');
  await fillActor(h.root);
  assert.equal(byAttr(h.root, 'data-action', 'split').props.disabled, true, '没勾选时拆分按钮禁用');
  await fire(byAttr(h.root, 'data-location', 51), 'onChange');
  await fire(byAttr(h.root, 'data-location', 3), 'onChange');
  await fire(byAttr(h.root, 'data-location', 51), 'onChange');
  await fire(byAttr(h.root, 'data-location', 51), 'onChange');
  await fire(byAttr(h.root, 'data-action', 'split'), 'onClick');
  const acted = h.calls.filter(c => c[0] === 'act').map(c => c[1]);
  assert.equal(acted.length, 1);
  assert.deepEqual({ ...acted[0], operationId: 'x' }, { operationId: 'x', action: 'split', incidentId: 11, expectedRevision: 3, actor: '李四', reason: '人工判断后的理由', locationIds: [3, 51] });
});

test('P1 UI a merge reads the target first and sends its revision; self and merged targets are refused', async (t) => {
  const h = incidentsHarness(t);
  await openIncident(h);
  await fillActor(h.root);
  assert.equal(byAttr(h.root, 'data-action', 'merge').props.disabled, true, '未读取目标事件前合并按钮禁用');
  const target = byAttr(h.root, 'data-field', 'merge-target');
  await typeInto(target, 11);
  await fire(byAttr(h.root, 'data-action', 'read-target'), 'onClick');
  assert.equal(byAttr(h.root, 'data-action', 'merge').props.disabled, true, '不能并入自己');
  await typeInto(target, 13);
  await fire(byAttr(h.root, 'data-action', 'read-target'), 'onClick');
  assert.equal(byAttr(h.root, 'data-action', 'merge').props.disabled, true, '已被合并的事件不能作为目标');
  await typeInto(target, 12);
  await fire(byAttr(h.root, 'data-action', 'read-target'), 'onClick');
  assert.match(textOf(h.root), /目标 #12 同一份文件（v5/);
  await fire(byAttr(h.root, 'data-action', 'merge'), 'onClick');
  const acted = h.calls.filter(c => c[0] === 'act').map(c => c[1]);
  assert.equal(acted.length, 1, '只有读到合法目标后才发出一次合并');
  assert.deepEqual({ ...acted[0], operationId: 'x' }, { operationId: 'x', action: 'merge', incidentId: 11, expectedRevision: 3, actor: '李四', reason: '人工判断后的理由', targetIncidentId: 12, targetExpectedRevision: 5 });
});

test('P1 UI merge and split payloads are validated before any IPC', () => {
  const m = incidents();
  const base = { actor: '李四', reason: '人工判断后的理由' };
  assert.equal(m.buildIncidentAction({ id: 11, revision: 3 }, { ...base, action: 'merge', targetIncidentId: 11, targetExpectedRevision: 1 }, 'op-12345678').ok, false);
  assert.equal(m.buildIncidentAction({ id: 11, revision: 3 }, { ...base, action: 'merge', targetIncidentId: 12 }, 'op-12345678').ok, false, '没有目标版本不能合并');
  assert.equal(m.buildIncidentAction({ id: 11, revision: 3 }, { ...base, action: 'split', locationIds: [] }, 'op-12345678').ok, false);
  assert.equal(m.buildIncidentAction({ id: 11, revision: 3 }, { ...base, action: 'split', locationIds: [1.5] }, 'op-12345678').ok, false);
  const ok = m.buildIncidentAction({ id: 11, revision: 3 }, { ...base, action: 'split', locationIds: [5, 5, 2] }, 'op-12345678');
  assert.deepEqual(ok.payload.locationIds, [5, 2]);
});

test('P1 UI coverage rows carry their source key and time window, and a malformed drill page is an error', () => {
  const m = incidents();
  const [row] = m.coverageRows({ rows: [{ capability: 'first_party_web', sourceKey: 'passive_web', state: 'partial', denominator: {}, cells: [], window: { runId: 9, runStatus: 'partial', createdAt: '2026-10-09 10:00:00', completedAt: '2026-10-09 10:05:00' } }] });
  assert.equal(row.sourceKey, 'passive_web');
  assert.match(row.window, /第 9 轮（部分完成）· 开始 2026-10-09 10:00:00 · 结束 2026-10-09 10:05:00/);
  assert.throws(() => m.coverageDrillView({ items: [] }), /结构无效/);
  assert.throws(() => m.coverageDrillView({ items: [], total: 3, hasMore: true, nextCursor: '' }), /没有游标/);
  const view = m.coverageDrillView({ items: [], total: 0, hasMore: false, unitAxis: { state: 'not_reported', reason: '投影读取失败' } });
  assert.match(view.unitNote, /单位维度未报告.*（不是 0）/);
});

const drillPage = (over = {}) => ({
  sourceKey: 'passive_web', window: { days: 0, label: '最近一轮', runIds: [9] },
  rounds: [{ runId: 9, runStatus: 'partial', createdAt: 't0', completedAt: 't1', itemCounts: { completed: 2, blocked_quota: 2, failed: 1 }, sourceResult: 'partial' }],
  unitAxis: { state: 'reported', rows: [{ entityId: 0, legalName: '未归属到单位', total: 1, completed: 0, gaps: 1 }, { entityId: 4, legalName: '咪咕杭州分公司', total: 3, completed: 1, gaps: 2 }] },
  items: [{ id: 1, runId: 9, status: 'completed', reason: '', origin: 'https://a.example.test', requests: 3 }, { id: 2, runId: 9, status: 'blocked_quota', reason: '预算用完', origin: 'https://b.example.test', requests: 0 }],
  total: 5, hasMore: true, nextCursor: 'v1:x:2', ...over,
});

test('P1 UI the coverage drill-down changes window, unit and gap filter and pages items from the panel', async (t) => {
  const calls = [];
  const api = {
    exposureCoverageDrilldown: async (...args) => {
      calls.push(args);
      const [, , , , , , cursor] = args;
      return cursor ? drillPage({ items: [{ id: 3, runId: 9, status: 'failed', reason: '超时', origin: 'https://c.example.test', requests: 1 }], hasMore: false, nextCursor: '' }) : drillPage();
    },
  };
  const c = contracts();
  const i = incidents();
  const comp = loadModule(E + 'useExposureCoverage.ts', name => ({ vue, './contracts': c, './incidents': i })[name] ?? (() => { throw Error(name); })());
  const rows = i.coverageRows({ rows: [{ capability: 'first_party_web', sourceKey: 'passive_web', state: 'partial', denominator: {}, cells: [], window: { runId: 9, runStatus: 'partial', createdAt: 't0', completedAt: 't1' } }] });
  const root = mountSfc(t, E + 'ExposureCoverageDrill.vue', { projectId: 7, rows, source: 'passive_web' }, { './api': { exposureIncidentApi: api }, './useExposureCoverage': comp, './incidents': i });
  await flushMany();
  assert.deepEqual(calls[0], [7, 'passive_web', 0, null, false, 50, '']);
  const page = textOf(root);
  assert.match(page, /钻取：第一方网页/);
  assert.match(page, /第 9 轮（部分完成）t0 → t1：已完成 2 · 配额阻塞 2 · 失败 1 · 来源结果 partial/);
  assert.match(page, /咪咕杭州分公司：1 \/ 3 完成，缺口 2/);
  assert.match(page, /工作项（已显示 2 \/ 共 5）/);
  await fire(byAttr(root, 'data-action', 'more-items'), 'onClick');
  assert.deepEqual(calls.at(-1), [7, 'passive_web', 0, null, false, 50, 'v1:x:2']);
  assert.match(textOf(root), /工作项（已显示 3 \/ 共 5）/);
  await fire(byAttr(root, 'data-field', 'window'), 'onChange', { target: { value: '30' } });
  assert.deepEqual(calls.at(-1), [7, 'passive_web', 30, null, false, 50, '']);
  await fire(byAttr(root, 'data-field', 'unit'), 'onChange', { target: { value: '4' } });
  assert.deepEqual(calls.at(-1), [7, 'passive_web', 30, 4, false, 50, '']);
  await fire(byAttr(root, 'data-field', 'gaps'), 'onChange', { target: { checked: true } });
  assert.deepEqual(calls.at(-1), [7, 'passive_web', 30, 4, true, 50, '']);
});

test('P1 UI a failed drill-down says so and never reads as "no gaps"; the page mounts the drill-down', async (t) => {
  const api = { exposureCoverageDrilldown: async () => { throw Error('[exposure_coverage_unit_unavailable] 单位投影未报告'); } };
  const c = contracts();
  const i = incidents();
  const comp = loadModule(E + 'useExposureCoverage.ts', name => ({ vue, './contracts': c, './incidents': i })[name] ?? (() => { throw Error(name); })());
  const root = mountSfc(t, E + 'ExposureCoverageDrill.vue', { projectId: 7, rows: [], source: 'passive_web' }, { './api': { exposureIncidentApi: api }, './useExposureCoverage': comp, './incidents': i });
  await flushMany();
  assert.match(textOf(root), /钻取读取失败.*（不是“没有缺口”）/);
  assert.doesNotMatch(textOf(root), /工作项（已显示/);
  const page = readSource('src/features/assets/exposure/ExposureSurface.vue');
  assert.match(page, /<ExposureCoverageDrill/, '覆盖页挂上钻取面板');
  assert.match(page, /:data-drill="row\.sourceKey"/, '每一行都有钻取入口');
  assert.match(readSource(E + 'api.ts'), /exposure_coverage_drilldown/);
  assert.match(readSource('src-tauri/src/lib.rs'), /commands::exposure_coverage_drilldown,/, '钻取命令已注册');
});

// ───────────── FIX3（2026-10-09 17:29） ─────────────

test('FIX3-R1 opening incident 5 then 6 in the same project: a late answer for 5 never overwrites 6', async (t) => {
  const waiting = {};
  const h = harness(t, {
    readExposureIncident: (pid, id) => new Promise(resolve => { waiting[id] = () => resolve({ id, projectId: pid, revision: 1, status: 'candidate', title: `t${id}` }); }),
  });
  const first = h.state.open(5);
  const second = h.state.open(6);
  waiting[6]();
  await second;
  waiting[5]();
  await first;
  await flush();
  assert.equal(h.state.detail.value.id, 6, '迟到的旧详情覆盖了新选中的事件');
});
