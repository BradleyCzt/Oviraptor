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
const node = () => ({ children: [], parent: null, addEventListener() {}, removeEventListener() {} });
const remove = child => { if (child.parent) {
  const siblings = child.parent.children; siblings.splice(siblings.indexOf(child), 1); child.parent = null;
} };
const renderer = vue.createRenderer({ createElement: node, createText: node, createComment: node,
  insert(child, parent, anchor = null) { remove(child); parent.children ||= [];
    const index = anchor ? parent.children.indexOf(anchor) : -1;
    parent.children.splice(index < 0 ? parent.children.length : index, 0, child); child.parent = parent; },
  remove, setText() {}, setElementText() {}, patchProp() {}, parentNode: child => child.parent,
  nextSibling: child => child.parent?.children[child.parent.children.indexOf(child) + 1] || null });
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
    model: hash('e'), context: hash('f'), sessions: hash('a'), history: hash('b') }, historicalRootsChecked: 1,
  taskPurpose: 'new_work_after_original_cost_confirmation', originalSemanticReplayAllowed: false,
  taskDescription: { purpose: 'fresh_root_assessment_of_frozen_web_evidence', targetKey: 'https://authorized.example.test',
    originalPlanHash: hash('a'), originalSemanticReplayAllowed: false, limits: 'remaining_original_budget_and_deadline' },
  executionAllowed: false, automaticResumeAllowed: false, issuanceImplemented: true, proposalHash: hash('d'), ...extra });
const issued = (extra = {}) => ({ schema: 1, approvalId: '00000000-0000-4000-8000-000000000001',
  proposalHash: hash('d'), scanId: 'scan-a', sourceAttemptNumber: 1, newAttemptNumber: 2,
  sourceRootRunId: 'root-a', newRootRunId: 'new-root-a', sourceReceiptId: hash('e'), status: 'issued',
  assessmentCompleted: false, attemptClosed: false, automaticResumeAllowed: false, originalSemanticReplayAllowed: false,
  financialSource: proposal().financialSource, taskDescription: proposal().taskDescription,
  originalDeadline: proposal().financialSource.originalDeadline, reviewReason: null, workerStartAllowed: true, ...extra });
