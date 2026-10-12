// Shared real SFC setup/render fixture; transport and host nodes only are mocked.
// Execute the actual SFC setup in Vue's renderer; only transport and host nodes are mocked.
const assert = require('node:assert/strict');
const { loadWorkspaceModule } = require('./result_component_harness.cjs');

const fs = require('node:fs');

const path = require('node:path');

const test = require('node:test');

const ts = require('typescript');

const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');

const vue = require('vue');

const { renderToString } = require('@vue/server-renderer');

const filename = path.join(__dirname, '../src/features/sentinel/components/AgentDialog.vue');

const { descriptor } = parse(fs.readFileSync(filename, 'utf8'), { filename });

const compiledScript = compileScript(descriptor, { id: 'dialog-test' });

const compiled = ts.transpileModule(compiledScript.content, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

const tasksFilename = path.join(__dirname, '../src/features/sentinel/composables/useAgentDialogTasks.ts');
const taskPageContract = {};
new Function('exports', ts.transpileModule(fs.readFileSync(path.join(__dirname,
  '../src/features/sentinel/results/taskPageContract.ts'), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText)(taskPageContract);

const compiledTasks = ts.transpileModule(fs.readFileSync(tasksFilename, 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

const statusFilename = path.join(__dirname, '../src/features/sentinel/composables/useAgentDialogStatus.ts');
const eventContractExports = {};
new Function('exports', ts.transpileModule(fs.readFileSync(path.join(__dirname,
  '../src/features/sentinel/timeline/eventContract.ts'), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText)(eventContractExports);

const compiledStatus = ts.transpileModule(fs.readFileSync(statusFilename, 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

const readingFilename = path.join(__dirname, '../src/features/sentinel/composables/useAgentDialogReading.ts');

const compiledReading = ts.transpileModule(fs.readFileSync(readingFilename, 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

const historyFilename = path.join(__dirname, '../src/features/sentinel/composables/useAgentDialogHistory.ts');
const compiledHistory = ts.transpileModule(fs.readFileSync(historyFilename, 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

const projectionExports = {};
new Function('require', 'exports', ts.transpileModule(fs.readFileSync(path.join(__dirname,
  '../src/features/sentinel/timeline/projectionContract.ts'), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText)(name => name === './rootDecisionContract'
  ? loadWorkspaceModule('src/features/sentinel/timeline/rootDecisionContract.ts') : require(name), projectionExports);

const statusContractExports = {};
new Function('require', 'exports', ts.transpileModule(fs.readFileSync(path.join(__dirname,
  '../src/features/sentinel/timeline/statusContract.ts'), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText)((name) => {
  if (name === './projectionContract') return projectionExports;
  throw new Error(`Unexpected status contract import: ${name}`);
}, statusContractExports);

const orderedPlanExports = {};
new Function('require', 'exports', ts.transpileModule(fs.readFileSync(path.join(__dirname,
  '../src/features/sentinel/directives/orderedAssessmentPlan.ts'), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText)(require, orderedPlanExports);

const directivesFilename = path.join(__dirname, '../src/features/sentinel/composables/useAgentDialogDirectives.ts');

const compiledDirectives = ts.transpileModule(fs.readFileSync(directivesFilename, 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

const followupFilename = path.join(__dirname, '../src/features/sentinel/composables/useAgentDialogFollowup.ts');

const compiledFollowup = ts.transpileModule(fs.readFileSync(followupFilename, 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

const labelsFilename = path.join(__dirname, '../src/features/sentinel/components/agentDialogLabels.ts');

const compiledLabels = ts.transpileModule(fs.readFileSync(labelsFilename, 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;

const compiledTemplate = compileTemplate({
  source: descriptor.template.content, filename, id: 'dialog-test',
  compilerOptions: { bindingMetadata: compiledScript.bindings },
});

assert.deepEqual(compiledTemplate.errors, []);

const templateExports = {};

new Function('require', 'exports', ts.transpileModule(compiledTemplate.code, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText)(require, templateExports);

const renderDialogTree = (bindings) => templateExports.render({}, [], {}, vue.proxyRefs(bindings), {}, {});
const renderDialog = (bindings) => renderToString(vue.createSSRApp({
  render() { return renderDialogTree(bindings); },
}));

const renderer = vue.createRenderer({
  createElement: () => ({}), createText: () => ({}), createComment: () => ({}),
  insert() {}, remove() {}, setText() {}, setElementText() {}, patchProp() {},
  parentNode: () => null, nextSibling: () => null,
});

const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};

const flush = async () => { for (let i = 0; i < 8; i++) await vue.nextTick(); };

function eventRefreshClock() {
  const timers = new Map();
  let next = 0;
  return {
    timers,
    setTimeout(callback, delay) { const id = ++next; timers.set(id, { callback, delay }); return id; },
    clearTimeout(id) { timers.delete(id); },
    async tick(delay = 50) {
      for (const [id, timer] of [...timers]) {
        if (timer.delay !== delay || !timers.delete(id)) continue;
        timer.callback();
      }
      await flush();
    },
  };
}

const scan = (id, projectId = 1) => ({ id, projectId, updatedAt: id, status: 'completed' });

const status = (id) => ({ scanId: id, status: 'completed', attemptNumber: 1,
  latestSequence: 0, isIncremental: false, timelineBeforeSequence: 0, hasEarlierTimeline: false,
  timeline: [], directiveDrafts: [],
  stopDiagnostic: { code: 'completed_with_gaps', obligations: [] } });

const longTimeline = (count) => Array.from({ length: count }, (_, index) => ({
  id: `message-${index + 1}`, sequence: index + 1, timestamp: 'stamp',
  eventType: 'mailbox_message', fromRole: 'source_analyst', toRole: 'coordinator',
  summary: `message body ${index + 1}`, status: 'persisted',
}));

const directive = (scanId, state = 'need_confirmation') => ({
  id: `${scanId}-draft`, scanId, attemptNumber: 1, status: state, revision: 1, draftHash: 'hash',
  sourceMessageId: `${scanId}-message`, rootRunId: `${scanId}-root`, targetKey: 'https://example.test',
  recipientRole: 'coordinator', threadKey: 'team', text: 'redacted message', intent: 'analysis_focus',
  requestedRoles: [], referencedFactIds: [], requestedContracts: [], priorityChanges: [],
  estimatedTokens: 0, estimatedRequests: 0, sideEffectClass: 'read_only', requiredApprovals: [],
  validationResult: state === 'rejected' ? 'rejected' : 'confirmation_required', reasonCodes: [],
  coordinatorDecision: state === 'rejected' ? 'reject' : 'need_confirmation',
  confirmationRequired: state !== 'rejected', safeExecutionText: '', confirmedDirectiveId: '',
});

async function mount(overrides = {}, options = {}) {
  const api = {
    getSentinelScanIdentityMaterial: async () => [],
    listSentinelScans: async (projectId) => [scan('A', projectId), scan('B', projectId)],
    getNativeScanStatus: async (id) => status(id),
    getAgentDialogSelection: async (projectId) => ({ projectId: projectId ?? null, revision: 0,
      selectedScanId: null, selectedScan: null, selectionUnavailable: false }),
    saveAgentDialogSelection: async (input) => ({ projectId: input.projectId ?? null,
      revision: input.expectedRevision + 1, selectedScanId: input.scanId,
      selectedScan: { ...scan(input.scanId), projectId: input.projectId ?? null }, selectionUnavailable: false }),
    getAgentDialogTask: async (id, projectId) => ({ ...scan(id), projectId: projectId ?? null }),
    getAgentDialogView: async (scanId, attemptNumber) => ({ scanId, attemptNumber,
      revision: 0, selectedThread: '', allReadSequence: 0, threadReadSequences: {} }),
    saveAgentDialogView: async (input) => ({ scanId: input.scanId, attemptNumber: input.attemptNumber,
      revision: input.expectedRevision + 1, selectedThread: input.selectedThread,
      allReadSequence: !input.selectedThread ? (input.markReadThrough || 0) : 0,
      threadReadSequences: input.selectedThread && input.markReadThrough !== undefined
        ? { [input.selectedThread]: input.markReadThrough } : {} }),
    draftScanDirective: async (id, _text, threadKey) => ({ ...directive(id), threadKey }),
    confirmScanDirective: async () => {}, cancelScanDirective: async () => {},
    reconcileScanDirectiveReceipt: async () => {},
    ...overrides,
  };
  const tasksExports = {};
  new Function('require', 'exports', compiledTasks)((name) => {
    if (name === '../api') return { sentinelApi: api };
    if (name === '../results/taskPageContract') return taskPageContract;
    if (name === 'vue') return vue;
    throw new Error(`Unexpected task navigation import: ${name}`);
  }, tasksExports);
  const statusExports = {};
  new Function('require', 'exports', compiledStatus)((name) => {
    if (name === '../api') return { sentinelApi: api };
    if (name === 'vue') return vue;
    if (name === '@tauri-apps/api/event') return { listen: options.listen || (async () => () => {}) };
    if (name === '../timeline/projectionContract') return projectionExports;
    if (name === '../timeline/humanAssessmentContract') return loadWorkspaceModule('src/features/sentinel/timeline/humanAssessmentContract.ts');
    if (name === '../timeline/statusContract') return statusContractExports;
    if (name === '../timeline/eventContract') return eventContractExports;
    throw new Error(`Unexpected status import: ${name}`);
  }, statusExports);
  const readingExports = {};
  const historyExports = {};
  new Function('require', 'exports', compiledHistory)((name) => {
    if (name === '../api') return { sentinelApi: api };
    if (name === 'vue') return vue;
    if (name === '../timeline/projectionContract') return projectionExports;
    throw new Error(`Unexpected history import: ${name}`);
  }, historyExports);
  new Function('require', 'exports', compiledReading)((name) => {
    if (name === '../api') return { sentinelApi: api };
    if (name === 'vue') return vue;
    if (name === './useAgentDialogHistory') return historyExports;
    if (name === '../timeline/projectionContract') return projectionExports;
    if (name === '../timeline/humanAssessmentContract') return loadWorkspaceModule('src/features/sentinel/timeline/humanAssessmentContract.ts');
    throw new Error(`Unexpected reading import: ${name}`);
  }, readingExports);
  const directivesExports = {};
  new Function('require', 'exports', compiledDirectives)((name) => {
    if (name === '../api') return { sentinelApi: api };
    if (name === 'vue') return vue;
    if (name === '../directives/orderedAssessmentPlan') return orderedPlanExports;
    throw new Error(`Unexpected directives import: ${name}`);
  }, directivesExports);
  const labelsExports = {};
  new Function('require', 'exports', compiledLabels)(require, labelsExports);
  const followupExports = {};
  new Function('require', 'exports', compiledFollowup)((name) => {
    if (name === '../api') return { sentinelApi: api };
    if (name === 'vue') return vue;
    throw new Error(`Unexpected follow-up import: ${name}`);
  }, followupExports);
  const exports = {};
  new Function('require', 'exports', 'setTimeout', 'clearTimeout', compiled)((name) => {
    if (name === '../api') return { sentinelApi: api };
    if (name === '../composables/useAgentDialogTasks') return tasksExports;
    if (name === '../composables/useAgentDialogStatus') return statusExports;
    if (name === '../composables/useAgentDialogReading') return readingExports;
    if (name === '../composables/useAgentDialogDirectives') return directivesExports;
    if (name === '../composables/useAgentDialogFollowup') return followupExports;
    if (name === './agentDialogLabels') return labelsExports;
    if (name === './RootDecisionSummary.vue')
      return loadWorkspaceModule('src/features/sentinel/components/RootDecisionSummary.vue');
    if (name === './AgentIdentityMaterial.vue')
      return loadWorkspaceModule('src/features/sentinel/components/AgentIdentityMaterial.vue',
        { '../../../api': { api } });
    if (name === '../timeline/projectionContract') return projectionExports;
    if (name === '../timeline/humanAssessmentContract') return loadWorkspaceModule('src/features/sentinel/timeline/humanAssessmentContract.ts');
    if (name === '../../../i18n') return { useI18n: () => ({ tr: (zh) => zh }) };
    if (name === '@tauri-apps/api/event') return { listen: options.listen || (async () => () => {}) };
    if (name === 'vue') return vue;
    throw new Error(`Unexpected import: ${name}`);
  }, exports, options.setTimeout || setTimeout, options.clearTimeout || clearTimeout);
  let bindings;
  const originalSetup = exports.default.setup;
  const component = { ...exports.default, setup(props, context) {
    bindings = originalSetup(props, context);
    return () => null;
  } };
  const rootProps = vue.reactive({ projectId: 1, initialScanId: 'A', ...options.props });
  const events = [];
  const app = renderer.createApp({ setup: () => () => vue.h(component, { ...rootProps,
    onPrepareFollowup: (preview) => events.push(['followup', preview]),
  }) });
  app.mount({});
  try {
    await flush();
    assert.equal(bindings.scanId.value, options.expectedScanId ?? 'A');
  } catch (error) {
    app.unmount();
    throw error;
  }
  return { b: bindings, api, rootProps, events, unmount: () => app.unmount() };
}

const readingView = (scanId = 'A', attemptNumber = 1, extra = {}) => ({ scanId, attemptNumber,
  revision: 0, selectedThread: '', allReadSequence: 0, threadReadSequences: {}, ...extra });

const readingStatus = (scanId = 'A', attemptNumber = 1) => ({ ...status(scanId), attemptNumber,
  latestSequence: 3, timelineBeforeSequence: 1, timeline: [
    { id: 'one', eventType: 'mailbox_message', threadKey: 'team', sequence: 1, timestamp: '1', summary: 'one' },
    { id: 'two', eventType: 'mailbox_message', threadKey: 'assignment-A', sequence: 2, timestamp: '2', summary: 'two' },
    { id: 'three', eventType: 'mailbox_message', threadKey: 'team', sequence: 3, timestamp: '3', summary: 'three' },
  ] });

const selectionRecord = (id, revision = 1, projectId = 1) => ({ projectId, revision,
  selectedScanId: id, selectedScan: id ? { ...scan(id), projectId } : null,
  selectionUnavailable: revision > 0 && !id });

const pendingReceipt = () => ({
  sequence: 1, timestamp: '2026-09-26T00:00:00Z',
  id: 'saved-receipt', eventType: 'user_directive', fromRole: 'operator', toRole: 'coordinator',
  threadKey: 'team', summary: '分析已有证据', status: 'deferred',
  taskClosure: { disposition: 'receipt_pending', requiresReconciliation: true, automaticRetry: false },
  proposalAction: { state: 'received', summary: '<script>unsafe()</script>' },
});

const event = (id, eventType, sequence, summary = id) => ({ id, eventType, sequence, summary,
  timestamp: '2026-09-26T00:00:00Z', threadKey: 'team', fromRole: 'coordinator', toRole: 'operator' });
module.exports = { assert, test, renderDialog, renderDialogTree, deferred, flush, eventRefreshClock,
  scan, status, longTimeline, directive, mount, readingView, readingStatus,
  selectionRecord, pendingReceipt, event };
