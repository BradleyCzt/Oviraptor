const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vue = require('vue');
const { transpile } = require('./shell_module_harness.cjs');
const source = fs.readFileSync(path.join(__dirname, '../src/features/conversations/useConversationWorkspace.ts'), 'utf8');
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const flush = async () => { for (let i = 0; i < 40; i++) await vue.nextTick(); };
const key = (projectId = 1, id = 'a') => `oviraptor.conversation.steer.v1:${projectId}:${id}`;
function lockManager() {
  const held = new Set();
  return { held, async request(name, options, callback) {
    assert.deepEqual(options, { mode: 'exclusive', ifAvailable: true });
    await Promise.resolve();
    if (held.has(name)) return callback(null);
    held.add(name);
    try { return await callback({ name, mode: 'exclusive' }); }
    finally { held.delete(name); }
  } };
}
function memoryStorage() {
  const values = new Map();
  return { values, locks: lockManager(), getItem: name => values.get(name) ?? null, setItem: (name, value) => values.set(name, value), removeItem: name => values.delete(name) };
}
const project = id => ({ id, status: 'active', name: `Project ${id}` });
const conversation = (projectId = 1, id = 'a', revision = 2) => ({ id, projectId, title: 'Conversation', archived: false, revision, messageCount: 1, lastMessagePreview: 'original', createdAt: 't', updatedAt: 't' });
const reply = (status = 'running', extra = {}) => ({ id: 'original-reply', userMessageId: 'original-user', status, code: '', message: '', assistantMessageId: status === 'completed' ? 'original-answer' : null, outcomeUnknown: false, createdAt: 't', updatedAt: 't', ...extra });
const snapshot = (projectId = 1, id = 'a') => ({ conversation: conversation(projectId, id), messages: [{ id: 'original-user', conversationId: id, sequence: 1, role: 'user', content: 'original', replyToMessageId: null, createdAt: 't' }], hasMore: false, links: [], replyAvailability: { available: true, code: '', message: '', model: 'Synthetic model' }, latestReply: reply() });
const receipt = input => ({ conversation: conversation(input.projectId, input.conversationId, input.expectedRevision + 2), userMessage: { id: `user-${input.operationId}`, conversationId: input.conversationId, sequence: 3, role: 'user', content: input.content, replyToMessageId: null, createdAt: 't' }, reply: input.requestReply ? reply('completed', { id: `reply-${input.operationId}`, userMessageId: `user-${input.operationId}` }) : null });
function mount(t, options = {}) {
  const storage = options.storage === undefined ? memoryStorage() : options.storage;
  const timers = new Map(); let timerId = 0, operation = 0;
  const exports = {};
  new Function('require', 'exports', 'setTimeout', 'clearTimeout', transpile(source))(name => { assert.equal(name, 'vue'); return vue; }, exports, fn => { timers.set(++timerId, fn); return timerId; }, id => timers.delete(id));
  const selection = vue.ref(project(1));
  const active = vue.ref(true), writeBlocked = vue.ref(false);
  const data = { value: options.snapshot ?? snapshot() };
  const calls = [];
  const defaults = {
    listConversations: async input => ({ items: [conversation(input.projectId)], total: 1 }),
    readConversation: async input => input.projectId === data.value.conversation.projectId && input.conversationId === data.value.conversation.id ? structuredClone(data.value) : snapshot(input.projectId, input.conversationId),
    sendConversationMessage: async input => { const value = receipt(input); data.value = { ...data.value, conversation: value.conversation, latestReply: value.reply, messages: [...data.value.messages, value.userMessage] }; return value; },
    cancelConversationReply: async () => { data.value.latestReply = reply('cancel_requested'); return data.value.latestReply; },
  };
  const api = new Proxy({}, { get: (_, name) => (...args) => { calls.push([name, ...args]); return (options.api?.[name] ?? defaults[name])(...args); } });
  let state;
  const renderer = vue.createRenderer({ createComment: () => ({}), insert() {}, remove() {}, parentNode: () => null, nextSibling: () => null });
  const app = renderer.createApp({ setup() { state = exports.useConversationWorkspace({ project: () => selection.value, active: () => active.value, writeBlocked: () => writeBlocked.value, api, storage, locks: options.locks === undefined ? storage?.locks ?? lockManager() : options.locks, operationId: () => `steer-operation-${++operation}` }); return () => null; } });
  app.mount({}); if (t) t.after(() => app.unmount());
  return { state, data, selection, active, writeBlocked, calls, storage, timers, unmount: () => app.unmount(), async tick() { const pending = [...timers.values()]; timers.clear(); for (const fn of pending) fn(); await flush(); } };
}
const sends = m => m.calls.filter(([name]) => name === 'sendConversationMessage').map(([, input]) => input);
async function queue(m, content = 'Please make the next reply concise') {
  m.state.selectConversation('a'); await flush();
  m.state.composerDraft.value = content;
  assert.equal(m.state.canSteer.value, true);
  await m.state.queueSteer();
}
function completeOriginal(m, extra = {}) {
  m.data.value.conversation.revision = 3;
  m.data.value.latestReply = reply('completed', extra);
}

