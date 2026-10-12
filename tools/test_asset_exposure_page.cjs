// §Q0 — the real `ExposureSurface.vue` is rendered and mounted here: the wording has to
// match what the collector can actually do, the data sources have to follow the selected
// run, and neither mounting nor re-rendering may start a collection.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const vue = require('vue');
const { renderToString } = require('@vue/server-renderer');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');

const readSource = relative => fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');
const transpile = source => ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

const loadTsModule = (relative, requireModule) => {
  const exports = {};
  new Function('require', 'exports', transpile(readSource(relative)))(requireModule, exports);
  return exports;
};

/// Compile a real SFC and evaluate its script and template with the given require.
/// A child component is never stubbed away here: a panel that cannot compile fails this gate.
function compileSfc(relative, requireModule) {
  const filename = path.join(__dirname, '..', relative);
  const id = path.basename(filename);
  const { descriptor } = parse(readSource(relative), { filename });
  const script = compileScript(descriptor, { id });
  const template = compileTemplate({
    source: descriptor.template.content, filename, id,
    compilerOptions: { bindingMetadata: script.bindings },
  });
  assert.deepEqual(template.errors, [], `${relative} 模板必须能编译`);
  const scriptExports = {}, templateExports = {};
  new Function('require', 'exports', transpile(script.content))(requireModule, scriptExports);
  new Function('require', 'exports', transpile(template.code))(requireModule, templateExports);
  return { component: { ...scriptExports.default, render: templateExports.render }, renderSource: template.code };
}

/// The page under test, with only the IPC boundary faked. The composable, the contracts and
/// the settings panel are the real modules the page ships with.
function fakeApi(calls) {
  return {
    exposureScopeSummary: async projectId => { calls.push(['summary', projectId]); return null; },
    pageExposureFindings: async () => { calls.push(['findings']); return null; },
    pageExposureRuns: async () => { calls.push(['runs']); return null; },
    listExposureSourceResultsForProject: async () => { calls.push(['sources']); return []; },
    listAssets: async () => { calls.push(['assets']); return { items: [], total: 0, page: 1, pageSize: 100, summary: {} }; },
    startExposureScan: async projectId => { calls.push(['start', projectId]); return 77; },
    cancelExposureScan: async () => { calls.push(['cancel']); },
    reviewExposureFinding: async () => { calls.push(['review']); },
    saveExposureFinding: async () => { calls.push(['save']); return 1; },
    exportExposureFindings: async () => { calls.push(['export']); return { path: '/tmp/x.csv', rows: 0 }; },
    pageExposurePendingDiscoveries: async (projectId) => { calls.push(['pending', projectId]); throw Error('unknown command: page_exposure_pending_discoveries'); },
    readExposureEvidence: async () => { calls.push(['evidence']); throw Error('unknown command: read_exposure_evidence'); },
    resumeExposureScan: async () => { calls.push(['resume']); throw Error('unknown command: resume_exposure_scan'); },
    getExposureSettings: async projectId => { calls.push(['settings', projectId]); return null; },
    saveExposureSettings: async (projectId, expectedVersion) => { calls.push(['settingsSave', expectedVersion]); return 1; },
    listExposureSettingVersions: async () => { calls.push(['settingsHistory']); return null; },
    pageExposureClusters: async projectId => { calls.push(['clusters', projectId]); throw Error('unknown command: page_exposure_clusters'); },
    pageExposureClusterMembers: async () => { calls.push(['clusterMembers']); throw Error('unknown command: page_exposure_cluster_members'); },
    exposureReviewTaxonomy: async () => { calls.push(['taxonomy']); throw Error('unknown command: exposure_review_taxonomy'); },
    listExposureReviewTrail: async () => { calls.push(['trail']); throw Error('unknown command: list_exposure_review_trail'); },
    readExposureRetestOutcome: async () => { calls.push(['retestOutcome']); throw Error('unknown command: read_exposure_retest_outcome'); },
    pageExposureRemediations: async () => { calls.push(['remediations']); throw Error('unknown command: page_exposure_remediations'); },
    reviewExposureFindingScoped: async () => { calls.push(['reviewScoped']); throw Error('unknown command: review_exposure_finding_scoped'); },
    saveExposureRemediation: async () => { calls.push(['remediationSave']); throw Error('unknown command: save_exposure_remediation'); },
    startExposureRetest: async () => { calls.push(['retestStart']); throw Error('unknown command: start_exposure_retest'); },
    markExposureRemediation: async () => { calls.push(['remediationMark']); throw Error('unknown command: mark_exposure_remediation'); },
    verifyExposureExport: async () => { calls.push(['verifyExport']); throw Error('unknown command: verify_exposure_export'); },
  };
}

