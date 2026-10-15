// Real AssetWorkspace setup and template, with scripted local IPC and pointer targets.
// This regression harness is not installed-App or real database acceptance.
const test=require('node:test');const assert=require('node:assert/strict');
const fs=require('node:fs');const path=require('node:path');const ts=require('typescript');
const vue=require('vue');const {parse,compileScript,compileTemplate}=require('@vue/compiler-sfc');
// Native DOM v-model wiring is exercised separately in the actual App.
const renderVue={...vue,vModelText:{},vModelCheckbox:{},vModelSelect:{}};
const filename=path.join(__dirname,'../src/features/assets/components/AssetWorkspace.vue');
const {descriptor}=parse(fs.readFileSync(filename,'utf8'),{filename});
const script=compileScript(descriptor,{id:'asset-review'});
const transpile=s=>ts.transpileModule(s,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText;
const template=compileTemplate({source:descriptor.template.content,filename,id:'asset-review',compilerOptions:{bindingMetadata:script.bindings}});
assert.deepEqual(template.errors,[]);const tmpl={};new Function('require','exports',transpile(template.code))(name=>name==='vue'?renderVue:require(name),tmpl);
const renderer=vue.createRenderer({createElement:()=>({}),createText:()=>({}),createComment:()=>({}),insert(){},remove(){},setText(){},setElementText(){},patchProp(){},parentNode:()=>null,nextSibling:()=>null});
const flush=async()=>{for(let i=0;i<12;i++)await vue.nextTick();};
const row=(id,projectId=9001)=>({id,projectId,title:`Example ${id}`,host:`example-${id}`,extra:{},decision:'pending',ownershipStatus:'unreviewed',authorizationStatus:'unknown',sentinelStatus:'not_sent'});
const page=(rows=[row(1),row(2),row(3)])=>({items:rows,total:200,memberTotal:200,summary:{all:200,pending:200,uncertain:0,confirmed:0,rejected:0,notApplicable:0,sentToAgent:0}});
function storage(){const data=new Map();return {getItem:k=>data.get(k)||null,setItem:(k,v)=>data.set(k,v),removeItem:k=>data.delete(k)};}
async function mount(overrides={},session=storage(),initialProps={},host={}){
  global.localStorage=storage();global.sessionStorage=session;global.window={addEventListener(){},removeEventListener(){},getSelection:()=>({removeAllRanges(){}})};
  global.document={elementFromPoint:()=>null};const calls=[],events=[];
  const api=new Proxy({listAssets:async()=>page(),updateAssetDecisions:async()=>3,softDeleteAssetSelections:async()=>3,...overrides},{get(target,k){return async(...args)=>{calls.push([k,...args]);assert.equal(typeof target[k],'function',`Unexpected IPC ${String(k)}`);return target[k](...args);};}});
  const exports={};new Function('require','exports',transpile(script.content))(name=>{
    if(name==='vue')return vue;if(name==='../api')return {assetApi:api};if(name==='../../sentinel/api')return {sentinelApi:api};if(name==='../../../i18n')return {useI18n:()=>({tr:zh=>zh})};
    if(name==='@lucide/vue')return new Proxy({},{get:()=>()=>null});if(name==='@tauri-apps/plugin-opener')return {openUrl:host.openUrl||(()=>assert.fail('unexpected browser'))};
    if(name.endsWith('.css'))return {};if(name.endsWith('.vue'))return {default:{render:()=>null}};throw new Error(name);
  },exports);
  let b;const props=vue.reactive({projects:[{id:9001,name:'Main'},{id:9002,name:'Other'}],selectedProjectId:9001,...initialProps});
  const component={...exports.default,setup(p,ctx){b=exports.default.setup(p,ctx);return()=>tmpl.render({},[],p,vue.proxyRefs(b),{},{});}};
  const app=renderer.createApp({setup:()=>()=>vue.h(component,{...props,onNotify:(...v)=>events.push(['notify',...v]),onChanged:()=>events.push(['changed'])})});
  app.mount({});await flush();return {b,props,calls,events,unmount:()=>app.unmount()};
}
function pointer(index,options={}){return {button:0,pointerId:1,pointerType:'mouse',clientX:10,clientY:10,target:{closest:()=>null},...options};}
function move(h,index){document.elementFromPoint=()=>({closest:()=>({getAttribute:()=>String(index),closest:()=>h.b.workspace.value})});h.b.moveRowDrag({pointerId:1,clientX:20,clientY:30,preventDefault(){}});}

test('page rule application refreshes current page without clearing filters or closing rule management',async()=>{
  const h=await mount();h.b.query.value.page=3;h.b.query.value.search='Example';h.b.query.value.conditions=[{field:'reviewTier',operator:'equals',value:'P1',join:'and'}];h.b.pageRuleOpen.value=true;
  await h.b.pageRulesChanged();assert.equal(h.b.query.value.page,3);assert.equal(h.b.query.value.search,'Example');assert.equal(h.b.query.value.conditions[0].value,'P1');assert.equal(h.b.pageRuleOpen.value,true);
  assert.equal(h.calls.at(-1)[1].page,3);assert.deepEqual(h.events,[['changed']]);h.unmount();
});
test('a stale query success or failure cannot overwrite a newer result or loading state',async()=>{
  const pending=[];const h=await mount({listAssets:q=>new Promise((resolve,reject)=>pending.push({q,resolve,reject}))});
  h.b.query.value.search='new';const newest=h.b.refresh();pending[1].resolve(page([row(9)]));await newest;
  pending[0].resolve(page([row(1)]));await flush();assert.equal(h.b.assets.value[0].id,9);assert.equal(h.b.loading.value,false);assert.deepEqual(h.events,[]);
  const old=h.b.refresh(),last=h.b.refresh();pending[2].reject(new Error('obsolete'));await old;assert.equal(h.b.loading.value,true);pending[3].resolve(page([row(10)]));await last;assert.equal(h.b.assets.value[0].id,10);assert.deepEqual(h.events,[]);h.unmount();
});
test('dragging selects an exact contiguous range, preserves other-project keys, and writes only on explicit action',async()=>{
  const h=await mount();h.b.toggleAsset(row(1,9002),true);h.b.startRowDrag(pointer(0),0);move(h,2);h.b.finishRowDrag({pointerId:1});
  assert.deepEqual(h.b.selectionPayload(),[{projectId:9002,assetId:1},{projectId:9001,assetId:1},{projectId:9001,assetId:2},{projectId:9001,assetId:3}]);
  assert.equal(h.calls.length,1);await h.b.act('confirmed');assert.equal(h.calls[1][0],'updateAssetDecisions');assert.equal(h.calls[1][1].length,4);assert.equal(h.calls[1][2],'confirmed');h.unmount();
});
test('reverse drag deselects, Escape restores prior selection, and title/input/link gestures do not select rows',async()=>{
  const h=await mount();h.b.selectedAll.value=true;h.b.startRowDrag(pointer(2),2);move(h,0);h.b.finishRowDrag({pointerId:1});assert.equal(h.b.selectedRows.value.length,0);
  h.b.toggleAsset(row(2),true);h.b.startRowDrag(pointer(0),0);move(h,2);h.b.escapeRowDrag({key:'Escape'});assert.deepEqual(h.b.selectionPayload(),[{projectId:9001,assetId:2}]);
  for(const target of ['button','input','.title-rule-source']){h.b.startRowDrag(pointer(0,{target:{closest:()=>({target})}}),0);move(h,2);assert.equal(h.b.selectedRows.value.length,1);}
  assert.equal(h.calls.length,1);h.unmount();
});
test('trash explicitly operates selected associations and leaves the current query in place',async()=>{
  const h=await mount();h.b.query.value.page=2;h.b.startRowDrag(pointer(0),0);move(h,1);h.b.finishRowDrag({pointerId:1});await h.b.archive(true);
  assert.deepEqual(h.calls[1],['softDeleteAssetSelections',[{projectId:9001,assetId:1},{projectId:9001,assetId:2}],true]);assert.equal(h.b.query.value.page,2);assert.equal(h.b.selectedRows.value.length,0);h.unmount();
});
test('workspace reopening restores query independently of quarantine and another parent project',async()=>{
  const session=storage();let h=await mount({},session);h.b.query.value.page=4;h.b.query.value.search='saved';await flush();h.unmount();
  h=await mount({},session);assert.equal(h.calls[0][1].page,4);assert.equal(h.b.query.value.search,'saved');h.props.selectedProjectId=9002;await flush();assert.equal(h.b.query.value.search,'');assert.equal(h.b.query.value.projectId,9002);h.unmount();
});

test('response groups expand within current filters and export retains every matching URL',async()=>{
 const h=await mount({exportAssets:async()=>({rows:12,path:'review.csv'})});h.b.query.value.search='Example';h.b.query.value.conditions=[{field:'reviewTier',operator:'equals',value:'P1',join:'and'}];
 assert.equal(h.calls[0][1].collapseIdenticalResponses,true);const grouped={...row(8),extra:{probe_response_fingerprint:'a'.repeat(64),response_display_group_count:12}};assert.equal(h.b.responseGroupCount(grouped),12);
 h.b.showResponseMembers(grouped);await flush();const expanded=h.calls.at(-1)[1];assert.equal(expanded.projectId,9001);assert.equal(expanded.responseFingerprint,'a'.repeat(64));assert.equal(expanded.responseRepresentativeAssetId,8);assert.equal(expanded.collapseIdenticalResponses,false);assert.equal(expanded.search,'Example');assert.equal(expanded.conditions[0].value,'P1');
 await h.b.exportCurrent();assert.equal(h.calls.at(-1)[0],'exportAssets');assert.equal(h.calls.at(-1)[3],true);h.unmount();
});
test('response-group representative persists only with its project and fingerprint, and scope reset clears it',async()=>{
 const session=storage();let h=await mount({},session);const grouped={...row(8),extra:{probe_response_fingerprint:'b'.repeat(64),response_display_group_count:2}};
 h.b.showResponseMembers(grouped);await flush();h.unmount();h=await mount({},session);
 assert.equal(h.calls[0][1].responseRepresentativeAssetId,8);assert.equal(h.calls[0][1].responseFingerprint,'b'.repeat(64));
 h.b.query.value.projectId=9002;h.b.changeQueryProject();await flush();assert.equal(h.calls.at(-1)[1].responseFingerprint,'');assert.equal(h.calls.at(-1)[1].responseRepresentativeAssetId,undefined);
 h.b.showResponseMembers(grouped);await flush();h.b.resetQueryFilters();await flush();assert.equal(h.calls.at(-1)[1].responseFingerprint,'');assert.equal(h.calls.at(-1)[1].responseRepresentativeAssetId,undefined);
 h.unmount();
});
test('a historical session query without a fingerprint never sends an orphan response representative',async()=>{
 const session=storage();session.setItem('asset-query-v2:assets:9001',JSON.stringify({projectId:9001,page:1,pageSize:50,conditions:[],responseFingerprint:'',responseRepresentativeAssetId:8}));
 const h=await mount({},session);assert.equal(h.calls[0][1].responseRepresentativeAssetId,undefined);h.unmount();
});
test('opening a folded response representative reloads its changed seen partition and coherent totals',async()=>{
 const grouped={...row(8),link:'https://example.test/8',extra:{probe_response_fingerprint:'b'.repeat(64),response_display_group_count:2}};let reads=0;const opened=[];
 const h=await mount({listAssets:async()=>++reads===1?{...page([grouped]),total:1,memberTotal:2}:{...page([{...grouped,entryOpenedAt:'2026-10-15',extra:{}},row(9)]),total:2,memberTotal:2},markAssetEntryOpened:async()=> '2026-10-15'},storage(),{},{openUrl:async url=>opened.push(url)});
 await h.b.openAsset(grouped);assert.deepEqual(opened,['https://example.test/8']);assert.equal(reads,2);assert.equal(h.b.assets.value.length,2);assert.equal(h.b.total.value,2);assert.equal(h.b.memberTotal.value,2);assert.equal(h.b.query.value.collapseIdenticalResponses,true);h.unmount();
});
test('quarantine includes restored and isolated records without collapsing response members',async()=>{
 const h=await mount({},storage(),{quarantineOnly:true});const query=h.calls[0][1];assert.equal(query.probeView,'blocked');assert.equal(query.deletedView,'all');assert.equal(query.collapseIdenticalResponses,false);h.unmount();
});

const review = require('./asset_workspace_review_harness.cjs');
test('replacing the rendered page-rule example invalidates its old preview and loads the new project', async t => {
 const delayed=review.deferred();const h=await review.mountReview('PageIsolationDialog',{previewPageRule:()=>delayed.promise});t.after(h.unmount);
 const inspecting=h.click('Preview all matches');h.props.asset=review.asset(2,9002);h.props.projectId=9002;await review.flush();
 delayed.resolve(review.preview());await inspecting;
 assert.equal(h.state().project.value,9002);assert.equal(h.state().preview.value,undefined);
 assert.equal(h.buttons('Enable rule and isolate').length,0);assert.match(h.text(),/Example 2/);
 assert.equal(h.calls.filter(([name])=>name==='applyPageRule').length,0);
});
test('project-rule A to B to A navigation cannot restore an older enabled rule list', async t => {
 const reads=[];const h=await review.mountReview('PageIsolationDialog',{listPageRules:id=>{const d=review.deferred();reads.push({id,...d});return d.promise;}},{asset:undefined});t.after(h.unmount);
 await h.model('select',9002);await h.model('select',9001);assert.deepEqual(reads.map(r=>r.id),[9001,9002,9001]);
 reads[2].resolve([review.rule(9001,false)]);await review.flush();reads[0].resolve([review.rule(9001,true)]);reads[1].resolve([review.rule(9002,true)]);await review.flush();
 assert.equal(h.state().rules.value[0].enabled,false);assert.equal(h.buttons('Disable and restore').length,0);
});
test('a page-rule write finishing after the modal unmounts never reloads or emits stale changes', async () => {
 const pending=review.deferred();const h=await review.mountReview('PageIsolationDialog',{applyPageRule:()=>pending.promise});
 await h.click('Preview all matches');const applying=h.click('Enable rule and isolate');h.unmount();const reads=h.calls.filter(([name])=>name==='listPageRules').length;
 pending.resolve({ruleId:1,isolated:1,protected:1,backupPath:'/synthetic/backup'});await applying;
 assert.equal(h.calls.filter(([name])=>name==='listPageRules').length,reads);assert.deepEqual(h.events,[]);
});
test('disabling a rule reloads the rendered catalog even while an older catalog refresh is pending', async t => {
 const old=review.deferred();let reads=0;const h=await review.mountReview('AssetRuleSettings',{getAssetRuleCatalog:()=>++reads===2?old.promise:Promise.resolve({projectRules:[review.rule(9001,reads===1)],legacyRules:[]})});t.after(h.unmount);
 await h.click('View and manage');const refreshing=h.click('Refresh');await h.click('Disable and restore');
 assert.equal(reads,3,'the committed write must trigger a new catalog read');
 old.resolve({projectRules:[review.rule(9001,true)],legacyRules:[]});await refreshing;assert.equal(h.state().catalog.value.projectRules[0].enabled,false);
 assert.equal(h.calls.filter(([name])=>name==='disablePageRule').length,1);
});
test('same-tick project navigation cannot save the former project query under the next project key', async () => {
 const session=storage();const h=await mount({},session);h.b.query.value.search='Main draft';h.props.selectedProjectId=9002;await flush();
 assert.equal(h.b.query.value.projectId,9002);assert.equal(h.b.query.value.search,'');
 h.props.selectedProjectId=9001;await flush();assert.equal(h.b.query.value.search,'Main draft');h.unmount();
});
test('assets new IPC methods preserve exact native command and camel-case request contracts', async () => {
 const code=transpile(fs.readFileSync(path.join(__dirname,'../src/features/assets/api.ts'),'utf8'));const wire=[],exports={};
 new Function('require','exports',code)(name=>{assert.equal(name,'@tauri-apps/api/core');return{invoke:async(command,args)=>{wire.push([command,args]);return null;}};},exports);
 const input={projectId:9001,sourceAssetId:2,kind:'title_contains',value:'Example'};
 await exports.assetApi.previewPageRule(input);await exports.assetApi.applyPageRule(input,'synthetic-preview');await exports.assetApi.listPageRules(9001);await exports.assetApi.getAssetRuleCatalog();await exports.assetApi.disablePageRule(9001,7);
 assert.deepEqual(wire,[['preview_page_rule',{input}],['apply_page_rule',{input:{rule:input,previewToken:'synthetic-preview'}}],['list_page_rules',{projectId:9001}],['get_asset_rule_catalog',undefined],['disable_page_rule',{projectId:9001,ruleId:7}]]);
});
test('editing the rendered rule draft discards its preview without applying the previous token', async t => {
 const h=await review.mountReview();t.after(h.unmount);await h.click('Preview all matches');
 assert.equal(h.buttons('Enable rule and isolate').length,1);await h.model('input','Another example');
 assert.equal(h.buttons('Enable rule and isolate').length,0);assert.equal(h.calls.filter(([name])=>name==='applyPageRule').length,0);
});
test('an earlier rule write cannot unlock or publish into the next example preview', async t => {
 const write=review.deferred(),nextPreview=review.deferred();const h=await review.mountReview('PageIsolationDialog',{applyPageRule:()=>write.promise,previewPageRule:input=>input.projectId===9002?nextPreview.promise:Promise.resolve(review.preview(input))});t.after(h.unmount);
 await h.click('Preview all matches');const applying=h.click('Enable rule and isolate');h.props.asset=review.asset(2,9002);h.props.projectId=9002;await review.flush();
 const inspecting=h.click('Preview all matches');write.resolve({ruleId:1,isolated:1,protected:1,backupPath:'/synthetic/backup'});await applying;
 assert.equal(h.state().busy.value,true);assert.deepEqual(h.events,[]);
 nextPreview.resolve(review.preview({projectId:9002,sourceAssetId:2,kind:'title_contains',value:'example 2'}));await inspecting;
 assert.equal(h.state().preview.value.rule.projectId,9002);assert.equal(h.state().busy.value,false);assert.equal(h.calls.filter(([name])=>name==='applyPageRule').length,1);
});
test('invalid body proof falls back to a title draft and archived rule controls stay read only', async t => {
 const example={...review.asset(),extra:{probe_body_complete:'true',probe_body_truncated:'false',body_sha256:'invalid'}};
 const h=await review.mountReview('PageIsolationDialog',{}, {asset:example,projects:[{id:9001,name:'Archived',status:'archived'}]});t.after(h.unmount);
 assert.equal(h.state().kind.value,'title_contains');assert.equal(h.nodes().find(n=>n.type==='option'&&n.props.value==='body_sha256').props.disabled,true);
 assert.equal(h.buttons('Preview all matches')[0].props.disabled,true);assert.equal(h.buttons('Disable and restore')[0].props.disabled,true);assert.match(h.text(),/read only/);
 assert.equal(h.calls.filter(([name])=>name!=='listPageRules').length,0);
});
