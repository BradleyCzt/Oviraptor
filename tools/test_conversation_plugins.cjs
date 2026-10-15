const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const ts=require('typescript');
const vue=require('vue');
const {parse,compileScript,compileTemplate}=require('@vue/compiler-sfc');
const {renderToString}=require('@vue/server-renderer');
const source=relative=>fs.readFileSync(path.join(__dirname,'..',relative),'utf8');
const transpile=text=>ts.transpileModule(text,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText;
const flush=async()=>{for(let i=0;i<24;i++)await vue.nextTick();};
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return{promise,resolve,reject};};
const chat=(id='a',projectId=1,revision=1,links=[])=>({conversation:{id,projectId,revision,title:'Chat',archived:false,messageCount:0,createdAt:'t',updatedAt:'t',lastMessagePreview:''},messages:[],links,hasMore:false,replyAvailability:{available:false,code:'no-model',message:'No model',model:null},latestReply:null});
const candidate=(targetId='10',projectId=1,kind='asset')=>({targetId,projectId,kind,label:`Record ${targetId}`,detail:'Existing local record'});
const link=(targetId='10',kind='asset',available=true)=>({targetId,kind,label:`Record ${targetId}`,available});
function moduleFrom(file,imports){const exports={};new Function('require','exports',transpile(source(file)))(imports,exports);return exports;}
function mountPicker(t,options={}){
  const timers=new Map();let timerId=0;const exports={};
  new Function('require','exports','setTimeout','clearTimeout',transpile(source('src/features/conversations/useConversationPlugins.ts')))(name=>{assert.equal(name,'vue');return vue;},exports,fn=>{timers.set(++timerId,fn);return timerId;},id=>timers.delete(id));
  const snapshot=vue.ref(options.snapshot??chat()),projectId=vue.ref(1),active=vue.ref(true),blocked=vue.ref(false),enabled=vue.ref({asset:true,sentinel:true,hackerone:true});
  const calls=[];
  const host={isEnabled:id=>enabled.value[id],listLinkCandidates:async input=>{calls.push(['list',input]);return options.list?options.list(input):{items:[candidate(input.pluginId==='asset'?'10':'scan-1',input.projectId,input.pluginId==='asset'?'asset':'scan')]};},openWorkspace:intent=>{calls.push(['open',intent]);if(options.open)return options.open(intent);},invalidate(){}};
  let state;const renderer=vue.createRenderer({createComment:()=>({}),insert(){},remove(){},parentNode:()=>null,nextSibling:()=>null});
  const app=renderer.createApp({setup(){state=exports.useConversationPlugins({host:()=>host,snapshot:()=>snapshot.value,projectId:()=>projectId.value,projectActive:()=>active.value,blocked:()=>blocked.value,save:async input=>{calls.push(['save',input]);return options.save?options.save(input):true;}});return()=>null;}});app.mount({});t.after(()=>app.unmount());
  return{state,host,snapshot,projectId,active,blocked,enabled,calls,unmount:()=>app.unmount(),async tick(){const jobs=[...timers.values()];timers.clear();for(const job of jobs)job();await flush();}};
}
test('plugin panel does not query records or dispatch on Chat mount',async t=>{
  const m=mountPicker(t);await flush();assert.deepEqual(m.calls,[]);assert.equal(m.state.editing.value,false);
  m.state.openWorkspace('asset');assert.deepEqual(m.calls,[['open',{pluginId:'asset',projectId:1,page:'assets'}]]);
});
test('picker reads real scoped candidates and saves only typed references with original revision',async t=>{
  const m=mountPicker(t,{snapshot:chat('a',1,7,[link('old-scan','scan')])});m.enabled.value.sentinel=false;
  m.state.openEditor('asset');await flush();m.state.addCandidate(m.state.candidates.value[0]);await m.state.save();
  assert.deepEqual(m.calls.find(([op])=>op==='list')[1],{pluginId:'asset',projectId:1,search:'',cursor:undefined});
  assert.deepEqual(m.calls.find(([op])=>op==='save')[1],{projectId:1,conversationId:'a',expectedRevision:7,links:[{kind:'scan',targetId:'old-scan'},{kind:'asset',targetId:'10'}]});assert.equal(m.state.editing.value,false);
});
test('cross-project or wrong-kind candidate responses are rejected before display',async t=>{
  for(const bad of [candidate('wrong',2),candidate('wrong',1,'scan')]){
    const m=mountPicker(t,{list:async()=>({items:[bad]})});m.state.openEditor();await flush();
    assert.deepEqual(m.state.candidates.value,[]);assert.match(m.state.error.value,/do not match/);m.state.addCandidate(bad);assert.deepEqual(m.state.draftLinks.value,[]);
  }
});
test('project round-trip invalidates old lookup even when conversation ID returns',async t=>{
  const old=deferred();let calls=0;const m=mountPicker(t,{list:()=>++calls===1?old.promise:Promise.resolve({items:[candidate('current')]})});
  m.state.openEditor();m.projectId.value=2;m.snapshot.value=chat('a',2);await flush();m.projectId.value=1;m.snapshot.value=chat();m.state.openEditor();await flush();old.resolve({items:[candidate('stale')]});await flush();
  assert.deepEqual(m.state.candidates.value.map(item=>item.targetId),['current']);
});
test('search invalidates old lookup before debounce and passes only latest query',async t=>{
  const old=deferred();const m=mountPicker(t,{list:input=>input.search?Promise.resolve({items:[candidate('search')]}):old.promise});m.state.openEditor();m.state.query.value='new';old.resolve({items:[candidate('old')]});await flush();assert.deepEqual(m.state.candidates.value,[]);await m.tick();assert.equal(m.state.candidates.value[0].targetId,'search');assert.equal(m.calls.filter(([op])=>op==='list').at(-1)[1].search,'new');
});
test('disabled plugin fences pending lookup and newly selected references',async t=>{
  const old=deferred();const m=mountPicker(t,{list:()=>old.promise});m.state.openEditor();m.enabled.value.asset=false;old.resolve({items:[candidate()]});await flush();assert.deepEqual(m.state.candidates.value,[]);assert.equal(m.state.loading.value,false);
  const next=mountPicker(t);next.state.openEditor();await flush();next.state.addCandidate(next.state.candidates.value[0]);assert.equal(next.state.canSave.value,true);next.enabled.value.asset=false;assert.equal(next.state.disabledNewLinks.value,true);assert.equal(next.state.canSave.value,false);await next.state.save();assert.equal(next.calls.some(([op])=>op==='save'),false);
});
test('historical unavailable references remain until explicit removal',async t=>{
  const m=mountPicker(t,{snapshot:chat('a',1,4,[link('gone','scan',false),link('keep')])});m.state.openEditor();await flush();assert.equal(m.state.draftLinks.value.length,2);assert.equal(m.state.hasUnavailable.value,true);assert.equal(m.state.canSave.value,false);
  m.state.removeLink(link('gone','scan',false));await m.state.save();assert.deepEqual(m.calls.find(([op])=>op==='save')[1].links,[{kind:'asset',targetId:'keep'}]);
});
test('revision changes require explicit picker reopening before link save',async t=>{
  const m=mountPicker(t);m.state.openEditor();await flush();m.state.addCandidate(m.state.candidates.value[0]);m.snapshot.value=chat('a',1,2);assert.equal(m.state.stale.value,true);await m.state.save();assert.equal(m.calls.some(([op])=>op==='save'),false);
  m.state.closeEditor();m.state.openEditor();await flush();m.state.addCandidate(m.state.candidates.value[0]);await m.state.save();assert.equal(m.calls.find(([op])=>op==='save')[1].expectedRevision,2);
});
test('active reply, archived project and archived conversation block edits but not guarded navigation',async t=>{
  const m=mountPicker(t);m.blocked.value=true;m.state.openEditor();assert.equal(m.state.editing.value,false);m.blocked.value=false;m.active.value=false;m.state.openEditor();assert.equal(m.state.editing.value,false);m.active.value=true;m.snapshot.value.conversation.archived=true;m.state.openEditor();assert.equal(m.state.editing.value,false);m.state.openWorkspace('sentinel');assert.deepEqual(m.calls,[['open',{pluginId:'sentinel',projectId:1,page:'sentinel',section:'queue'}]]);
});
test('close and unmount invalidate pending reads without saving references',async t=>{
  const pending=deferred();const m=mountPicker(t,{list:()=>pending.promise});m.state.openEditor();m.state.closeEditor();pending.resolve({items:[candidate()]});await flush();assert.deepEqual(m.state.candidates.value,[]);assert.equal(m.calls.some(([op])=>op==='save'),false);
});
test('repeated save has one mutation and failure retains explicit draft without retry',async t=>{
  const pending=deferred();const m=mountPicker(t,{save:()=>pending.promise});m.state.openEditor();await flush();m.state.addCandidate(m.state.candidates.value[0]);const first=m.state.save();await m.state.save();assert.equal(m.calls.filter(([op])=>op==='save').length,1);pending.resolve(false);await first;assert.equal(m.state.editing.value,true);assert.equal(m.state.draftLinks.value.length,1);assert.match(m.state.error.value,/not saved/);await m.tick();assert.equal(m.calls.filter(([op])=>op==='save').length,1);
});
test('late save cannot close another conversation picker',async t=>{
  const pending=deferred();const m=mountPicker(t,{save:()=>pending.promise});m.state.openEditor();await flush();m.state.addCandidate(m.state.candidates.value[0]);const first=m.state.save();m.snapshot.value=chat('b');m.state.openEditor();await flush();pending.resolve(true);await first;assert.equal(m.state.editing.value,true);assert.equal(m.state.saving.value,false);assert.equal(m.state.error.value,'');
});
test('pagination uses opaque returned cursor and deduplicates results',async t=>{
  const cursor={pluginId:'asset',projectId:1,search:'',page:2};const m=mountPicker(t,{list:async input=>input.cursor?{items:[candidate('10'),candidate('11')]}:{items:[candidate('10')],nextCursor:cursor}});m.state.openEditor();await flush();await m.state.loadCandidates(true);assert.deepEqual(m.calls.filter(([op])=>op==='list')[1][1].cursor,cursor);assert.deepEqual(m.state.candidates.value.map(item=>item.targetId),['10','11']);assert.equal(m.state.nextCursor.value,undefined);
});
test('switching plugins rejects old candidate selections and resets paging',async t=>{
  const m=mountPicker(t);m.state.openEditor();await flush();const old=m.state.candidates.value[0];m.state.selectPlugin('sentinel');await flush();m.state.addCandidate(old);assert.deepEqual(m.state.draftLinks.value,[]);m.state.addCandidate(m.state.candidates.value[0]);assert.equal(m.state.draftLinks.value[0].kind,'scan');
});
test('disabled navigation is inert and valid handoffs contain no fake entity deep link',async t=>{
  const m=mountPicker(t);m.enabled.value.asset=false;m.state.openWorkspace('asset');assert.deepEqual(m.calls,[]);m.state.openWorkspace('sentinel');const intent=m.calls[0][1];assert.equal(Object.hasOwn(intent,'scanId'),false);assert.equal(Object.hasOwn(intent,'assetId'),false);assert.equal(intent.projectId,1);
});
test('link limit never silently drops existing records',async t=>{
  const m=mountPicker(t,{snapshot:chat('a',1,1,Array.from({length:50},(_,i)=>link(String(100+i))))});m.state.openEditor();await flush();m.state.addCandidate(m.state.candidates.value[0]);assert.equal(m.state.draftLinks.value.length,50);assert.match(m.state.error.value,/at most 50/);
});
function mountWorkspace(t,overrides={}){
  const exports=moduleFrom('src/features/conversations/useConversationWorkspace.ts',name=>{assert.equal(name,'vue');return vue;});const project=vue.ref({id:1,status:'active'});let current=chat();const calls=[];
  const api={listConversations:async()=>({items:[current.conversation],total:1}),readConversation:async input=>overrides.read?overrides.read(input):current,setConversationLinks:async input=>{calls.push(input);return overrides.save?overrides.save(input):(current=chat('a',1,2,input.links.map(item=>({...item,label:item.targetId,available:true})))).conversation;}};
  let state;const renderer=vue.createRenderer({createComment:()=>({}),insert(){},remove(){},parentNode:()=>null,nextSibling:()=>null});const app=renderer.createApp({setup(){state=exports.useConversationWorkspace({project:()=>project.value,api});return()=>null;}});app.mount({});t.after(()=>app.unmount());return{state,project,calls};
}
test('parent link writer validates exact project conversation and revision before IPC',async t=>{
  const m=mountWorkspace(t);m.state.selectConversation('a');await flush();for(const input of [{projectId:2,conversationId:'a',expectedRevision:1},{projectId:1,conversationId:'b',expectedRevision:1},{projectId:1,conversationId:'a',expectedRevision:2}])assert.equal(await m.state.setLinks({...input,links:[]}),false);assert.deepEqual(m.calls,[]);
  assert.equal(await m.state.setLinks({projectId:1,conversationId:'a',expectedRevision:1,links:[{kind:'asset',targetId:'10'}]}),true);assert.equal(m.state.snapshot.value.conversation.revision,2);assert.equal(m.calls.length,1);
});
test('parent link writer fences late mutation result after project change',async t=>{
  const pending=deferred();const m=mountWorkspace(t,{save:()=>pending.promise,read:async input=>chat(input.conversationId,input.projectId)});m.state.selectConversation('a');await flush();const saving=m.state.setLinks({projectId:1,conversationId:'a',expectedRevision:1,links:[]});m.project.value={id:2,status:'active'};await flush();m.state.selectConversation('b');await flush();pending.resolve(chat('a',1,2).conversation);assert.equal(await saving,false);assert.equal(m.state.snapshot.value.conversation.id,'b');assert.equal(m.state.busy.value,false);
});
test('Chat plugin surface imports no direct execution or business endpoint APIs',()=>{
  for(const file of ['useConversationPlugins.ts','ConversationPluginPanel.vue']){const text=source(`src/features/conversations/${file}`);assert.doesNotMatch(text,/from ["'][^"']*(features\/assets\/api|sentinel\/api|agent_runtime)/);assert.doesNotMatch(text,/\b(startScan|startWorkbenchScan|startJob|confirmSentinelScan|sendConversationMessage)\s*\(/);}
});
test('rendered plugin panel distinguishes workspace navigation from linked evidence and execution',async()=>{
  const file='src/features/conversations/ConversationPluginPanel.vue';const {descriptor}=parse(source(file),{filename:file});const script=compileScript(descriptor,{id:file});const template=compileTemplate({source:descriptor.template.content,filename:file,id:file,compilerOptions:{bindingMetadata:script.bindings}});assert.deepEqual(template.errors,[]);
  const catalog=moduleFrom('src/plugins/catalog.ts',()=>{throw Error('Unexpected registry import');});
  const picker=moduleFrom('src/features/conversations/useConversationPlugins.ts',name=>{assert.equal(name,'vue');return vue;});
  const component=moduleFromText(script.content,name=>{if(name==='vue')return vue;if(name==='@lucide/vue')return new Proxy({},{get:()=>()=>null});if(name==='../../plugins')return catalog;if(name==='../../i18n')return{useI18n:()=>({tr:(_,en)=>en})};if(name==='./useConversationPlugins')return picker;throw Error(`Unexpected ${name}`);});
  const rendered=moduleFromText(template.code,require);const host={isEnabled:id=>id==='asset',listLinkCandidates:async()=>{throw Error('SSR must not query');},openWorkspace(){},invalidate(){}};
  const props={host,projectId:1,projectActive:true,snapshot:chat('a',1,1,[link('old-scan','scan',false)]),blocked:false,saveLinks:async()=>true};
  const collapsed=await renderToString(vue.createSSRApp({...component.default,render:rendered.render},props));
  assert.match(collapsed,/Plugins &amp; links/);assert.match(collapsed,/aria-expanded="false"/);assert.doesNotMatch(collapsed,/chat-plugin-card|Open Assets workspace|chat-link-list/);
  const expandedPicker={useConversationPlugins:deps=>{const state=picker.useConversationPlugins(deps);state.togglePanel();return state;}};
  const expandedComponent=moduleFromText(script.content,name=>{if(name==='vue')return vue;if(name==='@lucide/vue')return new Proxy({},{get:()=>()=>null});if(name==='../../plugins')return catalog;if(name==='../../i18n')return{useI18n:()=>({tr:(_,en)=>en})};if(name==='./useConversationPlugins')return expandedPicker;throw Error(`Unexpected ${name}`);});
  const html=await renderToString(vue.createSSRApp({...expandedComponent.default,render:rendered.render},props));
  assert.match(html,/aria-expanded="true"/);assert.match(html,/built-in plugins/);assert.match(html,/Open Assets workspace/);assert.match(html,/Open Scanning workspace/);assert.match(html,/do not send records to the model or start scans/);assert.match(html,/Record unavailable; reference retained/);assert.match(html,/Disabled/);assert.doesNotMatch(html,/Run scan|Scan started|Evidence imported/);
});
function moduleFromText(text,imports){const exports={};new Function('require','exports',transpile(text))(imports,exports);return exports;}

test('compact panel defaults closed, reopening preserves picks and scope switch clears only picker state',async t=>{
  const m=mountPicker(t);assert.equal(m.state.expanded.value,false);m.state.togglePanel();assert.equal(m.state.expanded.value,true);assert.deepEqual(m.calls,[]);
  m.state.openEditor();await flush();m.state.addCandidate(m.state.candidates.value[0]);m.state.togglePanel();assert.equal(m.state.expanded.value,false);m.state.togglePanel();assert.equal(m.state.draftLinks.value.length,1);assert.equal(m.calls.filter(([op])=>op==='list').length,1);
  m.snapshot.value=chat('b');assert.equal(m.state.expanded.value,false);assert.equal(m.state.editing.value,false);assert.deepEqual(m.state.draftLinks.value,[]);assert.equal(m.calls.some(([op])=>op==='save'),false);
});