test('Steer persists one bounded follow-up without changing or resending the running request', async t => {
  const m = mount(t); await queue(m);
  assert.equal(m.state.queuedSteer.value.status, 'queued');
  assert.equal(m.state.steerDraft.value, '');
  assert.equal(m.state.steerNotice.value, '');
  assert.equal(JSON.parse(m.storage.getItem(key())).content, 'Please make the next reply concise');
  m.state.steerDraft.value = 'A duplicate click cannot replace it'; await m.state.queueSteer(); await m.tick();
  assert.equal(m.state.queuedSteer.value.content, 'Please make the next reply concise');
  assert.equal(sends(m).length, 0);
  assert.equal(m.calls.filter(([name]) => name === 'cancelConversationReply').length, 0);
});

test('successful original completion dispatches exactly once with persisted identity and fresh revision', async t => {
  const storage = memoryStorage(); let observed;
  const m = mount(t, { storage, api: { sendConversationMessage: async input => { observed = JSON.parse(storage.getItem(key())); return receipt(input); } } });
  await queue(m); completeOriginal(m); await m.state.readSelected(); await flush(); await m.state.readSelected();
  assert.equal(sends(m).length, 1);
  assert.deepEqual(observed.input, sends(m)[0]);
  assert.equal(observed.status, 'sending'); assert.equal(sends(m)[0].expectedRevision, 3);
  assert.equal(m.state.queuedSteer.value, undefined); assert.equal(storage.getItem(key()), null);
  assert.equal(m.state.steerNotice.value, 'reply_saved');
});

test('queue waits for original send IPC to settle and protects the original retry draft', async t => {
  const original = deferred(); let count = 0;
  const data = snapshot(); data.latestReply = null; data.conversation.revision = 1;
  const m = mount(t, { snapshot: data, api: { sendConversationMessage: input => ++count === 1 ? original.promise : Promise.resolve(receipt(input)) } });
  m.state.selectConversation('a'); await flush(); m.state.draft.value = 'original'; const pending = m.state.send();
  m.data.value.latestReply = reply(); m.data.value.conversation.revision = 2; await m.state.readSelected();
  assert.equal(m.state.busy.value, true); assert.equal(m.state.composerDraft.value, '');
  m.state.composerDraft.value = 'New direction'; await m.state.queueSteer();
  assert.equal(m.state.draft.value, 'original'); assert.equal(m.state.queuedSteer.value.content, 'New direction');
  completeOriginal(m); await m.state.readSelected(); await flush(); assert.equal(sends(m).length, 1);
  original.resolve(receipt(sends(m)[0])); await pending; await flush();
  assert.equal(sends(m).length, 2); assert.equal(sends(m)[1].content, 'New direction');
});

for (const status of ['cancel_requested', 'cancelled', 'failed', 'interrupted', 'unavailable']) {
  test(`original ${status} pauses accepted follow-up without auto-dispatch`, async t => {
    const m = mount(t); await queue(m); m.data.value.latestReply = reply(status); await m.state.readSelected(); await flush();
    assert.equal(m.state.queuedSteer.value.status, 'paused'); assert.equal(sends(m).length, 0);
  });
}

test('outcomeUnknown, a different anchor, and missing model each pause safely', async t => {
  for (const mode of ['unknown', 'different', 'model']) {
    const m = mount(t); await queue(m); completeOriginal(m, mode === 'unknown' ? { outcomeUnknown: true } : mode === 'different' ? { id: 'another-reply' } : {});
    if (mode === 'model') m.data.value.replyAvailability.available = false;
    await m.state.readSelected(); await flush(); assert.equal(m.state.queuedSteer.value.status, 'paused'); assert.equal(sends(m).length, 0);
  }
});