function loadPage(calls, api = fakeApi(calls)) {
  const contractsLoader = () => loadTsModule('src/features/assets/exposure/contracts.ts', () => { throw Error('unexpected'); });
  const requireComposable = name => {
    if (name === 'vue') return vue;
    if (name === './contracts') return contractsLoader();
    throw Error(`Unexpected exposure module import: ${name}`);
  };
  const evidence = loadTsModule('src/features/assets/exposure/useExposureEvidence.ts', requireComposable);
  const surface = loadTsModule('src/features/assets/exposure/useExposureSurface.ts', requireComposable);
  const settings = loadTsModule('src/features/assets/exposure/useExposureSettings.ts', requireComposable);
  const reviewing = loadTsModule('src/features/assets/exposure/useExposureReview.ts', name => {
    if (name === 'vue') return vue;
    if (name === './contracts') return contractsLoader();
    if (name === './api') return { exposureApi: api };
    throw Error(`Unexpected review composable import: ${name}`);
  });
  const clustering = loadTsModule('src/features/assets/exposure/useExposureClusters.ts', name => {
    if (name === 'vue') return vue;
    if (name === './contracts') return contractsLoader();
    if (name === './api') return { exposureApi: api };
    throw Error(`Unexpected cluster composable import: ${name}`);
  });
  // Both the page and its panel resolve through this one require: the page reaches modules
  // as `../features/assets/exposure/x`, the panel as `./x`, and nothing outside that
  // directory is allowed in.
  const asExposureModule = name => {
    for (const prefix of ['../features/assets/exposure/', './']) {
      if (name.startsWith(prefix)) return name.slice(prefix.length);
    }
    return null;
  };
  // G4 the incident ledger panel and its modules are compiled for real as well.
  const incidentsModule = loadTsModule('src/features/assets/exposure/incidents.ts', () => { throw Error('unexpected'); });
  let incidentsComposable;
  let panel, clusterPanel, reviewPanel, incidentsPanel, coverageComposable, coverageDrill;
  const requireModule = name => {
    if (name === 'vue') return vue;
    if (name === '../api') return { api };
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    if (name.endsWith('.css')) return {};
    const key = asExposureModule(name);
    if (key === 'contracts') return contractsLoader();
    if (key === 'api') return { exposureApi: api };
    if (key === 'useExposureSurface') return surface;
    if (key === 'useExposureEvidence') return evidence;
    if (key === 'useExposureSettings') return settings;
    if (key === 'useExposureClusters') return clustering;
    if (key === 'useExposureReview') return reviewing;
    if (key === 'incidents') return incidentsModule;
    if (key === 'useExposureIncidents') {
      incidentsComposable ??= loadTsModule('src/features/assets/exposure/useExposureIncidents.ts', requireModule);
      return incidentsComposable;
    }
    if (key === 'ExposureIncidentsPanel.vue') {
      incidentsPanel ??= compileSfc('src/features/assets/exposure/ExposureIncidentsPanel.vue', requireModule);
      return incidentsPanel.component;
    }
    // P1 UI the coverage drill-down is compiled for real, never stubbed.
    if (key === 'useExposureCoverage') {
      coverageComposable ??= loadTsModule('src/features/assets/exposure/useExposureCoverage.ts', requireModule);
      return coverageComposable;
    }
    if (key === 'ExposureCoverageDrill.vue') {
      coverageDrill ??= compileSfc('src/features/assets/exposure/ExposureCoverageDrill.vue', requireModule);
      return coverageDrill.component;
    }
    if (key === 'ExposureSettingsPanel.vue') {
      panel ??= compileSfc('src/features/assets/exposure/ExposureSettingsPanel.vue', requireModule);
      return panel.component;
    }
    if (key === 'ExposureClustersPanel.vue') {
      clusterPanel ??= compileSfc('src/features/assets/exposure/ExposureClustersPanel.vue', requireModule);
      return clusterPanel.component;
    }
    if (key === 'ExposureReviewPanel.vue') {
      reviewPanel ??= compileSfc('src/features/assets/exposure/ExposureReviewPanel.vue', requireModule);
      return reviewPanel.component;
    }
    throw Error(`Unexpected exposure page import: ${name}`);
  };
  const page = compileSfc('src/components/ExposureSurface.vue', requireModule);
  // Touch both children so a page that imports them is always compiled with them.
  requireModule('../features/assets/exposure/ExposureSettingsPanel.vue');
  requireModule('../features/assets/exposure/ExposureClustersPanel.vue');
  requireModule('../features/assets/exposure/ExposureReviewPanel.vue');
  requireModule('../features/assets/exposure/ExposureIncidentsPanel.vue');
  requireModule('../features/assets/exposure/ExposureCoverageDrill.vue');
  return { ...page, panel, clusters: clusterPanel, review: reviewPanel, requireModule };
}

