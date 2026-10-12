// Actual compiled Vue SFC + actual API module, mounted with Vue's host renderer.
// Native invoke is mocked; this is component interaction, not cryptographic approval.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vue = require('vue');
const { File } = require('node:buffer');
const { compile, text, descendants, mountHost } = require('./vue_interaction_host.cjs');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const repo = path.join(__dirname, '..');
const id = '11111111-1111-4111-8111-111111111111';
const approved = extra => ({ schemaVersion: 1, approvalId: id, toolId: 'verified-fixture-tool',
  version: '1.0', platform: 'darwin-arm64', packageDigest: `sha256:${'a'.repeat(64)}`,
  runtimeAdapter: 'static-parser', runtimeDigest: 'b'.repeat(64), approvalHash: 'c'.repeat(64),
  status: 'approved_inventory_only', executionAllowed: false, preparationFencePresent: false, ...extra });
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
async function flush() { await new Promise(resolve => setImmediate(resolve)); for (let i = 0; i < 4; i++) await vue.nextTick(); }
function decodeApprovalFrame(frame) {
  assert.ok(frame instanceof Uint8Array, 'approval uses a raw Uint8Array IPC body');
  assert.ok(frame.byteLength >= 20);
  assert.equal(Buffer.from(frame.subarray(0, 8)).toString('ascii'), 'OVTA0001');
  const header = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
  const lengths = [header.getUint32(8, true), header.getUint32(12, true), header.getUint32(16, true)];
  assert.ok(lengths[0] > 0 && lengths[0] <= 16 * 1024);
  assert.ok(lengths[1] > 0 && lengths[1] <= 16 * 1024);
  assert.ok(lengths[2] > 0 && lengths[2] <= 64 * 1024 * 1024);
  assert.equal(frame.byteLength, 20 + lengths.reduce((sum, n) => sum + n, 0), 'no trailing or omitted bytes');
  let offset = 20;
  return lengths.map(n => { const segment = frame.subarray(offset, offset + n); offset += n; return segment; });
}
async function mount(t, invoke) {
  const calls = [];
  const api = compile(fs.readFileSync(path.join(repo, 'src/features/environment/toolApprovalApi.ts'), 'utf8'), {
    '@tauri-apps/api/core': { invoke: async (command, args) => { if (command === 'approve_tool_inventory') decodeApprovalFrame(args); calls.push({ command, args }); return invoke(command, args); } },
  });
  const filename = path.join(repo, 'src/features/environment/ToolApprovalPanel.vue');
  const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
  const script = compileScript(descriptor, { id: 'tool-approval-interaction' });
  const template = compileTemplate({ source: descriptor.template.content, filename, id: 'tool-approval-interaction',
    compilerOptions: { bindingMetadata: script.bindings } });
  assert.deepEqual(template.errors, []);
  const component = compile(script.content, { './toolApprovalApi': api, '../../i18n': { useI18n: () => ({ tr: zh => zh }) } });
  const render = compile(template.code, {});
  const { root } = mountHost(t, { ...component.default, render: render.render });
  await flush();
  return { calls, root, text: () => text(root),
    async select(index, file) { const input = descendants(root, 'input').filter(n => n.props.type === 'file')[index];
      assert.ok(input, 'actual rendered file input exists'); await input.props.onChange({ target: { files: [file], value: 'owned-fixture' } }); await flush(); },
    button(label) { const found = descendants(root, 'button').find(n => text(n) === label); assert.ok(found, `actual button: ${label}`); return found; },
    async query(value) { const input = descendants(root, 'input').find(n => n.props.type !== 'file'); assert.ok(input);
      input.value = value; input.dispatchEvent({ type: 'input' }); await flush(); await this.button('读取原库存').props.onClick(); await flush(); },
  };
}
async function selectApproval(h) {
  await h.select(0, new File(['{"signed":"fixture-admin-decision"}'], 'decision.json'));
  await h.select(1, new File([JSON.stringify({ payload: { id: 'unverified-preview-tool', version: '1.0',
    platform: 'darwin-arm64', digest: 'UNVERIFIED_FILE_DIGEST', entrypoint: 'bin/parser', networkPolicy: 'none' } })], 'manifest.json'));
  await h.select(2, new File(['owned fixed fixture package bytes'], 'package.bin'));
  assert.match(h.text(), /未验证的清单预览/); assert.match(h.text(), /UNVERIFIED_FILE_DIGEST/);
}

