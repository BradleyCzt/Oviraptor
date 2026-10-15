// §Q4/§Q5 — the pending index queue and the evidence package on the exposure page. These
// tests pin the honest readings: an unregistered command is never shown as an empty queue, a
// late answer from another project cannot overwrite the screen, an index hit is not proof of
// current accessibility, and a frozen-plan refusal is never rephrased as a started round.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');

const flush = async () => { for (let i = 0; i < 20; i++) await vue.nextTick(); };
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
const readSource = relative => fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');
const transpile = source => ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

const loadModule = (relative, requireModule) => {
  const exports = {};
  new Function('require', 'exports', transpile(readSource(relative)))(requireModule, exports);
  return exports;
};
const loadContracts = () => loadModule('src/features/assets/exposure/contracts.ts', () => { throw Error('unexpected'); });

const pendingRow = (id, projectId, entity = `pay-${id}.corp.invalid`) => ({
  id, projectId, runId: 9, sourceKey: 'certificate_transparency', kind: 'domain',
  entity, canonicalKey: entity, evidenceUrl: 'https://crt.sh/?q=corp.invalid',
  status: 'unreviewed', createdAt: '2026-10-07 12:00:00',
});
const page = (items, total, { hasMore = false, nextCursor = '', pageSize = 50 } = {}) => ({
  items, total, hasMore, nextCursor, pageSize,
});
const observation = (id, runId, over = {}) => ({
  id, runId, sourceKey: 'passive_web', kind: 'source_map', url: `https://corp.invalid/a${id}.js.map`,
  finalUrl: `https://corp.invalid/a${id}.js.map`, statusCode: 200, contentType: 'application/json',
  contentHash: 'a'.repeat(64), headersHash: 'b'.repeat(64), bodyBytes: 512, truncated: false,
  ruleVersion: 'oviraptor.exposure.rules.v1', redactionVersion: 'oviraptor.exposure.redaction.v2',
  boundary: '', currentAccessible: true, indexedAt: '', queriedAt: '2026-10-07 12:00:00',
  evidence: {}, ...over,
});
const evidencePackage = (over = {}) => ({
  finding: { id: 3, category: 'source_code', title: '公开 Source Map 还原业务源码', confidence: 82, status: 'confirmed' },
  observations: [observation(1, 9)],
  versions: [{ revision: 1, runId: 9, contentHash: 'a'.repeat(64), ruleVersion: 'oviraptor.exposure.rules.v1', redactionVersion: 'oviraptor.exposure.redaction.v2', severity: 'high', manualStatus: 'confirmed', reviewNeeded: false, createdAt: '2026-10-07 12:00:00' }],
  reviewEvents: [{ fromStatus: 'new', toStatus: 'confirmed', note: '已确认', actor: 'user', createdAt: '2026-10-07 12:30:00' }],
  reviewNeeded: null,
  ...over,
});

function mountEvidence(t, read = {}) {
  const calls = [];
  const api = {
    pageExposurePendingDiscoveries: (projectId, status, pageSize, cursor) => {
      calls.push(['pending', projectId, cursor]);
      return (read.pending ?? (async () => page([pendingRow(1, projectId)], 1)))(projectId, status, pageSize, cursor);
    },
    readExposureEvidence: (findingId, expectedProjectId) => {
      calls.push(['evidence', findingId, expectedProjectId]);
      return (read.evidence ?? (async () => evidencePackage()))(findingId, expectedProjectId);
    },
    resumeExposureScan: (runId, expectedProjectId) => {
      calls.push(['resume', runId, expectedProjectId]);
      return (read.resume ?? (async () => runId))(runId, expectedProjectId);
    },
  };
  const contracts = loadContracts();
  const composable = loadModule('src/features/assets/exposure/useExposureEvidence.ts', name => {
    if (name === 'vue') return vue;
    // G2 the page itself now reads source labels from the real contracts module.
    if (name === './contracts' || name === './contracts') return contracts;
    throw Error(`Unexpected evidence import: ${name}`);
  });
  const events = [];
  const selection = vue.reactive({ project: 10, active: true });
  let state;
  const renderer = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {}, parentNode: () => null, nextSibling: () => null });
  const app = renderer.createApp({
    setup() {
      state = composable.useExposureEvidence({
        projectId: () => selection.project,
        active: () => selection.active,
        api,
        notify: (kind, text) => events.push([kind, text]),
      });
      return () => null;
    },
  });
  app.mount({});
  let closed = false;
  const unmount = () => { if (closed) return; closed = true; app.unmount(); };
  if (t && typeof t.after === 'function') t.after(unmount);
  return { state, calls, events, selection, contracts, unmount };
}

