// Real AgentDialog setup/render regressions: timeline.
require('./agent_dialog/timeline_identity.cjs');
const { assert, test, renderDialog, deferred, flush, scan, status, longTimeline,
  directive, mount, readingView, readingStatus, selectionRecord } = require('./agent_dialog_harness.cjs');

const malformedTimelines = {
  'non-array messages': () => ({}),
  'null message': rows => [null, rows[1]],
  'empty identity': rows => [{ ...rows[0], id: ' ' }, rows[1]],
  'missing event type': rows => [{ ...rows[0], eventType: undefined }, rows[1]],
  'zero sequence': rows => [{ ...rows[0], sequence: 0 }, rows[1]],
  'fractional sequence': rows => [{ ...rows[0], sequence: 3.5 }, rows[1]],
  'unsafe sequence': rows => [{ ...rows[0], sequence: Number.MAX_SAFE_INTEGER + 1 }, rows[1]],
  'duplicate identity': rows => [rows[0], { ...rows[1], id: rows[0].id }],
  'duplicate sequence': rows => [rows[0], { ...rows[1], sequence: rows[0].sequence }],
  'invalid timestamp': rows => [{ ...rows[0], timestamp: {} }, rows[1]],
  'object summary': rows => [{ ...rows[0], summary: { secret: 'not-displayable' } }, rows[1]],
  'object thread': rows => [{ ...rows[0], threadKey: {} }, rows[1]],
  'non-array reasons': rows => [{ ...rows[0], reasonCodes: 'proposal_invalid' }, rows[1]],
  'non-string reason': rows => [{ ...rows[0], reasonCodes: [42] }, rows[1]],
};
for (const incremental of [false, true]) {
  for (const [label, corrupt] of Object.entries(malformedTimelines)) {
    test(`${incremental ? 'incremental' : 'full'} status rejects ${label} before publishing messages`, async (t) => {
      const h = await mount({ getNativeScanStatus: async () => ({ ...status('A'),
        latestSequence: 2, timelineBeforeSequence: 1, timeline: longTimeline(2) }) });
      t.after(h.unmount);
      const previous = h.b.state.value;
      const rows = longTimeline(4).slice(2);
      const response = () => ({ ...status('A'), latestSequence: 4,
        timelineBeforeSequence: 3, isIncremental: incremental, timeline: rows });
      h.api.getNativeScanStatus = async () => ({ ...response(), timeline: corrupt(rows) });
      await h.b.loadStatus(incremental ? 2 : undefined);
      assert.equal(h.b.statusError.value, '任务状态读取失败，请重试。');
      assert.equal(h.b.state.value, previous);
      assert.equal(h.b.statusSync.latestSequence.value, 2);
      assert.deepEqual(h.b.timelinePage.value.map(row => row.sequence), [1, 2]);
      h.api.getNativeScanStatus = async () => response();
      await h.b.loadStatus(incremental ? 2 : undefined);
      assert.equal(h.b.statusError.value, '');
      assert.equal(h.b.statusSync.latestSequence.value, 4);
      assert.deepEqual(h.b.timelinePage.value.map(row => row.sequence), incremental ? [1, 2, 3, 4] : [3, 4]);
    });
  }
}

test('a delta cannot reuse another cached message sequence and a later valid update still applies', async (t) => {
  const h = await mount({ getNativeScanStatus: async () => ({ ...status('A'),
    latestSequence: 2, timelineBeforeSequence: 1, timeline: longTimeline(2) }) });
  t.after(h.unmount);
  const previous = h.b.state.value;
  h.api.getNativeScanStatus = async () => ({ ...status('A'), latestSequence: 3, isIncremental: true, timelineBeforeSequence: 3,
    timeline: [{ ...longTimeline(1)[0], id: 'different-entity' }] });
  await h.b.loadStatus(2);
  assert.equal(h.b.statusError.value, '任务状态读取失败，请重试。');
  assert.equal(h.b.state.value, previous);
  h.api.getNativeScanStatus = async () => ({ ...status('A'), latestSequence: 3, isIncremental: true, timelineBeforeSequence: 3,
    timeline: [{ ...longTimeline(3)[2], id: 'message-1', summary: 'updated entity' }] });
  await h.b.loadStatus(2);
  assert.equal(h.b.statusError.value, '');
  assert.deepEqual(h.b.state.value.timeline.map(row => row.sequence), [2, 3]);
  assert.equal(h.b.state.value.timeline[1].summary, 'updated entity');
});

