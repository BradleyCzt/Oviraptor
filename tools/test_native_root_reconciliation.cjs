// Production Vue setup and template; IPC is substituted. Rust proves original bills.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');
const vue = require('vue');
const { renderToString } = require('@vue/server-renderer');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const root = path.join(__dirname, '..');
const transpile = source => ts.transpileModule(source, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020,
} }).outputText;
const contract = {};
new Function('exports', transpile(fs.readFileSync(path.join(root,
  'src/features/sentinel/reconciliation/contract.ts'), 'utf8')))(contract);
const continuationContract = {};
new Function('exports', transpile(fs.readFileSync(path.join(root,
  'src/features/sentinel/reconciliation/continuation.ts'), 'utf8')))(continuationContract);
const issuanceContract = {};
new Function('exports', transpile(fs.readFileSync(path.join(root,
  'src/features/sentinel/reconciliation/issuance.ts'), 'utf8')))(issuanceContract);
const discoveryContract = {};
new Function('require', 'exports', transpile(fs.readFileSync(path.join(root,
  'src/features/sentinel/reconciliation/discovery.ts'), 'utf8')))(name => {
  const modules = { './contract': contract, './continuation': continuationContract, './issuance': issuanceContract };
  assert.ok(Object.prototype.hasOwnProperty.call(modules, name), `Unexpected discovery import: ${name}`);
  return modules[name];
}, discoveryContract);
const filename = path.join(root, 'src/features/sentinel/components/NativeRootReconciliation.vue');
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'root-cost-test' });
const template = compileTemplate({ source: descriptor.template.content, filename, id: 'root-cost-test',
  compilerOptions: { bindingMetadata: script.bindings } });
assert.deepEqual(template.errors, []);
const rendered = {};
new Function('require', 'exports', transpile(template.code))(require, rendered);
const renderer = vue.createRenderer({ createElement: () => ({}), createText: () => ({}),
  createComment: () => ({}), insert() {}, remove() {}, setText() {}, setElementText() {},
  patchProp() {}, parentNode: () => null, nextSibling: () => null });
const flush = async () => { for (let n = 0; n < 10; n++) await vue.nextTick(); };
const hash = ch => ch.repeat(64);
const preview = (extra = {}) => ({ version: 1, scanId: 'scan-a', attemptNumber: 1, rootRunId: 'root-a',
  callId: hash('a'), leaseAttemptId: 'lease-a', round: 2, requestHash: hash('b'), basisHash: hash('c'),
  invoiceHash: hash('d'), confirmationHash: hash('e'), usage: { inputTokens: 23, cachedInputTokens: 3,
    outputTokens: 7, totalTokens: 30, modelRequests: 1 }, operation: 'settle_received', ...extra });
const receipt = (item = preview(), extra = {}) => ({ ...item, receiptId: item.confirmationHash,
  usage: { ...item.usage }, actualCostConfirmed: true, executionAllowed: false, automaticResumeAllowed: false, ...extra });
const inventory = (extra = {}) => ({ schema: 1, executionAllowed: false, automaticResumeAllowed: false,
  items: [preview()], blocked: [], receipts: [], truncated: false, ...extra });
