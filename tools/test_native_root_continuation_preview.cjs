// Real production SFC and contract. IPC is scripted; Rust proves local authority.
const assert = require('node:assert/strict'), test = require('node:test');
const fs = require('node:fs'), path = require('node:path'), ts = require('typescript');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { renderToString } = require('@vue/server-renderer');
const root = path.join(__dirname, '..');
const transpile = source => ts.transpileModule(source, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
} }).outputText;
const contracts = {};
for (const name of ['contract', 'continuation', 'issuance', 'discovery']) {
  const exports = {};
  new Function('require', 'exports', transpile(fs.readFileSync(path.join(root,
    `src/features/sentinel/reconciliation/${name}.ts`), 'utf8')))(dependency => {
    assert.ok(['./contract','./continuation','./issuance'].includes(dependency) && contracts[dependency.slice(2)]);
    return contracts[dependency.slice(2)];
  }, exports);
  contracts[name] = exports;
}
const filename = path.join(root, 'src/features/sentinel/components/NativeRootReconciliation.vue');
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'continuation-test' });
const template = compileTemplate({ source: descriptor.template.content, filename, id: 'continuation-test',
  compilerOptions: { bindingMetadata: script.bindings } });
assert.deepEqual(template.errors, []);
const rendered = {};
new Function('require', 'exports', transpile(template.code))(require, rendered);
const renderer = vue.createRenderer({ createElement: () => ({}), createText: () => ({}),
  createComment: () => ({}), insert() {}, remove() {}, setText() {}, setElementText() {},
  patchProp() {}, parentNode: () => null, nextSibling: () => null });
const flush = async () => { for (let n = 0; n < 10; n++) await vue.nextTick(); };
const hash = ch => ch.repeat(64);
const receipt = () => ({ version: 1, scanId: 'scan-a', attemptNumber: 1, rootRunId: 'root-a', callId: hash('a'),
  leaseAttemptId: 'owner-a', round: 2, requestHash: hash('b'), basisHash: hash('c'), invoiceHash: hash('d'),
  confirmationHash: hash('e'), usage: { inputTokens: 23, cachedInputTokens: 3, outputTokens: 7, totalTokens: 30, modelRequests: 1 },
  operation: 'settle_received', receiptId: hash('e'), actualCostConfirmed: true, executionAllowed: false, automaticResumeAllowed: false });
const inventory = (extra = {}) => ({ schema: 1, executionAllowed: false, automaticResumeAllowed: false,
  items: [], blocked: [], receipts: [receipt()], truncated: false, ...extra });
const proposal = (extra = {}) => ({ schema: 1, scanId: 'scan-a', attemptNumber: 1, rootRunId: 'root-a', receiptId: hash('e'),
  proposedAttemptNumber: 2, targetKey: 'https://authorized.example.test', previousStatus: 'paused',
  financialSource: { version: 1, proofKind: 'settled_original_finance', rootRunId: 'root-a', ownerId: 'owner-a',
    scanId: 'scan-a', attemptNumber: 1, targetKey: 'https://authorized.example.test', coordinatorEpoch: 1,
    coordinatorFence: 'fence-a', receiptId: hash('e'), remaining: [100, 90, 80, 4, 7, 0, 0, 0, 3, 600000],
    dimensions: [...contracts.continuation.continuationDimensions], sharedRemainingTokens: 170,
    sharedRemainingRequests: 4, originalDeadline: '2099-01-01 00:00:00.000', rootPhysicalHash: hash('f'),
    ownerPhysicalHash: hash('a'), financialPhysicalHash: hash('b'), executionAllowed: false },
  mode: 'multi', originalModeReceiptId: 'original-mode-a', modelBindingHash: hash('c'), identityMode: 'AnonymousOnly',
  identityCount: 0, evidenceHashes: { scope: hash('a'), authority: hash('b'), mode: hash('c'), plan: hash('d'),
    model: hash('e'), context: hash('f'), sessions: hash('a'), history: hash('b') }, historicalRootsChecked: 2,
  taskPurpose: 'new_work_after_original_cost_confirmation', originalSemanticReplayAllowed: false,
  taskDescription: { purpose: 'fresh_root_assessment_of_frozen_web_evidence', targetKey: 'https://authorized.example.test',
    originalPlanHash: hash('a'), originalSemanticReplayAllowed: false, limits: 'remaining_original_budget_and_deadline' },
  executionAllowed: false, automaticResumeAllowed: false, issuanceImplemented: true, proposalHash: hash('d'), ...extra });