async function mount(handlers = {}, realParent = false) {
  const calls = [], exports = {};
  const api = new Proxy({}, { get(_, key) {
    assert.ok(['getNativeRootCostReconciliations', 'confirmNativeRootCostReconciliation', 'previewNativeRootFinancialContinuation', 'approveNativeRootFinancialContinuation', 'getNativeRootFinancialContinuation',
      ...(realParent ? ['getNativeScanStatus'] : [])].includes(key),
      `Unexpected IPC: ${String(key)}`);
    return async (...args) => {
      calls.push([key, ...args]);
      if (key === 'getNativeScanStatus') return handlers.scanStatus(...args);
      const defaults = { getNativeRootCostReconciliations: () => inventory(), confirmNativeRootCostReconciliation: () => receipt(),
        previewNativeRootFinancialContinuation: () => proposal(), approveNativeRootFinancialContinuation: () => issued(),
        getNativeRootFinancialContinuation: () => issued() };
      const names = { getNativeRootCostReconciliations: 'read', confirmNativeRootCostReconciliation: 'confirm',
        previewNativeRootFinancialContinuation: 'preview', approveNativeRootFinancialContinuation: 'approve', getNativeRootFinancialContinuation: 'status' };
      return (handlers[names[key]] || defaults[key])(...args);
    };
  } });
  new Function('require', 'exports', transpile(script.content))(name => {
    if (name === 'vue') return vue;
    if (name === '../api') return { sentinelApi: api };
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
  const component = { ...exports.default, setup(props, ctx) { b = setup(props, ctx);
    return realParent ? () => rendered.render({}, [], {}, vue.proxyRefs(b), {}, {}) : () => null; } };
  const props = vue.reactive({ scanId: 'scan-a', attempt: 1, status: 'paused' });
  const mountedComponent = realParent ? loadRealParent(api, component) : component;
  const app = renderer.createApp({ setup: () => () => vue.h(mountedComponent, props) });
  app.mount({}); await flush();
  return { b, props, calls, unmount: () => app.unmount(),
    open: async () => { b.onToggle({ target: { open: true } }); await flush(); },
    render: () => renderToString(vue.createSSRApp({ render: () => rendered.render({}, [], {}, vue.proxyRefs(b), {}, {}) })) };
}
function loadRealParent(api, costChild) {
  const file = path.join(root, 'src/features/sentinel/components/NativeRunStatus.vue');
  const { descriptor: d } = parse(fs.readFileSync(file, 'utf8'), { filename: file });
  const s = compileScript(d, { id: 'financial-parent-test' });
  const tpl = compileTemplate({ source: d.template.content, filename: file, id: 'financial-parent-test',
    compilerOptions: { bindingMetadata: s.bindings } });
  assert.deepEqual(tpl.errors, []); const templateExports = {}, exports = {}, labels = {}, event = {};
  new Function('require', 'exports', transpile(tpl.code))(require, templateExports);
  new Function('exports', transpile(fs.readFileSync(path.join(path.dirname(file), 'nativeRunStatusLabels.ts'), 'utf8')))(labels);
  new Function('exports', transpile(fs.readFileSync(path.join(root, 'src/features/sentinel/timeline/eventContract.ts'), 'utf8')))(event);
  const live = require('./live_runner_log_harness.cjs').liveLogHarness({ channel: 'nest://collaboration-event' });
  new Function('require', 'exports', transpile(s.content))(name => {
    if (name === 'vue') return vue;
    if (name === '../api') return { sentinelApi: api };
    if (name === '../reconciliation/api') return { rootCostReconciliationApi: api };
    if (name === '../../../i18n') return { useI18n: () => ({ tr: zh => zh }) };
    if (name === './nativeRunStatusLabels') return labels;
    if (name === '../../../composables/useCommittedRefresh') return live.committed;
    if (name === '../timeline/eventContract') return event;
    if (name === './NativeRootReconciliation.vue') return { default: costChild };
    if (['./NativeBudgetDiagnostics.vue','./NativeSourcePauseRecovery.vue','./NativeProtectionRecovery.vue'].includes(name)) return { default: { render: () => null } };
    throw Error(`Unexpected real parent dependency: ${name}`);
  }, exports);
  const setup = exports.default.setup;
  return { ...exports.default, setup(props, ctx) { const bindings = setup(props, ctx);
    return () => templateExports.render({}, [], {}, vue.proxyRefs(bindings), {}, {}); } };
}

async function ready(view) { await view.open(); await view.b.previewContinuation(view.b.inventory.value.receipts[0]); }
const writes = view => view.calls.filter(c => c[0] === 'approveNativeRootFinancialContinuation');
test('mount refresh cost confirmation and preview do not approve or automatically query execution status', async t => {
  const v = await mount(); t.after(v.unmount); await v.open(); await v.b.load();
  await v.b.previewContinuation(v.b.inventory.value.receipts[0]);
  assert.equal(writes(v).length, 0); await v.b.approveContinuation(); assert.equal(writes(v).length, 0);
  assert.equal(v.calls.filter(c => c[0] === 'getNativeRootFinancialContinuation').length, 0);
  const bill = { ...receipt() }; for (const key of ['receiptId','actualCostConfirmed','executionAllowed','automaticResumeAllowed']) delete bill[key];
  const c = await mount({ read: () => inventory({ receipts: [], items: [bill] }) }); t.after(c.unmount);
  await c.open(); c.b.select(c.b.inventory.value.items[0]); await c.b.confirm();
  assert.equal(writes(c).length, 0); assert.equal(c.b.approveAttempted.value, false);
});
test('checkbox then explicit click approves exact server proposal and separates issued from published', async t => {
  const v = await mount(); t.after(v.unmount); await ready(v);
  assert.match(await v.render(), /旧请求不重放/); v.b.approvalChecked.value = true; await v.b.approveContinuation();
  assert.deepEqual(writes(v), [['approveNativeRootFinancialContinuation', 'scan-a', 1, 'root-a', hash('e'), hash('d')]]);
  const html = await v.render(); assert.match(html, /已发行新 Root/); assert.doesNotMatch(html, /已实际发布|尝试已核验收口/);
  assert.equal(v.b.issuance.value.assessmentCompleted, false);
});
test('duplicate clicks and refresh during approval do not duplicate or erase the bound operation', async t => {
  let finish; const v = await mount({ approve: () => new Promise(r => { finish = r; }) }); t.after(v.unmount);
  await ready(v); v.b.approvalChecked.value = true; const pending = v.b.approveContinuation();
  await v.b.approveContinuation(); await v.b.load(); await v.b.readContinuationStatus();
  assert.equal(writes(v).length, 1); assert.equal(v.b.approving.value, true);
  finish(issued()); await pending; assert.ok(v.b.issuance.value);
});
test('unknown ACK is recovered by an explicit readonly status query without automatic retry', async t => {
  const v = await mount({ approve: () => { throw Error('Bearer private-secret /private/db.sqlite'); } }); t.after(v.unmount);
  await ready(v); v.b.approvalChecked.value = true; await v.b.approveContinuation();
  assert.equal(v.b.issuance.value, undefined); assert.equal(writes(v).length, 1);
  assert.match(v.b.issuanceError.value, /尚未核实/); assert.doesNotMatch(await v.render(), /private-secret|db.sqlite/);
  await v.b.readContinuationStatus(); assert.equal(writes(v).length, 1); assert.ok(v.b.issuance.value);
  assert.deepEqual(v.calls.at(-1), ['getNativeRootFinancialContinuation', 'scan-a', 1, 'root-a', hash('e'), hash('d')]);
});
test('same-binding explicit retry retains approval identity and refuses another Root or approval receipt', async t => {
  let n = 0; const v = await mount({ approve: () => ++n === 1 ? issued() : issued({ newRootRunId: 'another-root' }) }); t.after(v.unmount);
  await ready(v); v.b.approvalChecked.value = true; await v.b.approveContinuation(); await v.b.approveContinuation();
  assert.equal(writes(v).length, 2); assert.deepEqual(writes(v)[0], writes(v)[1]);
  assert.equal(v.b.issuance.value.newRootRunId, 'new-root-a'); assert.match(v.b.issuanceError.value, /尚未核实/);
});
test('unknown ACK with unavailable status permits only an explicit retry of the exact original approval', async t => {
  let n = 0; const v = await mount({ approve: () => { if (++n === 1) throw Error('lost ACK'); return issued(); },
    status: () => { throw Error('not found'); } }); t.after(v.unmount);
  await ready(v); v.b.approvalChecked.value = true; await v.b.approveContinuation(); await v.b.readContinuationStatus();
  assert.equal(writes(v).length, 1); assert.equal(v.b.issuance.value, undefined);
  await v.b.approveContinuation(); assert.equal(writes(v).length, 2);
  assert.deepEqual(writes(v)[0], writes(v)[1]); assert.equal(v.b.issuance.value.newRootRunId, 'new-root-a');
});
test('strict decoder rejects foreign scope budget task deadline permissive flags and inconsistent states', () => {
  const changes = [null, { schema: 2 }, { approvalId: 'bad' }, { proposalHash: hash('a') }, { scanId: 'foreign' },
    { sourceAttemptNumber: 2 }, { newAttemptNumber: 3 }, { sourceRootRunId: 'foreign' }, { sourceReceiptId: hash('a') },
    { newRootRunId: 'root-a' }, { automaticResumeAllowed: true }, { originalSemanticReplayAllowed: true },
    { originalDeadline: '2099-01-02 00:00:00.000' }, { financialSource: { ...proposal().financialSource, sharedRemainingTokens: 999 } },
    { financialSource: { ...proposal().financialSource, remaining: [null,90,80,4,7,0,0,0,3,600000] } },
    { taskDescription: { ...proposal().taskDescription, purpose: 'replay_old_request' } },
    { taskDescription: { ...proposal().taskDescription, extraGrant: true } }, { rawBody: 'private-secret' },
    { attemptClosed: true }, { status: 'assessment_published', assessmentCompleted: false },
    { status: 'assessment_published', assessmentCompleted: true, attemptClosed: true, workerStartAllowed: true },
    { status: 'review_required', reviewReason: 'authority_revalidation_required', workerStartAllowed: true },
    { status: 'review_required', reviewReason: 'unknown raw secret', workerStartAllowed: false },
    { status: 'review_required', reviewReason: 'authority_revalidation_required', workerStartAllowed: false, assessmentCompleted: true, attemptClosed: true },
    { reviewReason: 'extra' }];
  for (const c of changes) assert.equal(contracts.issuance.validRootContinuationIssuance(c === null ? null : issued(c), proposal()), false, JSON.stringify(c));
});
test('bad server receipt never populates a successful approval and status failures preserve old evidence', async t => {
  const v = await mount({ approve: () => issued({ proposalHash: hash('a') }), status: () => { throw Error('secret'); } }); t.after(v.unmount);
  await ready(v); v.b.approvalChecked.value = true; await v.b.approveContinuation(); await v.b.readContinuationStatus();
  assert.equal(v.b.issuance.value, undefined); assert.ok(v.b.approvedProposal.value); assert.equal(writes(v).length, 1);
});
test('same scan expected next attempt during approval preserves the binding and accepts the late ACK', async t => {
  let finish; const v = await mount({ approve: () => new Promise(r => { finish = r; }) }); t.after(v.unmount);
  await ready(v); v.b.approvalChecked.value = true; const p = v.b.approveContinuation();
  v.props.attempt = 2; await flush(); assert.equal(v.b.approving.value, true); assert.equal(v.b.approveAttempted.value, true);
  finish(issued()); await p; assert.equal(v.b.issuance.value.newAttemptNumber, 2); await v.b.readContinuationStatus();
  assert.deepEqual(v.calls.at(-1).slice(1), ['scan-a',1,'root-a',hash('e'),hash('d')]);
});
test('actual parent and child SFCs preserve pending approval through status reset and fence other scopes', async () => {
  const savedState = { scanId:'scan-a',attemptNumber:1,status:'paused', branches:[{branch:'web',status:'paused'}],
    timeline:[],unresolvedContainers:[],sourceGaps:[],targets:[],stopDiagnostic:{category:'stopped',code:'paused',nextAction:'review',obligations:[]} };
  for (const [keep, alter] of [[true,v => { v.props.attempt=2;v.props.status='scanning'; }],
    [false,v => { v.props.scanId='scan-b'; }], [false,v => { v.props.attempt=3; }]]) {
    let finish, reads=0; const v=await mount({ approve:()=>new Promise(r=>{finish=r;}),
      scanStatus:()=>++reads===1?savedState:new Promise(()=>{}) },true);
    try { const first=v.b; await ready(v); v.b.approvalChecked.value=true; const p=v.b.approveContinuation();
      alter(v); await flush(); finish(issued()); await p; await flush();
      assert.equal(v.b.issuance.value?.assessmentCompleted,keep?false:undefined);
      assert.equal(v.b.approveAttempted.value,keep); if(keep) assert.equal(v.b,first);
      assert.equal(writes(v).length,1);
    } finally { v.unmount(); }
  }
});
test('other scans attempts closed panels and unmount fence late approval results', async () => {
  for (const alter of [v => { v.props.scanId = 'scan-b'; }, v => { v.props.attempt = 3; },
    v => v.b.onToggle({ target: { open: false } }), v => v.unmount()]) {
    let finish; const v = await mount({ approve: () => new Promise(r => { finish = r; }) });
    try { await ready(v); v.b.approvalChecked.value = true; const p = v.b.approveContinuation();
      alter(v); await flush(); finish(issued()); await p; assert.equal(v.b.issuance.value, undefined);
      assert.equal(v.b.approveAttempted.value, false); assert.equal(v.b.issuanceError.value, '');
    } finally { v.unmount(); }
  }
});
test('late readonly status follows only the explicitly approved source or expected new attempt', async () => {
  for (const [keep, alter] of [[true, v => { v.props.attempt = 2; }],
    [false, v => { v.props.scanId = 'scan-b'; }], [false, v => { v.props.attempt = 3; }],
    [false, v => v.b.onToggle({ target: { open: false } })], [false, v => v.unmount()]]) {
    let finish; const v = await mount({ status: () => new Promise(r => { finish = r; }) });
    try { await ready(v); v.b.approvalChecked.value = true; await v.b.approveContinuation();
      const p = v.b.readContinuationStatus(); alter(v); await flush();
      finish(issued({ status: 'assessment_published', assessmentCompleted: true, attemptClosed: true, workerStartAllowed: false }));
      await p; assert.equal(v.b.issuance.value?.assessmentCompleted, keep ? true : undefined);
      assert.equal(v.b.approveAttempted.value, keep); assert.equal(writes(v).length, 1);
      assert.equal(v.b.issuanceError.value, '');
    } finally { v.unmount(); }
  }
});
test('published pending closure and closed partial are distinct and never claim the Web task completed', async t => {
  let closed = false; const v = await mount({ status: () => issued({ status: 'assessment_published', assessmentCompleted: true,
    attemptClosed: closed, workerStartAllowed: !closed }) }); t.after(v.unmount);
  await ready(v); v.b.approvalChecked.value = true; await v.b.approveContinuation(); await v.b.readContinuationStatus();
  let html = await v.render(); assert.match(html, /已实际发布/); assert.match(html, /收口尚未完成/); assert.match(html, /整个 Web 目标尚未完成/);
  assert.doesNotMatch(html, /尝试已核验收口/); closed = true; await v.b.readContinuationStatus();
  html = await v.render(); assert.match(html, /尝试已核验收口/); assert.match(html, /partial/); assert.doesNotMatch(html, /显式重试同一批准/);
  const count = writes(v).length; await v.b.approveContinuation(); assert.equal(writes(v).length, count);
});
test('review and unproven multiple-root chains permit reads and never approve additional work', async t => {
  const v = await mount({ status: () => issued({ status: 'review_required', assessmentCompleted: true,
    reviewReason: 'authority_revalidation_required', workerStartAllowed: false }) }); t.after(v.unmount);
  await ready(v); v.b.approvalChecked.value = true; await v.b.approveContinuation(); await v.b.readContinuationStatus();
  assert.match(await v.render(), /人工复核/); assert.match(await v.render(), /已实际发布/);
  assert.match(await v.render(), /收口尚未完成/); const n = writes(v).length; await v.b.approveContinuation(); assert.equal(writes(v).length, n);
  const m = await mount({ preview: () => proposal({ historicalRootsChecked: 2 }) }); t.after(m.unmount);
  await ready(m); m.b.approvalChecked.value = true; await m.b.approveContinuation(); assert.equal(writes(m).length, 0);
  assert.match(await m.render(), /授权链尚未证明/);
});
test('published and closed immutable proofs cannot regress on later status receipts', async t => {
  let n = 0; const v = await mount({ status: () => ++n === 1
    ? issued({ status: 'assessment_published', assessmentCompleted: true, attemptClosed: true, workerStartAllowed: false })
    : issued() }); t.after(v.unmount);
  await ready(v); v.b.approvalChecked.value = true; await v.b.approveContinuation(); await v.b.readContinuationStatus();
  await v.b.readContinuationStatus(); assert.equal(v.b.issuance.value.assessmentCompleted, true);
  assert.equal(v.b.issuance.value.attemptClosed, true); assert.match(v.b.issuanceError.value, /仍未核实/);
  assert.equal(contracts.issuance.validRootContinuationIssuance(issued(), proposal(), issued({
    status: 'assessment_published', assessmentCompleted: true, workerStartAllowed: true })), false);
});
test('real production adapters send exact approval binding and independent readonly IPC', async () => {
  const { loadSentinelApi } = require('./sentinel_api_harness.cjs'); const calls = [];
  const api = loadSentinelApi(async (...args) => { calls.push(args); return issued(); });
  const args = ['scan-a',1,'root-a',hash('e'),hash('d')];
  await api.approveNativeRootFinancialContinuation(...args); await api.getNativeRootFinancialContinuation(...args);
  const binding = { scanId:'scan-a',attemptNumber:1,rootRunId:'root-a',receiptId:hash('e'),proposalHash:hash('d') };
  assert.deepEqual(calls, [['approve_native_root_financial_continuation',binding], ['get_native_root_financial_continuation',binding]]);
});