test('pending queue keeps a real total and refuses malformed pages', async () => {
  const contracts = loadContracts();
  const parsed = contracts.readExposurePage(page([pendingRow(1, 10), pendingRow(2, 10)], 7, { hasMore: true, nextCursor: 'abc' }), contracts.isPendingRow, '待归并队列');
  assert.equal(parsed.total, 7);
  assert.equal(parsed.items.length, 2);
  assert.equal(parsed.nextCursor, 'abc');
  assert.throws(() => contracts.readExposurePage(page([pendingRow(1, 10), pendingRow(1, 10)], 2), contracts.isPendingRow, '待归并队列'), /重复 id/);
  assert.throws(() => contracts.readExposurePage({ items: [pendingRow(1, 10)], hasMore: false, nextCursor: '', pageSize: 50 }, contracts.isPendingRow, '待归并队列'), /真实总数/);
  assert.throws(() => contracts.readExposurePage(page([{ id: 1 }], 1), contracts.isPendingRow, '待归并队列'), /无效条目/, '缺来源与状态的行不能算待归并结果');
  assert.throws(() => contracts.readExposurePage(page([pendingRow(1, 10)], 1, { hasMore: false, nextCursor: 'x' }), contracts.isPendingRow, '待归并队列'), /末尾页不能带游标/);
});

test('unregistered pending command is reported as missing, never as an empty queue', async (t) => {
  const other = mountEvidenceWithout(t);
  await other.state.loadPending();
  await flush();
  assert.equal(other.state.pagedPending.value, false, '未注册必须记为“接口未注册”');
  assert.match(other.state.pendingNotice.value, /尚未注册/, '提示必须是缺接口，而不是空队列');
  assert.equal(other.state.pending.value, null, '未注册时不能伪造一个空列表');
  assert.equal(other.state.readError.value, '', '缺接口不是读取失败');
  await other.state.openEvidence(3);
  assert.match(other.state.readError.value, /证据包接口尚未注册/);
});

function mountEvidenceWithout(t) {
  const contracts = loadContracts();
  const composable = loadModule('src/features/assets/exposure/useExposureEvidence.ts', name => {
    if (name === 'vue') return vue;
    // G2 the page itself now reads source labels from the real contracts module.
    if (name === './contracts' || name === './contracts') return contracts;
    throw Error(`Unexpected evidence import: ${name}`);
  });
  let state;
  const renderer = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {}, parentNode: () => null, nextSibling: () => null });
  const app = renderer.createApp({
    setup() {
      state = composable.useExposureEvidence({
        projectId: () => 10,
        active: () => true,
        api: {},
        notify: () => {},
      });
      return () => null;
    },
  });
  app.mount({});
  if (t && typeof t.after === 'function') t.after(() => app.unmount());
  return { state };
}

test('a late pending page from an older project cannot overwrite the current one', async (t) => {
  const first = deferred(), second = deferred();
  let count = 0;
  const h = mountEvidence(t, {
    pending: (projectId) => {
      count += 1;
      return (count === 1 ? first : second).promise;
    },
  });
  void h.state.loadPending();
  await flush();
  h.selection.project = 20;
  void h.state.loadPending();
  await flush();
  second.resolve(page([pendingRow(30, 20, 'new.corp.invalid')], 1));
  await flush();
  first.resolve(page([pendingRow(11, 10, 'old.corp.invalid')], 1));
  await flush();
  assert.deepEqual(h.state.pending.value.items.map(row => row.entity), ['new.corp.invalid'], '旧项目的队列不能覆盖新项目');
  h.unmount();
});