test('actual approval panel refuses oversized files before reading or invoking', async t => {
  const h = await mount(t, () => { throw new Error('oversize must not invoke'); });
  let reads = 0;
  for (const [index, size, name] of [[0, 16 * 1024 + 1, 'large-decision.json'], [1, 16 * 1024 + 1, 'large-manifest.json'], [2, 64 * 1024 * 1024 + 1, 'large-package.bin'], [3, 16 * 1024 + 1, 'large-revoke.json']]) {
    // A metadata-only oversized fixture avoids allocating a 64-MiB test Blob.
    await h.select(index, { name, size, arrayBuffer() { reads++; throw new Error('must not read oversized file'); } });
    assert.match(h.text(), /文件为空或超过大小上限/);
    assert.equal(h.calls.length, 0); assert.equal(reads, 0);
  }
  assert.equal(h.button('验证并保存批准库存').props.disabled, true);
});
test('actual approval panel busy guard prevents duplicate native calls while submission waits', async t => {
  const pending = deferred(); const h = await mount(t, command => { assert.equal(command, 'approve_tool_inventory'); return pending.promise; });
  await selectApproval(h); const button = h.button('验证并保存批准库存'); assert.equal(button.props.disabled, false);
  const first = button.props.onClick(); await flush(); assert.equal(h.calls.length, 1);
  assert.equal(button.props.disabled, true); assert.ok(descendants(h.root, 'fieldset').every(n => n.props.disabled === true));
  // Deliver a second handler event despite disabled DOM to verify code-level fencing.
  await button.props.onClick(); await flush(); assert.equal(h.calls.length, 1);
  const [decision, manifest, packageBytes] = decodeApprovalFrame(h.calls[0].args);
  assert.deepEqual(JSON.parse(Buffer.from(decision).toString('utf8')), { signed: 'fixture-admin-decision' });
  assert.equal(JSON.parse(Buffer.from(manifest).toString('utf8')).payload.id, 'unverified-preview-tool');
  assert.equal(Buffer.from(packageBytes).toString('utf8'), 'owned fixed fixture package bytes');
  pending.resolve(approved()); await first; await flush();
  assert.match(h.text(), /批准库存已保存；仍不可执行/); assert.match(h.text(), /executionAllowed false/);
  assert.equal(h.calls.length, 1);
});
test('actual original inventory query displays durable preparation fence without execution permission', async t => {
  const h = await mount(t, (command, args) => { assert.equal(command, 'read_tool_approval_inventory'); assert.deepEqual(args, { approvalId: id }); return approved({ preparationFencePresent: true }); });
  await h.query(id);
  assert.equal(h.calls.length, 1);
  assert.match(h.text(), /批准\/撤销记录已保存；环境准备门禁尚未解除，请先检查准备状态/);
  assert.match(h.text(), /后端验证的原批准库存 · 不可执行/); assert.match(h.text(), /executionAllowed false/);
  assert.doesNotMatch(h.text(), /已可运行|已可执行/);
});
test('actual saved-but-incomplete approval error is retained and never rendered as success', async t => {
  const raw = `tool_administrator_receipt_saved_preparation_incomplete:${id}:owned-fence-remains`;
  const h = await mount(t, command => { assert.equal(command, 'approve_tool_inventory'); throw raw; });
  await selectApproval(h); await h.button('验证并保存批准库存').props.onClick(); await flush();
  assert.equal(h.calls.length, 1); assert.ok(h.text().includes(raw));
  assert.match(h.text(), /批准收据已保存，但环境准备阶段未完成/);
  assert.doesNotMatch(h.text(), /签名与摘要经后端验证，批准库存已保存|后端验证的原批准库存/);
  const query = descendants(h.root, 'input').find(n => n.props.type !== 'file'); assert.equal(query.value, id);
});
test('actual failed revocation removes previously displayed approved inventory and preserves original error', async t => {
  const raw = 'tool_administrator_revocation_saved_preparation_incomplete:owned-fence-remains';
  const h = await mount(t, command => { if (command === 'approve_tool_inventory') return approved(); assert.equal(command, 'revoke_tool_inventory'); throw raw; });
  await selectApproval(h); await h.button('验证并保存批准库存').props.onClick(); await flush();
  assert.match(h.text(), /verified-fixture-tool/); assert.match(h.text(), /后端验证的原批准库存/);
  await h.select(3, new File(['{"signed":"fixture-admin-revocation"}'], 'revoke.json'));
  await h.button('验证并保存撤销').props.onClick(); await flush();
  assert.deepEqual(h.calls.map(c => c.command), ['approve_tool_inventory', 'revoke_tool_inventory']);
  assert.ok(h.text().includes(raw)); assert.match(h.text(), /撤销收据已保存，但环境准备阶段未完成/);
  assert.doesNotMatch(h.text(), /verified-fixture-tool|后端验证的原批准库存|原签名撤销收据已保存；未授予/);
});

test('actual approval API frames exact subarray bytes and rejects empty or oversized segments before IPC', async () => {
  const calls = [];
  const api = compile(fs.readFileSync(path.join(repo, 'src/features/environment/toolApprovalApi.ts'), 'utf8'), {
    '@tauri-apps/api/core': { invoke: async (command, body) => { calls.push({ command, body }); return approved(); } },
  }).toolApprovalApi;
  const d = new Uint8Array([99, 1, 2, 99]).subarray(1, 3);
  const m = new Uint8Array([98, 3, 4, 5, 98]).subarray(1, 4);
  const p = new Uint8Array([97, 6, 7, 8, 9, 97]).subarray(1, 5);
  await api.approve(d, m, p);
  assert.equal(calls.length, 1); assert.equal(calls[0].command, 'approve_tool_inventory');
  const decoded = decodeApprovalFrame(calls[0].body);
  assert.deepEqual(decoded.map(bytes => [...bytes]), [[1, 2], [3, 4, 5], [6, 7, 8, 9]]);
  for (const args of [[new Uint8Array(), m, p], [d, new Uint8Array(), p], [d, m, new Uint8Array()],
    [new Uint8Array(16 * 1024 + 1), m, p], [d, new Uint8Array(16 * 1024 + 1), p], [d, m, new Uint8Array(64 * 1024 * 1024 + 1)]]) {
    await assert.rejects(api.approve(...args), /文件为空或超出/);
    assert.equal(calls.length, 1);
  }
});