test('the exposure page describes a bounded passive collection and never starts one on its own', async () => {
  const calls = [];
  const { component } = loadPage(calls);
  const html = await renderToString(vue.createSSRApp(component, {
    projects: [{ id: 10, name: '样本', status: 'active' }], selectedProjectId: 10,
  }));
  assert.match(html, /开始被动采集/);
  assert.doesNotMatch(html, /完整采集/, '按钮不能再承诺全量覆盖');
  assert.match(html, /尚未读取采集范围/, '读不到范围时必须明说，而不是显示一个看起来完整的数字');
  assert.match(html, /数据源/, '数据源入口必须存在');
  assert.match(html, /0 \/ 0 条当前筛选结果/, '线索数量必须带上筛选总数');
  assert.doesNotMatch(html, /undefined|NaN/);
  // 补充静态检查：数据源区必须按选中的 run 取数，页面不再假定一页 500 就是全量。
  const source = readSource('src/components/ExposureSurface.vue');
  assert.match(source, /selectedRunId[\s\S]*listExposureSourceResultsForProject|showSourcesFor\(run\.id\)/,
    '数据源必须跟随所选任务');
  assert.doesNotMatch(source, /pageSize: *500/, '页面不能再请求一页 500 然后假定拿全了');
  assert.deepEqual(calls.filter(row => row[0] === 'start'), [], '渲染页面不得启动采集');
});

test('the exposure page without a project offers no collection at all', async () => {
  const calls = [];
  const { component } = loadPage(calls);
  const html = await renderToString(vue.createSSRApp(component, { projects: [], selectedProjectId: undefined }));
  assert.match(html, /请先选择工作空间/);
  assert.match(html, /disabled[^>]*><!---->开始被动采集/, '没有工作空间时启动按钮必须是禁用的');
  assert.deepEqual(calls.filter(row => row[0] === 'start'), []);
});

test('the exposure page wires collection to one explicit control only', async () => {
  const calls = [];
  const { component, renderSource } = loadPage(calls);
  await renderToString(vue.createSSRApp(component, {
    projects: [{ id: 10, name: '样本', status: 'active' }], selectedProjectId: 10,
  }));
  // The compiled render function is the authority: exactly one click binding may reach
  // the collection starter, and no lifecycle hook or watcher may call it.
  const starters = renderSource.match(/onClick:[^\n]*startScan/g) || [];
  assert.equal(starters.length, 1, `编译后的渲染里只能有一处启动入口，实际 ${starters.length} 处`);
  // Nothing else in the compiled render may reach the collector starter.
  assert.doesNotMatch(renderSource, /onMounted[\s\S]{0,80}startScan/, '挂载不能顺手启动采集');
  assert.deepEqual(calls, [], 'SSR 渲染阶段不得发出任何 IPC，更不得启动采集');
});

