const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const vue = require('vue');
const { renderToString } = require('@vue/server-renderer');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { liveLogHarness } = require('./live_runner_log_harness.cjs');
require('./test_runner_log_panel.cjs');
require('./test_asset_log_panel.cjs');
require('./test_install_log_panel.cjs');

const filename = path.join(__dirname, '../src/features/sentinel/components/SentinelExecutionDetails.vue');
const transpile = text => ts.transpileModule(text, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
} }).outputText;
const presentation = {};
new Function('exports', transpile(fs.readFileSync(path.join(__dirname, '../src/features/sentinel/execution/presentation.ts'), 'utf8')))(presentation);
const historicalContract = {};
new Function('exports', transpile(fs.readFileSync(path.join(__dirname,
  '../src/features/sentinel/results/historicalPreviewContract.ts'), 'utf8')))(historicalContract);
const Review = { props: ['scanId', 'targetUrl', 'attemptNumber'], setup(props) {
  return () => vue.h('aside', { 'data-review': `${props.scanId}|${props.targetUrl}|${props.attemptNumber}` });
} };

function component() {
  const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
  const script = compileScript(descriptor, { id: 'execution-details-test' });
  const template = compileTemplate({ source: descriptor.template.content, filename,
    id: 'execution-details-test', compilerOptions: { bindingMetadata: script.bindings } });
  assert.deepEqual(template.errors, []);
  const exports = {}, rendered = {};
  new Function('require', 'exports', transpile(script.content))(name => {
    if (name === 'vue') return vue;
    if (name === '../execution/presentation') return presentation;
    if (name === './AgentRequestUsage.vue') return { default: () => null };
    if (name === './AgentRequestReview.vue') return { default: Review };
    throw Error(`Unexpected import ${name}`);
  }, exports);
  new Function('require', 'exports', transpile(template.code))(require, rendered);
  return { ...exports.default, render: rendered.render };
}

const execution = () => ({ backend: 'native', mode: 'standard', targetStatusText: '运行中',
  coverage: { required: ['api'], requiredLabels: ['API'], covered: [], completedRatio: .5,
    ledgerReported: true, confirmedFindings: 0, ledger: { uncoveredFamilies: [
      { family: 'api', label: 'API', status: 'insufficient_evidence', reasonCode: 'budget', reason: '资源不足' },
    ], manualDeepDiveSuggestions: ['人工复核'] } },
  runtime: { tokenUsage: { totalTokens: 12 }, requestAccounting: { attemptNumber: 2 } },
  hardLimits: { hardTotalTokens: 100 }, targetStatus: 'scanning' });
const render = props => renderToString(vue.createSSRApp(component(), props));

test('execution details preserve coverage, budget and honest incomplete labels', async () => {
  const html = await render({ execution: execution(), scanId: 'A', targetUrl: 'http://localhost/a',
    scope: { scanId: 'A', targetUrl: 'http://localhost/a' } });
  for (const label of ['执行计划', 'API', '因预算未完成', '资源不足', '12 已消耗', '人工深入建议：人工复核']) {
    assert.ok(html.includes(label), label);
  }
  assert.match(html, /data-review="A\|http:\/\/localhost\/a\|2"/);
});

test('request review never renders against another selected task or target', async () => {
  for (const scope of [undefined, { scanId: 'B', targetUrl: 'http://localhost/a' },
    { scanId: 'A', targetUrl: 'http://localhost/b' }]) {
    const html = await render({ execution: execution(), scanId: 'A', targetUrl: 'http://localhost/a', scope });
    assert.doesNotMatch(html, /data-review=/);
  }
});

test('a saved retired execution plan is collapsed and does not gain a review action', async () => {
  const saved = { ...execution(), backend: 'legacy_backend_removed', runtime: {} };
  const html = await render({ execution: saved, scanId: 'A', targetUrl: 'http://localhost/a' });
  assert.match(html, /class="agent-execution-details"/);
  assert.doesNotMatch(html, /<details[^>]*\sopen(?:\s|=|>)/);
  assert.doesNotMatch(html, /data-review=/);
});