test('explicit Stop pauses before cancellation completes, even when stop fails', async t => {
  const stopping = deferred(); const m = mount(t, { api: { cancelConversationReply: () => stopping.promise } });
  await queue(m); const pending = m.state.cancelReply(); assert.equal(m.state.queuedSteer.value.status, 'paused');
  completeOriginal(m); await m.state.readSelected(); stopping.reject(Error('stop failed')); await pending; await flush();
  assert.equal(sends(m).length, 0); assert.equal(m.state.canSendSteer.value, true);
});

test('reload recovers queued text paused and requires an explicit reviewed send', async t => {
  const storage = memoryStorage(); const previous = mount(null, { storage }); await queue(previous); previous.unmount();
  const data = snapshot(); data.latestReply = reply('completed'); data.conversation.revision = 3;
  const m = mount(t, { storage, snapshot: data }); m.state.selectConversation('a'); await flush(); await m.state.readSelected();
  assert.equal(m.state.queuedSteer.value.status, 'paused'); assert.equal(m.state.queuedSteer.value.reason, 'reopened'); assert.equal(sends(m).length, 0);
  await m.state.sendQueuedSteer(); assert.equal(sends(m).length, 1);
});

test('conversation and project navigation never auto-send a restored queue or leak scope', async t => {
  const m = mount(t); await queue(m); m.state.selectConversation('b'); await flush(); completeOriginal(m); m.state.selectConversation('a'); await flush();
  assert.equal(m.state.queuedSteer.value.status, 'paused'); assert.equal(sends(m).length, 0);
  m.selection.value = project(2); await flush(); m.state.selectConversation('a'); await flush();
  assert.equal(m.state.queuedSteer.value, undefined); assert.equal(m.state.steerDraft.value, '');
  assert.equal(JSON.parse(m.storage.getItem(key())).projectId, 1); assert.equal(sends(m).length, 0);
});

test('an old terminal read cannot dispatch after navigation', async t => {
  const delayed = deferred(); let delay = false;
  const m = mount(t, { api: { readConversation: input => delay && input.conversationId === 'a' ? delayed.promise : Promise.resolve(snapshot(input.projectId, input.conversationId)) } });
  await queue(m); delay = true; const read = m.state.readSelected(); m.state.selectConversation('b'); await flush();
  const finished = snapshot(); finished.latestReply = reply('completed'); delayed.resolve(finished); await read; await flush();
  assert.equal(m.state.selectedId.value, 'b'); assert.equal(sends(m).length, 0);
});

test('write failure preserves text and never reports accepted or calls the model', async t => {
  const storage = memoryStorage(); storage.setItem = () => { throw Error('quota exceeded'); };
  const m = mount(t, { storage }); m.state.selectConversation('a'); await flush(); m.state.steerDraft.value = 'keep me'; await m.state.queueSteer();
  assert.equal(m.state.steerDraft.value, 'keep me'); assert.equal(m.state.queuedSteer.value, undefined); assert.match(m.state.actionError.value, /quota/); assert.equal(sends(m).length, 0);
});

test('failure persisting dispatch prevents IPC and keeps the accepted text paused', async t => {
  const m = mount(t); await queue(m); m.storage.setItem = () => { throw Error('disk full'); }; completeOriginal(m); await m.state.readSelected(); await flush();
  assert.equal(sends(m).length, 0); assert.equal(m.state.queuedSteer.value.status, 'paused'); assert.match(m.state.actionError.value, /disk full/);
});

test('missing storage, oversized UTF-8, and NUL text cannot be queued', async t => {
  const m = mount(t, { storage: null }); m.state.selectConversation('a'); await flush(); m.state.steerDraft.value = 'unsaved'; await m.state.queueSteer();
  assert.equal(m.state.queuedSteer.value, undefined); assert.equal(m.state.steerDraft.value, 'unsaved'); assert.match(m.state.actionError.value, /local draft storage/);
  for (const content of ['字'.repeat(6000), 'bad\0text']) { m.state.steerDraft.value = content; assert.equal(m.state.canSteer.value, false); await m.state.queueSteer(); }
  assert.equal(sends(m).length, 0);
});