test('a malformed initial timeline cannot restore reading state or render object payloads', async (t) => {
  let reads = 0;
  const h = await mount({ getNativeScanStatus: async () => ({ ...status('A'), latestSequence: 1, timelineBeforeSequence: 1,
    timeline: [{ ...longTimeline(1)[0], summary: { secret: 'not-displayable' } }] }),
    getAgentDialogView: async () => { reads++; return readingView(); },
  }); t.after(h.unmount);
  assert.equal(h.b.state.value, undefined);
  assert.equal(reads, 0);
  assert.doesNotMatch(await renderDialog(h.b), /not-displayable/);
  h.api.getNativeScanStatus = async () => ({ ...status('A'), latestSequence: 1, timelineBeforeSequence: 1, timeline: longTimeline(1) });
  await h.b.loadStatus(); await flush();
  assert.equal(reads, 1);
  assert.equal(h.b.timelinePage.value.length, 1);
});

test('valid timeline envelopes preserve timestamp order, event-scoped identities and optional nulls', async (t) => {
  const rows = longTimeline(2).map((row, index) => ({ ...row, id: 'shared-id',
    eventType: index ? 'review_gate' : 'mailbox_message', threadKey: null, reasonCodes: null,
    requiredApprovals: null, timestamp: index ? 'earlier label' : 'later label' })).reverse();
  const h = await mount({ getNativeScanStatus: async () => ({ ...status('A'), latestSequence: 2, timelineBeforeSequence: 1, timeline: rows }) });
  t.after(h.unmount);
  assert.equal(h.b.statusError.value, '');
  assert.deepEqual(h.b.timelinePage.value.map(row => row.sequence), [2, 1]);
});

test('source specialists retain their own chat identities without claiming independent review', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), latestSequence: 2, timelineBeforeSequence: 1,
    stopDiagnostic: { code: 'completed_with_gaps', obligations: [] }, findingCandidateCount: 0,
    timeline: ['repo_mapper', 'source_analyst'].map((role, index) => ({
      id: `source-${index}`, sequence: index + 1, timestamp: 'stamp', eventType: 'mailbox_message',
      fromRole: role, toRole: 'coordinator', messageKind: 'source_tool_result',
      status: 'persisted', deliveryState: 'delivered', ackState: 'acknowledged',
      summary: '源码初步结果，独立复核尚未完成',
    })),
  }) });
  t.after(unmount);
  const html = await renderDialog(b);
  assert.match(html, /仓库梳理/);
  assert.match(html, /源码分析/);
  assert.match(html, /独立复核尚未完成/);
  assert.doesNotMatch(html, /<b>检查<\/b>/);
});

test('source reviewer chat preserves real sender and separates candidate review from CI closure', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), latestSequence: 1, timelineBeforeSequence: 1,
    stopDiagnostic: { code: 'completed_with_gaps', obligations: [] }, findingCandidateCount: 0,
    timeline: [{ id: 'source-review-receipt', sequence: 1, timestamp: 'stamp', eventType: 'mailbox_message',
      fromRole: 'evidence_reviewer', fromRunId: 'actual-source-reviewer', toRole: 'coordinator',
      messageKind: 'source_review_result', status: 'persisted', deliveryState: 'delivered', ackState: 'acknowledged',
      summary: '源码候选独立审查已交付；总体覆盖仍有缺口' }],
  }) });
  t.after(unmount);
  const html = await renderDialog(b);
  assert.match(html, /<b>检查<\/b>/);
  assert.match(html, /源码候选独立审查已交付/);
  assert.match(html, /不代表总体覆盖完成、漏洞已确认或 CI 通过/);
  assert.match(html, /agent-bubble reviewer/);
});