test('load more appends the next page without repeating rows or faking the total', async (t) => {
  const cursors = ['', 'p2'];
  const h = mountEvidence(t, {
    pending: (projectId, status, pageSize, cursor) => cursor === ''
      ? Promise.resolve(page([pendingRow(1, projectId)], 3, { hasMore: true, nextCursor: 'p2' }))
      : Promise.resolve(page([pendingRow(2, projectId), pendingRow(1, projectId)], 3, { hasMore: false, nextCursor: '' })),
  });
  await h.state.loadPending();
  await flush();
  assert.equal(h.state.pending.value.items.length, 1);
  assert.ok(h.calls.some(row => row[0] === 'pending' && row[2] === ''));
  await h.state.loadMorePending();
  await flush();
  assert.deepEqual(h.state.pending.value.items.map(row => row.id), [1, 2], '重复行必须被去掉');
  assert.equal(h.state.pending.value.total, 3, '总数仍来自后端');
  assert.equal(h.state.pending.value.hasMore, false);
  assert.match(h.state.pendingNotice.value, /待归并 3 条，已显示 2 条，已到底/);
  h.unmount();
});

test('the evidence package keeps rounds, index-only hits and the review marker apart', async () => {
  const contracts = loadContracts();
  const parsed = contracts.readEvidence(evidencePackage({
    observations: [
      observation(1, 9),
      observation(2, 12, { currentAccessible: null, indexedAt: '20240202120000', kind: 'archive_url', sourceKey: 'web_archive' }),
    ],
    reviewNeeded: { revision: 3, ruleVersion: 'oviraptor.exposure.rules.v1' },
  }));
  assert.equal(parsed.observations.length, 2);
  assert.match(contracts.evidenceHeadline(parsed), /2 条观察 · 跨 2 轮/);
  assert.match(contracts.evidenceHeadline(parsed), /1 条当前可访问性未知（仅索引命中）/, '归档命中不能被写成当前可访问');
  assert.match(contracts.evidenceHeadline(parsed), /需复核（版本 3）/);
  assert.throws(() => contracts.readEvidence(evidencePackage({ observations: [{ id: 1, url: 'x' }] })), /无效观察/, '缺规则或脱敏版本的观察不能被当成证据');
  assert.throws(() => contracts.readEvidence({ observations: [], versions: [] }), /缺少线索主体/);
});

test('an unredacted observation is rejected instead of shown', async () => {
  const contracts = loadContracts();
  const leaked = evidencePackage({ observations: [observation(1, 9, { redactionVersion: '' })] });
  assert.throws(() => contracts.readEvidence(leaked), /无效观察/);
});

test('resume reports a frozen-plan refusal as a refusal', async (t) => {
  const ok = mountEvidence(t, {});
  assert.equal(await ok.state.resumeRun(9), true);
  assert.deepEqual(ok.events[0], ['success', '任务 #9 已按冻结计划继续']);
  assert.deepEqual(ok.calls[0], ['resume', 9, 10], '恢复必须带上期望项目');
  ok.unmount();

  const refused = mountEvidence(t, {
    resume: () => Promise.reject(Error('[exposure_query_failed] exposure_plan_frozen_mismatch：授权范围或归属版本已变化')),
  });
  assert.equal(await refused.state.resumeRun(9), false);
  assert.deepEqual(refused.events[0], ['error', '冻结的计划与当前授权或配置不一致，需另起新一轮采集，旧结果保留']);
  assert.doesNotMatch(refused.events[0][1], /已按冻结计划继续|已启动/);
  refused.unmount();

  const legacy = mountEvidence(t, {
    resume: () => Promise.reject(Error('[exposure_query_failed] exposure_plan_missing:任务 9 没有冻结计划')),
  });
  await legacy.state.resumeRun(9);
  assert.match(legacy.events[0][1], /旧版有限采集留下的任务，没有工作项台账/);
  legacy.unmount();
});

test('opening evidence is scoped to the current project and closing clears it', async (t) => {
  const h = mountEvidence(t, {});
  await h.state.openEvidence(3);
  await flush();
  assert.deepEqual(h.calls.find(row => row[0] === 'evidence'), ['evidence', 3, 10], '读取证据必须带期望项目');
  assert.equal(h.state.evidenceFor.value, 3);
  assert.equal(h.state.evidence.value.finding.id, 3);
  h.state.closeEvidence();
  assert.equal(h.state.evidence.value, null);
  assert.equal(h.state.evidenceFor.value, null);
  h.unmount();
});

