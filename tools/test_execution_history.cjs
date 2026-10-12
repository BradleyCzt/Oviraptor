// Render the actual task-detail SFC. IPC/icons/form are mocked; a source-evidence
// child probe verifies parent scope wiring. Its actual SFC is tested separately.
const assert = require('node:assert/strict');
const test = require('node:test');
const { loadSentinelApi } = require('./sentinel_api_harness.cjs');
const { mount, flush, deferred, scan, page, source, searchClock } = require('./execution_history/harness.cjs');
require('./execution_history/search_contract.cjs');

test('task-center cost strip discloses loaded deployment scope without asserting zero findings', async t => {
  const h = await mount(async () => page()); t.after(h.unmount);
  h.props.tokenScopeLabel = '云端 AI';
  h.props.zeroYieldCount = 2; h.props.zeroYieldTokens = 110;
  const html = await h.render();
  assert.match(html, /已加载任务 · 云端 AI/);
  assert.match(html, /未关联漏洞记录/);
  assert.match(html, /不代表无漏洞或执行完整/);
  assert.match(html, /不随下方搜索或状态筛选改变/);
  assert.doesNotMatch(html, /零漏洞产出|累计 Token/);
});

test('actual API uses versioned execution history IPC and preserves an opaque cursor', async () => {
  const calls = [];
  const result = { schemaVersion: 2, invocations: [] };
  const exports = { sentinelApi: loadSentinelApi(async (...args) => { calls.push(args); return result; }) };
  const cursor = '{"schemaVersion":2,"recordKey":"source:a:0001:0000"}';
  assert.equal(await exports.sentinelApi.getNativeAttemptToolHistory('scan-A', 3, cursor), result);
  assert.equal(await exports.sentinelApi.getNativeAttemptToolHistory('scan-A', 3), result);
  assert.deepEqual(calls, [
    ['get_native_attempt_execution_history', { scanId: 'scan-A', attemptNumber: 3, before: cursor, limit: 50 }],
    ['get_native_attempt_execution_history', { scanId: 'scan-A', attemptNumber: 3, before: undefined, limit: 50 }],
  ]);
});

test('task detail distinguishes source plans, matched receipts, denial, damage and finish markers', async (t) => {
  const h = await mount(async () => page('A', [source('one', 'planned'), source('two', 'completed'),
    source('three', 'refused'), source('four', 'unverified'), source('finish', 'completed', { resultKind: 'control', toolName: 'assignment.finish' }),
    source('escaped', 'completed', { toolName: '<script>unsafe</script>' })]));
  t.after(h.unmount); h.b.detailTab.value = 'execution'; await flush();
  const html = await h.render();
  for (const text of ['不是执行开始时间', '计划待执行', '本地回执一致，不等于独立审查通过', '工具拒绝，未获得成功结果',
    '回执无法核对', '阶段结束标记，不计为漏洞证据']) assert.ok(html.includes(text), text);
  assert.ok(!html.includes('<script>unsafe</script>'));
  assert.ok(html.includes('&lt;script&gt;unsafe&lt;/script&gt;'));
  assert.deepEqual(h.calls, [['A', 1, undefined]]);
});

test('archive fences project round trips and old finally cannot unlock a new operation', async (t) => {
  const first = deferred(), second = deferred(), calls = [];
  const h = await mount(async () => page(), { archiveSentinelScan: (...args) => {
    calls.push(args); return calls.length === 1 ? first.promise : second.promise;
  } });
  t.after(h.unmount);
  const a = { ...scan('A'), projectId: 1 }, b = { ...scan('B'), projectId: 1 };
  h.props.projectId = 1; await flush();
  const old = h.b.toggleArchive(a);
  h.props.projectId = 2; await flush(); h.props.projectId = 1; await flush();
  const current = h.b.toggleArchive(b);
  first.resolve({ ...a, archivedAt: 'now' }); await old;
  assert.deepEqual(h.archived, []);
  assert.equal(h.b.archiveBusy.value, 'B');
  second.resolve({ ...b, archivedAt: 'now' }); await current;
  assert.deepEqual(h.archived.map(row => row.id), ['B']);
  assert.deepEqual(calls, [['A', 1, true], ['B', 1, true]]);
});

test('archive rejects mismatched results, sanitizes errors and permits retry without duplicate writes', async (t) => {
  const pending = deferred(), calls = [];
  const a = { ...scan('A'), projectId: 1 };
  const h = await mount(async () => page(), { archiveSentinelScan: (...args) => {
    calls.push(args); return calls.length === 1 ? pending.promise : Promise.resolve({ ...a, archivedAt: 'now' });
  } });
  t.after(h.unmount); h.props.projectId = 1; await flush();
  const first = h.b.toggleArchive(a); await h.b.toggleArchive(a);
  assert.equal(calls.length, 1);
  pending.reject(new Error('/private/token=secret')); await first;
  assert.equal(h.b.searchError.value, '任务归档更新失败，请刷新后重试');
  await h.b.toggleArchive(a); assert.equal(h.archived.length, 1);
  for (const result of [{ ...a, id: 'B' }, { ...a, projectId: 2 }, null]) {
    const bad = await mount(async () => page(), { archiveSentinelScan: async () => result });
    try { await bad.b.toggleArchive(a); assert.deepEqual(bad.archived, []); }
    finally { bad.unmount(); }
  }
});