async function mount(handlers = {}) {
  const calls = [], exports = {};
  const api = new Proxy({}, { get(_, key) {
    assert.ok(['getNativeRootCostReconciliations', 'previewNativeRootFinancialContinuation'].includes(key),
      `Read-only preview may not dispatch execution or cost writes: ${String(key)}`);
    return async (...args) => {
      calls.push([key, ...args]);
      return key === 'getNativeRootCostReconciliations' ? (handlers.read || (() => inventory()))(...args)
        : (handlers.preview || (() => proposal()))(...args);
    };
  } });
  new Function('require', 'exports', transpile(script.content))(name => {
    if (name === 'vue') return vue;
    if (name === '../reconciliation/api') return { rootCostReconciliationApi: api };
    if (name === '../../../i18n') return { useI18n: () => ({ tr: zh => zh }) };
    if (name === '../reconciliation/contract') return contracts.contract;
    if (name === '../reconciliation/continuation') return contracts.continuation;
    if (name === '../reconciliation/issuance') return contracts.issuance;
    if (name === '../reconciliation/discovery') return contracts.discovery;
    throw Error(`Unexpected dependency: ${name}`);
  }, exports);
  let b;
  const setup = exports.default.setup;
  const component = { ...exports.default, setup(props, ctx) { b = setup(props, ctx); return () => null; } };
  const props = vue.reactive({ scanId: 'scan-a', attempt: 1 });
  const app = renderer.createApp({ setup: () => () => vue.h(component, props) });
  app.mount({}); await flush();
  return { b, props, calls, unmount: () => app.unmount(),
    open: async () => { b.onToggle({ target: { open: true } }); await flush(); },
    render: () => renderToString(vue.createSSRApp({ render: () => rendered.render({}, [], {}, vue.proxyRefs(b), {}, {}) })) };
}
test('cost inventory mount open and refresh never automatically request continuation or start work', async t => {
  const view = await mount(); t.after(view.unmount); assert.equal(view.calls.length, 0);
  await view.open(); await view.b.load();
  assert.ok(view.calls.every(call => call[0] === 'getNativeRootCostReconciliations'));
  assert.equal(view.b.continuation.value, undefined);
  assert.match(await view.render(), /预览新工作剩余授权/);
});
test('explicit preview displays exact remaining balances original deadline and immutable evidence without execution', async t => {
  const view = await mount(); t.after(view.unmount); await view.open();
  await view.b.previewContinuation(view.b.inventory.value.receipts[0]);
  assert.deepEqual(view.calls[1], ['previewNativeRootFinancialContinuation', 'scan-a', 1, 'root-a', hash('e')]);
  const html = await view.render();
  for (const value of ['2099-01-01 00:00:00', '拟新尝试（尚未创建）', '仍未可执行', '原模式回执', '原模型绑定哈希',
    '历史财务证明哈希', '预览绑定哈希', '账本时间余额不是新超时额度', '独立批准前会再次核验', '用原冻结 Web 证据', '原冻结计划哈希']) assert.ok(html.includes(value), value);
  assert.doesNotMatch(html, /批准启动|确认恢复|已恢复执行/);
  assert.equal(view.b.inventory.value.receipts.length, 1);
  await view.b.load(); assert.equal(view.b.continuation.value, undefined);
  assert.equal(view.calls.filter(call => call[0] === 'previewNativeRootFinancialContinuation').length, 1);
});
test('duplicate preview clicks share a single read and block cost actions while in flight', async t => {
  let finish;
  const view = await mount({ preview: () => new Promise(resolve => { finish = resolve; }) }); t.after(view.unmount);
  await view.open(); const original = view.b.inventory.value.receipts[0];
  const action = view.b.previewContinuation(original); await view.b.previewContinuation(original); await view.b.load();
  assert.equal(view.calls.length, 2); assert.equal(view.b.continuationLoading.value, true);
  finish(proposal()); await action; assert.ok(view.b.continuation.value); assert.equal(view.b.continuationLoading.value, false);
});
test('unknown costs and foreign or synthetic receipts never create preview authority', async t => {
  const view = await mount({ read: () => inventory({ receipts: [], blocked: [{ rootRunId: 'root-a', callId: hash('a'), reason: 'budget_indeterminate' }] }) }); t.after(view.unmount);
  await view.open(); await view.b.previewContinuation(receipt()); assert.equal(view.calls.length, 1);
  assert.doesNotMatch(await view.render(), /预览新工作剩余授权/);
  const confirmed = await mount(); t.after(confirmed.unmount); await confirmed.open();
  await confirmed.b.previewContinuation({ ...confirmed.b.inventory.value.receipts[0], scanId: 'foreign' });
  await confirmed.b.previewContinuation({ ...confirmed.b.inventory.value.receipts[0] });
  assert.equal(confirmed.calls.length, 1);
});
test('failed authorization or expiry retains the original confirmed bill and hides sensitive reasons', async t => {
  const view = await mount({ preview: () => { throw Error('lease_expired Bearer private-secret /private/db.sqlite'); } }); t.after(view.unmount);
  await view.open(); await view.b.previewContinuation(view.b.inventory.value.receipts[0]);
  assert.equal(view.b.continuation.value, undefined); assert.equal(view.b.inventory.value.receipts.length, 1);
  assert.match(view.b.continuationError.value, /未通过核验/); assert.doesNotMatch(await view.render(), /private-secret|db.sqlite/);
  assert.equal(view.calls.length, 2);
});
test('bad bindings budgets deadlines identity modes and false grants cannot display a successful preview', async () => {
  const source = proposal().financialSource;
  const changes = [null, { schema: 2 }, { scanId: 'foreign' }, { attemptNumber: 2 }, { rootRunId: 'foreign' },
    { receiptId: hash('f') }, { proposedAttemptNumber: 1 }, { targetKey: 'https://foreign.test' }, { previousStatus: 'scanning' },
    { mode: 'single' }, { originalModeReceiptId: '' }, { modelBindingHash: 'bad' }, { proposalHash: 'bad' },
    { identityMode: 'IdentitySet', identityCount: 1 }, { identityMode: 'SingleIdentity', identityCount: 0 },
    { identityCount: -1 }, { historicalRootsChecked: 0 }, { executionAllowed: true }, { automaticResumeAllowed: true },
    { issuanceImplemented: false }, { originalSemanticReplayAllowed: true }, { taskPurpose: 'replay_old_tick' },
    { evidenceHashes: { ...proposal().evidenceHashes, scope: 'bad' } }, { rawBody: 'private-secret' }];
  for (const change of [{ version: 2 }, { proofKind: 'new_grant' }, { rootRunId: 'foreign' }, { receiptId: hash('f') },
    { coordinatorEpoch: 0 }, { coordinatorFence: '' }, { remaining: [-1, 90, 80, 4, 7, 0, 0, 0, 3, 600000] },
    { dimensions: [...source.dimensions].reverse() }, { sharedRemainingTokens: Number.MAX_SAFE_INTEGER + 1 },
    { originalDeadline: '2099-02-30 00:00:00' }, { financialPhysicalHash: 'bad' }, { executionAllowed: true }])
    changes.push({ financialSource: { ...source, ...change } });
  for (const change of [{ purpose: 'replay_original_tick' }, { targetKey: 'https://foreign.test' },
    { originalPlanHash: 'bad' }, { originalSemanticReplayAllowed: true }, { limits: 'fresh_full_allowance' }, { extraGrant: true }])
    changes.push({ taskDescription: { ...proposal().taskDescription, ...change } });
  for (const change of changes) {
    const view = await mount({ preview: () => change === null ? null : proposal(change) });
    try {
      await view.open(); await view.b.previewContinuation(view.b.inventory.value.receipts[0]);
      assert.equal(view.b.continuation.value, undefined, JSON.stringify(change));
      assert.equal(view.b.inventory.value.receipts.length, 1); assert.ok(view.b.continuationError.value);
    } finally { view.unmount(); }
  }
});
test('late previews cannot populate another scan attempt a closed panel or an unmounted view', async () => {
  for (const alter of [v => { v.props.scanId = 'scan-b'; }, v => { v.props.attempt = 2; },
    v => v.b.onToggle({ target: { open: false } }), v => v.unmount()]) {
    let finish;
    const view = await mount({ preview: () => new Promise(resolve => { finish = resolve; }) });
    await view.open(); const action = view.b.previewContinuation(view.b.inventory.value.receipts[0]);
    alter(view); await flush(); finish(proposal()); await action;
    assert.equal(view.b.continuation.value, undefined); assert.equal(view.b.continuationError.value, '');
    assert.equal(view.calls.length, 2); view.unmount();
  }
});
test('explicit unbounded dimensions remain distinguishable from zero without inventing a ceiling', async t => {
  const source = proposal().financialSource;
  const view = await mount({ preview: () => proposal({ financialSource: { ...source, remaining: [null, null, null, null, 7, 0, 0, 0, 3, 600000],
    sharedRemainingTokens: null, sharedRemainingRequests: null } }) }); t.after(view.unmount);
  await view.open(); await view.b.previewContinuation(view.b.inventory.value.receipts[0]);
  assert.match(await view.render(), /原合同未设硬顶/); assert.equal(view.b.continuation.value.financialSource.remaining[4], 7);
});
test('real public API sends exactly the original scan attempt Root and reconciliation receipt to readonly IPC', async () => {
  const { loadSentinelApi } = require('./sentinel_api_harness.cjs'); const calls = [];
  const api = loadSentinelApi(async (...args) => { calls.push(args); return proposal(); });
  await api.previewNativeRootFinancialContinuation('scan-a', 1, 'root-a', hash('e'));
  assert.deepEqual(calls, [['preview_native_root_financial_continuation', { scanId: 'scan-a', attemptNumber: 1,
    rootRunId: 'root-a', receiptId: hash('e') }]]);
});