async function mount(handlers = {}) {
  const calls = [], exports = {};
  const api = new Proxy({}, { get(_, key) {
    if (!['getNativeRootCostReconciliations', 'confirmNativeRootCostReconciliation'].includes(key))
      throw Error(`Unexpected execution API: ${String(key)}`);
    return async (...args) => {
      calls.push([key, ...args]);
      return key === 'getNativeRootCostReconciliations' ? (handlers.read || (() => inventory()))(...args)
        : (handlers.confirm || (() => receipt()))(...args);
    };
  } });
  new Function('require', 'exports', transpile(script.content))(name => {
    if (name === 'vue') return vue;
    if (name === '../../../api') return { api };
    if (name === '../../../i18n') return { useI18n: () => ({ tr: zh => zh }) };
    if (name === '../reconciliation/contract') return contract;
    if (name === '../reconciliation/continuation') return continuationContract;
    if (name === '../reconciliation/issuance') return issuanceContract;
    if (name === '../reconciliation/discovery') return discoveryContract;
    throw Error(`Unexpected import: ${name}`);
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
test('opening and refreshing read original bills without confirming costs or executing work', async t => {
  const view = await mount(); t.after(view.unmount);
  assert.equal(view.calls.length, 0); await view.b.confirm(); assert.equal(view.calls.length, 0);
  await view.open(); await view.b.load(); assert.equal(view.calls.length, 2);
  assert.ok(view.calls.every(call => call[0] === 'getNativeRootCostReconciliations'));
  const html = await view.render();
  for (const value of ['scan-a', 'root-a', 'lease-a', hash('a'), hash('b'), hash('c'), hash('d'), hash('e')])
    assert.ok(html.includes(value), value);
  assert.match(html, /供应商实际用量/); assert.match(html, /不会续跑/); assert.doesNotMatch(html, /原费用已确认/);
});
test('review then explicit confirmation sends the frozen binding and displays only a financial receipt', async t => {
  const view = await mount(); t.after(view.unmount); await view.open();
  const item = view.b.inventory.value.items[0]; view.b.select(item);
  assert.equal(view.calls.length, 1); assert.match(await view.render(), /确认上述实际费用/);
  await view.b.confirm();
  assert.deepEqual(view.calls[1], ['confirmNativeRootCostReconciliation', 'scan-a', 1, 'root-a', hash('a'), hash('e')]);
  assert.equal(view.b.inventory.value.items.length, 0); assert.equal(view.b.inventory.value.receipts.length, 1);
  assert.equal(view.b.selected.value, undefined);
  const html = await view.render(); assert.match(html, /不可变对账回执/); assert.match(html, /不授予执行权限/);
  assert.doesNotMatch(html, /启动新工作|已恢复|任务已完成/);
  await view.b.confirm(); assert.equal(view.calls.length, 2);
});
test('truncated inventory exposes later bills only after an explicit refresh following confirmation', async t => {
  const first = preview(), next = preview({ callId: hash('f'), round: 3 });
  let confirmed = false;
  const view = await mount({
    read: () => inventory(confirmed ? { items: [next], receipts: [receipt(first)], truncated: true } : { truncated: true }),
    confirm: () => { confirmed = true; return receipt(first); },
  });
  t.after(view.unmount); await view.open();
  assert.match(await view.render(), /确认本页费用后可刷新查看后续账单/);
  view.b.select(view.b.inventory.value.items[0]); await view.b.confirm();
  assert.equal(view.calls.length, 2, 'confirmation does not automatically fetch or execute');
  assert.equal(view.b.inventory.value.items.length, 0);
  await view.b.load();
  assert.equal(view.calls.length, 3);
  assert.equal(view.b.inventory.value.items[0].callId, next.callId);
  assert.equal(view.b.inventory.value.receipts[0].callId, first.callId);
  assert.equal(view.b.inventory.value.truncated, true);
  assert.equal(view.b.selected.value, undefined);
});
test('duplicate confirm clicks share one in-flight operation and receive no automatic retries', async t => {
  let done;
  const view = await mount({ confirm: () => new Promise(resolve => { done = resolve; }) }); t.after(view.unmount);
  await view.open(); view.b.select(view.b.inventory.value.items[0]);
  const action = view.b.confirm(); await view.b.confirm(); await view.b.load();
  assert.equal(view.calls.length, 2); done(receipt()); await action;
  assert.equal(view.b.inventory.value.receipts.length, 1);
});
test('mismatched financially or executively false receipts never mark the bill confirmed', async () => {
  const changes = [{ version: 2 }, { scanId: 'foreign' }, { attemptNumber: 2 }, { rootRunId: 'foreign' },
    { callId: hash('f') }, { invoiceHash: hash('f') }, { confirmationHash: hash('f') }, { receiptId: hash('f') },
    { leaseAttemptId: 'foreign' }, { round: 3 }, { requestHash: hash('f') }, { basisHash: hash('f') },
    { operation: 'reconcile_forfeit' }, { actualCostConfirmed: false }, { executionAllowed: true },
    { automaticResumeAllowed: true }, { usage: { ...preview().usage, cachedInputTokens: 4 } },
    { privateBody: 'Bearer private-secret /private/db.sqlite' }];
  for (const change of changes) {
    const view = await mount({ confirm: () => receipt(preview(), change) });
    try {
      await view.open(); const frozen = view.b.inventory.value.items[0]; view.b.select(frozen); await view.b.confirm();
      assert.equal(view.b.inventory.value.receipts.length, 0); assert.equal(view.b.inventory.value.items.length, 1);
      assert.deepEqual(view.b.selected.value, frozen); assert.equal(view.b.message.value, '');
      assert.match(view.b.error.value, /尚未核实/); assert.doesNotMatch(view.b.error.value, /private-secret|db.sqlite/);
    } finally { view.unmount(); }
  }
});
test('scope changes discard stale reads and stale confirmation completions', async t => {
  let readDone;
  const reading = await mount({ read: () => new Promise(resolve => { readDone = resolve; }) }); t.after(reading.unmount);
  await reading.open(); reading.props.attempt = 2; await flush(); readDone(inventory()); await flush();
  assert.equal(reading.b.inventory.value, undefined); assert.equal(reading.b.selected.value, undefined);
  let writeDone;
  const writing = await mount({ confirm: () => new Promise(resolve => { writeDone = resolve; }) }); t.after(writing.unmount);
  await writing.open(); writing.b.select(writing.b.inventory.value.items[0]); const action = writing.b.confirm();
  writing.props.scanId = 'scan-b'; await flush(); writeDone(receipt()); await action;
  assert.equal(writing.b.inventory.value, undefined); assert.equal(writing.b.message.value, '');
  await writing.b.confirm(); assert.equal(writing.calls.length, 2);
});
test('lost acknowledgement refresh reads immutable receipts without automatically retrying confirmation', async t => {
  let stored = false;
  const view = await mount({ read: () => stored ? inventory({ items: [], receipts: [receipt()] }) : inventory(),
    confirm: () => { stored = true; throw Error('Bearer private-secret /private/db.sqlite'); } }); t.after(view.unmount);
  await view.open(); view.b.select(view.b.inventory.value.items[0]); await view.b.confirm();
  assert.equal(view.calls.length, 2); assert.ok(view.b.selected.value); assert.match(view.b.error.value, /尚未核实/);
  assert.doesNotMatch(view.b.error.value, /private-secret|db.sqlite/);
  await view.b.load(); assert.equal(view.calls.length, 3);
  assert.equal(view.calls.filter(call => call[0] === 'confirmNativeRootCostReconciliation').length, 1);
  assert.equal(view.b.selected.value, undefined); assert.equal(view.b.inventory.value.receipts.length, 1);
  assert.match(await view.render(), /原费用已确认/); await view.b.confirm(); assert.equal(view.calls.length, 3);
});
test('an explicit retry after lost acknowledgement retains exactly the same original bill binding', async t => {
  let attempts = 0;
  const view = await mount({ confirm: () => { if (++attempts === 1) throw Error('lost acknowledgement'); return receipt(); } }); t.after(view.unmount);
  await view.open(); view.b.select(view.b.inventory.value.items[0]); await view.b.confirm();
  assert.equal(view.calls.length, 2); await view.b.confirm(); assert.deepEqual(view.calls[1], view.calls[2]);
  assert.equal(view.b.inventory.value.receipts.length, 1);
});
test('blocked bills display unresolved records without creating confirmation authority or leaking reasons', async t => {
  const view = await mount({ read: () => inventory({ items: [], blocked: [{ rootRunId: 'root-a', callId: hash('a'),
    reason: 'Bearer private-secret' }] }) }); t.after(view.unmount); await view.open();
  const html = await view.render(); assert.match(html, /原费用仍未解决/);
  assert.doesNotMatch(html, /private-secret|核对这张原账单|确认原费用对账/);
  view.b.select(preview()); await view.b.confirm(); assert.equal(view.calls.length, 1);
});
test('forfeit confirmation uses exact reported costs and truncated inventories never imply complete settlement', async t => {
  const item = preview({ operation: 'reconcile_forfeit' });
  const view = await mount({ read: () => inventory({ items: [item], truncated: true }),
    confirm: () => receipt(item) }); t.after(view.unmount);
  await view.open(); let html = await view.render();
  assert.match(html, /将匹配的原未决费用转为实际已用/); assert.match(html, /不能据此认为全部费用已结清/);
  view.b.select(view.b.inventory.value.items[0]); await view.b.confirm();
  html = await view.render(); assert.match(html, /不能据此认为全部费用已结清/);
  assert.match(html, /实际用量/); assert.match(html, /原租约尝试/);
  assert.equal(view.b.inventory.value.receipts[0].usage.totalTokens, 30);
  assert.equal(view.calls.length, 2);
});
test('closing or unmounting the panel discards late responses and never schedules a write', async t => {
  let finish;
  const view = await mount({ read: () => new Promise(resolve => { finish = resolve; }) }); t.after(view.unmount);
  await view.open(); view.b.onToggle({ target: { open: false } }); finish(inventory()); await flush();
  assert.equal(view.b.inventory.value, undefined); await view.b.confirm(); assert.equal(view.calls.length, 1);
  let confirmDone;
  const writing = await mount({ confirm: () => new Promise(resolve => { confirmDone = resolve; }) });
  await writing.open(); writing.b.select(writing.b.inventory.value.items[0]); const action = writing.b.confirm();
  writing.unmount(); confirmDone(receipt()); await action;
  assert.equal(writing.b.message.value, ''); assert.equal(writing.b.inventory.value.receipts.length, 0);
  assert.equal(writing.calls.length, 2);
});
test('corrupt inventories and read failures clear prior bills and retain no authority', async () => {
  const bad = [null, inventory({ schema: 2 }), inventory({ executionAllowed: true }), inventory({ automaticResumeAllowed: true }),
    inventory({ privateBody: 'private-secret' }), inventory({ truncated: 'yes' }), inventory({ items: [preview({ scanId: 'foreign' })] }),
    inventory({ items: [preview({ callId: 'bad' })] }), inventory({ items: [preview({ round: 0 })] }),
    inventory({ items: [preview({ usage: { ...preview().usage, totalTokens: 99 } })] }),
    inventory({ items: [preview({ usage: { ...preview().usage, inputTokens: -1 } })] }),
    inventory({ items: [preview()], receipts: [receipt()] }), inventory({ receipts: [receipt(preview(), { executionAllowed: true })] }),
    inventory({ blocked: [{ rootRunId: 'root-b', callId: hash('f'), reason: '\nprivate-secret' }] })];
  for (const report of bad) {
    let fresh = false;
    const view = await mount({ read: () => fresh ? report : inventory() });
    try {
      await view.open(); view.b.select(view.b.inventory.value.items[0]); fresh = true; await view.b.load();
      assert.equal(view.b.inventory.value, undefined); assert.equal(view.b.selected.value, undefined);
      await view.b.confirm(); assert.equal(view.calls.length, 2); assert.ok(view.b.error.value);
    } finally { view.unmount(); }
  }
  const failure = await mount({ read: () => { throw Error('Bearer private-secret /private/db.sqlite'); } });
  try { await failure.open(); assert.doesNotMatch(failure.b.error.value, /private-secret|db.sqlite/); }
  finally { failure.unmount(); }
});
test('production reconciliation API sends only the specified immutable scope and confirmation binding', async () => {
  const exports = {}, calls = [];
  new Function('require', 'exports', transpile(fs.readFileSync(path.join(root,
    'src/features/sentinel/reconciliation/api.ts'), 'utf8')))(name => {
    assert.equal(name, '@tauri-apps/api/core'); return { invoke: async (...args) => { calls.push(args); } };
  }, exports);
  await exports.rootCostReconciliationApi.getNativeRootCostReconciliations('scan-a', 1);
  await exports.rootCostReconciliationApi.confirmNativeRootCostReconciliation('scan-a', 1, 'root-a', hash('a'), hash('e'));
  assert.deepEqual(calls, [['get_native_root_cost_reconciliations', { scanId: 'scan-a', attemptNumber: 1 }],
    ['confirm_native_root_cost_reconciliation', { scanId: 'scan-a', attemptNumber: 1, rootRunId: 'root-a',
      callId: hash('a'), confirmationHash: hash('e') }]]);
});