test('uncertain send preserves exact payload across reload and only explicit retry uses its receipt', async t => {
  const storage = memoryStorage(); const previous = mount(null, { storage, api: { sendConversationMessage: async () => { throw Error('connection lost'); } } });
  await queue(previous); completeOriginal(previous); await previous.state.readSelected(); await flush();
  const originalInput = sends(previous)[0]; assert.equal(previous.state.queuedSteer.value.status, 'uncertain');
  await previous.state.discardQueuedSteer(); assert.ok(previous.state.queuedSteer.value); await previous.state.readSelected(); assert.equal(sends(previous).length, 1); previous.unmount();
  const data = snapshot(); data.latestReply = reply('failed', { outcomeUnknown: true }); data.conversation.revision = 20;
  const m = mount(t, { storage, snapshot: data }); m.state.selectConversation('a'); await flush(); assert.equal(sends(m).length, 0);
  assert.equal(m.state.queuedSteer.value.status, 'uncertain'); await m.state.sendQueuedSteer(); assert.deepEqual(sends(m), [originalInput]);
});

test('pre-save conflict pauses and a later explicit retry may use the new revision', async t => {
  let count = 0; const m = mount(t, { api: { sendConversationMessage: async input => { if (++count === 1) throw 'conversation_revision_conflict'; return receipt(input); } } });
  await queue(m); completeOriginal(m); await m.state.readSelected(); await flush();
  assert.equal(m.state.queuedSteer.value.status, 'paused'); assert.equal(m.state.queuedSteer.value.input, undefined);
  m.data.value.conversation.revision = 8; await m.state.readSelected(); assert.equal(sends(m).length, 1); await m.state.sendQueuedSteer(); assert.deepEqual(sends(m).map(input => input.expectedRevision), [3, 8]);
});

test('an explicit paused send without a model remains save-only', async t => {
  const m = mount(t); await queue(m); completeOriginal(m); m.data.value.replyAvailability.available = false; await m.state.readSelected(); await flush();
  assert.equal(sends(m).length, 0); await m.state.sendQueuedSteer(); assert.equal(sends(m)[0].requestReply, false); assert.equal(m.state.steerNotice.value, 'saved_only');
});

test('return-to-composer is allowed only before dispatch and preserves its text', async t => {
  const m = mount(t); await queue(m, 'Edit this direction'); await m.state.discardQueuedSteer();
  assert.equal(m.state.queuedSteer.value, undefined); assert.equal(m.state.composerDraft.value, 'Edit this direction'); assert.equal(m.storage.getItem(key()), null); assert.equal(sends(m).length, 0);
});

test('malformed and cross-project persisted input is never sent', async t => {
  for (const mutation of [value => { value.projectId = 2; }, value => { value.content = '字'.repeat(6000); }, value => { value.status = 'sending'; }, value => { value.input = { projectId: 2, conversationId: 'a', expectedRevision: 3, operationId: value.operationId, content: value.content, requestReply: true }; }, value => { value.input = { projectId: 1, conversationId: 'a', expectedRevision: 3, operationId: value.operationId, content: value.content, requestReply: 'true' }; }]) {
    const storage = memoryStorage(); const value = { version: 1, projectId: 1, conversationId: 'a', operationId: 'op', replyId: 'original-reply', content: 'safe', status: 'queued', reason: 'next_reply' }; mutation(value); storage.setItem(key(), JSON.stringify(value));
    const m = mount(t, { storage }); m.state.selectConversation('a'); await flush(); await m.state.sendQueuedSteer();
    assert.equal(sends(m).length, 0); assert.equal(m.state.queuedSteer.value, undefined); assert.ok(m.state.actionError.value);
  }
});

test('late follow-up result cannot clear a new conversation or its composer', async t => {
  const sending = deferred(); const m = mount(t, { api: { sendConversationMessage: () => sending.promise } });
  await queue(m); completeOriginal(m); await m.state.readSelected(); await flush(); const input = sends(m)[0];
  m.selection.value = project(2); await flush(); m.state.selectConversation('a'); await flush(); m.state.steerDraft.value = 'new project draft';
  sending.resolve(receipt(input)); await flush(); assert.equal(m.state.steerDraft.value, 'new project draft'); assert.equal(m.state.snapshot.value.conversation.projectId, 2);
  assert.equal(JSON.parse(m.storage.getItem(key())).status, 'sending');
});

