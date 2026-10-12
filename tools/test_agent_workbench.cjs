const assert = require('node:assert/strict');
const test = require('node:test');
const { renderToString } = require('@vue/server-renderer');
const { mount, flush, deferred, draft, followup, handoff, handoffReceipt, templateExports, vue } = require('./agent_workbench_harness.cjs');

test('closure handoff requires explicit confirmation and only saves a fresh independent draft', async (t) => {
  const inputs = [];
  const { b, calls, events, unmount } = await mount({
    previewWebClosureHandoff: async () => handoff,
    createWebClosureHandoff: async (input) => { inputs.push(input); return handoffReceipt(input); },
  }, { handoff });
  t.after(unmount);
  await b.start();
  assert.equal(inputs.length, 0);
  b.handoffControls.handoffConfirmed.value = true;
  await b.start();
  assert.equal(inputs.length, 1);
  assert.equal(inputs[0].operatorConfirmed, true);
  assert.deepEqual(inputs[0].authSessionIds, ['identity-a', 'identity-b']);
  assert.equal(inputs[0].maxBudgetUsd, 5);
  assert.equal(calls.length, 0, 'neither generic creation nor scan confirmation runs');
  assert.equal(events.filter(([name]) => name === 'prepare').length, 1);
  assert.deepEqual(b.form.authSessionIds, []);
  assert.equal(b.handoffControls.handoffConfirmed.value, false);
});

test('closure handoff retries a frozen input and recovers canonical task after remount', async (t) => {
  const inputs = []; let saved = null;
  const api = {
    previewWebClosureHandoff: async () => ({ ...handoff, savedHandoff: saved }),
    createWebClosureHandoff: async (input) => { inputs.push(input); if (inputs.length === 1) throw new Error('IPC lost'); saved = handoffReceipt(input); return saved; },
  };
  const first = await mount(api, { handoff }); t.after(first.unmount);
  first.b.handoffControls.handoffConfirmed.value = true;
  await first.b.start(true);
  first.b.form.maxBudgetUsd = 900;
  first.b.form.authSessionIds = [];
  await first.b.start(true);
  assert.equal(inputs.length, 2);
  assert.deepEqual(inputs[0], inputs[1]);
  const second = await mount(api, { handoff }); t.after(second.unmount);
  await second.b.start(true);
  assert.equal(inputs.length, 2);
  assert.equal(second.calls.length, 0);
  assert.equal(second.events.find(([name]) => name === 'prepare')[1].id, draft.id);
});

test('closure handoff rejects corrupt receipts and ignores double click and stale results', async (t) => {
  for (const mutation of [r => ({ ...r, executionGranted: true }), r => ({ ...r, sourceExecutionSettled: true }),
    r => ({ ...r, sourceScanId: 'foreign' }), r => ({ ...r, sourceHash: 'c'.repeat(64) }),
    r => ({ ...r, requestId: 'foreign' }), r => ({ ...r, extra: true }),
    r => ({ ...r, scan: { ...r.scan, status: 'scanning' } })]) {
    const app = await mount({ previewWebClosureHandoff: async () => handoff,
      createWebClosureHandoff: async input => mutation(handoffReceipt(input)) }, { handoff });
    t.after(app.unmount); app.b.handoffControls.handoffConfirmed.value = true;
    await app.b.start(true);
    assert.equal(app.events.some(([name]) => name === 'prepare' || name === 'open'), false);
    assert.ok(app.b.handoffControls.pendingHandoff.value);
  }
  const pending = deferred(); let input; let count = 0;
  const app = await mount({ previewWebClosureHandoff: async () => handoff,
    createWebClosureHandoff: data => { input = data; count++; return pending.promise; } }, { handoff });
  t.after(app.unmount); app.b.handoffControls.handoffConfirmed.value = true;
  const action = app.b.start(true); await flush();
  await app.b.start(true); assert.equal(count, 1);
  app.b.mode.value = 'code'; app.b.mode.value = 'web';
  pending.resolve(handoffReceipt(input)); await action;
  assert.equal(app.events.some(([name]) => name === 'prepare' || name === 'open'), false);
});