/// A minimal in-memory host: enough for Vue to mount a real template and run its lifecycle,
/// without a browser and without pretending the markup is correct.
function createMemoryRenderer() {
  return vue.createRenderer({
    createElement: tag => ({ tag, children: [], props: {} }),
    createText: text => ({ text }),
    createComment: text => ({ text }),
    setText: (node, text) => { node.text = text; },
    setElementText: (node, text) => { node.children = [{ text }]; node.text = text; },
    patchProp: (el, key, _prev, value) => { el.props[key] = value; },
    insert: (child, parent, anchor) => {
      child.parent = parent;
      const at = anchor ? parent.children.indexOf(anchor) : -1;
      if (at >= 0) parent.children.splice(at, 0, child);
      else parent.children.push(child);
    },
    remove: node => {
      if (node.parent) node.parent.children = node.parent.children.filter(row => row !== node);
    },
    parentNode: node => node.parent ?? null,
    nextSibling: node => {
      const at = node.parent ? node.parent.children.indexOf(node) : -1;
      return at >= 0 ? node.parent.children[at + 1] ?? null : null;
    },
    querySelector: () => null,
    setScopeId() {},
    cloneNode: node => ({ ...node, children: [] }),
    insertStaticContent(content, parent) {
      const first = { text: content };
      this.insert(first, parent, null);
      return [first, first];
    },
  });
}

test('the collection settings panel is a real mounted child and never reaches the collector', async (t) => {
  const calls = [];
  const { panel } = loadPage(calls);
  const html = await renderToString(vue.createSSRApp(panel.component, { projectId: 10 }));
  assert.match(html, /暴露面采集配置/);
  assert.match(html, /保存为新版本/);
  assert.doesNotMatch(html, /开始被动采集|停止采集|恢复采集/, '配置面板里不能出现采集入口');
  assert.deepEqual(calls, [], '渲染配置面板不得发出 IPC');

  // Mounting the panel is what triggers the read: exactly the settings call, nothing else.
  const renderer = createMemoryRenderer();
  const app = renderer.createApp(panel.component, { projectId: 10 });
  const container = { tag: '#root', children: [], props: {} };
  app.mount(container);
  t.after(() => app.unmount());
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(calls.map(row => row[0]), ['settings'],
    `挂载配置面板只应该读配置，实际 ${JSON.stringify(calls)}`);
  const source = readSource('src/features/assets/exposure/ExposureSettingsPanel.vue');
  assert.doesNotMatch(source, /startExposureScan|resumeExposureScan|cancelExposureScan/,
    '配置面板不得持有启动、恢复或停止采集的能力');
  assert.doesNotMatch(source, /from "\.\.\/\.\.\/api"|from "\.\.\/\.\.\/\.\.\/api"/,
    '配置面板只能走暴露面自己的 IPC 包装，不能借用全局 api');
});

// G0/P1-4 — the compiled page wires a finding's review button to the scoped panel only: the
// render function opens the panel and carries no unversioned status write.
test('the findings list opens the scoped review panel instead of writing a status', async () => {
  const calls = [];
  const { component, renderSource } = loadPage(calls);
  assert.match(renderSource, /openReview\(item\.id\)/, '列表按钮只打开复核整改面板');
  assert.doesNotMatch(renderSource, /review\(item, ['"](reviewing|confirmed|dismissed)['"]\)/, '列表不能直接写复核状态');
  const html = await renderToString(vue.createSSRApp(component, {
    projects: [{ id: 10, name: '样本', status: 'active' }], selectedProjectId: 10,
  }));
  assert.doesNotMatch(html, /undefined|NaN/);
  assert.deepEqual(calls.filter(row => row[0] === 'review'), [], '渲染不得调用旧复核入口');
});
