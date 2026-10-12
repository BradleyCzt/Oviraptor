const test = require('node:test'), assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), ts = require('typescript'), vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const root = path.join(__dirname, '..');
const transpile = s => ts.transpileModule(s, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } }).outputText;
const contract = {};
new Function('exports', transpile(fs.readFileSync(path.join(root, 'src/features/sentinel/recovery/contract.ts'), 'utf8')))(contract);
const filename = path.join(root, 'src/features/sentinel/components/NativeProtectionRecovery.vue');
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'protection-recovery' });
const template = compileTemplate({ source: descriptor.template.content, filename, id: 'protection-recovery', compilerOptions: { bindingMetadata: script.bindings } });
assert.deepEqual(template.errors, []);
const rendered = {};
new Function('require','exports',transpile(template.code))(require, rendered);
const selection = { scanId: 'old', attemptNumber: 1, rootRunId: 'root', protectionReceiptId: 'receipt' };
const hash = 'a'.repeat(64);
const report = scan => ({ schemaVersion: 1, purpose: 'select_original_signed_protection_source', scanId: scan, projectId: 1, taskName: 'original',
  approval: false, readinessAssessed: false, executionAllowed: false, targetRequestsGranted: 0,
  items: [{ selection: { ...selection, scanId: scan }, targetUrl: 'https://example.test/path/?a=1,2', responseStatus: 403,
    originalFactHash: hash, sourceVerified: true, readinessAssessed: false, executionAllowed: false }] });
const proposal = (s, scope) => ({ schemaVersion: 1, purpose: 'explicit_new_target_work_after_original_protection_stop', selection: s, newScope: scope,
  proposalHash: hash, approval: false, executionAllowed: false, targetRequestsGranted: 0, oldStopRetained: true, oldCostsRetained: true, oldScopeRenewed: false });
const approved = op => ({ schemaVersion: 1, purpose: 'explicit_new_target_work_approval', requestId: op.requestId, proposalHash: op.proposalHash,
  approvalId: 'approval', proposal: proposal(op.selection, op.newScope), status: 'approved_pending_fresh_birth', executionAllowed: false, targetRequestsGranted: 0, oldScopeRenewed: false });
global.Document = class Document {};
global.document = Object.assign(new global.Document(), { activeElement: null });
const node = (tag = '') => ({ tagName: tag.toUpperCase(), children: [], parent: null, value: '',
  addEventListener() {}, removeEventListener() {}, getRootNode() { return global.document; },
  get options() { return this.children.filter(v => v.tagName === 'OPTION'); } });
const remove = child => { if (child.parent) { const rows = child.parent.children; rows.splice(rows.indexOf(child),1); child.parent = null; } };
const renderer = vue.createRenderer({ createElement: node, createText: () => node(), createComment: () => node(),
  setText() {}, setElementText() {}, parentNode: n => n.parent,
  nextSibling: n => n.parent?.children[n.parent.children.indexOf(n)+1] || null,
  insert(child,parent,anchor) { remove(child); parent.children ||= []; const i = parent.children.indexOf(anchor);
    parent.children.splice(i < 0 ? parent.children.length : i,0,child); child.parent = parent; },
  remove, patchProp(el,key,old,value) { el[key] = value; } });
