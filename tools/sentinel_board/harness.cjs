// Execute the real board setup, template and confirmation component. Only
// desktop IPC/timers and unrelated child panels are substituted.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const vue = require('vue');
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
const board = compile('../../src/features/sentinel/components/SentinelBoard.vue', 'delete-board');
const inline = compile('../../src/shared/components/InlineConfirm.vue', 'delete-confirm');
const inlineExports = {};
new Function('require', 'exports', inline.script)((name) => name === 'vue' ? vue : { AlertTriangle: () => null }, inlineExports);
const InlineConfirm = { ...inlineExports.default, render: inline.render };
const presentation = {};
new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname, '../../src/features/sentinel/presentation.ts'), 'utf8')))(require, presentation);
const tokenUsage = {};
new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname,
  '../../src/features/sentinel/overview/useTokenUsage.ts'), 'utf8')))(
  name => name === '../presentation' ? presentation : require(name), tokenUsage);
const investigationSummary = {};
new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname,
  '../../src/features/sentinel/overview/useInvestigationSummary.ts'), 'utf8')))(require, investigationSummary);
const { loadWorkspaceModule } = require('../result_component_harness.cjs');
const rootDecision = loadWorkspaceModule('src/features/sentinel/components/RootDecisionSummary.vue');
const rootDecisionContract = loadWorkspaceModule('src/features/sentinel/traces/rootDecisionContract.ts');
function overviewComponent(filename, id) {
  const component = compile(`../../src/features/sentinel/components/overview/${filename}`, id);
  const exports = {};
  new Function('require', 'exports', component.script)(name => {
    if (name === '../../presentation') return presentation;
    if (name === '../RootDecisionSummary.vue') return rootDecision;
    if (name === '../../traces/rootDecisionContract') return rootDecisionContract;
    if (name === '../../../../i18n') return { useI18n: () => ({ tr: zh => zh }) };
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    return require(name);
  }, exports);
  return { ...exports.default, render: component.render };
}
const TokenOverview = overviewComponent('SentinelTokenOverview.vue', 'token-overview');
const OverviewSummary = overviewComponent('SentinelOverviewSummary.vue', 'overview-summary');
const OverviewSidebar = overviewComponent('SentinelOverviewSidebar.vue', 'overview-sidebar');
const TaskOverview = overviewComponent('SentinelTaskOverview.vue', 'task-overview');
const TraceTimeline = overviewComponent('../results/SentinelTraceTimeline.vue', 'trace-timeline');
const fusePresentation = {};
new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname, '../../src/features/sentinel/fuse/presentation.ts'), 'utf8')))(require, fusePresentation);
const fuseDetails = {};
new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname, '../../src/features/sentinel/fuse/useFuseDetails.ts'), 'utf8')))(require, fuseDetails);
const fuseDisposition = {};
new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname, '../../src/features/sentinel/fuse/useFuseDisposition.ts'), 'utf8')))(require, fuseDisposition);
const apiResults = {};
new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname, '../../src/features/sentinel/results/useApiResults.ts'), 'utf8')))(
  (name) => name === '../presentation' ? presentation : require(name), apiResults,
);
const taskSearch = {};
const traceContract = {};
new Function('exports', transpile(fs.readFileSync(path.join(__dirname,
  '../../src/features/sentinel/traces/traceDetailContract.ts'), 'utf8')))(traceContract);
const taskTrace = {};
new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname,
  '../../src/features/sentinel/traces/useTaskTrace.ts'), 'utf8')))(
  name => name === './traceDetailContract' ? traceContract : require(name), taskTrace);
new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname,
  '../../src/features/sentinel/results/useTaskSearch.ts'), 'utf8')))(require, taskSearch);
const scanPages = {};
const taskPageContract = {};
new Function('exports', transpile(fs.readFileSync(path.join(__dirname,
  '../../src/features/sentinel/results/taskPageContract.ts'), 'utf8')))(taskPageContract);
new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname,
  '../../src/features/sentinel/results/useScanPages.ts'), 'utf8')))(
  name => name === './taskPageContract' ? taskPageContract : require(name), scanPages);
const renderer = vue.createRenderer({
  createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
  insert() {}, remove() {}, setText() {}, setElementText() {}, patchProp() {},
  parentNode: () => null, nextSibling: () => null,
});
const flush = async () => { for (let i = 0; i < 8; i++) await vue.nextTick(); };
const boardLifecycleScript = transpile(fs.readFileSync(path.join(__dirname,
  '../../src/features/sentinel/results/useBoardLifecycle.ts'), 'utf8'));
const scan = (id = 'delete-a', status = 'completed') => ({ id, status, taskName: id,
  scanType: 'code', projectId: 1, projectName: 'Fixture', updatedAt: '2026-09-26 00:00:00' });
const investigationSnapshot = (gain = 0) => ({ targetCount: 8, nodeCount: 0, edgeCount: 0,
  apiCount: 0, parameterCount: 0, hypothesisCount: 0, readyHypothesisCount: 0, identityDiffCount: 0,
  tokenWorthyCount: 2, averageInformationGain: gain, factCount: 17, promotedStrategyCount: 4 });