test('closure handoff source mismatch fails closed and a recovered started task is only opened', async (t) => {
  const app = await mount({ previewWebClosureHandoff: async () => ({ ...handoff, sourceHash: 'changed' }),
    createWebClosureHandoff: async () => { throw new Error('must not create'); } }, { handoff });
  t.after(app.unmount); app.b.handoffControls.handoffConfirmed.value = true;
  await app.b.start(true);
  assert.equal(app.events.some(([name]) => name === 'prepare' || name === 'open'), false);
  const saved = handoffReceipt({ requestId: 'previous-request' }); saved.scan.status = 'scanning'; saved.scan.attemptCount = 1;
  const stale = await mount({ previewWebClosureHandoff: async () => ({ ...handoff, savedHandoff: saved, sourceHash: 'c'.repeat(64) }) }, { handoff });
  t.after(stale.unmount); await stale.b.start(true);
  assert.equal(stale.events.some(([name]) => name === 'prepare' || name === 'open'), false);
  const recovered = await mount({ previewWebClosureHandoff: async () => ({ ...handoff, savedHandoff: saved }) }, { handoff });
  t.after(recovered.unmount); await recovered.b.start(true);
  assert.equal(recovered.events.find(([name]) => name === 'open')[1].id, draft.id);
  assert.equal(recovered.calls.length, 0);
});

test('closure handoff real template exposes confirmation and draft-only controls', async (t) => {
  const { b, calls, props, unmount } = await mount({}, { handoff }); t.after(unmount);
  const html = await renderToString(vue.createSSRApp({ render() {
    return templateExports.render({}, [], props, vue.proxyRefs(b), {}, {});
  } }));
  assert.match(html, /人工结案后的独立任务/);
  assert.match(html, /未选为匿名/);
  assert.match(html, /保存草稿 \/ 配置控制组/);
  assert.doesNotMatch(html, /启动扫描/);
  assert.equal(calls.length, 0);
});

const durableInput = () => ({ requestId: 'durable-request', sourceScanId: followup.sourceScanId,
  assessmentMessageId: followup.assessmentMessageId, sourceHash: followup.sourceHash,
  taskName: '<saved task>', scanMode: 'standard', maxBudgetUsd: 4,
  authSessionIds: ['saved-owner', 'saved-tester'], authSessionScopeId: 'saved-scope', skillIds: [3],
  instruction: 'original instruction', closure: 'proof' });

test('remount restores the frozen submission and only explicit save replays it', async (t) => {
  let stored;
  const sent = [];
  const api = {
    getAgentGapFollowupSubmission: async () => stored || null,
    createAgentGapFollowup: async (input) => {
      sent.push(JSON.parse(JSON.stringify(input)));
      stored = { input: JSON.parse(JSON.stringify(input)), scan: null };
      if (sent.length === 1) throw new Error('response lost');
      stored.scan = draft;
      return draft;
    },
  };
  const first = await mount(api, { followup });
  await first.b.start(true);
  first.unmount();
  const second = await mount(api, { followup });
  t.after(second.unmount);
  assert.equal(sent.length, 1, 'restoration never submits');
  assert.deepEqual(JSON.parse(JSON.stringify(second.b.followupControls.pendingFollowupInput.value)), sent[0]);
  second.b.form.taskName = 'different';
  second.b.form.authSessionIds = ['wrong'];
  await second.b.start(true);
  assert.deepEqual(sent[1], sent[0]);
  assert.equal(second.calls.length, 0);
});

test('a committed task recovered after restart opens without another create or start', async (t) => {
  let creates = 0;
  const { b, events, calls, unmount } = await mount({
    getAgentGapFollowupSubmission: async () => ({ input: durableInput(), scan: { ...draft, status: 'completed_with_gaps' } }),
    createAgentGapFollowup: async () => { creates++; return draft; },
  }, { followup });
  t.after(unmount);
  assert.equal(events.length, 0, 'startup does not navigate');
  await b.start();
  assert.equal(creates, 0);
  assert.equal(calls.length, 0);
  assert.equal(events.filter((e) => e[0] === 'open').length, 1);
});