const tick = async () => { for (let i = 0; i < 8; i++) { await Promise.resolve(); await vue.nextTick(); } };
function harness(handlers = {}, failStore = false) {
  const store = new Map(), calls = [];
  global.localStorage = { getItem: k => store.get(k) || null, setItem: (k,v) => { if (failStore) throw Error('disk unavailable'); store.set(k,v); } };
  global.crypto = require('node:crypto').webcrypto;
  const api = new Proxy({}, { get: (_, name) => async (...args) => {
    calls.push([name,...args]);
    const defaults = { sources: report, preview: proposal, approve: approved, approval: () => null, draft: () => null, cancellation: () => null };
    if (!(name in defaults) && !handlers[name]) throw Error(`Unexpected IPC ${name}`);
    return (handlers[name] || defaults[name])(...args);
  } });
  const exports = {};
  new Function('require','exports',transpile(script.content))(name => {
    if (name === 'vue') return vue;
    if (name === '../recovery/api') return { protectionRecoveryApi: api };
    if (name === '../recovery/contract') return contract;
    if (name === './NativePricedRecoveryApproval.vue') return { default: { render: () => null } }; // Opt-in child is covered by its actual SFC tests.
    throw Error(name);
  }, exports);
  let b; const props = vue.reactive({ scanId: 'old' });
  const child = { ...exports.default, setup(p,c) { b = exports.default.setup(p,c);
    return () => rendered.render({}, [], {}, vue.proxyRefs(b), {}, {}); } };
  const app = renderer.createApp({ setup() { return () => vue.h(child, props); } });
  app.mount({});
  return { get b() { return b; }, props, calls, store, close: () => app.unmount() };
}
async function ready(h) { h.b.opened.value = true; await tick(); h.b.model.value = 'approved-model'; await h.b.review(); h.b.consent.value = true; }
test('real component stores exact operation before write; lost ACK retries original UUID', async () => {
  let attempts = 0; const h = harness({ approve: op => { assert.equal(h.store.size, 1); if (++attempts === 1) throw Error('lost ACK'); return approved(op); } });
  try {
    await ready(h); await h.b.approve(); const first = h.b.operation.value.requestId;
    assert.equal(h.b.approval.value, undefined); assert.equal(h.b.locked.value, true);
    await h.b.load(); await h.b.review(); h.b.consent.value = true; await h.b.approve();
    assert.equal(h.b.approval.value.requestId, first);
    assert.deepEqual(h.calls.filter(v => v[0] === 'approve').map(v => v[1].requestId), [first,first]);
    assert.deepEqual(h.b.operation.value.newScope.urls, ['https://example.test/path/?a=1,2']);
    assert.equal(h.calls.some(v => /confirm|resume|retry|start/i.test(v[0])), false);
  } finally { h.close(); }
});
test('storage failure sends no approval write', async () => {
  const h = harness({}, true); try { await ready(h); await h.b.approve(); assert.equal(h.calls.filter(v => v[0] === 'approve').length, 0); assert.equal(h.b.operation.value, undefined); } finally { h.close(); }
});
test('production IPC adapter preserves literal scope, fixed consent and original request without ordinary startup', async () => {
  const calls = [], exports = {};
  new Function('require','exports',transpile(fs.readFileSync(path.join(root,'src/features/sentinel/recovery/api.ts'),'utf8')))(name => {
    assert.equal(name,'@tauri-apps/api/core'); return { invoke: async (command,args) => { calls.push([command,args]); return null; } };
  }, exports);
  const api = exports.protectionRecoveryApi, scope = { urls: ['https://example.test/path/?a=1,2'] };
  const op = { selection, newScope: scope, requestId: require('node:crypto').randomUUID(), proposalHash: hash };
  await api.sources('old'); await api.preview(selection,scope); await api.approve(op);
  await api.approval(op.requestId); await api.createDraft(op.requestId); await api.draft(op.requestId);
  await api.cancellation(op.requestId); await api.cancel('new-scan');
  assert.deepEqual(calls.map(v => v[0]), ['discover_native_protection_recovery_sources','preview_native_protection_recovery_proposal',
    'approve_native_protection_recovery','get_native_protection_recovery_approval','create_native_protection_recovery_draft',
    'get_native_protection_recovery_draft','get_native_protection_recovery_cancellation_by_request','cancel_sentinel_scan']);
  assert.deepEqual(calls[2][1], { selection, newScope: scope, requestId: op.requestId, proposalHash: hash, consent: 'approve_exact_new_scope_preserve_old_stop_and_costs' });
  assert.deepEqual(calls.slice(3,7).map(v => v[1]), Array(4).fill({ requestId: op.requestId }));
  assert.deepEqual(calls[7][1], { scanId:'new-scan' });
});
test('pending approval cannot claim old-scope renewal and pending draft cannot claim bound identities', async () => {
  const h = harness({ approve: op => ({ ...approved(op), oldScopeRenewed: true }) });
  try { await ready(h); await h.b.approve(); assert.equal(h.b.approval.value,undefined); assert.equal(h.store.size,1); } finally { h.close(); }
  const g = harness({ createDraft: () => ({ schemaVersion: 1, purpose: 'explicit_protection_recovery_new_draft', scanId:'new', draftId:'draft',
    approval: approved(g.b.operation.value), status:'draft_pending_dedicated_fresh_birth', executionAllowed:false,targetRequestsGranted:0,
    oldScopeRenewed:false,identitiesBound:true }) });
  try { await ready(g); await g.b.approve(); await g.b.createDraft(); assert.equal(g.b.draft.value,undefined); assert.ok(g.b.error.value); } finally { g.close(); }
});
test('unique draft then signed cancellation are found with original request; reload is read only', async () => {
  let op, draft, cancel;
  const h = harness({ approve: value => { op = value; return approved(op); }, approval: () => op ? approved(op) : null,
    createDraft: request => { assert.equal(request,op.requestId); return draft = { schemaVersion: 1, purpose: 'explicit_protection_recovery_new_draft',
      scanId: 'new-scan', draftId: 'new-draft', approval: approved(op), status: 'draft_pending_dedicated_fresh_birth', executionAllowed: false, targetRequestsGranted: 0, oldScopeRenewed: false, identitiesBound: false }; },
    draft: () => draft || null, cancellation: () => cancel || null,
    cancel: scan => { assert.equal(scan,'new-scan'); cancel = { schemaVersion: 1, purpose: 'cancel_unconsumed_protection_recovery_draft',
      requestId: op.requestId, scanId: 'new-scan', draftId: 'new-draft', approvalId: 'approval', cancellationId: 'cancel-id',
      authorityRevoked: true, executionAllowed: false, targetRequestsGranted: 0, oldStopRetained: true, oldCostsRetained: true }; } });
  try {
    await ready(h); await h.b.approve(); await h.b.createDraft(); await h.b.cancel();
    assert.equal(h.b.cancellation.value.cancellationId,'cancel-id');
    const writes = h.calls.filter(v => ['approve','createDraft','cancel'].includes(v[0])).length;
    await h.b.load(); await h.b.createDraft(); await h.b.approve(); await h.b.cancel();
    assert.equal(h.calls.filter(v => ['approve','createDraft','cancel'].includes(v[0])).length,writes);
    assert.equal(h.b.cancellation.value.requestId,op.requestId);
  } finally { h.close(); }
});
test('corrupt saved operation or mismatched signed response blocks writes without replacing UUID', async () => {
  const h = harness(); try {
    h.store.set('oviraptor.protection-recovery.v1:old','{"version":1}'); h.b.opened.value = true; await tick();
    assert.equal(h.b.blocked.value,true); await h.b.review(); await h.b.approve(); assert.equal(h.calls.filter(v => v[0] === 'approve').length,0);
  } finally { h.close(); }
  const g = harness({ approve: op => ({ ...approved(op), requestId: require('node:crypto').randomUUID() }) });
  try { await ready(g); await g.b.approve(); assert.equal(g.b.approval.value,undefined); assert.equal(g.store.size,1); assert.ok(g.b.error.value); } finally { g.close(); }
});
test('scope edits invalidate consent and proposal; old async response cannot cross scan', async () => {
  const h = harness(); try {
    await ready(h); h.b.limits.value[0] = '0'; assert.equal(h.b.proposal.value, undefined); assert.equal(h.b.consent.value, false);
    await h.b.approve(); assert.equal(h.calls.filter(v => v[0] === 'approve').length, 0);
  } finally { h.close(); }
  let release; const g = harness({ preview: (s, scope) => new Promise(r => { release = () => r(proposal(s,scope)); }) });
  try { g.b.opened.value = true; await tick(); const pending = g.b.review(); g.props.scanId = 'another'; await tick(); release(); await pending; assert.equal(g.b.proposal.value, undefined); assert.equal(g.b.sources.value.scanId, 'another'); } finally { g.close(); }
});

