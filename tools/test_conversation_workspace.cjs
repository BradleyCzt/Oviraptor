const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vue = require('vue');
const { parse, compileScript, compileTemplate } = require('@vue/compiler-sfc');
const { renderToString } = require('@vue/server-renderer');
const { transpile } = require('./shell_module_harness.cjs');
const source = relative => fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const flush = async () => { for (let i=0;i<30;i++) await vue.nextTick(); };
const project = (id=1, status='active') => ({ id, status, name:`Project ${id}` });
const conversation = (id='a', projectId=1, over={}) => ({ id, projectId, title:`Chat ${id}`, archived:false, revision:1, messageCount:0, lastMessagePreview:'', createdAt:'2026-10-13T00:00:00Z', updatedAt:'2026-10-13T00:00:00Z', ...over });
const message = (sequence, id='a', role='user') => ({ id:`${id}-${sequence}`, conversationId:id, sequence, role, content:`message ${sequence}`, replyToMessageId:null, createdAt:'2026-10-13T00:00:00Z' });
const reply = (status='running', over={}) => ({ id:'r', userMessageId:'a-1', status, code:'', message:'', assistantMessageId:null, outcomeUnknown:false, createdAt:'t', updatedAt:'t', ...over });
const snapshot = (id='a', projectId=1, over={}) => ({ conversation:conversation(id,projectId), messages:[], hasMore:false, links:[], replyAvailability:{ available:true, code:'', message:'', model:'Configured model' }, latestReply:null, ...over });
function mount(t, overrides={}, initialProject=project()) {
  const timers=new Map(); let timerId=0, opId=0;
  const exports={};
  new Function('require','exports','setTimeout','clearTimeout',transpile(source('src/features/conversations/useConversationWorkspace.ts')))(name => { if(name==='vue') return vue; throw Error(`Unexpected import ${name}`); }, exports, fn => {timers.set(++timerId,fn);return timerId;}, id => timers.delete(id));
  const calls=[];
  const defaults={
    listConversations:async input => ({items:[conversation('a',input.projectId)],total:1}),
    readConversation:async input => snapshot(input.conversationId,input.projectId),
    createConversation:async input => conversation('new',input.projectId),
    sendConversationMessage:async input => ({conversation:conversation(input.conversationId,input.projectId,{revision:2}),userMessage:message(1,input.conversationId),reply:null}),
    updateConversation:async input => conversation(input.conversationId,input.projectId,{revision:2}),
    cancelConversationReply:async () => reply('cancel_requested'),
    deleteConversation:async () => {},
  };
  const api=new Proxy({}, {get:(_,name) => (...args) => {calls.push([name,...args]);return (overrides[name]||defaults[name]||(()=>{throw Error(`Unexpected IPC ${String(name)}`);} ))(...args);}});
  const selection=vue.ref(initialProject);
  let b;
  const renderer=vue.createRenderer({createComment:()=>({}),insert(){},remove(){},parentNode:()=>null,nextSibling:()=>null});
  const app=renderer.createApp({setup(){b=exports.useConversationWorkspace({project:()=>selection.value,api,operationId:()=>`operation-${++opId}`});return()=>null;}});
  app.mount({}); if(t)t.after(()=>app.unmount());
  return {b,api,calls,selection,timers,unmount:()=>app.unmount(),async tick(){const callbacks=[...timers.values()];timers.clear();for(const fn of callbacks)fn();await flush();}};
}
function compileComponent(relative, imports) {
  const filename=path.join(__dirname,'..',relative);
  const {descriptor}=parse(source(relative),{filename});
  const script=compileScript(descriptor,{id:relative});
  const template=compileTemplate({source:descriptor.template.content,filename,id:relative,compilerOptions:{bindingMetadata:script.bindings}});
  assert.deepEqual(template.errors,[]);
  const exports={};new Function('require','exports',transpile(script.content))(imports,exports);
  const rendered={};new Function('require','exports',transpile(template.code))(require,rendered);
  return {...exports.default,render:rendered.render};
}
async function renderWorkspace(props, stateOverrides={}) {
  const empty={viewActive:vue.ref(true),conversations:vue.ref([]),total:vue.ref(0),includeArchived:vue.ref(false),query:vue.ref(''),selectedId:vue.ref(''),snapshot:vue.ref(),draft:vue.ref(''),titleDraft:vue.ref(''),loadingList:vue.ref(false),loading:vue.ref(false),loadingEarlier:vue.ref(false),busy:vue.ref(false),cancelling:vue.ref(false),listError:vue.ref(''),readError:vue.ref(''),actionError:vue.ref(''),hasListSnapshot:vue.ref(true),pendingSend:vue.ref(),pendingDelete:vue.ref(),canDelete:vue.ref(true),writable:vue.ref(true),replyActive:vue.ref(false),messageBytes:vue.ref(0),messageTooLong:vue.ref(false),canSend:vue.ref(false),queuedSteer:vue.ref(),steering:vue.ref(false),steerSupported:vue.ref(true),steerSaving:vue.ref(false),steerNotice:vue.ref(''),canSteer:vue.ref(false),canSendSteer:vue.ref(false),...stateOverrides};
  empty.composerDraft=stateOverrides.composerDraft||empty.draft;empty.composerBytes=stateOverrides.composerBytes||empty.messageBytes;empty.composerTooLong=stateOverrides.composerTooLong||empty.messageTooLong;
  const component=compileComponent('src/features/conversations/ConversationWorkspace.vue',name=>{
    if(name==='vue')return vue;
    if(name==='@lucide/vue')return new Proxy({},{get:()=>()=>null});
    if(name==='../../i18n')return {useI18n:()=>({tr:(_,en)=>en})};
    if(name==='./api')return {conversationApi:{}};
    if(name==='./ConversationAccessPanel.vue')return {default:{props:['projectId','conversationId'],render:()=>null}};
    if(name==='./ConversationToolActivity.vue')return {default:{props:['activity'],setup:props=>()=>props.activity.length?vue.h('div',{class:'conversation-tool-activity'},props.activity.map(item=>item.summary).join(' ')):null}};
    if(name==='./ConversationMessage.vue')return {default:{props:['message'],setup:props=>()=>vue.h('article',{class:`conversation-message ${props.message.role}`},props.message.content)}};
    if(name==='./useConversationWorkspace')return {useConversationWorkspace:()=>empty};
    throw Error(`Unexpected UI dependency ${name}`);
  });
  return renderToString(vue.createSSRApp(component,props));
}
test('chat is a separate feature with no scan, directive or execution API imports',()=>{
  for(const file of fs.readdirSync(path.join(__dirname,'../src/features/conversations'))){
    const text=source(`src/features/conversations/${file}`);
    assert.doesNotMatch(text,/from ["'][^"']*(sentinel|AgentDialog|agent_runtime)/,file);
    assert.doesNotMatch(text,/\b(startScan|draftScanDirective|confirmScanDirective|startJob)\s*\(/,file);
  }
});
test('standalone IPC wrapper always passes the exact project-scoped input',async()=>{
  const calls=[],exports={};
  new Function('require','exports',transpile(source('src/features/conversations/api.ts')))(name=>{assert.equal(name,'@tauri-apps/api/core');return {invoke:async(...args)=>calls.push(args)};},exports);
  const input={projectId:1,conversationId:'a',expectedRevision:3,operationId:'op',content:'text',requestReply:true};
  await exports.conversationApi.sendConversationMessage(input);
  assert.deepEqual(calls,[['send_conversation_message',{input}]]);
});
test('first run offers shared project creation; existing workspaces never require another project',async()=>{
  const blank=await renderWorkspace({projects:[]});
  assert.match(blank,/Create workspace/);
  const existing=await renderWorkspace({projects:[project()]});
  assert.match(existing,/Choose an existing workspace/);assert.match(existing,/Project 1/);assert.doesNotMatch(existing,/Create workspace/);
});
test('unavailable model truth renders saved user text without fabricated assistant output',async()=>{
  const data=snapshot('a',1,{messages:[message(1)],replyAvailability:{available:false,code:'missing',message:'Configure a model first',model:null}});
  const html=await renderWorkspace({projects:[project()],projectId:1},{selectedId:vue.ref('a'),snapshot:vue.ref(data)});
  assert.match(html,/Configure a model first/);assert.match(html,/Save message/);assert.match(html,/message 1/);assert.doesNotMatch(html,/class="conversation-message assistant"/);
});
test('failed first list stays an error rather than a successful empty list',async t=>{
  const m=mount(t,{listConversations:async()=>{throw Error('database unavailable');}});await flush();
  assert.equal(m.b.hasListSnapshot.value,false);assert.match(m.b.listError.value,/database unavailable/);
});
test('project switches fence late list and transcript reads, including same conversation IDs',async t=>{
  const oldList=deferred(),oldRead=deferred();
  const m=mount(t,{listConversations:input=>input.projectId===1?oldList.promise:Promise.resolve({items:[conversation('a',2)],total:1}),readConversation:input=>input.projectId===1?oldRead.promise:Promise.resolve(snapshot('a',2))});
  m.b.selectConversation('a');m.selection.value=project(2);await flush();m.b.selectConversation('a');await flush();
  oldList.resolve({items:[conversation('old',1)],total:1});oldRead.resolve(snapshot('a',1));await flush();
  assert.equal(m.b.conversations.value[0].projectId,2);assert.equal(m.b.snapshot.value.conversation.projectId,2);
});
test('reopening and polling an active reply only read committed state',async t=>{
  const m=mount(t,{readConversation:async input=>snapshot(input.conversationId,input.projectId,{latestReply:reply()})});
  m.b.selectConversation('a');await flush();assert.equal(m.timers.size,1);await m.tick();
  assert.equal(m.calls.filter(([name])=>name==='readConversation').length,2);
  assert.equal(m.calls.filter(([name])=>/send|create|update|cancel/.test(name)).length,0);
});
test('double send is fenced and unavailable models are explicitly save-only',async t=>{
  const send=deferred();
  const m=mount(t,{readConversation:async()=>snapshot('a',1,{replyAvailability:{available:false,code:'missing',message:'No model',model:null}}),sendConversationMessage:()=>send.promise});
  m.b.selectConversation('a');await flush();m.b.draft.value='hello';const pending=m.b.send();await m.b.send();
  const calls=m.calls.filter(([name])=>name==='sendConversationMessage');assert.equal(calls.length,1);assert.equal(calls[0][1].requestReply,false);
  send.resolve({conversation:conversation('a'),userMessage:message(1),reply:null});await pending;assert.equal(m.b.draft.value,'');assert.equal(m.b.busy.value,false);
});
test('unknown send failure retries identical payload and idempotency key only after explicit action',async t=>{
  let sends=0;const m=mount(t,{sendConversationMessage:async input=>{if(++sends===1)throw Error('connection lost');return {conversation:conversation('a'),userMessage:message(1),reply:null};}});
  m.b.selectConversation('a');await flush();m.b.draft.value='hello';await m.b.send();
  const first=m.calls.find(([name])=>name==='sendConversationMessage')[1];assert.equal(m.b.pendingSend.value.operationId,first.operationId);assert.equal(m.b.draft.value,'hello');
  await m.b.readSelected();assert.equal(sends,1);await m.b.send();
  assert.deepEqual(m.calls.filter(([name])=>name==='sendConversationMessage').map(([,input])=>input),[first,first]);
});
test('deterministic pre-save revision conflict preserves text and permits a fresh explicit send',async t=>{
  let revision=1,sends=0;const m=mount(t,{readConversation:async()=>snapshot('a',1,{conversation:conversation('a',1,{revision})}),sendConversationMessage:async input=>{if(++sends===1){revision=2;throw 'conversation_revision_conflict';}return {conversation:conversation('a',1,{revision:3}),userMessage:message(1),reply:null};}});
  m.b.selectConversation('a');await flush();m.b.draft.value='hello';await m.b.send();
  assert.equal(m.b.pendingSend.value,undefined);assert.equal(m.b.draft.value,'hello');assert.equal(m.b.snapshot.value.conversation.revision,2);
  await m.b.send();const calls=m.calls.filter(([name])=>name==='sendConversationMessage');assert.deepEqual(calls.map(([,input])=>input.expectedRevision),[1,2]);assert.notEqual(calls[0][1].operationId,calls[1][1].operationId);
});
test('old send completion cannot clear the new project draft or release its active operation',async t=>{
  const old=deferred(),fresh=deferred();const m=mount(t,{sendConversationMessage:input=>input.projectId===1?old.promise:fresh.promise});
  m.b.selectConversation('a');await flush();m.b.draft.value='project one';const first=m.b.send();
  m.selection.value=project(2);await flush();m.b.selectConversation('a');await flush();m.b.draft.value='project two';const second=m.b.send();
  old.resolve({conversation:conversation('a',1),userMessage:message(1),reply:null});await first;
  assert.equal(m.b.draft.value,'project two');assert.equal(m.b.busy.value,true);assert.equal(m.b.snapshot.value.conversation.projectId,2);
  fresh.resolve({conversation:conversation('a',2),userMessage:message(1),reply:null});await second;assert.equal(m.b.busy.value,false);
});
test('pending creation is single-flight and a late result cannot select a different project',async t=>{
  const create=deferred();const m=mount(t,{createConversation:()=>create.promise});await flush();const first=m.b.createConversation();await m.b.createConversation();
  assert.equal(m.calls.filter(([name])=>name==='createConversation').length,1);
  m.selection.value=project(2);await flush();create.resolve(conversation('new',1));await first;assert.equal(m.b.selectedId.value,'');assert.equal(m.b.busy.value,false);
});
test('archived projects and conversations reject writes while retaining readable history',async t=>{
  const m=mount(t,{readConversation:async()=>snapshot('a',1,{conversation:conversation('a',1,{archived:true})})});m.b.selectConversation('a');await flush();m.b.draft.value='hello';await m.b.send();assert.equal(m.b.canSend.value,false);
  m.selection.value=project(1,'archived');await flush();await m.b.createConversation();await m.b.updateConversation({archived:false});await m.b.cancelReply();
  assert.equal(m.calls.filter(([name])=>/^(send|create|update|cancel)/.test(name)).length,0);assert.equal(m.b.snapshot.value.conversation.id,'a');
});
test('earlier pages remain present when polling a newer tail',async t=>{
  let read=0;const m=mount(t,{readConversation:async input=>input.beforeSequence?snapshot('a',1,{messages:[message(1)],hasMore:false}):snapshot('a',1,{messages:++read===1?[message(2)]:[message(2),message(3,'a','assistant')],hasMore:true})});
  m.b.selectConversation('a');await flush();await m.b.loadEarlier();await m.b.readSelected();
  assert.deepEqual(m.b.snapshot.value.messages.map(item=>item.sequence),[1,2,3]);assert.equal(m.b.snapshot.value.hasMore,false);
});
test('cancel reply is single-flight and never treated as another send',async t=>{
  const stop=deferred();const m=mount(t,{readConversation:async()=>snapshot('a',1,{latestReply:reply()}),cancelConversationReply:()=>stop.promise});m.b.selectConversation('a');await flush();const first=m.b.cancelReply();await m.b.cancelReply();
  assert.equal(m.calls.filter(([name])=>name==='cancelConversationReply').length,1);stop.resolve(reply('cancel_requested'));await first;
  assert.equal(m.calls.filter(([name])=>name==='sendConversationMessage').length,0);
});
test('unmount prevents late writes from mutating retained state and stops polling',async()=>{
  const send=deferred();const m=mount(null,{sendConversationMessage:()=>send.promise});m.b.selectConversation('a');await flush();m.b.draft.value='preserve';const operation=m.b.send();m.unmount();send.resolve({conversation:conversation('a'),userMessage:message(1),reply:null});await operation;
  assert.equal(m.b.draft.value,'preserve');assert.equal(m.timers.size,0);
});
test('message byte ceiling blocks oversized UTF-8 before IPC and correcting text remains possible',async t=>{
  const m=mount(t);m.b.selectConversation('a');await flush();
  for(const content of ['x'.repeat(20_000),'字'.repeat(6_000)]){
    m.b.draft.value=content;assert.equal(m.b.messageTooLong.value,true);assert.equal(m.b.canSend.value,false);await m.b.send();
    assert.equal(m.calls.filter(([name])=>name==='sendConversationMessage').length,0);assert.equal(m.b.pendingSend.value,undefined);
  }
  m.b.draft.value='short enough';assert.equal(m.b.messageTooLong.value,false);await m.b.send();assert.equal(m.calls.filter(([name])=>name==='sendConversationMessage').length,1);
});
test('deterministic server message rejection leaves draft editable for correction',async t=>{
  let sends=0;const m=mount(t,{sendConversationMessage:async()=>{if(++sends===1)throw 'conversation_invalid_message';return {conversation:conversation('a'),userMessage:message(1),reply:null};}});
  m.b.selectConversation('a');await flush();m.b.draft.value='rejected';await m.b.send();assert.equal(m.b.pendingSend.value,undefined);
  m.b.draft.value='corrected';await m.b.send();assert.equal(sends,2);
});
test('history search is debounced, project-scoped and rejects older responses during the debounce',async t=>{
  const slow=deferred();const m=mount(t,{listConversations:async input=>input.query==='old'?slow.promise:{items:[conversation(input.query||'all',input.projectId)],total:1}});await flush();
  m.b.query.value='o';m.b.query.value='old';assert.equal(m.calls.filter(([name])=>name==='listConversations').length,1);
  await m.tick();assert.equal(m.calls.filter(([name])=>name==='listConversations').length,2);
  m.b.query.value='new';slow.resolve({items:[conversation('wrong')],total:1});await flush();assert.equal(m.b.conversations.value.length,0);
  await m.tick();assert.equal(m.b.conversations.value[0].id,'new');
  const input=m.calls.filter(([name])=>name==='listConversations').at(-1)[1];assert.equal(input.query,'new');assert.equal(input.projectId,1);assert.equal(input.offset,0);
  m.selection.value=project(2);await flush();assert.equal(m.b.conversations.value[0].projectId,2);
});
test('a non-overlapping latest tail never presents missing messages as continuous history',async t=>{
  let reads=0;const m=mount(t,{readConversation:async()=>++reads===1?snapshot('a',1,{messages:[message(1),message(2)],hasMore:false}):snapshot('a',1,{messages:[message(200),message(201)],hasMore:true})});
  m.b.selectConversation('a');await flush();await m.b.readSelected();assert.deepEqual(m.b.snapshot.value.messages.map(item=>item.sequence),[200,201]);assert.equal(m.b.snapshot.value.hasMore,true);
});
test('deletion requires inline confirmation and removes only the selected conversation after success',async t=>{
  const deleting=deferred();let deleted=false;const m=mount(t,{listConversations:async()=>({items:deleted?[conversation('b')]:[conversation('a'),conversation('b')],total:deleted?1:2}),deleteConversation:async()=>{await deleting.promise;deleted=true;}});await flush();m.b.selectConversation('a');await flush();m.b.draft.value='unsent draft';
  await m.b.confirmDelete();assert.equal(m.calls.filter(([name])=>name==='deleteConversation').length,0);
  m.b.requestDelete();const operation=m.b.confirmDelete();assert.equal(m.b.selectedId.value,'a');assert.equal(m.b.draft.value,'unsent draft');await m.b.confirmDelete();m.b.cancelDelete();assert.ok(m.b.pendingDelete.value);
  assert.deepEqual(m.calls.filter(([name])=>name==='deleteConversation'),[['deleteConversation',{projectId:1,conversationId:'a',expectedRevision:1}]]);
  deleting.resolve();await operation;assert.equal(m.b.selectedId.value,'');assert.equal(m.b.draft.value,'');assert.deepEqual(m.b.conversations.value.map(item=>item.id),['b']);assert.equal(m.b.total.value,1);
  assert.equal(m.calls.filter(([name])=>/Project|Asset|Scan/.test(name)).length,0);
});
test('refused deletion preserves conversation, draft and unrelated history',async t=>{
  const m=mount(t,{deleteConversation:async()=>{throw 'conversation_reply_active';}});m.b.selectConversation('a');await flush();m.b.draft.value='keep this';m.b.requestDelete();await m.b.confirmDelete();
  assert.equal(m.b.selectedId.value,'a');assert.equal(m.b.draft.value,'keep this');assert.equal(m.b.conversations.value[0].id,'a');assert.match(m.b.actionError.value,/conversation_reply_active/);
});
test('a reply or changed revision after delete prompt cannot authorize a stale deletion',async t=>{
  const m=mount(t);m.b.selectConversation('a');await flush();m.b.draft.value='preserve';m.b.requestDelete();m.b.snapshot.value.latestReply=reply();await m.b.confirmDelete();assert.equal(m.calls.filter(([name])=>name==='deleteConversation').length,0);
  m.b.snapshot.value.latestReply=reply('completed');m.b.snapshot.value.conversation.revision=2;await m.b.confirmDelete();assert.equal(m.calls.filter(([name])=>name==='deleteConversation').length,0);assert.equal(m.b.draft.value,'preserve');assert.equal(m.b.pendingDelete.value,undefined);
});
test('late delete completion cannot clear a newer project selection, draft or list',async t=>{
  const deleting=deferred();const m=mount(t,{deleteConversation:()=>deleting.promise});m.b.selectConversation('a');await flush();m.b.requestDelete();const operation=m.b.confirmDelete();
  m.selection.value=project(2);await flush();m.b.selectConversation('a');await flush();m.b.draft.value='new workspace draft';deleting.resolve();await operation;
  assert.equal(m.b.selectedId.value,'a');assert.equal(m.b.draft.value,'new workspace draft');assert.equal(m.b.conversations.value[0].projectId,2);assert.equal(m.b.snapshot.value.conversation.projectId,2);
});
test('delete confirmation renders irreversible scope explicitly without authorizing business deletion',async()=>{
  const html=await renderWorkspace({projects:[project()],projectId:1},{selectedId:vue.ref('a'),snapshot:vue.ref(snapshot()),pendingDelete:vue.ref({projectId:1,conversationId:'a',expectedRevision:1})});
  assert.match(html,/Delete conversation permanently/);assert.match(html,/cannot be undone/);assert.match(html,/Projects, assets, scans, and evidence are kept/);
});
test('a list issued before successful deletion cannot resurrect the removed conversation',async t=>{
  const late=deferred();let reads=0;const m=mount(t,{listConversations:async()=>++reads===1?{items:[conversation('a'),conversation('b')],total:2}:reads===2?late.promise:{items:[conversation('b')],total:1}});await flush();m.b.selectConversation('a');await flush();
  const pending=m.b.loadList();m.b.requestDelete();await m.b.confirmDelete();await flush();late.resolve({items:[conversation('a'),conversation('b')],total:2});await pending;
  assert.deepEqual(m.b.conversations.value.map(item=>item.id),['b']);assert.equal(m.b.total.value,1);assert.equal(m.b.selectedId.value,'');
});
test('deleting a selected conversation absent from the current search leaves its total intact',async t=>{
  const refreshed=deferred();let reads=0;const m=mount(t,{listConversations:async()=>++reads===1?{items:[conversation('a')],total:1}:reads===2?{items:[conversation('b')],total:1}:refreshed.promise});await flush();m.b.selectConversation('a');await flush();
  m.b.query.value='find b';await m.tick();assert.equal(m.b.total.value,1);m.b.requestDelete();await m.b.confirmDelete();assert.equal(m.b.total.value,1);assert.deepEqual(m.b.conversations.value.map(item=>item.id),['b']);
  refreshed.resolve({items:[conversation('b')],total:1});await flush();assert.equal(m.b.total.value,1);
});
test('an earlier page anchored to an old tail cannot bridge a gap after concurrent refresh',async t=>{
  const earlier=deferred();let reads=0;const range=(from,to)=>Array.from({length:to-from+1},(_,index)=>message(from+index));
  const m=mount(t,{readConversation:async input=>input.beforeSequence?earlier.promise:++reads===1?snapshot('a',1,{messages:range(101,200),hasMore:true}):snapshot('a',1,{messages:range(301,400),hasMore:true})});m.b.selectConversation('a');await flush();
  const pending=m.b.loadEarlier();await m.b.readSelected();earlier.resolve(snapshot('a',1,{messages:range(1,100),hasMore:false}));await pending;
  assert.deepEqual(m.b.snapshot.value.messages.map(item=>item.sequence),range(301,400).map(item=>item.sequence));assert.equal(m.b.snapshot.value.hasMore,true);assert.equal(m.b.loadingEarlier.value,false);
});


test('active reply puts one stop action in the composer and reports committed reply semantics',async()=>{
  const html=await renderWorkspace({projects:[project()],projectId:1},{selectedId:vue.ref('a'),snapshot:vue.ref(snapshot('a',1,{latestReply:reply()})),replyActive:vue.ref(true)});
  assert.equal((html.match(/aria-label="Stop reply"/g)||[]).length,1);
  assert.match(html,/<form class="conversation-composer"[\s\S]*aria-label="Stop reply"/);
  assert.doesNotMatch(html,/aria-label="Send"/);
  assert.match(html,/The complete reply will appear when ready/);
});

test('chat chrome keeps management discoverable in a disclosure and retry explicit',async()=>{
  const html=await renderWorkspace({projects:[project()],projectId:1},{selectedId:vue.ref('a'),snapshot:vue.ref(snapshot()),pendingSend:vue.ref({operationId:'same-request'})});
  assert.match(html,/<details class="conversation-actions"/);
  assert.match(html,/Conversation options/);
  assert.match(html,/Retry reuses the same request/);
  assert.match(html,/aria-label="Retry send"/);
  assert.match(html,/role="log"/);
  assert.match(html,/aria-relevant="additions"/);
});

test('shell owns a stable hidden history target and starts collapsed without conditioning workspace on collapse',()=>{
  const app=source('src/App.vue');
  assert.match(app,/sidebarCollapsed = ref\(true\)/);
  assert.match(app,/id="conversation-history" class="shell-conversation-history"/);
  const workspace=app.match(/<ConversationWorkspace[^>]+>/)[0];
  assert.match(workspace,/history-target="#conversation-history"/);
  assert.doesNotMatch(workspace,/sidebarCollapsed/);
  assert.match(app,/aria-controls="app-navigation"/);
  assert.match(app,/:inert="isMobile&&!sidebarCollapsed"/);
  assert.match(source('src/features/conversations/ConversationWorkspace.vue'),/<Teleport :to="historyTarget\|\|'body'" :disabled="!historyTarget">/);
});

test('chat scroll uses a bounded flex viewport and preserves access at small sizes',()=>{
  const workspace=source('src/features/conversations/ConversationWorkspace.vue');
  const shell=source('src/shell/agent-shell.css');
  assert.doesNotMatch(workspace,/max-height:calc\(100vh - 410px\)|max-height:55vh/);
  assert.match(workspace,/conversation-reading-area\{[^}]*flex:1;min-height:0/);
  assert.match(workspace,/oldTop \+ element.scrollHeight - oldHeight/);
  assert.match(workspace,/event.keyCode !== 229/);
  assert.match(shell,/height:100dvh/);
  assert.match(shell,/prefers-reduced-motion:reduce/);
});


test('Steer leaves active reply composer editable and names next-reply queue semantics',async()=>{
  const html=await renderWorkspace({projects:[project()],projectId:1},{selectedId:vue.ref('a'),snapshot:vue.ref(snapshot('a',1,{latestReply:reply()})),replyActive:vue.ref(true),busy:vue.ref(true),canSteer:vue.ref(true)});
  assert.doesNotMatch(html.match(/<textarea[^>]+>/)[0],/disabled/);
  assert.match(html,/Enter to queue next reply/);
  assert.match(html,/does not change the reply in progress/);
  assert.match(html,/>Steer · Next reply<\/button>/);
  assert.match(html,/aria-label="Stop reply"/);
});

test('uncertain queued Steer offers explicit retry and never a discard action',async()=>{
  const html=await renderWorkspace({projects:[project()],projectId:1},{selectedId:vue.ref('a'),snapshot:vue.ref(snapshot()),canSendSteer:vue.ref(true),queuedSteer:vue.ref({content:'Use the shorter approach',status:'uncertain',reason:'dispatch_unknown',input:{operationId:'same-request'}})});
  assert.match(html,/Delivery is uncertain/);assert.match(html,/Retry steer/);
  assert.doesNotMatch(html,/Move back to composer/);
  assert.match(html,/Use the shorter approach/);
});


test('unsupported Steer explains safe queue requirement without hiding ordinary chat',async()=>{
  const html=await renderWorkspace({projects:[project()],projectId:1},{selectedId:vue.ref('a'),snapshot:vue.ref(snapshot('a',1,{latestReply:reply()})),replyActive:vue.ref(true),steerSupported:vue.ref(false)});
  assert.match(html,/Steer needs safe local queue support\. Ordinary chat still works/);
  assert.match(html,/aria-describedby="composer-guidance steer-support-notice"/);
  assert.match(html,/class="composer-steer" disabled/);
  assert.match(html,/aria-label="Stop reply"/);
});

test('unsupported saved queue requires recovery and disables returning it to composer',async()=>{
  const html=await renderWorkspace({projects:[project()],projectId:1},{selectedId:vue.ref('a'),snapshot:vue.ref(snapshot()),steerSupported:vue.ref(false),queuedSteer:vue.ref({content:'Review this direction',status:'paused',reason:'reopened'})});
  assert.match(html,/safe local queue support to review this saved direction before continuing/);
  assert.doesNotMatch(html,/Ordinary chat still works/);
  assert.match(html,/<button[^>]*disabled[^>]*>Move back to composer<\/button>/);
});

test('atomic queue save exposes progress and disables management and return actions',async()=>{
  const html=await renderWorkspace({projects:[project()],projectId:1},{selectedId:vue.ref('a'),snapshot:vue.ref(snapshot()),steerSaving:vue.ref(true),queuedSteer:vue.ref({content:'Queue this',status:'paused',reason:'reopened'})});
  assert.match(html,/Saving direction safely/);
  assert.match(html,/<button[^>]*disabled[^>]*>Move back to composer<\/button>/);
  assert.match(html,/class="conversation-new-chat" disabled/);
});


test('polling AI title preserves a dirty manual rename draft',async t=>{
 let title='Original';const m=mount(t,{readConversation:async()=>snapshot('a',1,{conversation:conversation('a',1,{title,titleSource:'ai'})})});m.b.selectConversation('a');await flush();
 m.b.titleDraft.value='My edited title';title='Generated title';await m.b.readSelected();assert.equal(m.b.titleDraft.value,'My edited title');assert.equal(m.b.snapshot.value.conversation.title,'Generated title');
 m.b.titleDraft.value='';title='Another title';await m.b.readSelected();assert.equal(m.b.titleDraft.value,'');
});
test('polling title updates a pristine rename field',async t=>{
 let title='Original';const m=mount(t,{readConversation:async()=>snapshot('a',1,{conversation:conversation('a',1,{title})})});m.b.selectConversation('a');await flush();title='AI title';await m.b.readSelected();assert.equal(m.b.titleDraft.value,'AI title');
});
const toolActivity=(id,userMessageId)=>({id,replyId:'reply-'+id,userMessageId,round:1,callId:'call-'+id,name:'assets_search',status:'completed',summary:'Saved tool '+id,createdAt:'2026-10-14T01:00:00Z'});
test('tool activity follows loaded message pages without leaking from replaced history',async t=>{
 let tail=2;const m=mount(t,{readConversation:async input=>input.beforeSequence?snapshot('a',1,{messages:[message(1)],toolActivity:[toolActivity('old','a-1')],hasMore:false}):snapshot('a',1,{messages:[message(tail)],toolActivity:[toolActivity('latest','a-'+tail)],hasMore:true})});m.b.selectConversation('a');await flush();await m.b.loadEarlier();await m.b.readSelected();assert.deepEqual(new Set(m.b.snapshot.value.toolActivity.map(item=>item.id)),new Set(['old','latest']));
 tail=200;await m.b.readSelected();assert.deepEqual(m.b.snapshot.value.toolActivity.map(item=>item.userMessageId),['a-200']);
});
test('tool activity can attach to an assistant page when its parent user message is older',async t=>{
 const answer={...message(2,'a','assistant'),replyToMessageId:'a-1'};const m=mount(t,{readConversation:async()=>snapshot('a',1,{messages:[answer],toolActivity:[toolActivity('saved','a-1')]})});m.b.selectConversation('a');await flush();assert.equal(m.b.snapshot.value.toolActivity.length,1);
});
test('invalid tool activity is a read error, never simulated execution',async t=>{
 const m=mount(t,{readConversation:async()=>snapshot('a',1,{toolActivity:[{...toolActivity('bad','a-1'),status:'executing_shell'}]})});m.b.selectConversation('a');await flush();assert.match(m.b.readError.value,/Invalid saved tool activity/);assert.equal(m.b.snapshot.value,undefined);
});