test('coverage reviewer receipt is shown without claiming sufficient coverage or CI success', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), latestSequence: 1, timelineBeforeSequence: 1,
    stopDiagnostic: { code: 'completed_with_gaps', obligations: [] }, findingCandidateCount: 0,
    timeline: [{ id: 'coverage-review-receipt', sequence: 1, timestamp: 'stamp', eventType: 'mailbox_message',
      fromRole: 'evidence_reviewer', fromRunId: 'actual-coverage-reviewer', toRole: 'coordinator',
      messageKind: 'source_coverage_review_result', status: 'persisted', deliveryState: 'delivered', ackState: 'acknowledged',
      summary: '源码覆盖独立审查已交付；仍存在证据缺口' }],
  }) });
  t.after(unmount);
  const html = await renderDialog(b);
  assert.match(html, /源码覆盖独立审查已交付/);
  assert.match(html, /审查已交付不等于覆盖充分或 CI 通过/);
  assert.match(html, /agent-bubble reviewer/);
  assert.doesNotMatch(html, /这是源码候选审查回执/);
});

test('long timelines render bounded pages without discarding loaded evidence', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({ ...status(id),
    latestSequence: 450, timelineBeforeSequence: 351, timeline: longTimeline(450).slice(-100) }) });
  t.after(unmount);
  // Renderer stress fixture, not a single IPC response or a production cache size.
  b.state.value = { ...b.state.value, timelineBeforeSequence: 1, timeline: longTimeline(450) };
  const html = await renderDialog(b);
  assert.equal((html.match(/class="agent-bubble/g) || []).length, 100);
  assert.equal(b.timelinePage.value.length, 100);
  assert.equal(b.timelinePage.value[0].sequence, 351);
  assert.equal(b.state.value.timeline.length, 450);
  assert.match(html, /351–450/);
  assert.match(html, /aria-label="聊天消息时间线"/);
  assert.match(html, /tabindex="0"/);
  b.showEarlierMessages();
  assert.equal(b.timelinePage.value[0].sequence, 251);
  b.showEarlierMessages(); b.showEarlierMessages(); b.showEarlierMessages();
  assert.equal(b.timelinePage.value[0].sequence, 1);
  b.showLaterMessages();
  assert.equal(b.timelinePage.value[0].sequence, 101);
  b.showLatestMessages();
  assert.equal(b.timelinePage.value[0].sequence, 351);
});

test('reading an earlier page is stable as new events arrive and does not mark messages read', async (t) => {
  let writes = 0;
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({ ...status(id),
    latestSequence: 450, timelineBeforeSequence: 351, timeline: longTimeline(450).slice(-100) }),
  saveAgentDialogView: async () => { writes++; throw new Error('unexpected read acknowledgement'); } });
  t.after(unmount);
  // Exercise rendering of an already populated local cache, independently of transport limits.
  b.state.value = { ...b.state.value, timelineBeforeSequence: 1, timeline: longTimeline(450) };
  b.showEarlierMessages();
  b.state.value.timeline = longTimeline(650);
  b.state.value.latestSequence = 650;
  await flush();
  assert.equal(b.timelinePage.value[0].sequence, 251);
  assert.equal(b.timelinePage.value.at(-1).sequence, 350);
  b.showLatestMessages();
  assert.equal(b.timelinePage.value[0].sequence, 551);
  assert.equal(writes, 0);
  assert.equal(b.visibleUnreadCount.value, 100, 'only the visible page is eligible for a read cursor');
});

test('timeline page anchors reset across thread, attempt and task boundaries', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({ ...status(id),
    latestSequence: 450, timelineBeforeSequence: 351, timeline: longTimeline(450).slice(-100) }) });
  t.after(unmount);
  b.state.value = { ...b.state.value, timelineBeforeSequence: 1, timeline: longTimeline(450) };
  b.showEarlierMessages();
  b.threadFilter.value = 'absent-thread';
  assert.equal(b.timelinePage.value.length, 0);
  b.threadFilter.value = '';
  assert.equal(b.timelinePage.value[0].sequence, 351);
  b.showEarlierMessages();
  b.state.value.attemptNumber = 2;
  assert.equal(b.timelinePage.value[0].sequence, 351);
  b.showEarlierMessages();
  b.scanId.value = 'B';
  await flush();
  assert.equal(b.timelinePage.value[0].sequence, 351);
});