test('archive ignores late unmounted success/error and cannot submit from another project', async () => {
  for (const fail of [false, true]) {
    const pending = deferred(); let calls = 0;
    const a = { ...scan('A'), projectId: 1 };
    const h = await mount(async () => page(), { archiveSentinelScan: () => { calls++; return pending.promise; } });
    try {
      h.props.projectId = 2; await flush(); const skipped = h.b.toggleArchive(a);
      assert.equal(calls, 0); await skipped;
      h.props.projectId = 1; await flush(); const request = h.b.toggleArchive(a);
      h.unmount();
      if (fail) pending.reject(new Error('secret')); else pending.resolve({ ...a, archivedAt: 'now' });
      await request; await h.b.toggleArchive(a);
      assert.equal(calls, 1); assert.deepEqual(h.archived, []); assert.equal(h.b.searchError.value, '');
    } finally { pending.resolve({ ...a, archivedAt: 'now' }); h.unmount(); }
  }
});

test('restore keeps exact IPC parameters and an old archive error cannot replace a new search error', async (t) => {
  const pending = deferred(), calls = [];
  const h = await mount(async () => page(), { archiveSentinelScan: (...args) => {
    calls.push(args); return pending.promise;
  } });
  t.after(h.unmount);
  const request = h.b.toggleArchive({ ...scan('A'), projectId: 1, archivedAt: 'before' });
  h.b.search.value = 'new query'; await flush(); h.b.searchError.value = 'current search error';
  pending.reject(new Error('old archive error')); await request;
  assert.deepEqual(calls, [['A', 1, false]]);
  assert.equal(h.b.searchError.value, 'current search error');
  assert.equal(h.b.archiveBusy.value, '');
});

test('task search debounces edits and cancels all reads after unmount', async () => {
  const clock = searchClock(), calls = [];
  const h = await mount(async () => page(), {
    searchSentinelScanPage: async (...args) => { calls.push(args); return []; },
  }, clock);
  h.b.search.value = ' first '; await flush();
  h.b.search.value = ' latest '; await flush();
  assert.equal(clock.size, 1);
  clock.fire(); await flush();
  assert.deepEqual(calls, [[undefined, 'latest', 'attention', 100, undefined]]);
  h.b.search.value = 'never send'; await flush();
  h.unmount();
  assert.equal(clock.size, 0, 'unmount must release the queued debounce callback');
  clock.fire(); await h.b.loadSearchPage();
  assert.equal(calls.length, 1);
});

test('unmounted search ignores both late success and late failure including finally', async () => {
  for (const fail of [false, true]) {
    const pending = deferred(), clock = searchClock();
    const h = await mount(async () => page(), { searchSentinelScanPage: () => pending.promise }, clock);
    h.b.search.value = 'saved task'; await flush(); clock.fire();
    assert.equal(h.b.searchLoading.value, true);
    h.unmount();
    if (fail) pending.reject(Error('Bearer raw-backend-secret'));
    else pending.resolve([scan('late')]);
    await flush();
    assert.deepEqual(h.b.searchResults.value, []);
    assert.equal(h.b.searchError.value, '');
    assert.equal(h.b.searchLoading.value, true, 'an old finally must not mutate destroyed state');
  }
});

test('search project, query and view changes fence old failures and preserve current loading', async (t) => {
  const clock = searchClock(), requests = [];
  const h = await mount(async () => page(), {
    listHistoricalImportRuns: async () => [],
    searchSentinelScanPage: (...args) => { const pending = deferred(); requests.push({ args, ...pending }); return pending.promise; },
  }, clock); t.after(h.unmount);
  h.props.projectId = 1; h.b.search.value = 'one'; await flush(); clock.fire();
  h.props.projectId = 2; h.b.search.value = 'two'; h.b.view.value = 'history'; await flush(); clock.fire();
  requests[0].reject(Error('obsolete')); await flush();
  assert.equal(h.b.searchLoading.value, true); assert.equal(h.b.searchError.value, '');
  assert.deepEqual(requests[1].args, [2, 'two', 'history', 100, undefined]);
  requests[1].resolve([{ ...scan('current'), projectId: 2 }]); await flush();
  assert.deepEqual(h.b.searchResults.value.map(row => row.id), ['current']);
  assert.equal(h.b.searchLoading.value, false);
  h.b.search.value = ' '; await flush();
  assert.equal(clock.size, 0); assert.deepEqual(h.b.searchResults.value, []);
  assert.equal(h.b.searchHasMore.value, false);
});