test('a deleted task receipt blocks recreation and release uses the original task id', async (t) => {
  let creates = 0;
  const releases = [];
  const { b, events, unmount } = await mount({
    getAgentGapFollowupSubmission: async () => ({ input: durableInput(), scan: null, createdScanId: 'deleted-task' }),
    createAgentGapFollowup: async () => { creates++; return draft; },
    releaseAgentGapFollowupSubmission: async (...args) => { releases.push(args); },
  }, { followup });
  t.after(unmount);
  await b.start();
  assert.equal(creates, 0);
  assert.match(events.at(-1)[2], /不会重建/);
  await b.followupControls.releaseFollowupSubmission();
  assert.deepEqual(releases, [['durable-request', 'deleted-task']]);
});

test('reconciliation failure blocks creation and can be retried without dropping saved data', async (t) => {
  let broken = true, creates = 0;
  const { b, unmount } = await mount({
    getAgentGapFollowupSubmission: async () => { if (broken) throw new Error('database unavailable'); return { input: durableInput(), scan: null }; },
    createAgentGapFollowup: async () => { creates++; return draft; },
  }, { followup });
  t.after(unmount);
  await b.start();
  assert.equal(creates, 0);
  assert.match(b.followupControls.followupRecoveryError.value, /unavailable/);
  broken = false;
  await b.start();
  assert.equal(creates, 1);
});

test('late recovery across source A→B→A cannot replace the newer recovery or navigate', async (t) => {
  const oldRead = deferred();
  let reads = 0;
  const { b, props, events, unmount } = await mount({
    getAgentGapFollowupSubmission: async () => { reads++; return reads === 1 ? oldRead.promise : null; },
  }, { followup });
  t.after(unmount);
  props.followup = { ...followup, assessmentMessageId: 'assessment-b' };
  await flush();
  props.followup = { ...followup };
  await flush();
  oldRead.resolve({ input: durableInput(), scan: draft });
  await flush();
  assert.equal(b.followupControls.pendingFollowupInput.value, undefined);
  assert.equal(b.followupControls.followupSubmission.value, undefined);
  assert.equal(events.length, 0);
});

test('unmount during read-only preflight prevents the delayed create', async () => {
  const read = deferred();
  let reads = 0, creates = 0;
  const { b, events, unmount } = await mount({
    getAgentGapFollowupSubmission: async () => ++reads === 1 ? null : read.promise,
    createAgentGapFollowup: async () => { creates++; return draft; },
  }, { followup });
  const operation = b.start();
  unmount();
  read.resolve(null);
  await operation;
  assert.equal(creates, 0);
  assert.equal(events.length, 0);
});

test('release reconciles an unseen committed task before a second explicit decision', async (t) => {
  let stored = { input: durableInput(), scan: null };
  const released = [];
  const { b, unmount } = await mount({
    getAgentGapFollowupSubmission: async () => stored,
    releaseAgentGapFollowupSubmission: async (request, expected) => {
      released.push([request, expected]);
      if (!expected) { stored = { ...stored, scan: draft }; throw new Error('followup_submission_changed_reconcile'); }
      stored = null;
    },
  }, { followup });
  t.after(unmount);
  await b.followupControls.releaseFollowupSubmission();
  assert.equal(b.followupControls.followupSubmission.value.scan.id, draft.id);
  assert.equal(released.length, 1, 'never auto-release after seeing a new result');
  await b.followupControls.releaseFollowupSubmission();
  assert.deepEqual(released[1], ['durable-request', draft.id]);
  assert.equal(b.followupControls.pendingFollowupInput.value, undefined);
  assert.equal(b.form.authSessionIds.length, 0);
});

test('restored submission metadata renders as escaped text and exposes explicit reset', async (t) => {
  const { b, props, unmount } = await mount({ getAgentGapFollowupSubmission: async () => ({ input: durableInput(), scan: null }) }, { followup });
  t.after(unmount);
  const html = await renderToString(vue.createSSRApp({ render() {
    return templateExports.render({}, [], props, vue.proxyRefs(b), {}, {});
  } }));
  assert.match(html, /&lt;saved task&gt;/);
  assert.match(html, /冻结提交/);
  assert.match(html, /结束原提交并重新准备/);
});