test('unknown original-send failure pauses Steer and retains a usable original retry path', async t => {
  const first = deferred(); let count = 0; const data = snapshot(); data.latestReply = null; data.conversation.revision = 1;
  const m = mount(t, { snapshot: data, api: { sendConversationMessage: input => ++count === 1 ? first.promise : Promise.resolve(receipt(input)) } });
  m.state.selectConversation('a'); await flush(); m.state.draft.value = 'original'; const sending = m.state.send();
  m.data.value.latestReply = reply(); m.data.value.conversation.revision = 2; await m.state.readSelected(); m.state.steerDraft.value = 'next'; await m.state.queueSteer();
  completeOriginal(m); first.reject(Error('IPC lost')); await sending; await flush();
  assert.equal(m.state.queuedSteer.value.status, 'paused'); assert.ok(m.state.pendingSend.value); assert.equal(m.state.canSend.value, true);
  await m.state.send(); assert.deepEqual(sends(m)[0], sends(m)[1]); assert.equal(sends(m).length, 2); assert.equal(m.state.queuedSteer.value.status, 'paused'); assert.equal(m.state.canSendSteer.value, true);
});

test('moving queued text back never overwrites a newer composer draft', async t => {
  const m = mount(t); await queue(m, 'queued direction'); m.state.composerDraft.value = 'a newer unsent thought';
  await m.state.discardQueuedSteer(); assert.equal(m.state.queuedSteer.value.content, 'queued direction');
  assert.equal(m.state.composerDraft.value, 'a newer unsent thought'); assert.ok(m.storage.getItem(key())); assert.match(m.state.actionError.value, /current draft/);
});

test('original uncertain retry displays the exact original payload and preserves unqueued steer text', async t => {
  const first = deferred(); let count = 0; const data = snapshot(); data.latestReply = null; data.conversation.revision = 1;
  const m = mount(t, { snapshot: data, api: { sendConversationMessage: input => ++count === 1 ? first.promise : Promise.resolve(receipt(input)) } });
  m.state.selectConversation('a'); await flush(); m.state.draft.value = 'original'; const sending = m.state.send();
  m.data.value.latestReply = reply(); m.data.value.conversation.revision = 2; await m.state.readSelected(); m.state.composerDraft.value = 'not queued yet';
  completeOriginal(m); first.reject(Error('IPC lost')); await sending; await flush();
  assert.equal(m.state.composerDraft.value, 'original'); assert.equal(m.state.steerDraft.value, 'not queued yet');
  await m.state.send(); assert.deepEqual(sends(m)[0], sends(m)[1]); assert.equal(m.state.composerDraft.value, 'not queued yet');
});

test('cross-window interleaving cannot accept two different drafts into one storage record', async t => {
  const storage = memoryStorage(); const a = mount(t, { storage }); const b = mount(t, { storage });
  a.state.selectConversation('a'); b.state.selectConversation('a'); await flush();
  a.state.steerDraft.value = 'direction A'; b.state.steerDraft.value = 'direction B';
  const originalWrite = storage.setItem; let interleaved;
  storage.setItem = (name, value) => {
    if (!interleaved) interleaved = b.state.queueSteer();
    originalWrite(name, value);
  };
  await a.state.queueSteer(); await interleaved; await flush();
  assert.equal(a.state.queuedSteer.value.content, 'direction A'); assert.equal(a.state.steerDraft.value, '');
  assert.equal(b.state.queuedSteer.value, undefined); assert.equal(b.state.steerDraft.value, 'direction B');
  assert.match(b.state.actionError.value, /busy in another window/);
  assert.equal(JSON.parse(storage.getItem(key())).content, 'direction A'); assert.equal(sends(a).length + sends(b).length, 0);
});

test('missing or contended Web Locks preserve Steer text while ordinary chat remains available', async t => {
  const missing = mount(t, { locks: null }); missing.state.selectConversation('a'); await flush(); missing.state.steerDraft.value = 'unsent direction';
  assert.equal(missing.state.steerSupported.value, false); assert.equal(missing.state.canSteer.value, false); await missing.state.queueSteer();
  assert.equal(missing.state.steerDraft.value, 'unsent direction'); assert.equal(missing.storage.getItem(key()), null); assert.match(missing.state.actionError.value, /safe local queue locking/);
  completeOriginal(missing); await missing.state.readSelected(); await missing.state.send(); assert.equal(sends(missing).length, 1);
  const m = mount(t); m.state.selectConversation('a'); await flush(); m.state.steerDraft.value = 'blocked direction'; m.storage.locks.held.add(key()); await m.state.queueSteer();
  assert.equal(m.state.steerDraft.value, 'blocked direction'); assert.equal(m.state.queuedSteer.value, undefined); assert.equal(sends(m).length, 0);
});

