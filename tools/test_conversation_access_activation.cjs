// Mount the actual Chat workspace and AccessPanel. Reactivation/configuration
// props flow through the real Vue components; only IPC and desktop UI are faked.
const test = require('node:test');
const assert = require('node:assert/strict');
const vue = require('vue');
const { loadWorkspaceModule } = require('./result_component_harness.cjs');
const flush = async () => { for (let i = 0; i < 35; i++) await vue.nextTick(); };
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };

async function mount(t) {
  const server = { recipient: 'recipient-a', skillRevision: 'skills-a', skillEnabled: true, readAssets: true };
  const calls = [];
  let nextAccess, nextSkills, nextMutation, workspace, panel, panelMounts = 0;
  const conversation = { id: 'a', projectId: 1, title: 'Synthetic conversation', archived: false, revision: 1,
    messageCount: 0, lastMessagePreview: '', createdAt: 'synthetic', updatedAt: 'synthetic' };
  const access = () => ({ version: 1, projectId: 1, conversationId: 'a', revision: 1,
    agentEnabled: true, directorySupported: true, readAssets: server.readAssets, readScans: false,
    selectedSkillIds: [1], directories: [], providerIdentity: server.recipient,
    providerLabel: `https://${server.recipient}.invalid`, skillRevision: server.skillRevision,
    skillsRequireReview: server.skillRevision !== 'skills-a' || !server.skillEnabled,
    providerDisclosureAccepted: server.recipient === 'recipient-a' && server.skillRevision === 'skills-a' && server.skillEnabled });
  const skills = () => [{ id: 1, name: 'Synthetic skill', description: 'Saved guidance', enabled: server.skillEnabled }];
  const api = new Proxy({}, { get: (_, name) => async input => {
    calls.push([name, structuredClone(input)]);
    if (name === 'listConversations') return { items: [conversation], total: 1 };
    if (name === 'readConversation') return { conversation, messages: [], hasMore: false, links: [], toolActivity: [], latestReply: null,
      replyAvailability: { available: true, code: '', message: '', model: 'Synthetic model' } };
    if (name === 'getConversationAccess') { const hold = nextAccess; nextAccess = undefined; return hold ? hold.promise : access(); }
    if (name === 'listConversationAccessSkills') { const hold = nextSkills; nextSkills = undefined; return hold ? hold.promise : skills(); }
    if (name === 'setConversationAccess' && nextMutation) {
      const hold = nextMutation; nextMutation = undefined; server.readAssets = input.readAssets; return hold.promise;
    }
    throw Error(`Unexpected mutation or IPC: ${String(name)}`);
  } });
  const dependencies = { './api': { conversationApi: api },
    // This renderer checks component lifecycle and state. Browser QA exercises
    // native input directives and events against the same components.
    vue: { ...vue, vModelText: {}, vModelCheckbox: {}, vModelSelect: {} },
    '@tauri-apps/plugin-dialog': { open: async () => null },
    '@lucide/vue': new Proxy({}, { get: () => () => null }),
    './ConversationMessage.vue': { default: () => null },
    './ConversationToolActivity.vue': { default: () => null } };
  const accessComponent = loadWorkspaceModule('src/features/conversations/ConversationAccessPanel.vue', dependencies).default;
  dependencies['./ConversationAccessPanel.vue'] = { default: { ...accessComponent, setup(props, context) {
    panelMounts++; panel = accessComponent.setup(props, context); return panel;
  } } };
  const workspaceComponent = loadWorkspaceModule('src/features/conversations/ConversationWorkspace.vue', dependencies).default;
  const component = { ...workspaceComponent, setup(props, context) { workspace = workspaceComponent.setup(props, context); return workspace; } };
  const props = vue.reactive({ projects: [{ id: 1, name: 'Synthetic project', status: 'active' }], projectId: 1, active: true, configurationRevision: 0 });
  const renderer = vue.createRenderer({
    createElement: () => ({ style: {} }), createText: () => ({}), createComment: () => ({}),
    insert() {}, remove() {}, setText() {}, setElementText() {}, patchProp() {},
    parentNode: () => null, nextSibling: () => null, querySelector: () => ({}),
  });
  const app = renderer.createApp({ render: () => vue.h(component, props) });
  app.mount({}); t.after(() => app.unmount());
  await flush(); workspace.state.selectConversation('a'); await flush();
  return { server, props, calls, access, skills,
    get workspace() { return workspace; },
    get panel() { return panel; }, get panelMounts() { return panelMounts; },
    holdAccess() { return nextAccess = deferred(); }, holdSkills() { return nextSkills = deferred(); },
    holdMutation() { return nextMutation = deferred(); } };
}
const reads = m => m.calls.filter(([name]) => name === 'getConversationAccess');
const writes = m => m.calls.filter(([name]) => !['listConversations', 'readConversation', 'getConversationAccess', 'listConversationAccessSkills'].includes(name));