test('search pagination is single-flight, uses only cursor fields and retains rows on failure', async (t) => {
  const clock = searchClock(), calls = [], pending = deferred();
  const rows = Array.from({ length: 100 }, (_, i) => scan(`task-${i}`));
  const h = await mount(async () => page(), {
    searchSentinelScanPage: (...args) => {
      calls.push(args);
      return calls.length === 1 ? Promise.resolve(rows) : calls.length === 2 ? pending.promise : Promise.resolve([scan('older')]);
    },
  }, clock); t.after(h.unmount);
  h.b.search.value = 'task'; await flush(); clock.fire(); await flush();
  assert.equal(h.b.searchHasMore.value, true);
  const older = h.b.loadSearchPage(true); await h.b.loadSearchPage(true);
  pending.reject(Error('/private/data.db: Bearer raw-backend-secret')); await older;
  assert.equal(calls.length, 2);
  assert.deepEqual(calls[1][4], { id: rows[99].id, updatedAt: rows[99].updatedAt });
  assert.deepEqual(h.b.searchResults.value.map(row => row.id), rows.map(row => row.id));
  assert.equal(h.b.searchHasMore.value, true); assert.equal(h.b.searchLoading.value, false);
  assert.equal(h.b.searchError.value, '全库搜索失败，请重试');
  await h.b.loadSearchPage(true);
  assert.deepEqual(calls[2][4], calls[1][4], 'retry keeps the last successful page cursor');
  assert.equal(h.b.searchResults.value.length, 101);
  assert.equal(h.b.searchResults.value[100].id, 'older');
  assert.equal(h.b.searchHasMore.value, false); assert.equal(h.b.searchError.value, '');
  await h.b.loadSearchPage(true); assert.equal(calls.length, 3);
});

test('older tool history passes the opaque cursor exactly and prepends records once', async (t) => {
  const pending = deferred();
  const h = await mount(async (_, __, before) => before ? pending.promise : page('A', [source('new')], { hasOlder: true, olderCursor: 'opaque-v2-cursor' }));
  t.after(h.unmount); await h.b.loadTools('A', 1);
  const request = h.b.loadTools('A', 1, true); await h.b.loadTools('A', 1, true);
  assert.deepEqual(h.calls, [['A', 1, undefined], ['A', 1, 'opaque-v2-cursor']]);
  pending.resolve(page('A', [source('old')])); await request;
  assert.deepEqual(h.b.toolPage.value.invocations.map((item) => item.id), ['old', 'new']);
});

test('an old history response cannot replace a newly selected task', async (t) => {
  const pending = deferred();
  const h = await mount(async (id) => id === 'A' ? pending.promise : page('B', [source('B-only')]));
  t.after(h.unmount); const old = h.b.loadTools('A', 1);
  h.props.preview = scan('B'); await flush(); await h.b.loadTools('B', 1);
  pending.resolve(page('A', [source('stale')])); await old;
  assert.equal(h.b.toolPage.value.scanId, 'B');
  assert.deepEqual(h.b.toolPage.value.invocations.map((item) => item.id), ['B-only']);
});

test('mismatched scope or old numeric-cursor schema is not accepted as current history', async (t) => {
  for (const invalid of [page('B'), page('A', [], { attemptNumber: 2 }), page('A', [], { schemaVersion: 1 })]) {
    const h = await mount(async () => invalid); t.after(h.unmount);
    await h.b.loadTools('A', 1);
    assert.equal(h.b.toolPage.value, undefined);
    assert.equal(h.b.toolError.value, '此轮工具调用暂时不可读取');
  }
});

test('task evidence tab passes the selected source attempt and keeps history separate from audited evidence', async (t) => {
  const h = await mount(async () => page()); t.after(h.unmount);
  h.b.detailTab.value = 'evidence'; await flush();
  h.b.historicalPreviews.value = [{ membershipId: 1, title: 'Historical fixture', kind: 'finding',
    producer: 'legacy fixture', attemptNumber: 1 }];
  let html = await h.render();
  assert.ok(html.includes('data-source-scan="A"')); assert.ok(html.includes('data-source-attempt="1"'));
  assert.ok(html.includes('源码审查执行轮次'));
  assert.ok(html.includes('Historical fixture')); assert.ok(html.includes('未审核 / 只读'));
  assert.ok(html.includes('不能当作已确认漏洞或新任务授权'));
  h.b.selectedAttempt.value = 2; await flush();
  html = await h.render(); assert.ok(html.includes('data-source-attempt="2"'));
  h.props.preview = { ...scan('web-task'), scanType: 'web' }; await flush();
  h.b.detailTab.value = 'evidence';
  html = await h.render(); assert.ok(!html.includes('data-source-scan='));
});

