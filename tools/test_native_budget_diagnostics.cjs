const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const vue = require('vue');
const { parse, compileScript } = require('@vue/compiler-sfc');

const filename = path.join(__dirname, '../src/features/sentinel/components/NativeBudgetDiagnostics.vue');
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'native-budget-test' });
const compiled = ts.transpileModule(script.content, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
const renderer = vue.createRenderer({
  createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
  insert() {}, remove() {}, setText() {}, setElementText() {}, patchProp() {},
  parentNode: () => null, nextSibling: () => null,
});
const flush = async () => { for (let i = 0; i < 8; i++) await vue.nextTick(); };

async function mount(read) {
  const calls = [];
  let refresh;
  const exports = {};
  new Function('require', 'exports', compiled)((name) => {
    if (name === 'vue') return vue;
    if (name === '../api') return { sentinelApi: { getNativeBudgetDiagnostics: async (...args) => {
      calls.push(args); return read(...args);
    } } };
    if (name === '../../../i18n') return { useI18n: () => ({ tr: (zh) => zh }) };
    if (name === '../../../composables/useCommittedRefresh') return { useCommittedRefresh: (...args) => { refresh = args; } };
    if (name === '../timeline/eventContract') return { validCollaborationEvent: () => true };
    if (name === './NativeBudgetVector.vue') return { default: () => null };
    throw Error(`Unexpected import: ${name}`);
  }, exports);
  let bindings;
  const setup = exports.default.setup;
  const component = { ...exports.default, setup(props, ctx) {
    bindings = setup(props, ctx); return () => null;
  } };
  const props = vue.reactive({ scanId: 'scan-a', attempt: 1 });
  const app = renderer.createApp({ setup: () => () => vue.h(component, props) });
  app.mount({});
  await flush();
  return { b: bindings, props, calls, refresh, unmount: () => app.unmount() };
}

test('budget panel is on demand and never displays raw transport errors', async () => {
  const view = await mount(async () => { throw Error('Bearer private-budget-secret /private/db.sqlite'); });
  assert.equal(view.calls.length, 0);
  view.b.onToggle({ target: { open: true } });
  await view.b.load();
  assert.deepEqual(view.calls, [['scan-a', 1]]);
  assert.match(view.b.error.value, /读取失败/);
  assert.doesNotMatch(view.b.error.value, /private-budget-secret|db\.sqlite/);
  assert.equal(view.b.report.value, undefined);
  view.unmount();
});

test('budget panel rejects malformed receipts and ignores late attempt responses', async () => {
  let resolve;
  const pending = new Promise(done => { resolve = done; });
  const view = await mount(async () => pending);
  view.b.onToggle({ target: { open: true } });
  const loading = view.b.load();
  view.props.attempt = 2;
  resolve({ schema: 'summary_gap_v1', authoritative: false, roots: [], totalRoots: 0, truncated: false });
  await loading;
  await flush();
  assert.equal(view.b.report.value, undefined);
  view.unmount();

  const bad = await mount(async () => ({ schema: 'unknown', authoritative: true, roots: [] }));
  bad.b.onToggle({ target: { open: true } });
  await bad.b.load();
  assert.equal(bad.b.report.value, undefined);
  assert.match(bad.b.error.value, /读取失败/);
  bad.unmount();
});

test('budget panel rejects malformed journal costs without exposing their raw values', async () => {
  const view = await mount(async () => ({ schema: 'summary_gap_v1', authoritative: false,
    roots: [{ rootRunId: 'root-a', gaps: { appendJournal: [{ dimension: 'target_requests',
      hardLimit: 20, reserved: 0, consumed: 0, indeterminate: 'Bearer private-vector-secret', coverage: 'multi_agent_broker' }] } }],
    totalRoots: 1, truncated: false }));
  view.b.onToggle({ target: { open: true } }); await view.b.load();
  assert.equal(view.b.report.value, undefined);
  assert.match(view.b.error.value, /读取失败/); assert.doesNotMatch(view.b.error.value, /private-vector-secret/);
  view.unmount();
});

test('actual vector component renders unknown costs and keeps coverage gaps visible', async () => {
  const { renderWorkspaceComponent } = require('./result_component_harness.cjs');
  const html = await renderWorkspaceComponent('src/features/sentinel/components/NativeBudgetVector.vue', {
    dimensions: [{ dimension: 'target_requests', hardLimit: 20, reserved: 0, consumed: 2,
      indeterminate: 1, coverage: 'multi_agent_broker' }],
  }, { '../../../i18n': { useI18n: () => ({ tr: zh => zh }) } });
  assert.match(html, /目标请求/); assert.match(html, /未决/); assert.match(html, /unresolved/);
  assert.match(html, /已接入的目标请求账目/); assert.match(html, /不代表全部通道均已覆盖/);
  assert.match(html, /不代表十维完整验收/);
});

test('budget panel rejects invalid unclosed call counts and preserves zero and missing distinctions', async () => {
  for (const count of [-1, 0.5, Number.MAX_SAFE_INTEGER + 1, 'Bearer private-call-secret']) {
    const view = await mount(async () => ({ schema: 'summary_gap_v1', authoritative: false,
      roots: [{ rootRunId: 'root-a', gaps: { unclosedWebModelCalls: count } }], totalRoots: 1, truncated: false }));
    view.b.onToggle({ target: { open: true } }); await view.b.load();
    assert.equal(view.b.report.value, undefined);
    assert.match(view.b.error.value, /读取失败/);
    assert.doesNotMatch(view.b.error.value, /private-call-secret/);
    view.unmount();
  }
  for (const count of [undefined, 0, 3]) {
    const view = await mount(async () => ({ schema: 'summary_gap_v1', authoritative: false,
      roots: [{ rootRunId: 'root-a', gaps: { unclosedWebModelCalls: count } }], totalRoots: 1, truncated: false }));
    view.b.onToggle({ target: { open: true } }); await view.b.load();
    assert.equal(view.b.report.value.roots[0].gaps.unclosedWebModelCalls, count);
    view.unmount();
  }
});

test('fresh Root race-batch coverage is accepted and rendered as unavailable', async () => {
  const dimensions = ['model_input_tokens', 'model_cached_tokens', 'model_output_tokens', 'model_requests',
    'target_requests', 'browser_actions', 'controlled_writes', 'upload_bytes', 'concurrency_batches', 'wall_time_ms'];
  const rows = dimensions.map(dimension => ({ dimension, hardLimit: null, reserved: 0, consumed: 0,
    indeterminate: 0, coverage: dimension === 'concurrency_batches'
      ? 'race_batch_dispatch_unimplemented' : 'assignment_settlement' }));
  const view = await mount(async () => ({ schema: 'summary_gap_v1', authoritative: false,
    roots: [{ rootRunId: 'fresh-root', gaps: { appendJournal: rows } }], totalRoots: 1, truncated: false }));
  view.b.onToggle({ target: { open: true } });
  await view.b.load();
  assert.equal(view.b.error.value, '');
  assert.equal(view.b.report.value.roots[0].gaps.appendJournal[8].coverage,
    'race_batch_dispatch_unimplemented');
  view.unmount();
  const { renderWorkspaceComponent } = require('./result_component_harness.cjs');
  const html = await renderWorkspaceComponent('src/features/sentinel/components/NativeBudgetVector.vue', {
    dimensions: [rows[8]],
  }, { '../../../i18n': { useI18n: () => ({ tr: zh => zh }) } });
  assert.match(html, /并发批次派发未接入/);
  assert.match(html, /不授予执行权限/);
});

test('budget panel still rejects unknown coverage instead of treating it as supported', async () => {
  const dimensions = ['model_input_tokens', 'model_cached_tokens', 'model_output_tokens', 'model_requests',
    'target_requests', 'browser_actions', 'controlled_writes', 'upload_bytes', 'concurrency_batches', 'wall_time_ms'];
  const rows = dimensions.map(dimension => ({ dimension, hardLimit: null, reserved: 0, consumed: 0,
    indeterminate: 0, coverage: dimension === 'concurrency_batches'
      ? 'future_unverified_capability' : 'assignment_settlement' }));
  const view = await mount(async () => ({ schema: 'summary_gap_v1', authoritative: false,
    roots: [{ rootRunId: 'fresh-root', gaps: { appendJournal: rows } }], totalRoots: 1, truncated: false }));
  view.b.onToggle({ target: { open: true } });
  await view.b.load();
  assert.equal(view.b.report.value, undefined);
  assert.match(view.b.error.value, /读取失败/);
  view.unmount();
});

function supervisionCapacity(overrides = {}) {
  return { schemaVersion: 1, authoritative: false, executionAllowed: false,
    pendingPhysicalRequests: 2, minimumFollowupModelRequests: 6, nextHttpRequiredModelRequests: 9,
    remainingSharedModelRequests: 10, remainingSharedModelTokens: 31000,
    nextHttpRequestBalanceSufficient: true, freshWorkBlocked: null,
    futureTokenCapacityGuaranteed: false, usdCapacityCoverage: 'not_enforced_by_this_ledger', ...overrides };
}

function capacityReceipt(capacity) {
  return { schema: 'summary_gap_v1', authoritative: false, totalRoots: 1, truncated: false,
    roots: [{ rootRunId: 'root-a', gaps: { missingDimensions: [], supervisionRequestCapacity: capacity } }] };
}

test('supervision inventory keeps unknown costs separate from sufficient numeric capacity', async () => {
  for (const capacity of [supervisionCapacity({ freshWorkBlocked: 'cost_reconciliation_required' }),
    supervisionCapacity({ freshWorkBlocked: 'wall_time_exhausted' }),
    supervisionCapacity({ freshWorkBlocked: 'elapsed_finalized' }),
    supervisionCapacity({ remainingSharedModelRequests: null, remainingSharedModelTokens: null }),
    supervisionCapacity({ remainingSharedModelRequests: 8, nextHttpRequestBalanceSufficient: false }),
    undefined, null]) {
    const view = await mount(async () => capacityReceipt(capacity));
    view.b.onToggle({ target: { open: true } }); await view.b.load();
    assert.equal(view.b.error.value, '');
    assert.deepEqual(view.b.report.value.roots[0].gaps.supervisionRequestCapacity, capacity);
    view.unmount();
  }
});

test('supervision inventory rejects grants, inconsistent arithmetic and unsafe counts', async () => {
  for (const changed of [
    { authoritative: true }, { executionAllowed: true }, { futureTokenCapacityGuaranteed: true },
    { nextHttpRequiredModelRequests: 8 }, { minimumFollowupModelRequests: 5 },
    { pendingPhysicalRequests: Number.MAX_SAFE_INTEGER }, { remainingSharedModelTokens: -1 },
    { remainingSharedModelRequests: undefined }, { nextHttpRequestBalanceSufficient: false },
    { freshWorkBlocked: 'Bearer private-supervision-secret' }, { usdCapacityCoverage: 'guaranteed' },
  ]) {
    const view = await mount(async () => capacityReceipt(supervisionCapacity(changed)));
    view.b.onToggle({ target: { open: true } }); await view.b.load();
    assert.equal(view.b.report.value, undefined);
    assert.match(view.b.error.value, /读取失败/);
    assert.doesNotMatch(view.b.error.value, /private-supervision-secret/);
    view.unmount();
  }
});

test('actual budget template states supervision obligations and missing token/USD guarantees', async () => {
  const { loadWorkspaceModule } = require('./result_component_harness.cjs');
  const { renderToString } = require('@vue/server-renderer');
  const base = loadWorkspaceModule('src/features/sentinel/components/NativeBudgetDiagnostics.vue', {
    '../api': { sentinelApi: {} },
    '../../../i18n': { useI18n: () => ({ tr: zh => zh }) },
    '../../../composables/useCommittedRefresh': { useCommittedRefresh: () => {} },
    '../timeline/eventContract': { validCollaborationEvent: () => true },
  }).default;
  for (const [blocked, reason] of [
    ['cost_reconciliation_required', '原费用尚未完成对账'],
    ['wall_time_exhausted', '原任务截止时间已到'],
    ['elapsed_finalized', '原耗时账目已封存'],
  ]) {
    const component = { ...base, setup(props, ctx) {
      const bindings = base.setup(props, ctx);
      bindings.report.value = capacityReceipt(supervisionCapacity({ freshWorkBlocked: blocked }));
      return bindings;
    } };
    const html = await renderToString(vue.createSSRApp(component, { scanId: 'scan-a', attempt: 1 }));
    assert.ok(html.includes(reason), `missing original blocking reason: ${blocked}`);
    assert.match(html, /待独立付费监督的物理请求/);
    assert.match(html, /含后续最小 6 次/);
    assert.match(html, /余额足够也不能继续/);
    assert.match(html, /未来 Token 容量尚未保证/);
    assert.match(html, /未执行美元容量门禁/);
    assert.match(html, /不授予执行权限/);
  }
});

function singleHistory(changed = {}) {
  return { schemaVersion: 1, rootRunId: 'root-a', state: 'verified_closed_history',
    authoritative: false, executionAllowed: false, targetRequestsGranted: 0,
    basis: 'original_runtime_records_not_provider_dollar_invoice', ...changed };
}
function singleHistoryReceipt(history) {
  return { schema: 'summary_gap_v1', authoritative: false, totalRoots: 1, truncated: false,
    roots: [{ rootRunId: 'root-a', singleOriginalHistory: history, gaps: { missingDimensions: [] } }] };
}
test('Single original history remains read-only and accepts unverified or legacy records', async () => {
  for (const history of [singleHistory(), singleHistory({ state: 'not_verified' }), null, undefined]) {
    const view = await mount(async () => singleHistoryReceipt(history));
    view.b.onToggle({ target: { open: true } });
    await view.b.load();
    assert.equal(view.b.error.value, '');
    assert.deepEqual(view.b.report.value.roots[0].singleOriginalHistory, history);
    assert.equal(view.calls.length, 1);
    view.unmount();
  }
});
test('Single history cannot claim execution, foreign ownership or provider dollar settlement', async () => {
  for (const change of [{ rootRunId: 'other-root' }, { schemaVersion: 2 }, { state: 'approved' },
    { authoritative: true }, { executionAllowed: true }, { targetRequestsGranted: 1 },
    { basis: 'provider_dollar_invoice' }]) {
    const view = await mount(async () => singleHistoryReceipt(singleHistory(change)));
    view.b.onToggle({ target: { open: true } });
    await view.b.load();
    assert.equal(view.b.report.value, undefined);
    assert.match(view.b.error.value, /读取失败/);
    view.unmount();
  }
});

test('actual Single diagnostic template separates closed history from permission and dollar invoices', async () => {
  const { loadWorkspaceModule } = require('./result_component_harness.cjs');
  const { renderToString } = require('@vue/server-renderer');
  const base = loadWorkspaceModule('src/features/sentinel/components/NativeBudgetDiagnostics.vue', {
    '../api': { sentinelApi: {} }, '../../../i18n': { useI18n: () => ({ tr: zh => zh }) },
    '../../../composables/useCommittedRefresh': { useCommittedRefresh: () => {} },
    '../timeline/eventContract': { validCollaborationEvent: () => true },
  }).default;
  for (const [state, label] of [['verified_closed_history', '已核验闭合'], ['not_verified', '尚未核验闭合']]) {
    const component = { ...base, setup(props, ctx) {
      const bindings = base.setup(props, ctx);
      bindings.report.value = singleHistoryReceipt(singleHistory({ state }));
      return bindings;
    } };
    const html = await renderToString(vue.createSSRApp(component, { scanId: 'scan-a', attempt: 1 }));
    assert.ok(html.includes(label));
    assert.match(html, /不是真实美元账单/);
    assert.match(html, /不授予恢复或新的请求额度/);
    assert.match(html, /不适用（Single）/);
    assert.match(html, /旧汇总 Token/);
    assert.doesNotMatch(html, /continuation_single_|Bearer|\/private\//);
  }
});
