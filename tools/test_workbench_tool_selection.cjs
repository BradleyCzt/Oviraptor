// Real SFC, creation composable and API modules; only native IPC is simulated.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { compile, text, descendants, mountHost } = require('./vue_interaction_host.cjs');
const { loadSentinelApi } = require('./sentinel_api_harness.cjs');
const repo = path.join(__dirname, '..');
const id = '11111111-1111-4111-8111-111111111111';
const second = '22222222-2222-4222-8222-222222222222';
const readFile = file => fs.readFileSync(path.join(repo, file), 'utf8');
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
async function flush() { await new Promise(r => setImmediate(r)); for (let i = 0; i < 4; i++) await vue.nextTick(); }
function modules(invoke) {
  const native = { invoke };
  const api = loadSentinelApi(invoke);
  const creation = compile(readFile('src/features/sentinel/workbench/useWorkbenchTaskCreation.ts'), { '../api': { sentinelApi: api } });
  const approval = compile(readFile('src/features/environment/toolApprovalApi.ts'), { '@tauri-apps/api/core': native });
  return { api, creation, approval };
}
async function mount(t, invoke) {
  const calls = [];
  const m = modules(async (command, args) => { calls.push({ command, args }); return invoke(command, args); });
  const filename = path.join(repo, 'src/features/sentinel/workbench/WorkbenchToolSelection.vue');
  const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
  const script = compileScript(descriptor, { id: 'workbench-tool-interaction' });
  const template = compileTemplate({ source: descriptor.template.content, filename, id: 'workbench-tool-interaction', compilerOptions: { bindingMetadata: script.bindings } });
  assert.deepEqual(template.errors, []);
  const component = compile(script.content, { '../../../i18n': { useI18n: () => ({ tr: zh => zh }) }, '../../environment/toolApprovalApi': m.approval, './useWorkbenchTaskCreation': m.creation });
  component.default.render = compile(template.code, {}).render;
  const state = vue.reactive({ ids: [], disabled: false, eligible: true });
  const { root } = mountHost(t, { render() { return vue.h(component.default, { modelValue: state.ids, disabled: state.disabled, eligible: state.eligible, 'onUpdate:modelValue': ids => { state.ids = ids; } }); } });
  await flush();
  return { calls, state, text: () => text(root), button: () => descendants(root, 'button')[0],
    async edit(value) { const input = descendants(root, 'textarea')[0]; input.value = value; input.props.onInput({ target: input }); await flush(); },
    async read() { await this.button().props.onClick(); await flush(); }, root };
}
function approved(approvalId = id) { return { schemaVersion: 1, approvalId, toolId: 'actual-api-fixture', version: '1.2', platform: 'darwin-arm64', packageDigest: 'digest', runtimeAdapter: 'static-parser', runtimeDigest: 'runtime', approvalHash: 'hash', status: 'approved_inventory_only', executionAllowed: false, preparationFencePresent: true }; }
test('actual tool selector retains invalid and duplicate input, enforces 16 and permits explicit empty', async t => {
  const h = await mount(t, () => { throw new Error('validation must not invoke'); });
  for (const [value, pattern] of [[`${id}\n${id}`, /重复/], ['INVALID', /小写完整 UUID/], [Array.from({ length: 17 }, (_, n) => `${n.toString(16).padStart(8, '0')}-1111-4111-8111-111111111111`).join('\n'), /最多 16/]]) {
    await h.edit(value); assert.match(h.text(), pattern); assert.equal(h.button().props.disabled, true);
    assert.deepEqual(h.state.ids, value.split('\n')); await h.read(); assert.equal(h.calls.length, 0);
  }
  await h.edit(''); assert.deepEqual(h.state.ids, []); assert.equal(h.button().props.disabled, true);
  await h.edit(Array.from({ length: 16 }, (_, n) => `${n.toString(16).padStart(8, '0')}-1111-4111-8111-111111111111`).join('\n'));
  assert.equal(h.button().props.disabled, false); assert.doesNotMatch(h.text(), /最多 16 个 \/ At most/);
});
test('actual inventory API null and revoked errors never render approved preview', async t => {
  let response = null; const h = await mount(t, () => { if (response instanceof Error) throw response; return response; });
  await h.edit(id); await h.read(); assert.match(h.text(), /未找到原批准记录/); assert.equal(descendants(h.root, 'li').length, 0);
  response = new Error('tool_approval_original_revoked'); await h.read(); assert.match(h.text(), /tool_approval_original_revoked/); assert.equal(descendants(h.root, 'li').length, 0);
  response = approved(); await h.read(); assert.match(h.text(), /actual-api-fixture.*1\.2/); assert.match(h.text(), /准备门禁尚未解除/); assert.match(h.text(), /不可执行/);
  assert.deepEqual(h.calls.map(c => c.command), Array(3).fill('read_tool_approval_inventory'));
  assert.ok(h.calls.every(c => c.args.approvalId === id));
});
test('actual vmodel generation and busy fence suppress stale inventory and duplicate IPC', async t => {
  const pending = deferred(); const h = await mount(t, () => pending.promise); await h.edit(id);
  const first = h.button().props.onClick(); await flush(); assert.equal(h.calls.length, 1); assert.equal(h.button().props.disabled, true);
  await h.button().props.onClick(); assert.equal(h.calls.length, 1);
  // External parent changes remain possible while the textarea is disabled.
  h.state.ids = [second]; await flush(); pending.resolve(approved()); await first; await flush();
  assert.equal(descendants(h.root, 'textarea')[0].value, second); assert.equal(descendants(h.root, 'li').length, 0);
  h.state.eligible = false; await flush(); assert.match(h.text(), /不会自动丢弃/); assert.deepEqual(h.state.ids, [second]);
});
function creationFixture(ids, options = {}) {
  const calls = []; const notices = []; const prepared = [];
  const m = modules(async (command, args) => { calls.push({ command, args }); if (!['create_sentinel_url_scan', 'create_sentinel_url_scan_with_tool_selection'].includes(command)) throw new Error(`Unexpected IPC ${command}`); return { id: 'actual-created-draft', status: 'draft' }; });
  const form = { projectId: 1, taskName: 'test', urls: 'https://example.test', closure: 'breadth', orchestrationMode: 'multi', approvedToolIds: ids, authSessionIds: [], skillIds: [], instruction: '', ...options.form };
  const context = { form, mode: vue.ref(options.mode || 'web'), busy: vue.ref(false), followup: () => options.followup, hasHandoff: () => !!options.handoff, saveClosureHandoff: async () => { throw new Error('must reject tools before handoff'); }, restoreFollowupSubmission: async () => { throw new Error('must reject tools before followup'); }, pendingFollowupInput: vue.ref(), followupSubmission: vue.ref(), authSessionScopeId: vue.ref('scope'), resetIdentitySelection() {}, generation: () => 1, isDisposed: () => false, tr: zh => zh, notify: (type, value) => notices.push({ type, value }), reload() {}, prepareScan: scan => prepared.push(scan), openScan() { throw new Error('save draft only'); } };
  return { calls, notices, prepared, form, start: m.creation.useWorkbenchTaskCreation(context).start };
}
test('actual creation composable routes empty to original API and sorted selection to new IPC', async () => {
  for (const ids of [[], [second, id]]) {
    const h = creationFixture(ids); await h.start(true); assert.equal(h.calls.length, 1); assert.equal(h.prepared.length, 1);
    assert.equal(h.calls[0].command, ids.length ? 'create_sentinel_url_scan_with_tool_selection' : 'create_sentinel_url_scan');
    assert.deepEqual(h.calls[0].args.urls, ['https://example.test']); assert.equal(h.calls[0].args.orchestrationMode, 'multi');
    if (ids.length) assert.deepEqual(h.calls[0].args.approvedIds, [id, second]); else assert.equal(h.calls[0].args.approvedIds, undefined);
    assert.deepEqual(h.form.approvedToolIds, []); assert.equal(h.notices.at(-1).type, 'success');
  }
});
test('actual creation refuses Single, Source, followup, handoff and malformed selections without IPC', async () => {
  for (const options of [{ form: { orchestrationMode: 'single' } }, { mode: 'code' }, { followup: {} }, { handoff: true }]) {
    const h = creationFixture([id], options); await h.start(true); assert.equal(h.calls.length, 0); assert.equal(h.notices.at(-1).type, 'error'); assert.deepEqual(h.form.approvedToolIds, [id]);
  }
  for (const ids of [[id, id], ['INVALID']]) { const h = creationFixture(ids); await h.start(true); assert.equal(h.calls.length, 0); assert.equal(h.notices.at(-1).type, 'error'); assert.deepEqual(h.form.approvedToolIds, ids); }
});