async function mount(deleteResult = async () => {}, extraApi = {}, environment = {}) {
  const calls = [], events = [];
  const timers = environment.timers || { setInterval: () => 1, clearInterval() {}, setTimeout: () => 2, clearTimeout() {} };
  const storage = environment.storage || { getItem: () => 'done', setItem() {} };
  const boardLifecycle = {};
  new Function('require', 'exports', 'window', boardLifecycleScript)(require, boardLifecycle, timers);
  const api = new Proxy({}, { get(_, name) {
    if (Object.hasOwn(extraApi, name)) return extraApi[name];
    if (name === 'deleteSentinelScan') return async (id) => { calls.push([name, id]); return deleteResult(id); };
    if (['listSentinelScans', 'listSentinelTargets', 'listSentinelVulnerabilityScanIds', 'listSentinelOpportunities'].includes(name)) {
      return async () => { calls.push([name]); return []; };
    }
    if (name === 'sentinelOverviewStats') return async () => ({});
    if (name === 'investigationOverview') return async () => investigationSnapshot();
    throw new Error(`Unexpected IPC ${String(name)}`);
  } });
  const exports = {};
  new Function('require', 'exports', 'window', 'sessionStorage', 'document', board.script)((name) => {
    if (name === 'vue') return vue;
    if (name === '../api') return { sentinelApi: api };
    if (name === '../../../i18n') return { useI18n: () => ({ tr: (zh) => zh }) };
    if (name === '../presentation') return presentation;
    if (name === '../fuse/presentation') return fusePresentation;
    if (name === '../fuse/useFuseDetails') return fuseDetails;
    if (name === '../fuse/useFuseDisposition') return fuseDisposition;
    if (name === '../results/useApiResults') return apiResults;
    if (name === '../results/useBoardLifecycle') return boardLifecycle;
    if (name === '../results/useTaskSearch') return taskSearch;
    if (name === '../results/useScanPages') return scanPages;
    if (name === '../traces/useTaskTrace') return taskTrace;
    if (name === '../overview/useTokenUsage') return tokenUsage;
    if (name === '../overview/useInvestigationSummary') return investigationSummary;
    if (name === './overview/SentinelTokenOverview.vue') return { default: TokenOverview };
    if (name === './overview/SentinelOverviewSummary.vue') return { default: OverviewSummary };
    if (name === './overview/SentinelOverviewSidebar.vue') return { default: OverviewSidebar };
    if (name === './overview/SentinelTaskOverview.vue') return { default: TaskOverview };
    if (name === './results/SentinelTraceTimeline.vue') return { default: TraceTimeline };
    if (name === '@tauri-apps/api/event') return { listen: environment.listen || (async () => () => {}) };
    if (name === '@tauri-apps/plugin-opener') return { openUrl: () => { throw new Error('No external IO'); } };
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    if (name === '../../../shared/components/InlineConfirm.vue') return { default: InlineConfirm };
    // The board lazy-loads these unrelated panels; this harness exercises its
    // own setup/template and keeps the panels outside this deletion boundary.
    if (name === './lazyPanels') return {
      AgentDialog: () => null, AgentTraceHub: () => null,
      AgentWorkbench: () => null, SentinelSourceResults: () => null,
      SentinelTaskCenter: () => null,
    };
    if (name.endsWith('.vue')) return { default: () => null };
    throw new Error(`Unexpected import ${name}`);
  }, exports, timers, storage, { hidden: false });
  let bindings;
  const setup = exports.default.setup;
  const component = { ...exports.default, setup(props, ctx) { bindings = setup(props, ctx); return () => null; } };
  const props = vue.reactive({ projects: [], section: 'help', active: false, search: '', ...environment.props });
  const app = renderer.createApp({ setup: () => () => vue.h(component, { ...props,
    onNotify: (...args) => events.push(args),
  }) });
  if (environment.onError) app.config.errorHandler = environment.onError;
  app.mount({});
  await flush();
  return { b: bindings, calls, events, props, unmount: () => app.unmount(),
    vnode: () => board.render({}, [], props, vue.proxyRefs(bindings), {}, {}),
    render: () => renderToString(vue.createSSRApp({ render: () => board.render({}, [], props, vue.proxyRefs(bindings), {}, {}) })),
  };
}

function mountView(render) {
  const node = tag => ({ tag, props: {}, children: [] });
  const remove = child => {
    if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1);
    child.parent = undefined;
  };
  const renderer = vue.createRenderer({
    createElement: node, createText: text => ({ ...node('text'), text }),
    createComment: text => ({ ...node('comment'), text }),
    insert(child, parent, anchor) {
      remove(child); child.parent = parent;
      const index = anchor ? parent.children.indexOf(anchor) : -1;
      parent.children.splice(index < 0 ? parent.children.length : index, 0, child);
    },
    remove, setText: (child, text) => { child.text = text; },
    setElementText: (child, text) => { child.text = text; child.children = []; },
    patchProp: (child, key, _old, value) => { child.props[key] = value; },
    parentNode: child => child.parent,
    nextSibling: child => child.parent?.children[child.parent.children.indexOf(child) + 1],
  });
  const root = node('root');
  const findAll = tag => {
    const found = [];
    const visit = child => { if (child.tag === tag) found.push(child); child.children.forEach(visit); };
    visit(root); return found;
  };
  const app = renderer.createApp({ setup: () => render });
  app.mount(root);
  return { findAll, unmount: () => app.unmount() };
}

module.exports = { mount, scan, flush, presentation, TokenOverview, TaskOverview, investigationSnapshot, mountView };
