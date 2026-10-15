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
async function mount(handlers = {}, realParent = false, initialProps = {}) {
  const calls = [], exports = {};
  const api = new Proxy({}, { get(_, key) {
    assert.ok(['getNativeRootCostReconciliations', 'confirmNativeRootCostReconciliation', 'previewNativeRootFinancialContinuation', 'approveNativeRootFinancialContinuation', 'getNativeRootFinancialContinuation', 'discoverNativeRootFinancialContinuation',
      ...(realParent ? ['getNativeScanStatus'] : [])].includes(key),
      `Unexpected IPC: ${String(key)}`);
    return async (...args) => {
      calls.push([key, ...args]);
      if (key === 'getNativeScanStatus') return handlers.scanStatus(...args);
      const defaults = { getNativeRootCostReconciliations: () => inventory({ receipts: [] }), confirmNativeRootCostReconciliation: () => receipt(),
        previewNativeRootFinancialContinuation: () => proposal(), approveNativeRootFinancialContinuation: () => issued(),
        getNativeRootFinancialContinuation: () => issued(), discoverNativeRootFinancialContinuation: () => discovery() };
      const names = { getNativeRootCostReconciliations: 'read', confirmNativeRootCostReconciliation: 'confirm',
        previewNativeRootFinancialContinuation: 'preview', approveNativeRootFinancialContinuation: 'approve', getNativeRootFinancialContinuation: 'status', discoverNativeRootFinancialContinuation: 'discover' };
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
  const props = vue.reactive({ scanId: 'scan-a', attempt: 2, status: 'partial', ...initialProps });
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
const discovery = (extra = {}) => ({ schema: 1, scanId: 'scan-a', attemptNumber: 2,
  issued: { originalReceipt: receipt(), proposal: proposal(), status: issued(), historicalProposal: true }, ...extra });
const writes = v => v.calls.filter(c => ['approveNativeRootFinancialContinuation','confirmNativeRootCostReconciliation'].includes(c[0]));
const reads = v => v.calls.filter(c => c[0] === 'discoverNativeRootFinancialContinuation');
const clone = v => JSON.parse(JSON.stringify(v));
async function find(v) { await v.open(); await v.b.discoverContinuation(); }
test('mount opening cost refresh confirmation and preview never discover or approve prior work', async t => {
  const v = await mount(); t.after(v.unmount); assert.equal(v.calls.length, 0);
  await v.open(); await v.b.load(); assert.equal(reads(v).length, 0); assert.equal(writes(v).length, 0);
  const bill = receipt(); for (const key of ['receiptId','actualCostConfirmed','executionAllowed','automaticResumeAllowed']) delete bill[key];
  const c = await mount({ read: () => inventory({ items:[bill],receipts:[] }) },false,{attempt:1}); t.after(c.unmount);
  await c.open(); c.b.select(c.b.inventory.value.items[0]); await c.b.confirm();
  await c.b.previewContinuation(c.b.inventory.value.receipts[0]); assert.equal(reads(c).length,0);
  assert.equal(writes(c).filter(x=>x[0]==='approveNativeRootFinancialContinuation').length,0);
});
test('explicit discovery binds current new attempt to original receipt and leaves new intent unchecked', async t => {
  const v=await mount();t.after(v.unmount);await find(v);
  assert.deepEqual(reads(v),[['discoverNativeRootFinancialContinuation','scan-a',2]]);
  assert.equal(v.b.approvedProposal.value.attemptNumber,1);assert.equal(v.b.issuance.value.newAttemptNumber,2);
  assert.equal(v.b.recoveredReceipt.value.invoiceHash,receipt().invoiceHash);
  assert.equal(v.b.approvalChecked.value,false);assert.equal(v.b.recoveredOperation.value,true);
  await v.b.approveContinuation();assert.equal(writes(v).length,0);
  const html=await v.render();assert.match(html,/只读找回/);assert.match(html,/重新确认上述原冻结评估/);
  assert.match(html,/原实际输入/);assert.match(html,/23 \/ 3 \/ 7 \/ 1/);
  assert.match(html,/600000/);assert.match(html,/2099-01-01/);assert.match(html,/170/);
});
test('closing reopening and actual parent remount recover durable source proof without automatic writes', async () => {
  let v=await mount({scanStatus:()=>new Promise(()=>{})},true);
  try {await find(v);v.b.approvalChecked.value=true;v.b.onToggle({target:{open:false}});await v.open();
    assert.equal(v.b.approveAttempted.value,false);assert.equal(v.b.approvalChecked.value,false);
    assert.equal(reads(v).length,1);await v.b.discoverContinuation();assert.equal(writes(v).length,0);
  }finally{v.unmount();}
  v=await mount({scanStatus:()=>new Promise(()=>{})},true);
  try {assert.equal(reads(v).length,0);await find(v);assert.equal(v.b.issuance.value.sourceAttemptNumber,1);
    assert.equal(v.b.approvalChecked.value,false);assert.equal(writes(v).length,0);
  }finally{v.unmount();}
});
test('recovered pending publication closure needs a fresh checkbox then exact same approval binding', async t => {
  const published=issued({status:'assessment_published',assessmentCompleted:true,workerStartAllowed:true});
  const v=await mount({discover:()=>discovery({issued:{...discovery().issued,status:published}}),approve:()=>published});t.after(v.unmount);
  await find(v);await v.b.approveContinuation();assert.equal(writes(v).length,0);
  v.b.approvalChecked.value=true;await v.b.approveContinuation();
  assert.deepEqual(writes(v),[['approveNativeRootFinancialContinuation','scan-a',1,'root-a',hash('e'),hash('d')]]);
  assert.match(await v.render(),/收口尚未完成/);assert.match(await v.render(),/整个 Web 目标尚未完成/);
});
test('closed or review receipts grant no checkbox or execution retry even when directly requested', async () => {
  for(const status of [issued({status:'assessment_published',assessmentCompleted:true,attemptClosed:true,workerStartAllowed:false}),
    issued({status:'review_required',assessmentCompleted:true,reviewReason:'authority_revalidation_required',workerStartAllowed:false})]){
    const v=await mount({discover:()=>discovery({issued:{...discovery().issued,status}})});
    try {await find(v);v.b.approvalChecked.value=true;await v.b.approveContinuation();assert.equal(writes(v).length,0);
      const html=await v.render();assert.doesNotMatch(html,/重新确认上述原冻结评估|显式重试同一批准/);
      assert.match(html,status.attemptClosed?/尝试已核验收口/:/人工复核/);
    }finally{v.unmount();}
  }
});
test('null discovery is honest and preserves original costs and an already bound operation', async t => {
  let n=0;const v=await mount({discover:()=>++n===1?discovery():discovery({issued:null})});t.after(v.unmount);
  await find(v);const binding=v.b.approvedProposal.value,status=v.b.issuance.value,bills=v.b.inventory.value;
  await v.b.discoverContinuation();assert.match(v.b.discoveryMessage.value,/未找到该尝试签发记录/);
  assert.equal(v.b.approvedProposal.value,binding);assert.equal(v.b.issuance.value,status);assert.equal(v.b.inventory.value,bills);
  assert.equal(writes(v).length,0);
});
test('historical false rollout remains unchanged and never passes the live preview grant decoder', async t => {
  const original=proposal({issuanceImplemented:false});const report=discovery({issued:{...discovery().issued,proposal:original}});
  assert.equal(contracts.continuation.validRootContinuationPreview(original,receipt()),false);
  assert.equal(contracts.discovery.validRootContinuationDiscovery(report,'scan-a',2),true);
  const v=await mount({discover:()=>report});t.after(v.unmount);await find(v);
  assert.equal(v.b.approvedProposal.value.issuanceImplemented,false);assert.equal(v.b.approvalChecked.value,false);
  await v.b.approveContinuation();assert.equal(writes(v).length,0);
  v.b.approvalChecked.value=true;await v.b.approveContinuation();assert.equal(writes(v).length,1);
  assert.equal(v.b.approvedProposal.value.issuanceImplemented,false);
});
test('cold receipt proposal status scope hashes and permission corruption all fail closed', async () => {
  const mutations=[x=>x.schema=2,x=>x.scanId='foreign',x=>x.attemptNumber=3,x=>x.extra=true,x=>x.issued.historicalProposal=false,
    x=>x.issued.extraGrant=true,x=>x.issued.originalReceipt.attemptNumber=2,x=>x.issued.originalReceipt.callId='bad',
    x=>x.issued.originalReceipt.receiptId=hash('f'),x=>x.issued.originalReceipt.executionAllowed=true,
    x=>x.issued.proposal.issuanceImplemented='unknown',x=>x.issued.proposal.automaticResumeAllowed=true,
    x=>x.issued.proposal.proposedAttemptNumber=3,x=>x.issued.proposal.taskDescription.originalSemanticReplayAllowed=true,
    x=>x.issued.proposal.financialSource.remaining[0]=-1,x=>x.issued.proposal.financialSource.executionAllowed=true,
    x=>x.issued.proposal.proposalHash=hash('b'),x=>x.issued.proposal.taskDescription.originalPlanHash='bad',
    x=>x.issued.status.newAttemptNumber=3,x=>x.issued.status.sourceRootRunId='foreign',
    x=>x.issued.status.originalDeadline='2099-01-02 00:00:00.000',x=>x.issued.status.financialSource.sharedRemainingTokens=999,
    x=>x.issued.status.taskDescription.purpose='replay_old',x=>x.issued.status.reviewReason='raw private secret',
    x=>x.issued.status.workerStartAllowed='yes',x=>delete x.issued.originalReceipt.invoiceHash];
  for(const mutate of mutations){const report=clone(discovery());mutate(report);
    assert.equal(contracts.discovery.validRootContinuationDiscovery(report,'scan-a',2),false);
    const v=await mount({discover:()=>report});try{await find(v);assert.equal(v.b.issuance.value,undefined);
      assert.equal(v.b.approvedProposal.value,undefined);assert.ok(v.b.discoveryError.value);assert.equal(writes(v).length,0);
    }finally{v.unmount();}}
});
test('duplicate discovery clicks serialize one read and block competing writes and cost refresh', async t => {
  let finish;const v=await mount({discover:()=>new Promise(r=>{finish=r;})});t.after(v.unmount);await v.open();
  const pending=v.b.discoverContinuation();await v.b.discoverContinuation();const count=v.calls.length;
  await v.b.load();await v.b.approveContinuation();await v.b.readContinuationStatus();assert.equal(v.calls.length,count);
  assert.equal(reads(v).length,1);finish(discovery());await pending;assert.ok(v.b.issuance.value);assert.equal(writes(v).length,0);
});
test('A-B-A scan and attempt changes fence late discoveries despite matching final props', async () => {
  for(const alter of [async v=>{v.props.scanId='scan-b';await flush();v.props.scanId='scan-a';},
    async v=>{v.props.attempt=3;await flush();v.props.attempt=2;}]){
    let finish;const v=await mount({discover:()=>new Promise(r=>{finish=r;})});
    try{await v.open();const p=v.b.discoverContinuation();await alter(v);await flush();finish(discovery());await p;
      assert.equal(v.b.issuance.value,undefined);assert.equal(v.b.approveAttempted.value,false);assert.equal(v.b.discoveryError.value,'');
    }finally{v.unmount();}
  }
});
test('close unmount and actual parent scan remount reject pending discovery results', async () => {
  for(const [parent,alter] of [[false,v=>v.b.onToggle({target:{open:false}})],[false,v=>v.unmount()],
    [true,v=>{v.props.scanId='scan-b';}],[true,v=>{v.props.attempt=3;}]]){
    let finish;const v=await mount({discover:()=>new Promise(r=>{finish=r;}),scanStatus:()=>new Promise(()=>{})},parent);
    try{await v.open();const p=v.b.discoverContinuation();alter(v);await flush();finish(discovery());await p;
      assert.equal(v.b.issuance.value,undefined);assert.equal(v.b.approvalChecked.value,false);assert.equal(writes(v).length,0);
    }finally{v.unmount();}
  }
});
test('discovery and readonly status errors hide sensitive details and retain previously verified binding', async t => {
  let n=0;const v=await mount({discover:()=>{if(++n>1)throw Error('Bearer private-secret /private/db.sqlite');return discovery();},
    status:()=>{throw Error('Bearer private-secret');}});t.after(v.unmount);await find(v);const before=v.b.issuance.value;
  await v.b.discoverContinuation();await v.b.readContinuationStatus();assert.equal(v.b.issuance.value,before);
  assert.doesNotMatch(await v.render(),/private-secret|db.sqlite/);assert.equal(writes(v).length,0);
  assert.deepEqual(v.calls.at(-1),['getNativeRootFinancialContinuation','scan-a',1,'root-a',hash('e'),hash('d')]);
});
test('later discovery cannot replace existing approval Root identity or regress immutable completion', async t => {
  let n=0;const closed=issued({status:'assessment_published',assessmentCompleted:true,attemptClosed:true,workerStartAllowed:false});
  const v=await mount({discover:()=>discovery({issued:{...discovery().issued,status:++n===1?closed:issued()}})});t.after(v.unmount);
  await find(v);await v.b.discoverContinuation();assert.equal(v.b.issuance.value.attemptClosed,true);assert.ok(v.b.discoveryError.value);
  assert.equal(contracts.discovery.validRootContinuationDiscovery(discovery({issued:{...discovery().issued,
    status:issued({approvalId:'00000000-0000-4000-8000-000000000002',newRootRunId:'another-root'})}}),'scan-a',2,closed),false);
});
test('same hash cannot replace pinned immutable proposal or financial facts during repeat discovery', async () => {
  for(const mutate of [x=>{x.issued.originalReceipt.invoiceHash=hash('a');},x=>{x.issued.proposal.evidenceHashes.context=hash('c');},
    x=>{x.issued.proposal.financialSource.remaining[0]=999;x.issued.status.financialSource.remaining[0]=999;},
    x=>{x.issued.proposal.taskDescription.originalPlanHash=hash('b');x.issued.status.taskDescription.originalPlanHash=hash('b');}]){
    let n=0;const changed=clone(discovery());mutate(changed);
    const v=await mount({discover:()=>++n===1?discovery():changed});
    try{await find(v);const before=v.b.approvedProposal.value;await v.b.discoverContinuation();
      assert.equal(v.b.approvedProposal.value,before);assert.ok(v.b.discoveryError.value);assert.equal(writes(v).length,0);
    }finally{v.unmount();}
  }
});
test('real production discovery adapter invokes only readonly IPC with exact current scope', async () => {
  const calls=[];const api=require('./sentinel_api_harness.cjs').loadSentinelApi(async(...args)=>{calls.push(args);return discovery();});
  const report=await api.discoverNativeRootFinancialContinuation('scan-a',2);
  assert.equal(contracts.discovery.validRootContinuationDiscovery(report,'scan-a',2),true);
  assert.deepEqual(calls,[['discover_native_root_financial_continuation',{scanId:'scan-a',attemptNumber:2}]]);
});