test('the exposure page offers the pending tab, the evidence control and no automatic round', async () => {
  const calls = [];
  const api = {
    exposureScopeSummary: async () => null,
    pageExposureFindings: async () => null,
    pageExposureRuns: async () => page([{
      id: 9, projectId: 10, status: 'interrupted', eligibleAssets: 3, scannedAssets: 1,
      fetchedResources: 1, findings: 1, stage: 'interrupted', currentSource: '', error: '',
      createdAt: 't', startedAt: 't', completedAt: 't',
    }], 1),
    listExposureSourceResultsForProject: async () => [],
    listAssets: async () => ({ items: [], total: 0, page: 1, pageSize: 100, summary: {} }),
    startExposureScan: async () => { calls.push('start'); return 1; },
    cancelExposureScan: async () => {},
    reviewExposureFinding: async () => {},
    saveExposureFinding: async () => 1,
    exportExposureFindings: async () => ({ path: '/tmp/x.csv', rows: 0 }),
    pageExposurePendingDiscoveries: async () => { throw Error('unknown command: page_exposure_pending_discoveries'); },
    readExposureEvidence: async () => { throw Error('unknown command: read_exposure_evidence'); },
    resumeExposureScan: async () => { throw Error('unknown command: resume_exposure_scan'); },
  };
  const contracts = loadContracts();
  const surface = loadModule('src/features/assets/exposure/useExposureSurface.ts', name => name === 'vue' ? vue : name === './contracts' ? contracts : (() => { throw Error(name); })());
  const evidence = loadModule('src/features/assets/exposure/useExposureEvidence.ts', name => name === 'vue' ? vue : name === './contracts' ? contracts : (() => { throw Error(name); })());
  const filename = path.join(__dirname, '../src/features/assets/exposure/ExposureSurface.vue');
  const { descriptor } = parse(readSource('src/features/assets/exposure/ExposureSurface.vue'), { filename });
  const script = compileScript(descriptor, { id: 'ExposureSurface.vue' });
  const template = compileTemplate({ source: descriptor.template.content, filename, id: 'ExposureSurface.vue', compilerOptions: { bindingMetadata: script.bindings } });
  assert.deepEqual(template.errors, [], '暴露面模板必须能编译');
  const settingsModule = loadModule('src/features/assets/exposure/useExposureSettings.ts', name => name === 'vue' ? vue : name === './contracts' ? contracts : (() => { throw Error(name); })());
  const clusterModule = loadModule('src/features/assets/exposure/useExposureClusters.ts', name => name === 'vue' ? vue : name === './contracts' ? contracts : name === './api' ? { exposureApi: api } : (() => { throw Error(name); })());
  let panel, clusterPanel, reviewPanel;
  const reviewModule = loadModule('src/features/assets/exposure/useExposureReview.ts', name => name === 'vue' ? vue : name === './contracts' ? contracts : name === './api' ? { exposureApi: api } : (() => { throw Error(name); })());
  // G4 the incident ledger panel is a real child of the page too.
  const incidentsModule = loadModule('src/features/assets/exposure/incidents.ts', name => (() => { throw Error(name); })());
  const incidentsComposable = loadModule('src/features/assets/exposure/useExposureIncidents.ts', name => name === 'vue' ? vue : name === './contracts' ? contracts : name === './incidents' ? incidentsModule : (() => { throw Error(name); })());
  let incidentsPanel;
  // P1 UI the coverage drill-down is a real child of the page as well.
  const coverageComposable = loadModule('src/features/assets/exposure/useExposureCoverage.ts', name => name === 'vue' ? vue : name === './contracts' ? contracts : name === './incidents' ? incidentsModule : (() => { throw Error(name); })());
  let coverageDrill;
  const imports = name => {
    if (name === 'vue') return vue;
    if (name === '../api') return { assetApi: api };
    if (name === './useExposureSurface') return surface;
    if (name === './useExposureEvidence') return evidence;
    if (name === './api' || name === './api') return { exposureApi: api };
    if (name === '../features/assets/exposure/useExposureSettings' || name === './useExposureSettings') return settingsModule;
    // G2 the page itself now reads source labels from the real contracts module.
    if (name === './contracts' || name === './contracts') return contracts;
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    if (name.endsWith('.css')) return {};
    // §E2 the settings panel is a real child of the page: compile it, never stub it.
    if (name === './ExposureSettingsPanel.vue') return panel;
    if (name === './ExposureSettingsPanel.vue') return panel;
    if (name === '../features/assets/exposure/useExposureClusters' || name === './useExposureClusters') return clusterModule;
    if (name === './ExposureClustersPanel.vue') return clusterPanel;
    if (name === '../features/assets/exposure/useExposureReview' || name === './useExposureReview') return reviewModule;
    if (name === './ExposureReviewPanel.vue') return reviewPanel;
    if (name === './incidents' || name === './incidents') return incidentsModule;
    if (name === './useExposureIncidents') return incidentsComposable;
    if (name === './ExposureIncidentsPanel.vue') return incidentsPanel;
    if (name === './useExposureCoverage') return coverageComposable;
    if (name === './ExposureCoverageDrill.vue') return coverageDrill;
    throw Error(`Unexpected exposure page import: ${name}`);
  };
  const panelFile = path.join(__dirname, '../src/features/assets/exposure/ExposureSettingsPanel.vue');
  const panelDescriptor = parse(readSource('src/features/assets/exposure/ExposureSettingsPanel.vue'), { filename: panelFile }).descriptor;
  const panelScript = compileScript(panelDescriptor, { id: 'ExposureSettingsPanel.vue' });
  const panelTemplate = compileTemplate({
    source: panelDescriptor.template.content, filename: panelFile, id: 'ExposureSettingsPanel.vue',
    compilerOptions: { bindingMetadata: panelScript.bindings },
  });
  assert.deepEqual(panelTemplate.errors, [], '配置面板模板必须能编译');
  const panelExports = {};
  new Function('require', 'exports', transpile(panelScript.content))(imports, panelExports);
  panel = panelExports.default;
  const clusterFile = path.join(__dirname, '..', 'src/features/assets/exposure/ExposureClustersPanel.vue');
  const clusterDescriptor = parse(readSource('src/features/assets/exposure/ExposureClustersPanel.vue'), { filename: clusterFile }).descriptor;
  const clusterScript = compileScript(clusterDescriptor, { id: 'ExposureClustersPanel.vue' });
  const clusterCompiled = compileTemplate({
    source: clusterDescriptor.template.content, filename: clusterFile, id: 'ExposureClustersPanel.vue',
    compilerOptions: { bindingMetadata: clusterScript.bindings },
  });
  assert.deepEqual(clusterCompiled.errors, [], '簇面板模板必须能编译');
  const clusterExports = {};
  new Function('require', 'exports', transpile(clusterScript.content))(imports, clusterExports);
  clusterPanel = clusterExports.default;
  const reviewFile = path.join(__dirname, '..', 'src/features/assets/exposure/ExposureReviewPanel.vue');
  const reviewDescriptor = parse(readSource('src/features/assets/exposure/ExposureReviewPanel.vue'), { filename: reviewFile }).descriptor;
  const reviewScript = compileScript(reviewDescriptor, { id: 'ExposureReviewPanel.vue' });
  const reviewCompiled = compileTemplate({
    source: reviewDescriptor.template.content, filename: reviewFile, id: 'ExposureReviewPanel.vue',
    compilerOptions: { bindingMetadata: reviewScript.bindings },
  });
  assert.deepEqual(reviewCompiled.errors, [], '复核面板模板必须能编译');
  const reviewExports = {};
  new Function('require', 'exports', transpile(reviewScript.content))(imports, reviewExports);
  reviewPanel = reviewExports.default;
  const incidentsFile = path.join(__dirname, '..', 'src/features/assets/exposure/ExposureIncidentsPanel.vue');
  const incidentsDescriptor = parse(readSource('src/features/assets/exposure/ExposureIncidentsPanel.vue'), { filename: incidentsFile }).descriptor;
  const incidentsScript = compileScript(incidentsDescriptor, { id: 'ExposureIncidentsPanel.vue' });
  const incidentsCompiled = compileTemplate({
    source: incidentsDescriptor.template.content, filename: incidentsFile, id: 'ExposureIncidentsPanel.vue',
    compilerOptions: { bindingMetadata: incidentsScript.bindings },
  });
  assert.deepEqual(incidentsCompiled.errors, [], '事件台账面板模板必须能编译');
  const incidentsExports = {};
  new Function('require', 'exports', transpile(incidentsScript.content))(imports, incidentsExports);
  incidentsPanel = incidentsExports.default;
  const drillFile = path.join(__dirname, '..', 'src/features/assets/exposure/ExposureCoverageDrill.vue');
  const drillDescriptor = parse(readSource('src/features/assets/exposure/ExposureCoverageDrill.vue'), { filename: drillFile }).descriptor;
  const drillScript = compileScript(drillDescriptor, { id: 'ExposureCoverageDrill.vue' });
  const drillCompiled = compileTemplate({
    source: drillDescriptor.template.content, filename: drillFile, id: 'ExposureCoverageDrill.vue',
    compilerOptions: { bindingMetadata: drillScript.bindings },
  });
  assert.deepEqual(drillCompiled.errors, [], '覆盖钻取面板模板必须能编译');
  const drillExports = {};
  new Function('require', 'exports', transpile(drillScript.content))(imports, drillExports);
  coverageDrill = drillExports.default;
  const scriptExports = {}, templateExports = {};
  new Function('require', 'exports', transpile(script.content))(imports, scriptExports);
  new Function('require', 'exports', transpile(template.code))(imports, templateExports);
  const { renderToString } = require('@vue/server-renderer');
  const html = await renderToString(vue.createSSRApp({ ...scriptExports.default, render: templateExports.render }, {
    projects: [{ id: 10, name: '样本', status: 'active' }], selectedProjectId: 10,
  }));
  assert.match(html, /待归并/, '待归并入口必须存在');
  assert.match(html, /开始被动采集/);
  assert.doesNotMatch(html, /完整采集|恢复完成/, '措辞不能预先承诺全量覆盖或恢复成功');
  assert.equal(calls.length, 0, '仅渲染页面绝不能启动采集');
  assert.doesNotMatch(html, /undefined|NaN/);

  // 恢复入口只在有数据时出现，所以这里钉住它的装配位置：总览必须能直接恢复中断的轮次，
  // 且必须走带 expectedProjectId 的接口，而不是重新开一轮。
  const source = readSource('src/features/assets/exposure/ExposureSurface.vue');
  const overview = source.slice(source.indexOf(`view === 'overview'" class="exposure-summary"`));
  const resumeSection = overview.slice(0, overview.indexOf('<section v-if="view === \'runs\'"'));
  assert.match(resumeSection, /按冻结计划恢复该轮/, '中断的轮次必须在总览就能恢复');
  assert.match(resumeSection, /resumeRun\(interruptedRun\.id\)/, '恢复必须针对该轮而不是新建一轮');
  assert.match(source, /status === "interrupted"/, '恢复入口只能出现在确实中断的轮次上');
  assert.match(source, /run\.status === 'interrupted'/, '任务列表同样要区分中断轮次');
});

// ───────────── FIX3（2026-10-09 17:29） ─────────────

test('FIX3-R1 opening finding 3 then 4 in the same project: the late evidence for 3 never overwrites 4', async (t) => {
  const waiting = {};
  const h = mountEvidence(t, {
    evidence: (findingId) => new Promise(resolve => {
      waiting[findingId] = () => resolve(evidencePackage({ finding: { ...evidencePackage().finding, id: findingId } }));
    }),
  });
  const first = h.state.openEvidence(3);
  const second = h.state.openEvidence(4);
  waiting[4]();
  await second;
  waiting[3]();
  await first;
  await flush();
  assert.equal(h.state.evidenceFor.value, 4);
  assert.equal(h.state.evidence.value.finding.id, 4, '迟到的旧证据覆盖了新选中的线索');
  assert.equal(h.state.evidenceLoading.value, false);
});