test('changing source attempt invalidates execution caches before returning to the execution tab', async (t) => {
  const pending = deferred();
  const h = await mount(async (_, attempt) => attempt === 1 ? page('A', [source('old-attempt')]) : pending.promise);
  t.after(h.unmount);
  h.b.detailTab.value = 'execution'; await flush();
  assert.equal(h.b.toolPage.value.attemptNumber, 1);
  assert.equal(h.b.mailboxPage.value.attemptNumber, 1);
  assert.equal(h.b.runnerLog.value.attempt, 1);
  h.b.detailTab.value = 'evidence'; await flush();
  (await h.sourceAttemptSelector())(2);
  assert.equal(h.b.selectedAttempt.value, 2);
  assert.equal(h.b.toolPage.value, undefined, 'old tools cannot be labelled as new-attempt tools');
  assert.equal(h.b.mailboxPage.value, undefined);
  assert.equal(h.b.runnerLog.value, undefined);
  await flush(); h.b.detailTab.value = 'execution'; await flush();
  assert.ok(!(await h.render()).includes('old-attempt'));
  pending.resolve(page('A', [source('new-attempt')], { attemptNumber: 2 })); await flush();
  assert.equal(h.b.toolPage.value.attemptNumber, 2);
  assert.equal(h.b.mailboxPage.value.attemptNumber, 2);
  assert.equal(h.b.runnerLog.value.attempt, 2);
  assert.deepEqual(h.calls, [['A', 1, undefined], ['A', 2, undefined]]);
});

test('mailbox and runner log reject responses from another task or attempt', async (t) => {
  for (const [scanId, attempt] of [['B', 1], ['A', 2]]) {
    const h = await mount(async () => page(), {
      getNativeAttemptMailboxHistory: async () => ({ scanId, attemptNumber: attempt, messages: [], hasOlder: false }),
      readSentinelRunnerLog: async () => ({ scanId, attempt, lines: ['wrong scope'] }),
    });
    t.after(h.unmount);
    await Promise.all([h.b.loadMailbox('A', 1), h.b.loadAttemptLog('A', 1)]);
    assert.equal(h.b.mailboxPage.value, undefined);
    assert.equal(h.b.runnerLog.value, undefined);
    assert.equal(h.b.mailboxError.value, '此轮协作消息暂时不可读取');
    assert.equal(h.b.runnerLogError.value, '此轮日志暂时不可读取');
  }
});

test('closing task details clears loading and ignores the pending detail response', async (t) => {
  const pending = deferred();
  const h = await mount(async () => page(), { listSentinelScanAttempts: () => pending.promise });
  t.after(h.unmount);
  assert.equal(h.b.detailLoading.value, true);
  h.props.preview = undefined; await flush();
  assert.equal(h.b.detailLoading.value, false);
  pending.resolve([{ attemptNumber: 1 }]); await flush();
  assert.deepEqual(h.b.attempts.value, []);
});

test('unmount fences pending detail, tool, mailbox and log publication and new reads', async () => {
  const pending = deferred(), mailbox = deferred(), log = deferred(), detail = deferred();
  let detailCalls = 0, mailboxCalls = 0, logCalls = 0;
  const h = await mount(() => pending.promise, {
    listSentinelScanAttempts: () => ++detailCalls === 1 ? Promise.resolve([{ attemptNumber: 1 }]) : detail.promise,
    getNativeAttemptMailboxHistory: () => { ++mailboxCalls; return mailbox.promise; },
    readSentinelRunnerLog: () => { ++logCalls; return log.promise; },
  });
  h.props.preview = { ...scan('A'), updatedAt: 'new revision' }; await flush();
  const requests = [h.b.loadTools('A', 1), h.b.loadMailbox('A', 1), h.b.loadAttemptLog('A', 1)];
  h.unmount();
  pending.resolve(page('A', [source('late')]));
  mailbox.resolve({ scanId: 'A', attemptNumber: 1, messages: [{ id: 'late' }], hasOlder: false });
  log.resolve({ scanId: 'A', attempt: 1, lines: ['late'] });
  detail.resolve([{ attemptNumber: 1 }]); await Promise.all(requests); await flush();
  assert.equal(h.b.toolPage.value, undefined);
  assert.equal(h.b.mailboxPage.value, undefined);
  assert.equal(h.b.runnerLog.value, undefined);
  assert.deepEqual(h.b.attempts.value, []);
  await Promise.all([h.b.loadTools('A', 1), h.b.loadMailbox('A', 1), h.b.loadAttemptLog('A', 1)]);
  assert.equal(h.calls.length, 1); assert.equal(mailboxCalls, 1); assert.equal(logCalls, 1);
});