for (const lostResponse of [false, true]) {
  test(`release ${lostResponse ? 'lost response' : 'late success'} cannot retain a cancelled request across A→B→A`, async (t) => {
    const release = deferred();
    let stored = { input: durableInput(), scan: null };
    const sent = [];
    const { b, props, unmount } = await mount({
      getAgentGapFollowupSubmission: async () => stored,
      releaseAgentGapFollowupSubmission: () => release.promise,
      createAgentGapFollowup: async (input) => { sent.push(input); return draft; },
    }, { followup });
    t.after(unmount);
    const operation = b.followupControls.releaseFollowupSubmission();
    props.followup = { ...followup, assessmentMessageId: 'assessment-b' };
    await flush();
    props.followup = { ...followup };
    await flush();
    stored = null; // Backend committed the explicit release.
    if (lostResponse) release.reject(new Error('release response lost'));
    else release.resolve();
    await operation;
    assert.equal(b.followupControls.pendingFollowupInput.value, undefined);
    await b.start(true);
    assert.equal(sent.length, 1);
    assert.notEqual(sent[0].requestId, 'durable-request');
  });
}

test('follow-up uses only linked draft IPC, never direct start, and renders source context safely', async (t) => {
  const submitted = [];
  const { b, calls, events, props, unmount } = await mount({ createAgentGapFollowup: async (input) => { submitted.push(input); return draft; } }, { followup });
  t.after(unmount);
  const html = await renderToString(vue.createSSRApp({ render() {
    return templateExports.render({}, [], props, vue.proxyRefs(b), {}, {});
  } }));
  assert.match(html, /补充证据草稿/);
  assert.match(html, /&lt;script&gt;missing/);
  assert.doesNotMatch(html, /启动扫描/);
  await b.start(); // Even a programmatic direct-start event saves only.
  assert.equal(submitted.length, 1);
  assert.equal(submitted[0].sourceHash, followup.sourceHash);
  assert.equal(submitted[0].sourceScanId, followup.sourceScanId);
  assert.equal('urls' in submitted[0], false);
  assert.equal(calls.length, 0);
  assert.equal(events.filter((event) => event[0] === 'prepare').length, 1);
});

test('uncertain follow-up response retries frozen payload and id, even after form edits', async (t) => {
  const submitted = [];
  const { b, calls, unmount } = await mount({ createAgentGapFollowup: async (input) => {
    submitted.push(JSON.parse(JSON.stringify(input)));
    if (submitted.length === 1) throw new Error('IPC response lost');
    return draft;
  } }, { followup });
  t.after(unmount);
  await b.start(true);
  b.form.taskName = 'Edited after ambiguous commit';
  b.form.authSessionIds = ['new-identity'];
  await b.start(true);
  assert.deepEqual(submitted[0], submitted[1]);
  assert.equal(b.followupControls.pendingFollowupInput.value, undefined);
  assert.equal(calls.length, 0);
});

test('definitively rejected follow-up can be corrected; original target and project remain fixed', async (t) => {
  let submissions = 0;
  const { b, unmount } = await mount({ createAgentGapFollowup: async () => { submissions++; throw 'followup_rejected:invalid_identity'; } }, { followup });
  t.after(unmount);
  await b.start(true);
  assert.equal(b.followupControls.pendingFollowupInput.value, undefined);
  b.form.urls = 'https://different.example.test';
  await b.start(true);
  assert.equal(submissions, 1);
  b.form.urls = followup.targetUrl;
  b.form.projectId = 2;
  await b.start(true);
  assert.equal(submissions, 1);
});

test('late follow-up create response after unmount never confirms or navigates', async (t) => {
  const pending = deferred();
  const { b, calls, events, unmount } = await mount({ createAgentGapFollowup: () => pending.promise }, { followup });
  const operation = b.start(true);
  await flush(); // Pass reconciliation and enter the pending create.
  unmount();
  pending.resolve(draft);
  await operation;
  assert.equal(calls.length, 0);
  assert.equal(events.some((event) => ['prepare', 'open'].includes(event[0])), false);
});

for (const rejects of [false, true]) {
  test(`follow-up source A→B→A drops late ${rejects ? 'error' : 'navigation'}`, async (t) => {
    const pending = deferred();
    const { b, calls, events, props, unmount } = await mount({ createAgentGapFollowup: () => pending.promise }, { followup });
    t.after(unmount);
    const operation = b.start(true);
    await flush(); // Exercise a late creation result, not a late read.
    props.followup = { ...followup, assessmentMessageId: 'assessment-2' };
    await flush();
    props.followup = { ...followup };
    await flush();
    if (rejects) pending.reject(new Error('late source failure'));
    else pending.resolve(draft);
    await operation;
    assert.equal(calls.length, 0);
    assert.equal(events.some((event) => ['prepare', 'open', 'notify'].includes(event[0])), false);
  });
}

