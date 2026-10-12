// §E1 — the entry-level truth of the exposure page: a read that never landed is an error,
// never an empty list; a refused scope or a corrupt cursor surfaces its own reason; and
// project switching, staleness and unmounting behave under out-of-order responses.
// Real SFC rendering plus the real composable; only the IPC boundary is faked.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const { renderToString } = require('@vue/server-renderer');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');

const flush = async () => { for (let i = 0; i < 30; i++) await vue.nextTick(); };
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

const summary = (projectId, over = {}) => ({
  projectId, projectActive: true, eligibleAssetsTotal: 4, eligibleAssetsPlanned: 4, assetsBeyondPlan: 0,
  findingsTotal: 2, findingsNew: 2, findingsReviewing: 0, findingsConfirmed: 0, findingsDismissed: 0,
  highRiskOpen: 0, knownLimits: [], ...over,
});
const finding = (id, projectId) => ({
  id, projectId, category: 'source_code', title: `线索 ${id}`, severity: 'high', confidence: 80,
  status: 'new', sourceUrl: `https://p.invalid/${id}`, evidenceExcerpt: '', assetLabel: '',
  sourceType: 'passive_web', exposureEligible: true, firstSeenAt: 't', lastSeenAt: 't',
});
const run = (id, projectId, status = 'completed') => ({
  id, projectId, status, eligibleAssets: 2, scannedAssets: 2, fetchedResources: 4, findings: 1,
  stage: 'completed', currentSource: '', error: '', createdAt: 't', startedAt: 't', completedAt: 't',
});
const page = (items, total, over = {}) => ({ items, total, hasMore: false, nextCursor: '', pageSize: 50, ...over });

function mountSurface(t, api) {
  const contracts = loadContracts();
  const composable = loadModule('src/features/assets/exposure/useExposureSurface.ts', name => {
    if (name === 'vue') return vue;
    // G2 the page itself now reads source labels from the real contracts module.
    if (name === './contracts' || name === '../features/assets/exposure/contracts') return contracts;
    throw Error(`Unexpected import ${name}`);
  });
  const selection = vue.reactive({ project: 10, active: true });
  const events = [];
  let state;
  const renderer = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {}, parentNode: () => null, nextSibling: () => null });
  const app = renderer.createApp({
    setup() {
      state = composable.useExposureSurface({
        projectId: () => selection.project,
        active: () => selection.active,
        api,
        notify: (kind, text) => events.push([kind, text]),
      });
      return () => null;
    },
  });
  app.mount({});
  if (t && typeof t.after === 'function') t.after(() => app.unmount());
  return { state, events, selection };
}

/// The page rendered once, with the IPC boundary scripted.
async function renderPage(api) {
  const contracts = loadContracts();
  const surface = loadModule('src/features/assets/exposure/useExposureSurface.ts', name => name === 'vue' ? vue : name === './contracts' ? contracts : (() => { throw Error(name); })());
  const evidence = loadModule('src/features/assets/exposure/useExposureEvidence.ts', name => name === 'vue' ? vue : name === './contracts' ? contracts : (() => { throw Error(name); })());
  const filename = path.join(__dirname, '..', 'src/components/ExposureSurface.vue');
  const { descriptor } = parse(readSource('src/components/ExposureSurface.vue'), { filename });
  const script = compileScript(descriptor, { id: 'ExposureSurface.vue' });
  const template = compileTemplate({ source: descriptor.template.content, filename, id: 'ExposureSurface.vue', compilerOptions: { bindingMetadata: script.bindings } });
  assert.deepEqual(template.errors, [], '模板必须能编译');
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
    if (name === '../api') return { api: { listAssets: async () => ({ items: [], total: 0, page: 1, pageSize: 100, summary: {} }) } };
    if (name === '../features/assets/exposure/api' || name === './api') return { exposureApi: api };
    if (name === '../features/assets/exposure/useExposureSurface') return surface;
    if (name === '../features/assets/exposure/useExposureEvidence') return evidence;
    if (name === '../features/assets/exposure/useExposureSettings' || name === './useExposureSettings') return settingsModule;
    // G2 the page itself now reads source labels from the real contracts module.
    if (name === './contracts' || name === '../features/assets/exposure/contracts') return contracts;
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    if (name.endsWith('.css')) return {};
    // §E2 the page mounts its own settings panel: it is compiled for real below, so a panel
    // that cannot build breaks this gate instead of being stubbed away.
    if (name === '../features/assets/exposure/ExposureSettingsPanel.vue') return panel;
    if (name === '../features/assets/exposure/useExposureClusters' || name === './useExposureClusters') return clusterModule;
    if (name === '../features/assets/exposure/ExposureClustersPanel.vue') return clusterPanel;
    if (name === '../features/assets/exposure/useExposureReview' || name === './useExposureReview') return reviewModule;
    if (name === '../features/assets/exposure/ExposureReviewPanel.vue') return reviewPanel;
    if (name === '../features/assets/exposure/incidents' || name === './incidents') return incidentsModule;
    if (name === './useExposureIncidents') return incidentsComposable;
    if (name === '../features/assets/exposure/ExposureIncidentsPanel.vue') return incidentsPanel;
    if (name === './useExposureCoverage') return coverageComposable;
    if (name === '../features/assets/exposure/ExposureCoverageDrill.vue') return coverageDrill;
    throw Error(`Unexpected page import: ${name}`);
  };
  const panelFile = path.join(__dirname, '..', 'src/features/assets/exposure/ExposureSettingsPanel.vue');
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
  const html = await renderToString(vue.createSSRApp({ ...scriptExports.default, render: templateExports.render }, {
    projects: [{ id: 10, name: '样本', status: 'active' }], selectedProjectId: 10,
  }));
  return html;
}