test('all Steer storage mutations hold the conversation-scoped exclusive lock', async t => {
  const storage = memoryStorage(); const set = storage.setItem, remove = storage.removeItem;
  storage.setItem = (name, value) => { assert.ok(storage.locks.held.has(name), 'write must hold scope lock'); return set(name, value); };
  storage.removeItem = name => { assert.ok(storage.locks.held.has(name), 'remove must hold scope lock'); return remove(name); };
  const m = mount(t, { storage }); await queue(m); await m.state.discardQueuedSteer(); await m.state.queueSteer();
  m.state.selectConversation('b'); await flush(); m.state.selectConversation('a'); await flush(); completeOriginal(m); await m.state.readSelected(); await m.state.sendQueuedSteer();
  assert.equal(sends(m).length, 1); assert.equal(m.state.steerNotice.value, 'reply_saved');
});

test('model loss while original reply is active pauses permission permanently until reviewed', async t => {
  const m = mount(t); await queue(m); m.data.value.replyAvailability.available = false; await m.state.readSelected(); await flush();
  assert.equal(m.state.queuedSteer.value.status, 'paused'); assert.equal(m.state.queuedSteer.value.reason, 'model_unavailable');
  m.data.value.replyAvailability.available = true; completeOriginal(m); await m.state.readSelected(); await flush(); assert.equal(sends(m).length, 0);
});

test('automatic permission is rechecked after asynchronous lock admission', async t => {
  const gate = deferred(); const underlying = lockManager(); let delayNext = false;
  const locks = { async request(...args) { if (delayNext) { delayNext = false; await gate.promise; } return underlying.request(...args); } };
  const m = mount(t, { locks }); await queue(m); delayNext = true; completeOriginal(m); await m.state.readSelected(); await flush();
  assert.equal(m.state.steerSaving.value, true); assert.equal(sends(m).length, 0);
  m.data.value.replyAvailability.available = false; await m.state.readSelected(); await flush();
  assert.equal(m.state.queuedSteer.value.status, 'paused'); gate.resolve(); await flush();
  assert.equal(sends(m).length, 0); assert.equal(m.state.queuedSteer.value.status, 'paused'); assert.equal(m.state.steerSaving.value, false);
});

test('the post-lock promise gap cannot send after model availability changes', async t => {
  const underlying = lockManager(); let changeAfterWrite = false, m;
  const locks = { async request(...args) { const result = await underlying.request(...args); if (changeAfterWrite) { changeAfterWrite = false; m.state.snapshot.value.replyAvailability.available = false; } return result; } };
  m = mount(t, { locks }); await queue(m); changeAfterWrite = true; completeOriginal(m); await m.state.readSelected(); await flush();
  assert.equal(sends(m).length, 0); assert.equal(m.state.queuedSteer.value.status, 'uncertain'); assert.equal(m.state.queuedSteer.value.reason, 'changed');
  assert.equal(m.state.queuedSteer.value.input.requestReply, true); assert.deepEqual(JSON.parse(m.storage.getItem(key())).input, m.state.queuedSteer.value.input);
});


test('hiding Chat pauses Steer and polling; reactivation only reads saved state', async t => {
 const m=mount(t);await queue(m);m.active.value=false;await flush();assert.equal(m.state.queuedSteer.value.status,'paused');assert.equal(m.timers.size,0);
 const before=m.calls.length;completeOriginal(m);await m.state.readSelected();await flush();assert.equal(m.calls.length,before);assert.equal(sends(m).length,0);
 m.active.value=true;await flush();assert.equal(m.state.snapshot.value.latestReply.status,'completed');assert.equal(sends(m).length,0);assert.equal(m.state.canSendSteer.value,true);
});
test('permission writes block follow-up dispatch until the write has settled', async t => {
 const m=mount(t);await queue(m);m.writeBlocked.value=true;completeOriginal(m);await m.state.readSelected();await flush();assert.equal(sends(m).length,0);m.writeBlocked.value=false;await flush();assert.equal(sends(m).length,1);
});