test('returning to the same Chat revalidates recipient before enabling the persistent panel', async t => {
  const m = await mount(t); const original = m.panel;
  assert.equal(m.panel.state.access.value.providerIdentity, 'recipient-a');
  m.props.active = false; await flush();
  assert.equal(m.panel.state.canRevoke.value, false);
  m.server.recipient = 'recipient-b';
  const hold = m.holdAccess(); m.props.active = true; await flush();
  m.workspace.state.draft.value = 'Synthetic question';
  assert.equal(m.panel.state.canExpand.value, false);
  assert.equal(m.panel.state.canRevoke.value, false);
  assert.equal(m.workspace.state.canSend.value, false);
  hold.resolve(m.access()); await flush();
  assert.equal(m.panel, original); assert.equal(m.panelMounts, 1);
  assert.equal(m.panel.state.access.value.providerIdentity, 'recipient-b');
  assert.equal(m.panel.state.disclosureAccepted.value, false);
  assert.equal(m.panel.state.access.value.readAssets, true);
  assert.equal(reads(m).length, 2); assert.deepEqual(writes(m), []);
});

test('a saved configuration signal refreshes an active Chat without replaying consent', async t => {
  const m = await mount(t); const original = m.panel;
  m.panel.state.disclosureAccepted.value = true;
  m.server.recipient = 'recipient-b'; m.props.configurationRevision++;
  await flush();
  assert.equal(m.panel, original); assert.equal(m.panelMounts, 1);
  assert.equal(m.panel.state.access.value.providerIdentity, 'recipient-b');
  assert.equal(m.panel.state.disclosureAccepted.value, false);
  assert.equal(reads(m).length, 2); assert.deepEqual(writes(m), []);
});

test('returning from Skills preserves saved selection while refreshing changed instructions and enabled metadata', async t => {
  const m = await mount(t);
  m.props.active = false; await flush();
  m.server.skillRevision = 'skills-b'; m.server.skillEnabled = false;
  m.props.active = true; await flush();
  assert.equal(m.panel.state.skills.value[0].enabled, false);
  assert.equal(m.panel.state.access.value.skillsRequireReview, true);
  assert.deepEqual(m.panel.state.access.value.selectedSkillIds, [1]);
  assert.equal(m.panel.state.disclosureAccepted.value, false);
  assert.equal(m.panel.state.canRevoke.value, true);
  assert.deepEqual(writes(m), []);
});

test('reactivation preserves pending directory review when recipient and skill content remain unchanged', async t => {
  const m = await mount(t);
  m.server.recipient = 'recipient-b'; m.props.configurationRevision++; await flush();
  m.panel.state.disclosureAccepted.value = true;
  m.panel.state.stagedDirectory.value = '/synthetic/pending-directory';
  m.props.active = false; await flush();
  m.props.active = true; await flush();
  assert.equal(m.panel.state.disclosureAccepted.value, true);
  assert.equal(m.panel.state.stagedDirectory.value, '/synthetic/pending-directory');
  assert.deepEqual(writes(m), []);
});

test('permission and skill reads from the prior activation cannot overwrite the current lease', async t => {
  const m = await mount(t);
  const oldAccess = m.holdAccess(), oldSkills = m.holdSkills();
  m.props.configurationRevision++; await flush();
  const oldSnapshot = m.access(), oldCatalog = m.skills();
  m.props.active = false; await flush();
  m.server.recipient = 'recipient-b'; m.server.skillRevision = 'skills-b'; m.server.skillEnabled = false;
  m.props.active = true; await flush();
  oldAccess.resolve(oldSnapshot); oldSkills.resolve(oldCatalog); await flush();
  assert.equal(m.panel.state.access.value.providerIdentity, 'recipient-b');
  assert.equal(m.panel.state.skills.value[0].enabled, false);
  assert.equal(m.panel.state.disclosureAccepted.value, false);
  assert.deepEqual(writes(m), []);
});

test('failed reactivation leaves saved permissions inoperable until an explicit successful read', async t => {
  const m = await mount(t);
  m.props.active = false; await flush();
  const hold = m.holdAccess(); m.props.active = true; await flush();
  hold.reject(Error('Synthetic access read failed')); await flush();
  assert.equal(m.panel.state.canExpand.value, false); assert.equal(m.panel.state.canRevoke.value, false);
  await m.panel.state.setTools({ readAssets: false });
  assert.deepEqual(writes(m), []);
  await m.panel.state.refresh();
  assert.equal(m.panel.state.canRevoke.value, true);
});

test('an old permission mutation response cannot replace a newer recipient refresh', async t => {
  const m = await mount(t), hold = m.holdMutation();
  const oldResponse = { ...m.access(), revision: 2, readAssets: false };
  const saving = m.panel.state.setTools({ readAssets: false });
  m.server.recipient = 'recipient-b'; m.props.configurationRevision++; await flush();
  assert.equal(m.panel.state.access.value.providerIdentity, 'recipient-b');
  hold.resolve(oldResponse); await saving; await flush();
  assert.equal(m.panel.state.access.value.providerIdentity, 'recipient-b');
  assert.equal(m.panel.state.disclosureAccepted.value, false);
  assert.equal(m.panel.state.access.value.readAssets, false);
  assert.equal(m.panel.state.saving.value, false);
  assert.equal(writes(m).length, 1);
});
