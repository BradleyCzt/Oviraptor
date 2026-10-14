// Exercise the real workbench setup and template; replace only IPC and host UI.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const vue = require('vue');

const filename = path.join(__dirname, '../src/features/sentinel/components/AgentWorkbench.vue');
const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });
const script = compileScript(descriptor, { id: 'workbench-test' });
const transpile = (content) => ts.transpileModule(content, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
const compiled = transpile(script.content);
const handoffModulePath = path.join(__dirname, '../src/features/sentinel/workbench/useClosureHandoff.ts');
const compiledHandoff = transpile(fs.readFileSync(handoffModulePath, 'utf8'));
const authModulePath = path.join(__dirname, '../src/features/sentinel/workbench/useBrowserAuthSessions.ts');
const compiledAuth = transpile(fs.readFileSync(authModulePath, 'utf8'));
const skillsModulePath = path.join(__dirname, '../src/features/skills/useSkillsCatalog.ts');
const compiledSkills = transpile(fs.readFileSync(skillsModulePath, 'utf8'));
const followupModulePath = path.join(__dirname, '../src/features/sentinel/workbench/useFollowupRecovery.ts');
const compiledFollowup = transpile(fs.readFileSync(followupModulePath, 'utf8'));
const creationModulePath = path.join(__dirname, '../src/features/sentinel/workbench/useWorkbenchTaskCreation.ts');
const compiledCreation = transpile(fs.readFileSync(creationModulePath, 'utf8'));
const presentationModulePath = path.join(__dirname, '../src/features/sentinel/workbench/useWorkbenchPresentation.ts');
const compiledPresentation = transpile(fs.readFileSync(presentationModulePath, 'utf8'));
function loadPanel(name) {
  const panelFile = path.join(__dirname, '../src/features/sentinel/workbench', name);
  const { descriptor: panel } = parse(fs.readFileSync(panelFile, 'utf8'), { filename: panelFile });
  const panelScript = compileScript(panel, { id: name });
  const panelTemplate = compileTemplate({ source: panel.template.content, filename: panelFile, id: name,
    compilerOptions: { bindingMetadata: panelScript.bindings },
  });
  assert.deepEqual(panelTemplate.errors, []);
  const imports = (name) => {
    if (name === 'vue') return vue;
    if (name === '../../../i18n') return { useI18n: () => ({ tr: (zh) => zh }) };
    if (name === '../../../shared/components/InlineConfirm.vue') return { default: () => null };
    if (name === '@lucide/vue') return new Proxy({}, { get: () => () => null });
    throw new Error(`Unexpected panel import: ${name}`);
  };
  const scriptExports = {}, templateExports = {};
  new Function('require', 'exports', transpile(panelScript.content))(imports, scriptExports);
  new Function('require', 'exports', transpile(panelTemplate.code))(imports, templateExports);
  return { ...scriptExports.default, render: templateExports.render };
}
const browserAuthPanel = loadPanel('WorkbenchBrowserAuth.vue');
const noticesPanel = loadPanel('WorkbenchTaskNotices.vue');
const webPolicyPanel = loadPanel('WorkbenchWebPolicy.vue');
const budgetPanel = loadPanel('WorkbenchBudgetField.vue');
const template = compileTemplate({ source: descriptor.template.content, filename, id: 'workbench-test',
  compilerOptions: { bindingMetadata: script.bindings },
});
assert.deepEqual(template.errors, []);
const templateExports = {};
new Function('require', 'exports', transpile(template.code))(require, templateExports);
const renderer = vue.createRenderer({
  createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
  insert() {}, remove() {}, setText() {}, setElementText() {}, patchProp() {},
  parentNode: () => null, nextSibling: () => null,
});
const flush = async () => { for (let i = 0; i < 8; i++) await vue.nextTick(); };
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const draft = { id: 'draft-1', projectId: 1, status: 'draft', scanType: 'web', currentCheckpoint: '待确认' };
async function mount(overrides = {}, initialProps = {}, host = {}) {
  const calls = [], events = [];
  const api = {
    listAgentSkills: async () => [], listBrowserAuthSessions: async () => [],
    getAgentGapFollowupSubmission: async () => null,
    releaseAgentGapFollowupSubmission: async () => {},
    createSentinelUrlScan: async (...args) => { calls.push(['create', args]); return draft; },
    confirmSentinelScan: async (id) => { calls.push(['confirm', id]); return { ...draft, status: 'scanning' }; },
    ...overrides,
  };
  const exports = {};
  const handoffExports = {};
  new Function('require', 'exports', compiledHandoff)((name) => {
    if (name === 'vue') return vue;
    if (name === '../../../api') return { api };
    throw new Error(`Unexpected handoff import: ${name}`);
  }, handoffExports);
  const authExports = {};
  new Function('require', 'exports', compiledAuth)((name) => {
    if (name === 'vue') return vue;
    if (name === '../../../api') return { api };
    if (name === '@tauri-apps/api/event') return { listen: host.listen || (async () => () => {}) };
    throw new Error(`Unexpected browser auth import: ${name}`);
  }, authExports);
  const skillsExports = {};
  new Function('require', 'exports', compiledSkills)((name) => {
    if (name === 'vue') return vue;
    if (name === './catalogApi') return { skillCatalogApi: api };
    if (name === '@tauri-apps/plugin-dialog') return { open: host.open || (async () => null) };
    throw new Error(`Unexpected skills import: ${name}`);
  }, skillsExports);
  const followupExports = {};
  new Function('require', 'exports', compiledFollowup)((name) => {
    if (name === 'vue') return vue;
    if (name === '../../../api') return { api };
    throw new Error(`Unexpected followup recovery import: ${name}`);
  }, followupExports);
  const creationExports = {};
  new Function('require', 'exports', compiledCreation)((name) => {
    if (name === '../../../api') return { api };
    throw new Error(`Unexpected task creation import: ${name}`);
  }, creationExports);
  const presentationExports = {};
  new Function('require', 'exports', compiledPresentation)((name) => {
    if (name === 'vue') return vue;
    if (name === '@tauri-apps/plugin-dialog') return { open: host.open || (async () => null) };
    throw new Error(`Unexpected presentation import: ${name}`);
  }, presentationExports);
  const icon = () => null;
  new Function('require', 'exports', compiled)((name) => {
    if (name === 'vue') return vue;
    if (name === '../api') return { api };
    if (name === '../../../i18n') return { useI18n: () => ({ tr: (zh) => zh }) };
    if (name === '@lucide/vue') return new Proxy({}, { get: () => icon });
    if (name === '@tauri-apps/plugin-dialog') return { open: async () => null };
    if (name === '@tauri-apps/api/event') return { listen: async () => () => {} };
    if (name === './InlineConfirm.vue') return { default: icon };
    if (name === '../workbench/WorkbenchGreyboxSettings.vue') return { default: icon };
    if (name === '../workbench/WorkbenchCicdSettings.vue') return { default: icon };
    if (name === '../../skills/SkillInstructionsCatalog.vue') return { default: icon };
    if (name === '../workbench/WorkbenchOverview.vue') return { default: icon };
    if (name === '../workbench/WorkbenchSkillSelection.vue') return { default: icon };
    if (name === '../workbench/WorkbenchToolSelection.vue') return { default: icon };
    if (name === '../workbench/WorkbenchBrowserAuth.vue') return { default: browserAuthPanel };
    if (name === '../workbench/WorkbenchTaskNotices.vue') return { default: noticesPanel };
    if (name === '../workbench/WorkbenchWebPolicy.vue') return { default: webPolicyPanel };
    if (name === '../workbench/WorkbenchBudgetField.vue') return { default: budgetPanel };
    if (name === '../workbench/useClosureHandoff') return handoffExports;
    if (name === '../workbench/useBrowserAuthSessions') return authExports;
    if (name === '../../skills/useSkillsCatalog') return skillsExports;
    if (name === '../workbench/useFollowupRecovery') return followupExports;
    if (name === '../workbench/useWorkbenchTaskCreation') return creationExports;
    if (name === '../workbench/useWorkbenchPresentation') return presentationExports;
    throw new Error(`Unexpected import: ${name}`);
  }, exports);
  let bindings;
  const setup = exports.default.setup;
  const component = { ...exports.default, setup(props, context) {
    bindings = setup(props, context);
    return () => null;
  } };
  const props = vue.reactive({ projects: [{ id: 1, name: 'One' }, { id: 2, name: 'Two' }], scans: [], projectId: 1, initialMode: 'web', ...initialProps });
  const app = renderer.createApp({ setup: () => () => vue.h(component, {
    ...props,
    onNotify: (...args) => events.push(['notify', ...args]),
    onReload: () => events.push(['reload']),
    onOpenScan: (scan) => events.push(['open', scan]),
    onPrepareScan: (scan) => events.push(['prepare', scan]),
  }) });
  app.mount({});
  await flush();
  Object.assign(bindings.form, { taskName: 'Missing controls', urls: 'https://draft.example.test',
    authSessionId: 'identity-a', authSessionIds: ['identity-a', 'identity-b'] });
  return { b: bindings, api, calls, events, props, unmount: () => app.unmount() };
}

const followup = { sourceScanId: 'source-1', assessmentMessageId: 'assessment-1', projectId: 1,
  targetUrl: 'https://draft.example.test', rootRunId: 'root-1', assignmentId: 'assignment-1',
  candidateId: 'candidate-1', candidateRevision: 2, sourceHash: 'source-hash',
  missingEvidence: ['<script>missing</script>'], proposal: { summary: 'request controls', prerequisites: ['fresh identities'] }, targetRequestsGranted: 0 };
const handoff = { sourceScanId: 'sealed-1', closureId: 'closure-1', attemptNumber: 1,
  snapshotHash: 'a'.repeat(64), sourceHash: 'b'.repeat(64), projectId: 1,
  targetUrls: ['https://draft.example.test'], executionSettled: false, targetRequestsGranted: 0, savedHandoff: null };
const handoffReceipt = (input) => ({ requestId: input.requestId, sourceScanId: handoff.sourceScanId,
  closureId: handoff.closureId, sourceHash: handoff.sourceHash, scanId: draft.id,
  createdAt: '2026-01-01T00:00:00Z', executionGranted: false, sourceExecutionSettled: false,
  scan: { ...draft, sourcePath: '', attemptCount: 0 } });

module.exports = { mount, flush, deferred, draft, followup, handoff, handoffReceipt, templateExports, vue };