const okApi = (over = {}) => ({
  exposureScopeSummary: async projectId => summary(projectId),
  pageExposureFindings: async input => page([finding(5, input.projectId)], 1),
  pageExposureRuns: async projectId => page([run(9, projectId, 'running')], 1),
  listExposureSourceResultsForProject: async () => [],
  listAssets: async () => ({ items: [], total: 0, page: 1, pageSize: 100, summary: {} }),
  startExposureScan: async () => 77,
  cancelExposureScan: async () => {},
  reviewExposureFinding: async () => {},
  saveExposureFinding: async () => 1,
  exportExposureFindings: async () => ({ path: '/tmp/x.csv', rows: 0 }),
  ...over,
});

test('a first read that fails is an error state, not an empty list', async (t) => {
  const refused = '[exposure_scope_mismatch] 该线索或任务不属于本项目，已拒绝读取';
  const h = mountSurface(t, okApi({
    exposureScopeSummary: async () => { throw Error(refused); },
    pageExposureFindings: async () => { throw Error(refused); },
    pageExposureRuns: async () => { throw Error(refused); },
  }));
  await h.state.load();
  await flush();
  assert.equal(h.state.hasSnapshot.value, false, '一次都没读到时不能假装已有快照');
  assert.match(h.state.readError.value, /不属于本项目/, '真实原因必须留在页面上');
  assert.equal(h.state.findings.value.length, 0);
  // The page's empty branch is gated on exactly that pair, so the state above is what
  // decides between “读失败” and “这个筛选真的没有线索”.
  const source = readSource('src/components/ExposureSurface.vue');
  assert.match(source, /v-else-if="readError && !hasSnapshot"/, '错误分支必须先于空列表判断');
  assert.match(source, /不能显示为“没有线索”/);
  assert.ok(
    source.indexOf('不能显示为“没有线索”') < source.indexOf('当前筛选没有公开暴露线索'),
    '错误分支必须排在空状态之前',
  );
});

test('a successful read with no rows still says the filter is empty', async () => {
  const html = await renderPage(okApi({
    pageExposureFindings: async input => page([], 0),
  }));
  assert.match(html, /当前筛选没有公开暴露线索/);
  assert.doesNotMatch(html, /不能显示为“没有线索”/);
});

test('corrupt cursor and refused scope keep their own reason on the page', async (t) => {
  const corrupt = mountSurface(t, okApi({
    pageExposureFindings: async () => { throw Error('[exposure_cursor_invalid] 游标无法解析，已拒绝而不是从头返回'); },
    pageExposureRuns: async () => page([], 0),
    exposureScopeSummary: async projectId => summary(projectId),
  }));
  await corrupt.state.load();
  await flush();
  assert.equal(corrupt.state.hasSnapshot.value, false);
  assert.match(corrupt.state.readError.value, /游标无法解析/);
  assert.doesNotMatch(corrupt.state.readError.value, /exposure_cursor_invalid/, '错误码不能直接倒给用户');

  const leaked = mountSurface(t, okApi({
    pageExposureFindings: async () => { throw Error('[exposure_query_failed] 暴露面读取失败：Bearer sk-live-secretvalue'); },
    pageExposureRuns: async () => page([], 0),
  }));
  await leaked.state.load();
  await flush();
  assert.doesNotMatch(leaked.state.readError.value, /sk-live-secretvalue/, '错误文本必须统一脱敏');
  assert.match(leaked.state.readError.value, /暴露面接口返回异常/);
});