test('empty approval identity keeps the original operation pending and cannot create a draft', async () => {
  const h = harness({ approve: op => ({ ...approved(op), approvalId: '' }) });
  try {
    await ready(h); await h.b.approve(); const id = h.b.operation.value.requestId;
    assert.equal(h.b.approval.value, undefined); assert.ok(h.b.error.value);
    await h.b.createDraft();
    assert.equal(h.calls.some(v => v[0] === 'createDraft'), false);
    assert.equal(h.b.operation.value.requestId, id); assert.equal(h.store.size, 1);
  } finally { h.close(); }
});
test('empty draft identity cannot become confirmed or authorize cancellation', async () => {
  const h = harness({ createDraft: () => ({ schemaVersion: 1, purpose: 'explicit_protection_recovery_new_draft',
    scanId: 'new', draftId: '', approval: approved(h.b.operation.value), status: 'draft_pending_dedicated_fresh_birth',
    executionAllowed: false, targetRequestsGranted: 0, oldScopeRenewed: false, identitiesBound: false }) });
  try {
    await ready(h); await h.b.approve(); const id = h.b.operation.value.requestId;
    await h.b.createDraft(); assert.equal(h.b.draft.value, undefined); assert.ok(h.b.error.value);
    await h.b.cancel(); assert.equal(h.calls.some(v => v[0] === 'cancel'), false);
    assert.equal(h.b.operation.value.requestId, id);
  } finally { h.close(); }
});
