const test=require('node:test');
const {assert,flush,deferred,mountWorkspace,source}=require('./plugin_workspace_harness.cjs');
test('host has no owned asset result/run or scanning log state after extraction',()=>{
 const app=source('src/App.vue');
 for(const symbol of ['const runs =','const stats =','const sentinelLogScans','useRunnerLogPanel','useAssetLogPanel','api.startJob(','api.resumeJob(','api.listSentinelScans('])assert.ok(!app.includes(symbol),symbol);
 assert.match(app,/BUILTIN_WORKSPACES/);assert.match(app,/configSection==='profiles'/);assert.match(app,/configSection==='workers'/);assert.match(app,/configSection==='runtime'/);
});
test('Assets owns dashboard statistics and drops stale project reads',async t=>{
 const first=deferred(),second=deferred();const m=mountWorkspace(t,'asset',{dashboardStats:id=>id===91?first.promise:second.promise});
 m.props.projectId=92;await flush();second.resolve({assetCount:92,runningJobs:2,blockedCount:0});await flush();first.resolve({assetCount:91,runningJobs:1,blockedCount:0});await flush();
 assert.equal(m.b.stats.value.assetCount,92);assert.equal(m.props.projectId,92);assert.equal(m.events.filter(([name])=>name==='metadata').at(-1)[1].badges.runningJobs,2);
});
test('Assets drops secondary reads after newer view and project selections',async t=>{
 const old=deferred();const m=mountWorkspace(t,'asset',{listRuns:()=>old.promise},{view:'tasks'});m.props.view='assets';m.props.projectId=92;await flush();old.resolve([{id:777,projectId:91}]);await flush();assert.deepEqual(m.b.runs.value,[]);
});
test('Assets does not lose committed state on host refresh and exposes read failures',async t=>{
 let fails=false;const m=mountWorkspace(t,'asset',{dashboardStats:async()=>{if(fails)throw Error('read unavailable');return{assetCount:4,runningJobs:1,blockedCount:0};}});await flush();fails=true;m.props.refreshToken++;await flush();assert.equal(m.b.stats.value.assetCount,4);assert.match(m.b.readError.value,/read unavailable/);assert.match(await m.render(),/displayed data may be stale/);
});
test('one-shot asset intent is consumed once, even when host refreshes and repeats its object',async t=>{
 const m=mountWorkspace(t,'asset',{}, {view:'tasks',intent:{id:1,kind:'start',projectId:91}});await flush();assert.equal(m.calls.filter(([name])=>name==='startJob').length,1);assert.deepEqual(m.events.filter(([name])=>name==='intentConsumed'),[['intentConsumed',1]]);
 m.props.refreshToken++;m.props.intent={id:1,kind:'start',projectId:91};await flush();assert.equal(m.calls.filter(([name])=>name==='startJob').length,1);
});
test('one-shot intent for another project is consumed but cannot start an operation',async t=>{
 const m=mountWorkspace(t,'asset',{}, {intent:{id:1,kind:'start',projectId:92}});await flush();assert.equal(m.calls.filter(([name])=>name==='startJob').length,0);assert.ok(m.events.some(([name,,text])=>name==='notify'&&text.includes('Workspace changed')));
});
test('switching project during quick-start cannot navigate or emit stale mutation results',async t=>{
 const operation=deferred();const m=mountWorkspace(t,'asset',{startJob:()=>operation.promise});await flush();const pending=m.b.quickStartProject(91);m.props.projectId=92;await flush();operation.resolve(8);await pending;
 assert.equal(m.events.filter(([name])=>name==='navigate'||name==='projectsChanged').length,0);assert.equal(m.b.starting.value,false);
});
for(const change of ['project','unmount'])test(`already-admitted restart completes exactly once after ${change} change without stale UI`,async t=>{
 const ack=deferred();const m=mountWorkspace(t,'asset',{acknowledgeInterruptedRun:()=>ack.promise});await flush();const pending=m.b.resumeInterrupted({runId:1,projectId:91,name:'Old run',pipeline:'full'});if(change==='project'){m.props.projectId=92;await flush();}else m.unmount();ack.resolve();await pending;
 assert.deepEqual(m.calls.filter(([name])=>name==='startJob'),[['startJob',91,1,'Old run · Resume','full']]);assert.equal(m.events.filter(([name])=>name==='interruptedResumed'||name==='navigate'||name==='projectsChanged').length,0);
});
test('asset operation errors stay visible beside its workspace and reset on project change',async t=>{
 const m=mountWorkspace(t,'asset',{startJob:async()=>{throw Error('execution refused');}});await flush();await m.b.quickStartProject(91);assert.match(m.b.actionError.value,/execution refused/);assert.match(await m.render(),/execution refused/);m.props.projectId=92;await flush();assert.equal(m.b.actionError.value,'');
});
test('unmount only releases listeners and never cancels or deletes backend work',async()=>{
 const m=mountWorkspace(null,'asset');await flush();m.unmount();assert.ok(m.subscriptions.every(sub=>sub.stopped));assert.equal(m.calls.filter(([name])=>/cancel|delete|archive/i.test(name)).length,0);
});
test('late listener installation is released after unmount',async()=>{
 const listener=deferred();let stopped=0;const m=mountWorkspace(null,'asset',{listen:()=>listener.promise});m.unmount();listener.resolve(()=>stopped++);await flush();assert.equal(stopped,1);
});
test('Scanning owns section navigation without creating scans or model messages',async t=>{
 const m=mountWorkspace(t,'sentinel',{}, {navigation:{key:'queue',view:'sentinel',section:'queue'}});await flush();assert.equal(m.b.sentinelSection.value,'queue');assert.equal(m.b.sentinelMenu.value,'queue');m.b.openWorkbench('greybox');assert.equal(m.b.sentinelWorkbenchMode.value,'greybox');assert.equal(m.b.sentinelSection.value,'workbench');assert.equal(m.calls.filter(([name])=>/start|create|send/.test(name)).length,0);
});
test('Scanning drops old project attempt reads even with the same scan ID',async t=>{
 const older=deferred(),newer=deferred();let reads=0;const m=mountWorkspace(t,'sentinel',{listSentinelScanAttempts:()=>++reads===1?older.promise:newer.promise});m.b.selectSentinelLogScan('same');m.props.projectId=92;await flush();m.b.selectSentinelLogScan('same');newer.resolve([{attemptNumber:2}]);await flush();older.resolve([{attemptNumber:1}]);await flush();assert.equal(m.b.sentinelLogAttempts.value[0].attemptNumber,2);
});
test('Scanning log list cannot update after leaving logs or unmounting',async t=>{
 const late=deferred();const m=mountWorkspace(t,'sentinel',{listSentinelScans:()=>late.promise},{view:'logs'});m.props.view='sentinel';await flush();late.resolve([{id:'old',status:'completed'}]);await flush();assert.deepEqual(m.b.sentinelLogScans.value,[]);m.unmount();assert.equal(m.calls.filter(([name])=>/cancel|delete/.test(name)).length,0);
});
test('HackerOne owns its activity read and ignores a result after leaving logs',async t=>{
 const late=deferred();const m=mountWorkspace(t,'hackerone',{listHackerOneEvents:()=>late.promise},{view:'logs'});m.props.view='hackerone';await flush();late.resolve([{id:1}]);await flush();assert.deepEqual(m.b.hackerOneLogEvents.value,[]);
});
const {mountLegacyAssets}=require('./plugin_workspace_harness.cjs');
for(const owner of ['act','archive'])test(`Scanning availability changes do not release ordinary ${owner} bulk lock`,async t=>{
 const pending=deferred();const m=mountLegacyAssets(t,{[owner==='act'?'updateAssetDecisions':'softDeleteAssetSelections']:()=>pending.promise});const operation=owner==='act'?m.b.act('confirmed'):m.b.archive(true);
 assert.equal(m.b.bulkBusy.value,true);m.props.scanningEnabled=false;await flush();assert.equal(m.b.bulkBusy.value,true);await m.b.sendToSentinel();if(owner==='act')await m.b.archive(true);else await m.b.act('confirmed');
 assert.equal(m.calls.filter(([name])=>['createSentinelScan','updateAssetDecisions','softDeleteAssetSelections'].includes(name)).length,1);
 pending.resolve(1);await operation;assert.equal(m.b.bulkBusy.value,false);
});
for(const transition of ['disable-reenable','project','unmount'])test(`pending cross-plugin draft cannot admit a second project after ${transition}`,async t=>{
 const first=deferred();const m=mountLegacyAssets(t,{createSentinelScan:()=>first.promise});const operation=m.b.sendToSentinel();assert.equal(m.calls.filter(([name])=>name==='createSentinelScan').length,1);
 if(transition==='disable-reenable'){m.props.scanningEnabled=false;await flush();m.props.scanningEnabled=true;await flush();}else if(transition==='project'){m.props.selectedProjectId=92;await flush();}else m.unmount();
 first.resolve('scan-one');await operation;assert.equal(m.calls.filter(([name])=>name==='createSentinelScan').length,1);assert.equal(m.events.filter(([kind])=>kind==='success').length,0);
});
test('disabled Scanning refuses an Assets action before IPC and keeps its selection',async t=>{
 const m=mountLegacyAssets(t);m.props.scanningEnabled=false;await flush();await m.b.sendToSentinel();assert.equal(m.calls.filter(([name])=>name==='createSentinelScan').length,0);assert.equal(m.b.selectedRows.value.length,2);assert.match(m.b.scanningActionError.value,/Scanning is disabled/);
});
test('rejected queued resume intent settles its host receipt without acknowledging a job',async t=>{
 const settled=[];const m=mountWorkspace(t,'asset',{}, {projects:[{id:91,name:'Archived',status:'archived'}],intent:{id:2,kind:'resume',projectId:91,job:{runId:19,projectId:91,name:'Old run',pipeline:'full'}},resumeWorkflow:{begin:()=>{throw Error('must not admit');},settle:(...args)=>settled.push(args)}});await flush();assert.equal(settled.length,1);assert.equal(settled[0][1].started,false);assert.equal(settled[0][1].acknowledged,false);assert.equal(m.calls.filter(([name])=>name==='startJob'||name==='acknowledgeInterruptedRun').length,0);
});
for(const owner of ['act','archive'])test(`old disabled scan completion cannot release a newer ${owner} operation lock`,async t=>{
 const scan=deferred(),ordinary=deferred();const m=mountLegacyAssets(t,{createSentinelScan:()=>scan.promise,[owner==='act'?'updateAssetDecisions':'softDeleteAssetSelections']:()=>ordinary.promise});const scanOperation=m.b.sendToSentinel();m.props.scanningEnabled=false;await flush();assert.equal(m.b.bulkBusy.value,false);
 const ordinaryOperation=owner==='act'?m.b.act('confirmed'):m.b.archive(true);assert.equal(m.b.bulkBusy.value,true);scan.resolve('first-draft');await scanOperation;assert.equal(m.b.bulkBusy.value,true);assert.equal(m.calls.filter(([name])=>name==='createSentinelScan').length,1);
 ordinary.resolve(1);await ordinaryOperation;assert.equal(m.b.bulkBusy.value,false);
});