test('A→B→A keeps the newest read for the project actually on screen', async (t) => {
  const first = deferred(), second = deferred(), back = deferred();
  const seen = [];
  const h = mountSurface(t, okApi({
    exposureScopeSummary: projectId => {
      seen.push(projectId);
      if (seen.length === 1) return first.promise;
      if (seen.length === 2) return second.promise;
      return back.promise;
    },
  }));
  void h.state.load();
  await flush();
  h.selection.project = 20;
  void h.state.load();
  await flush();
  second.resolve(summary(20, { findingsTotal: 5, eligibleAssetsTotal: 5, eligibleAssetsPlanned: 5 }));
  await flush();
  h.selection.project = 10;
  void h.state.load();
  await flush();
  back.resolve(summary(10, { findingsTotal: 4 }));
  await flush();
  first.resolve(summary(11, { findingsTotal: 999 }));
  await flush();
  await h.state.load();
  back.resolve(summary(10, { findingsTotal: 4 }));
  await flush();
  assert.equal(h.state.summary.value.projectId, 10, '停在 A 项目');
  assert.equal(h.state.summary.value.findingsTotal, 4, 'A 项目的最新读数不能被 B 或迟到结果覆盖');
});

test('a failed refresh keeps the last trustworthy snapshot and marks it stale', async (t) => {
  let fail = false;
  const h = mountSurface(t, okApi({
    exposureScopeSummary: async projectId => {
      if (fail) throw Error('[exposure_query_failed] 数据库繁忙');
      return summary(projectId);
    },
  }));
  await h.state.load();
  await flush();
  assert.equal(h.state.hasSnapshot.value, true);
  assert.equal(h.state.stale.value, false);
  const before = h.state.summary.value.findingsTotal;
  fail = true;
  await h.state.load();
  await flush();
  assert.equal(h.state.stale.value, true, '刷新失败必须标 stale');
  assert.equal(h.state.summary.value.findingsTotal, before, '旧快照必须保留');
  assert.equal(h.state.hasSnapshot.value, true, '已有快照时不能退回“没读到”状态');
  assert.match(h.state.readError.value, /数据库繁忙/);
});

test('unmounting stops the poll and hidden pages do not refresh', async (t) => {
  let calls = 0;
  const h = mountSurface(t, okApi({
    exposureScopeSummary: async projectId => { calls += 1; return summary(projectId); },
    pageExposureRuns: async projectId => page([run(9, projectId, 'running')], 1),
  }));
  await h.state.load();
  await flush();
  const at = calls;
  h.selection.active = false;
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.equal(calls, at, '页面不可见时不得继续轮询');
  h.selection.active = true;
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.ok(calls >= at, '恢复可见后可以继续');
});

test('summary and list use one project predicate and the same filtered total is explainable', async (t) => {
  const asked = [];
  const h = mountSurface(t, okApi({
    exposureScopeSummary: async projectId => { asked.push(['summary', projectId]); return summary(10, { findingsTotal: 7 }); },
    pageExposureFindings: async input => { asked.push(['findings', input.projectId, input.status]); return page([finding(1, 10)], 7, { hasMore: true, nextCursor: 'c2' }); },
  }));
  asked.length = 0;
  h.state.statusFilter.value = 'new';
  await h.state.load();
  await flush();
  assert.ok(asked.some(row => row[0] === 'summary' && row[1] === 10), `汇总必须按当前项目：${JSON.stringify(asked)}`);
  const findings = asked.find(row => row[0] === 'findings');
  assert.deepEqual(findings, ['findings', 10, 'new'], '列表必须带当前筛选谓词与同一项目');
  assert.equal(h.state.findingsTotal.value, 7, '总数来自后端筛选结果');
  assert.equal(h.state.hasMore.value, true);
  assert.equal(h.state.nextCursor.value, 'c2', '只有 hasMore 时才保留游标');
});