test('source guidance displays real delivery without claiming action or review completion', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), latestSequence: 1, timelineBeforeSequence: 1,
    timeline: [{ id: 'guidance', sequence: 1, timestamp: 'stamp', eventType: 'user_directive',
      fromRole: 'operator', status: 'accepted', deliveryState: 'model_received', ackState: 'accepted', summary: '分析权限检查',
      sourceGuidance: { phase: 'assessment:source_analyst', assignmentId: 'source-job', childRunId: 'source-child',
        eventSequence: 3, inputHash: 'hash', state: 'model_received', advisoryOnly: true } }],
  }) });
  t.after(unmount);
  const html = await renderDialog(b);
  assert.match(html, /源码分析建议已送达实际子任务/);
  assert.match(html, /source-child/);
  assert.match(html, /不代表建议被采纳、工具动作完成或漏洞确认/);
  assert.doesNotMatch(html, /尚无队列调整、任务派发或动作落实回执/);
});

test('source action unsupported as guidance offers explicit next step without fake success', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), latestSequence: 1, timelineBeforeSequence: 1,
    timeline: [{ id: 'guidance-deferred', sequence: 1, timestamp: 'stamp', eventType: 'user_directive',
      fromRole: 'operator', status: 'deferred', deliveryState: 'persisted', ackState: 'deferred', summary: '暂停分析',
      reasonCodes: ['source_guidance_requires_dedicated_action'] }],
  }) });
  t.after(unmount);
  const html = await renderDialog(b);
  assert.match(html, /暂停请使用任务控制/);
  assert.doesNotMatch(html, /源码分析建议已送达实际子任务/);
});

test('addressed source focus preview explains frozen-phase routing without promising execution', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), directiveDrafts: [{ ...directive(id, 'drafted'), text: '@source_analyst 请关注鉴权', threadKey: 'team',
      coordinatorDecision: 'accept', validationResult: 'valid', intent: 'source_analysis_focus',
      requestedRoles: ['source_analyst'], priorityChanges: ['evaluate_requested_priority_change'],
      sideEffectClass: 'read_only', estimatedTokens: 150, estimatedRequests: 0,
      requiredApprovals: ['directive_confirmation'], reasonCodes: ['source_focus_next_unfrozen_role_phase'],
    }],
  }) });
  t.after(unmount);
  const html = await renderDialog(b);
  assert.match(html, /指定源码角色下一次尚未冻结的阶段/);
  assert.match(html, /含工具阶段后续轮次/);
  assert.match(html, /已经发出的请求保持不变，后续轮次仍受原预算约束/);
  assert.match(html, /不新增 Agent 或模型请求/);
  assert.match(html, /不会送达，也不会改投其他角色/);
  assert.match(html, /送达不代表采纳建议或作出裁决/);
  assert.doesNotMatch(html, /源码分析建议已送达实际子任务/);
  assert.doesNotMatch(html, /调度动作/);
  b.state.value.directiveDrafts[0].status = 'rejected';
  const rejected = await renderDialog(b);
  assert.match(rejected, /草案被拒绝，不会进入主管队列/);
  assert.doesNotMatch(rejected, /仅发送给指定源码角色下一次尚未冻结的阶段/);
});

test('general source focus previews the round boundary without a scheduling claim', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), directiveDrafts: [{ ...directive(id, 'drafted'), text: '请关注鉴权', threadKey: 'team',
      coordinatorDecision: 'accept', validationResult: 'valid', intent: 'priority_adjustment',
      requestedRoles: ['coordinator'], priorityChanges: ['evaluate_requested_priority_change'],
      sideEffectClass: 'read_only', estimatedTokens: 150, estimatedRequests: 0,
      requiredApprovals: ['directive_confirmation'], reasonCodes: ['source_guidance_tool_round_eligible'],
    }],
  }) });
  t.after(unmount);
  const html = await renderDialog(b);
  assert.match(html, /下一尚未冻结的适用阶段或工具轮次/);
  assert.match(html, /不新增模型请求、不改变权限和已发出的请求/);
  assert.doesNotMatch(html, /调度动作/);
});