test('an ambiguous source A submission cannot be replayed as source B', async (t) => {
  let submissions = 0;
  const { b, props, events, unmount } = await mount({ createAgentGapFollowup: async () => {
    submissions++;
    throw new Error('IPC response lost');
  } }, { followup });
  t.after(unmount);
  await b.start(true);
  props.followup = { ...followup, assessmentMessageId: 'assessment-2' };
  await flush();
  await b.start(true);
  assert.equal(submissions, 1);
  assert.match(events.at(-1)[2], /另一补充任务/);
  props.followup = { ...followup };
  await flush();
  await b.start(true);
  assert.equal(submissions, 2);
});

test('source scope choices disclose actual fallback and diff limitations', async (t) => {
  const { b, props, unmount } = await mount({}, { initialMode: 'code' });
  t.after(unmount);
  for (const [scope, expected] of [['auto', /Auto 允许在对比基线或变更清单不可用时转为整仓分析/],
    ['diff', /Diff 基线无效或变更清单不可用时不执行整仓分析/],
    ['full', /Full 使用完整源码快照，不使用隐藏的旧对比基线/]]) {
    b.form.scopeMode = scope;
    const html = await renderToString(vue.createSSRApp({ render() {
      return templateExports.render({}, [], props, vue.proxyRefs(b), {}, {});
    } }));
    assert.match(html, expected);
    if (scope === 'diff') assert.match(html, /CodeQL 项目上下文尚未支持增量授权/);
  }
});

test('saved Web draft hands off to controls without starting; real template exposes both choices', async (t) => {
  const { b, calls, events, unmount } = await mount();
  t.after(unmount);
  const html = await renderToString(vue.createSSRApp({ render() {
    return templateExports.render({}, [], {}, vue.proxyRefs(b), {}, {});
  } }));
  assert.match(html, /保存草稿 \/ 配置控制组/);
  assert.match(html, /启动扫描/);
  const scope = b.authSessionScopeId.value;
  await b.start(true);
  assert.equal(calls.length, 1);
  assert.equal(calls[0][0], 'create');
  assert.deepEqual(calls[0][1][6], ['identity-a', 'identity-b']);
  assert.equal(calls[0][1][7], scope);
  assert.equal(events.filter(([name]) => name === 'prepare').length, 1);
  assert.equal(events.some(([name]) => name === 'open'), false);
  assert.notEqual(b.authSessionScopeId.value, scope);
  assert.deepEqual(b.form.authSessionIds, []);
  assert.equal(b.form.urls, '');
});

test('existing direct start still creates and confirms exactly once', async (t) => {
  const { b, calls, events, unmount } = await mount();
  t.after(unmount);
  await b.start();
  assert.deepEqual(calls.map(([name]) => name), ['create', 'confirm']);
  assert.equal(events.find(([name]) => name === 'open')[1].status, 'scanning');
  assert.equal(events.some(([name]) => name === 'prepare'), false);
});

test('creation failure preserves draft form and identities for retry', async (t) => {
  const { b, events, unmount } = await mount({ createSentinelUrlScan: async () => { throw new Error('creation failed'); } });
  t.after(unmount);
  const scope = b.authSessionScopeId.value;
  await b.start(true);
  assert.equal(b.authSessionScopeId.value, scope);
  assert.deepEqual(b.form.authSessionIds, ['identity-a', 'identity-b']);
  assert.equal(b.form.urls, 'https://draft.example.test');
  assert.equal(events.some(([name]) => name === 'prepare' || name === 'open'), false);
  assert.equal(b.busy.value, false);
});