// Exercise the actual business composable with Vue lifecycle; only IPC is replaced.
const flush = async () => { for (let i = 0; i < 10; i++) await vue.nextTick(); };
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
async function mountDetails(overrides = {}, eventOptions = {}) {
  const live = liveLogHarness(eventOptions);
  const api = {
    listSentinelScanAttempts: async scanId => [{ scanId, attemptNumber: 1 }],
    getNativeScanStatus: async scanId => ({ scanId, attemptNumber: 1 }),
    listAgentLearningCandidates: async () => [], listAgentKnowledge: async () => [],
    listHistoricalImportPreviews: async () => [],
    readSentinelRunnerLog: async (scanId, attempt) => ({ scanId, attempt, lines: [] }),
    getNativeAttemptMailboxHistory: async (scanId, attemptNumber) => ({ scanId, attemptNumber, messages: [] }),
    getNativeAttemptToolHistory: async (scanId, attemptNumber) => ({ schemaVersion: 2, scanId, attemptNumber, invocations: [] }),
    ...overrides,
  };
  const exports = {};
  const source = fs.readFileSync(path.join(__dirname, '../src/features/sentinel/execution/useTaskExecutionDetails.ts'), 'utf8');
  new Function('require', 'exports', transpile(source))(name => {
    if (name === 'vue') return vue;
    if (name === '../api') return { sentinelApi: api };
        if (name === '../../skills/catalogApi') return { knowledgeCatalogApi: api };
    if (name === './useLiveRunnerLog') return live.module;
    if (name === '../results/historicalPreviewContract') return historicalContract;
    throw Error(`Unexpected detail import ${name}`);
  }, exports);
  const renderer = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {},
    parentNode: () => null, nextSibling: () => null });
  const props = vue.reactive({ preview: { id: 'A', updatedAt: '1' } });
  let state;
  const app = renderer.createApp({ setup() {
    state = exports.useTaskExecutionDetails(props); return () => null;
  } });
  app.mount({}); await flush();
  return { state, props, live, unmount: () => app.unmount() };
}

require('./execution_log_events.cjs')({ mountDetails, flush, deferred });

test('slow native status does not block independent attempts, evidence or learning reads', async t => {
  const status = deferred();
  const h = await mountDetails({ getNativeScanStatus: () => status.promise,
    listAgentLearningCandidates: async () => [{ id: 1, scanId: 'A', title: 'candidate' }],
    listAgentKnowledge: async () => [{ id: 2, scanId: 'A', title: 'knowledge' }],
    listHistoricalImportPreviews: async () => [historicalPreview(3)],
  }); t.after(h.unmount);
  assert.equal(h.state.attempts.value.length, 1);
  assert.equal(h.state.selectedAttempt.value, 1);
  assert.equal(h.state.learningCandidates.value[0].title, 'candidate');
  assert.equal(h.state.knowledge.value[0].title, 'knowledge');
  assert.equal(h.state.historicalPreviews.value[0].membershipId, 3);
  assert.equal(h.state.detailLoading.value, true, 'one read really remains pending');
  h.state.detailTab.value = 'execution'; await flush();
  assert.equal(h.state.runnerLog.value.scanId, 'A');
  assert.equal(h.state.mailboxPage.value.scanId, 'A');
  assert.equal(h.state.toolPage.value.scanId, 'A');
  status.resolve({ scanId: 'A', attemptNumber: 1 }); await flush();
  assert.equal(h.state.detailLoading.value, false);
});

test('failed detail sections report promptly without exposing raw errors or hiding successful sections', async t => {
  const status = deferred();
  const h = await mountDetails({ getNativeScanStatus: () => status.promise,
    listAgentKnowledge: async () => { throw Error('/private/db Bearer secret'); },
  }); t.after(h.unmount);
  assert.deepEqual(h.state.detailErrors.value, ['知识记录暂时不可读取']);
  assert.equal(h.state.attempts.value.length, 1);
  status.reject(Error('native status is absent')); await flush();
  assert.equal(h.state.detailLoading.value, false);
  assert.equal(h.state.nativeStatus.value, undefined);
  assert.deepEqual(h.state.detailErrors.value, ['知识记录暂时不可读取']);
});