for (const phase of ['review:source_candidates', 'review:source_coverage', 'tools:source_analyst:round:2']) {
  test(`${phase} guidance identifies the real recipient without claiming a verdict`, async (t) => {
    const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
      ...status(id), latestSequence: 1, timelineBeforeSequence: 1,
      timeline: [{ id: `focus-${phase}`, sequence: 1, timestamp: 'stamp', eventType: 'user_directive',
        fromRole: 'operator', status: 'completed', deliveryState: 'model_received', ackState: 'completed', summary: '请关注证据缺口',
        sourceGuidance: { phase, assignmentId: `assignment-${phase}`, childRunId: `child-${phase}`,
          eventSequence: 3, inputHash: 'frozen-focus', state: 'model_received', advisoryOnly: true },
        taskClosure: { disposition: 'analysis_guidance_delivered', requiresReconciliation: false, automaticRetry: false } }],
    }) });
    t.after(unmount);
    const html = await renderDialog(b);
    assert.ok(html.includes(phase));
    assert.ok(html.includes(`child-${phase}`));
    assert.match(html, /不代表建议被采纳、工具动作完成或漏洞确认/);
    assert.match(html, /仅完成建议送达，不代表建议被采纳或检查通过/);
    assert.doesNotMatch(html, /尚无队列调整、任务派发或动作落实回执/);
  });
}

test('source guidance closure describes completed delivery without a passed check', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), latestSequence: 1, timelineBeforeSequence: 1,
    timeline: [{ id: 'guidance-closed', sequence: 1, timestamp: 'stamp', eventType: 'user_directive',
      fromRole: 'operator', status: 'completed', deliveryState: 'model_received', ackState: 'completed', summary: '分析权限检查',
      sourceGuidance: { phase: 'assessment:source_analyst', assignmentId: 'source-job', childRunId: 'source-child',
        eventSequence: 3, inputHash: 'hash', state: 'model_received', advisoryOnly: true },
      taskClosure: { disposition: 'analysis_guidance_delivered', requiresReconciliation: false, automaticRetry: false } }],
  }) });
  t.after(unmount);
  const html = await renderDialog(b);
  assert.match(html, /分析建议的送达已核验并归档/);
  assert.match(html, /不代表建议被采纳或检查通过/);
  assert.doesNotMatch(html, /指令未取得动作落实回执/);
});

test('unverified source receipt cannot display successful delivery or archived success', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), latestSequence: 1, timelineBeforeSequence: 1,
    timeline: [{ id: 'guidance-damaged', sequence: 1, timestamp: 'stamp', eventType: 'user_directive',
      fromRole: 'operator', status: 'completed', deliveryState: 'receipt_unverified', ackState: 'completed', summary: '分析权限检查',
      sourceGuidance: null, reasonCodes: ['source_guidance_receipt_unverified'],
      taskClosure: { disposition: 'analysis_guidance_delivered', requiresReconciliation: false, automaticRetry: false } }],
  }) });
  t.after(unmount);
  const html = await renderDialog(b);
  assert.match(html, /尚不能确认已送达/);
  assert.doesNotMatch(html, /源码分析建议已送达实际子任务|分析建议的送达已核验并归档/);
});

test('missing source receipt on an accepted directive never falls back to model delivery success', async (t) => {
  const { b, unmount } = await mount({ getNativeScanStatus: async (id) => ({
    ...status(id), latestSequence: 1, timelineBeforeSequence: 1,
    timeline: [{ id: 'guidance-missing', sequence: 1, timestamp: 'stamp', eventType: 'user_directive',
      fromRole: 'operator', status: 'accepted', deliveryState: 'receipt_unverified', ackState: 'accepted', summary: '分析权限检查',
      sourceGuidance: null, reasonCodes: ['source_guidance_receipt_unverified'] }],
  }) });
  t.after(unmount);
  const html = await renderDialog(b);
  assert.match(html, /尚不能确认已送达/);
  assert.doesNotMatch(html, /源码分析建议已送达实际子任务|分析建议的送达已核验并归档|请求已送达模型/);
});

test('team chat reports pausing without claiming workers have already stopped', async (t) => {
  const h = await mount({ getNativeScanStatus: async (id) => ({ ...status(id), status: 'pausing',
    stopDiagnostic: { code: 'pause_waiting_for_quiescence', obligations: [] }, findingCandidateCount: 0,
  }) });
  t.after(h.unmount);
  const html = await renderDialog(h.b);
  assert.ok(html.includes('暂停中，等待执行线程退出与清理确认'));
  assert.ok(!html.includes('已暂停，等待人工核对'));
});