test('uncertain start preserves created task, clears consumed identities, and never retries automatically', async (t) => {
  let confirms = 0;
  const { b, calls, events, unmount } = await mount({ confirmSentinelScan: async () => { confirms++; throw new Error('transport lost'); } });
  t.after(unmount);
  await b.start();
  assert.equal(confirms, 1);
  assert.equal(calls.filter(([name]) => name === 'create').length, 1);
  assert.deepEqual(b.form.authSessionIds, []);
  assert.equal(b.form.urls, '');
  assert.equal(events.find(([name]) => name === 'prepare')[1].id, draft.id);
  assert.match(events.find(([name, type]) => name === 'notify' && type === 'error')[2], /已保存.*不要重复创建/);
  assert.equal(events.some(([name]) => name === 'open'), false);
});

test('double-clicks cannot create or start two tasks', async (t) => {
  const pending = deferred();
  let creates = 0;
  const { b, calls, unmount } = await mount({ createSentinelUrlScan: () => { creates++; return pending.promise; } });
  t.after(unmount);
  const first = b.start(true);
  await b.start();
  assert.equal(creates, 1);
  pending.resolve(draft);
  await first;
  assert.equal(calls.length, 0);
});

test('workspace A→B→A switch revokes pending start even when final IDs match', async (t) => {
  const pending = deferred();
  const { b, calls, events, unmount } = await mount({ createSentinelUrlScan: () => pending.promise });
  t.after(unmount);
  const operation = b.start();
  b.form.projectId = 2;
  await flush();
  b.form.projectId = 1;
  await flush();
  b.form.urls = 'https://new-draft.example.test';
  pending.resolve(draft);
  await operation;
  assert.equal(calls.length, 0);
  assert.equal(events.some(([name]) => name === 'prepare' || name === 'open'), false);
  assert.equal(b.form.urls, 'https://new-draft.example.test');
});

test('unmount while saving leaves durable draft without starting or navigating', async () => {
  const pending = deferred();
  const { b, calls, events, unmount } = await mount({ createSentinelUrlScan: () => pending.promise });
  const operation = b.start();
  unmount();
  pending.resolve(draft);
  await operation;
  assert.equal(calls.length, 0);
  assert.equal(events.some(([name]) => name === 'prepare' || name === 'open'), false);
});

test('late auth-session list cannot resurrect identities consumed by saved draft', async (t) => {
  const { b, api, unmount } = await mount();
  t.after(unmount);
  const pending = deferred();
  api.listBrowserAuthSessions = () => pending.promise;
  const load = b.loadAuthSessions();
  await b.start(true);
  pending.resolve([{ id: 'identity-a', status: 'valid' }]);
  await load;
  assert.deepEqual(b.authControls.authSessions.value, []);
  assert.deepEqual(b.form.authSessionIds, []);
});

test('mode switch during a pending save revokes start but preserves edited task text', async (t) => {
  const pending = deferred();
  const { b, calls, events, unmount } = await mount({ createSentinelUrlScan: () => pending.promise });
  t.after(unmount);
  const operation = b.start();
  b.mode.value = 'code';
  b.form.taskName = 'Next code task';
  pending.resolve(draft);
  await operation;
  assert.equal(calls.length, 0);
  assert.equal(b.form.taskName, 'Next code task');
  assert.equal(events.some(([name]) => name === 'prepare' || name === 'open'), false);
});

test('homogeneous opt-in uses dedicated draft IPC and default stays legacy', async t => {
  const app=await mount({createSentinelHomogeneousUrlScan:async (...args)=>{app.calls.push(['homogeneous',args]);return draft;}});t.after(app.unmount);
  app.b.form.homogeneousWorkers=true;app.b.form.orchestrationMode='multi';
  await app.b.start(true);
  assert.equal(app.calls.filter(([kind])=>kind==='homogeneous').length,1);
  assert.equal(app.calls.filter(([kind])=>kind==='create').length,0);
  const legacy=await mount();t.after(legacy.unmount);await legacy.b.start(true);
  assert.equal(legacy.calls.filter(([kind])=>kind==='create').length,1);
});
test('homogeneous opt-in rejects nonweb single and approved tools without silently changing intent', async t=>{
  for(const setup of [a=>a.b.mode.value='code',a=>a.b.form.orchestrationMode='single',a=>a.b.form.approvedToolIds=['aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa']]) {
    const app=await mount();t.after(app.unmount);app.b.form.homogeneousWorkers=true;app.b.form.orchestrationMode='multi';setup(app);await app.b.start(true);assert.equal(app.calls.length,0);
  }
});