test('independent detail completions remain fenced across task round trips and unmount', async t => {
  const old = deferred(), current = deferred(); let reads = 0;
  const h = await mountDetails({ getNativeScanStatus: () => ++reads === 1 ? old.promise : current.promise });
  t.after(h.unmount);
  h.props.preview = { id: 'B', updatedAt: '1' }; await flush();
  h.props.preview = { id: 'A', updatedAt: '2' }; await flush();
  assert.equal(h.state.attempts.value[0].scanId, 'A');
  old.resolve({ scanId: 'A', attemptNumber: 99 }); await flush();
  assert.equal(h.state.nativeStatus.value, undefined);
  assert.equal(h.state.detailLoading.value, true, 'old finally cannot finish the current view');
  h.unmount(); current.resolve({ scanId: 'A', attemptNumber: 2 }); await flush();
  assert.equal(h.state.nativeStatus.value, undefined);
});

test('a synchronous IPC adapter failure does not stop other detail reads', async t => {
  const h = await mountDetails({ listAgentKnowledge: () => { throw Error('adapter-secret'); } });
  t.after(h.unmount);
  assert.equal(h.state.attempts.value.length, 1);
  assert.equal(h.state.nativeStatus.value.scanId, 'A');
  assert.equal(h.state.detailLoading.value, false);
  assert.deepEqual(h.state.detailErrors.value, ['知识记录暂时不可读取']);
});

const historicalPreview = (membershipId, extra = {}) => ({ membershipId, scanId: 'A', attemptNumber: 1,
  kind: 'finding_candidate', title: 'Old finding', severity: '', target: '', producer: 'historical fixture',
  reviewState: 'unreviewed', readOnly: true, executionEligible: false, ...extra });

test('task history refuses malformed or contradictory normalized authority without hiding other details', async () => {
  const invalidPages = [{ reviewState: 'confirmed' }, { readOnly: false }, { executionEligible: true },
    { reviewState: undefined }].map(extra => [historicalPreview(1, extra)]);
  invalidPages.push(null, {}, [null], Array.from({ length: 301 }, (_, i) => historicalPreview(i + 1)));
  for (const rows of invalidPages) {
    const h = await mountDetails({ listHistoricalImportPreviews: async () => rows });
    try {
      assert.deepEqual(h.state.historicalPreviews.value, []);
      assert.deepEqual(h.state.detailErrors.value, ['历史导入预览暂时不可读取']);
      assert.equal(h.state.attempts.value[0].scanId, 'A');
      assert.equal(h.state.detailLoading.value, false);
    } finally { h.unmount(); }
  }
});

test('task history rejects another scan instead of showing it under the current task', async () => {
  for (const scanId of ['B', undefined]) {
    const h = await mountDetails({ listHistoricalImportPreviews: async () => [historicalPreview(1, { scanId })] });
    try {
      assert.deepEqual(h.state.historicalPreviews.value, []);
      assert.deepEqual(h.state.detailErrors.value, ['历史导入预览暂时不可读取']);
    } finally { h.unmount(); }
  }
});

test('task history preserves valid mixed attempts and accepts empty or maximum-sized pages', async () => {
  for (const size of [0, 2, 300]) {
    const rows = Array.from({ length: size }, (_, i) => historicalPreview(i + 1, { attemptNumber: i % 2 + 1 }));
    const h = await mountDetails({ listHistoricalImportPreviews: async () => rows });
    try {
      assert.deepEqual(h.state.historicalPreviews.value, rows);
      assert.deepEqual(h.state.detailErrors.value, []);
      assert.equal(h.state.selectedAttempt.value, 1, 'history cannot change the selected native attempt');
    } finally { h.unmount(); }
  }
});
