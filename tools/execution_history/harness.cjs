// Render the actual task-detail SFC. IPC/icons/form are mocked; a source-evidence
// child probe verifies parent scope wiring. Its actual SFC is tested separately.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const vue = require('vue');
const { renderToString } = require('@vue/server-renderer');
const transpile = (text) => ts.transpileModule(text, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

const historicalContract = {};
new Function('exports', transpile(fs.readFileSync(path.join(__dirname,
  '../../src/features/sentinel/results/historicalPreviewContract.ts'), 'utf8')))(historicalContract);
const taskPageContract = {};
new Function('exports', transpile(fs.readFileSync(path.join(__dirname,
  '../../src/features/sentinel/results/taskPageContract.ts'), 'utf8')))(taskPageContract);

const filename = path.join(__dirname, '../../src/features/sentinel/components/SentinelTaskCenter.vue');
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'execution-history-test' });
const template = compileTemplate({ source: descriptor.template.content, filename, id: 'execution-history-test',
  compilerOptions: { bindingMetadata: script.bindings } });
assert.deepEqual(template.errors, []);
const templateExports = {};
new Function('require', 'exports', transpile(template.code))(require, templateExports);
const presentation = {};
new Function('require', 'exports', transpile(fs.readFileSync(path.join(__dirname, '../../src/features/sentinel/presentation.ts'), 'utf8')))(require, presentation);
const renderer = vue.createRenderer({ createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
  insert() {}, remove() {}, setText() {}, setElementText() {}, patchProp() {}, parentNode: () => null, nextSibling: () => null });
const flush = async () => { for (let i = 0; i < 10; i++) await vue.nextTick(); };
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
function searchClock() {
  let next = 0;
  const jobs = new Map();
  return {
    setTimeout(callback, delay) { assert.equal(delay, 250); jobs.set(++next, callback); return next; },
    clearTimeout(id) { jobs.delete(id); },
    fire() { const callbacks = [...jobs.values()]; jobs.clear(); callbacks.forEach(callback => callback()); },
    get size() { return jobs.size; },
  };
}
const scan = (id) => ({ id, status: 'completed_with_gaps', scanType: 'code', updatedAt: id, taskName: id, progress: 100 });
const page = (scanId = 'A', invocations = [], extra = {}) => ({ schemaVersion: 2, scanId, attemptNumber: 1,
  invocations, hasOlder: false, olderCursor: null, ...extra });
const source = (id, status = 'completed', extra = {}) => ({ id, origin: 'source_receipt', role: 'source_analyst',
  toolName: 'repo.inventory', status, receiptIntegrity: status === 'planned' ? 'pending' : status === 'unverified' ? 'unverified' : 'matched',
  recordedAt: '2026-09-27 01:02:03', startedAt: '', finishedAt: status === 'planned' ? '' : '2026-09-27 01:02:04',
  runId: 'child', invocationId: id, hasRequestArtifact: false, hasResponseArtifact: false, ...extra });
async function mount(history, overrides = {}, clock = { setTimeout, clearTimeout }) {
  const calls = [];
  const api = {
    listSentinelScanAttempts: async () => [1, 2].map(attemptNumber => ({ attemptNumber, status: 'completed_with_gaps' })),
    getNativeScanStatus: async () => ({ stopDiagnostic: { obligations: [] }, attemptNumber: 1 }),
    listAgentLearningCandidates: async () => [], listAgentKnowledge: async () => [], listHistoricalImportPreviews: async () => [],
    readSentinelRunnerLog: async (scanId, attempt) => ({ scanId, attempt, lines: [] }),
    getNativeAttemptMailboxHistory: async (scanId, attemptNumber) => ({ scanId, attemptNumber, messages: [], hasOlder: false }),
    getNativeAttemptToolHistory: (...args) => { calls.push(args); return history(...args); },
    ...overrides,
  };
  const exports = {};
  new Function('require', 'exports', 'setTimeout', 'clearTimeout', transpile(script.content))((name) => {
    if (name === 'vue') return vue;
    if (name === '../api') return { sentinelApi: api };
        if (name === '../../skills/catalogApi') return { knowledgeCatalogApi: api };
    if (name === '../results/taskPageContract') return taskPageContract;
    if (['../execution/useTaskExecutionDetails', '../results/useHistoricalImportLedger'].includes(name)) {
      const details = {};
      const source = fs.readFileSync(path.join(__dirname, '../../src/features/sentinel', `${name.slice(3)}.ts`), 'utf8');
      new Function('require', 'exports', transpile(source))((dependency) => {
        if (dependency === 'vue') return vue;
        if (dependency === '../api') return { sentinelApi: api };
        if (dependency === '../../skills/catalogApi') return { knowledgeCatalogApi: api };
        if (dependency === './useLiveRunnerLog') return { useLiveRunnerLog() {} };
        if (['../results/historicalPreviewContract', './historicalPreviewContract'].includes(dependency)) return historicalContract;
        throw new Error(`Unexpected business module import ${dependency}`);
      }, details);
      return details;
    }
    if (name === '../presentation') return presentation;
    if (name === '../../../i18n') return { useI18n: () => ({ tr: (zh) => zh }) };
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    if (name === './SourceReviewEvidence.vue') return { default: {
      props: ['scanId', 'attemptNumber'],
      setup: props => () => vue.h('div', { 'data-source-scan': props.scanId, 'data-source-attempt': props.attemptNumber }),
    } };
    if (name.endsWith('.vue')) return { default: () => null };
    throw new Error(`Unexpected import ${name}`);
  }, exports, clock.setTimeout, clock.clearTimeout);
  let bindings;
  const setup = exports.default.setup;
  const component = { ...exports.default, setup(props, ctx) { bindings = setup(props, ctx); return () => null; } };
  const props = vue.reactive({ scans: [], hasMore: false, loadingMore: false, preview: scan('A'), previewTargets: [],
    attentionCount: 0, totalTokens: 0, totalRequests: 0, tokenScopeLabel: '全部部署', zeroYieldCount: 0, zeroYieldTokens: 0,
    cacheHitRate: 0, controlBusy: '', highValueCount: () => 0 });
  const archived = [];
  const app = renderer.createApp({ setup: () => () => vue.h(component, { ...props, onArchived: value => archived.push(value) }) }); app.mount({}); await flush();
  return { b: bindings, props, calls, archived, unmount: () => app.unmount(),
    sourceAttemptSelector: async () => {
      let tree;
      const probe = vue.createSSRApp({ render: () => (tree = templateExports.render({}, [], props, vue.proxyRefs(bindings), {}, {})) });
      const warnings = []; probe.config.warnHandler = message => warnings.push(message);
      await renderToString(probe); assert.deepEqual(warnings, [], 'template probe must render without Vue warnings');
      const visit = node => node?.props?.['aria-label'] === '源码审查执行轮次' ? node
        : (Array.isArray(node?.children) ? node.children.map(visit).find(Boolean) : undefined);
      const select = visit(tree); assert.ok(select, 'actual source attempt selector must exist');
      return value => {
        if (select.props.onChange) select.props.onChange({ target: { value: String(value) } });
        else select.props['onUpdate:modelValue'](value);
      };
    },
    render: () => renderToString(vue.createSSRApp({ render: () => templateExports.render({}, [], props, vue.proxyRefs(bindings), {}, {}) })) };
}

module.exports = { mount, flush, deferred, scan, page, source, searchClock };
