// Actual SFC/composable + actual api.ts; only Tauri invoke/status transport is a fixture.
const { assert, test, renderDialog, deferred, flush, status, directive, mount } = require('../agent_dialog_harness.cjs');
const { loadSentinelApi } = require('../sentinel_api_harness.cjs');
const clone = value => JSON.parse(JSON.stringify(value));
function frozenDraft(scanId = 'A', extra = {}) {
  return { ...directive(scanId), intent: 'agent_proposal_request',
    requestedRoles: ['coordinator', 'deep_investigator'], ...extra };
}
function successor(item, extra = {}) {
  return { ...clone(item), id: `${item.id}-next`, sourceMessageId: `${item.sourceMessageId}-next`,
    revision: item.revision + 1, draftHash: `${item.draftHash}-next`,
    text: 'new proposal', status: 'need_confirmation', confirmationRequired: true, ...extra };
}
function decision(item, kind = 'revise', next = kind === 'revise' ? successor(item) : null) {
  return { receipt: {
    receiptId: 'human-review-receipt', sequence: 4, createdAt: '2026-10-02T00:00:00Z',
    kind, draftId: item.id, revision: item.revision, draftHash: item.draftHash,
    scanId: item.scanId, attemptNumber: item.attemptNumber, rootRunId: item.rootRunId,
    targetKey: item.targetKey, threadKey: item.threadKey, argumentHash: 'a'.repeat(64),
    reason: kind === 'reject' ? 'operator explanation' : '', directiveId: '',
    successorDraftId: next?.id ?? null, successorRevision: next?.revision ?? null,
    successorHash: next?.draftHash ?? null, requiresConfirmation: next?.confirmationRequired ?? false,
    terminal: true, executionCompleted: false,
    actions: item.requestedRoles.map((role, index) => ({ order: index + 1, role, intent: item.intent,
      reviewDisposition: 'not_queued', capabilityState: 'blocked', reasonCode: 'human_decision_terminal',
      executionState: 'not_started', directiveId: '', executionReceipt: null })),
  }, draft: next };
}
async function mountReview(handler) {
  const base = frozenDraft();
  let snapshot = { ...status('A'), directiveDrafts: [clone(base)] };
  let reads = 0;
  const calls = [];
  const api = loadSentinelApi((command, input) => {
    calls.push({ command, input: clone(input) });
    if (handler) return handler(command, input);
    if (command === 'confirm_scan_directive')
      return Promise.resolve({ id: 'queued-next', accepted: true, status: 'pending', message: 'queued' });
    if (command === 'cancel_scan_directive') return Promise.resolve();
    return Promise.resolve(decision(base, command === 'reject_scan_directive' ? 'reject' : 'revise'));
  });
  const h = await mount({
    getNativeScanStatus: async id => {
      reads++;
      return clone(snapshot.scanId === id ? snapshot : { ...status(id), directiveDrafts: [frozenDraft(id)] });
    },
    draftScanDirective: api.draftScanDirective,
    confirmScanDirective: api.confirmScanDirective, cancelScanDirective: api.cancelScanDirective,
    reviseScanDirective: api.reviseScanDirective, rejectScanDirective: api.rejectScanDirective,
  });
  reads = 0;
  let disposed = false;
  const unmount = () => { if (!disposed) { disposed = true; h.unmount(); } };
  return { ...h, unmount, base, calls, reads: () => reads, clearReads: () => { reads = 0; },
    setSnapshot: value => { snapshot = clone(value); },
    open(kind = 'revise', text = 'operator edit') {
      const item = h.b.state.value.directiveDrafts[0];
      h.b.beginReview(item, kind); h.b.reviewText.value = text;
      return item;
    } };
}
function assertPreserved(h, text = 'operator edit') {
  assert.equal(h.b.reviewingDraft.value?.id, h.base.id);
  assert.equal(h.b.reviewText.value, text);
  assert.equal(h.reads(), 0, 'an unverified response cannot cause a success refresh');
  assert.match(h.b.actionError.value, /回执无法核实|结果无法核实|判断提交结果/);
  assert.equal(h.b.sending.value, false);
  assert.equal(h.calls.length, 1, 'no automatic retry');
}
module.exports = { assert, test, renderDialog, deferred, flush, status, clone,
  frozenDraft, successor, decision, mountReview, assertPreserved };
