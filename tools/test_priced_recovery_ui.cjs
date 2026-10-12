const test=require('node:test'),assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path'),ts=require('typescript'),vue=require('vue');
const {parse,compileScript,compileTemplate}=require('@vue/compiler-sfc');
const root=path.join(__dirname,'..');
const transpile=s=>ts.transpileModule(s,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText;
const contract={},priced={};
new Function('exports',transpile(fs.readFileSync(path.join(root,'src/features/sentinel/recovery/contract.ts'),'utf8')))(contract);
new Function('require','exports',transpile(fs.readFileSync(path.join(root,'src/features/sentinel/recovery/priced.ts'),'utf8')))(name=>{assert.equal(name,'./contract');return contract;},priced);
const filename=path.join(root,'src/features/sentinel/components/NativePricedRecoveryApproval.vue');
const {descriptor}=parse(fs.readFileSync(filename,'utf8'),{filename});
const script=compileScript(descriptor,{id:'priced-recovery'});
const template=compileTemplate({source:descriptor.template.content,filename,id:'priced-recovery',compilerOptions:{bindingMetadata:script.bindings}});
assert.deepEqual(template.errors,[]);
const rendered={};new Function('require','exports',transpile(template.code))(require,rendered);
const hash=c=>c.repeat(64);
const clone=value=>JSON.parse(JSON.stringify(value));
const selection={scanId:'old',attemptNumber:1,rootRunId:'root',protectionReceiptId:'protection'};
const scope={projectId:1,taskName:'Explicit new task',urls:['https://example.test/path/?a=1,2'],allowedMethods:['GET'],
 authSessionIds:[],authSessionScopeId:'',model:'fixture-model',proxyBindingHash:null,orchestrationMode:'multi',
 scanMode:'standard',closure:'proof',skillIds:[],instruction:'Only this target',maxBudgetUsd:5,
 limits:[60000,60000,60000,20,80,0,0,0,0,600000],deadlineUtc:'2099-01-01T00:00:00Z'};
const proposal=(s,newScope,pricing)=>({schemaVersion:2,purpose:'explicit_priced_new_target_work_after_original_protection_stop',
 selection:s,newScope,pricingInput:pricing,proposalHash:hash('a'),budgetDimensionNames:contract.recoveryDimensions,
 approval:false,executionAllowed:false,targetRequestsGranted:0,oldStopRetained:true,oldCostsRetained:true,
 oldScopeRenewed:false,unknownOriginalWorkReplayAllowed:false,freshBirthRequired:true,legacyScopeBudgetUsdIsDisplayOnly:true,
 pricingBasis:'approved_rate_card_not_provider_dollar_invoice',
 modelBinding:{model:newScope.model,endpointBinding:hash('b'),deployment:'cloud',credentialBinding:hash('c'),proxyBinding:hash('d'),maxContextTokens:0,maxOutputTokens:null,temperature:0},
 financialTerms:{schemaVersion:1,maxNanoUsd:priced.decimalUsdNanos(pricing.maxBudgetUsd).toString(),pricing:{schemaVersion:1,basis:'approved_rate_card',
 tokenRateUnit:'nano_usd_per_million_tokens',cachedSemantics:'subset_of_input',rounding:'ceil_total_per_model_call',modelBindingHash:hash('e'),
 priceRevision:pricing.priceRevision,uncachedInput:pricing.uncachedInput,cachedInput:pricing.cachedInput,output:pricing.output,perRequest:pricing.perRequest}}});
const approved=op=>({schemaVersion:2,purpose:'explicit_priced_new_target_work_approval',requestId:op.requestId,proposalHash:op.proposalHash,
 approvalId:hash('f'),proposal:proposal(op.selection,op.newScope,op.pricing),originalModeReceiptId:'original-mode',
 consent:'approve_exact_priced_new_scope_preserve_old_stop_and_costs',status:'approved_pending_priced_fresh_birth',
 executionAllowed:false,targetRequestsGranted:0,oldScopeRenewed:false});
const revoked=(op,a)=>({schemaVersion:1,purpose:'cancel_unconsumed_priced_recovery_approval',cancellationId:hash('c'),requestId:op.requestId,
 approvalId:a.approvalId,approvalHash:hash('d'),originalModeReceiptId:a.originalModeReceiptId,reason:'user_cancelled_before_priced_fresh_birth',
 authorityRevoked:true,executionAllowed:false,targetRequestsGranted:0,oldStopRetained:true,oldCostsRetained:true});
global.Document=class Document {};global.document=Object.assign(new global.Document(),{activeElement:null});
const node=(tag='')=>({tagName:tag.toUpperCase(),children:[],parent:null,value:'',addEventListener(){},removeEventListener(){},
 getRootNode(){return global.document;},get options(){return this.children.filter(v=>v.tagName==='OPTION');}});
const remove=child=>{if(child.parent){const rows=child.parent.children;rows.splice(rows.indexOf(child),1);child.parent=null;}};
const renderer=vue.createRenderer({createElement:node,createText:()=>node(),createComment:()=>node(),setText(){},setElementText(){},parentNode:n=>n.parent,
 nextSibling:n=>n.parent?.children[n.parent.children.indexOf(n)+1]||null,
 insert(child,parent,anchor){remove(child);parent.children||=[];const i=parent.children.indexOf(anchor);parent.children.splice(i<0?parent.children.length:i,0,child);child.parent=parent;},
 remove,patchProp(el,key,old,value){el[key]=value;}});
const tick=async()=>{for(let i=0;i<8;i++){await Promise.resolve();await vue.nextTick();}};
function harness(handlers={},options={}){
 const store=options.store||new Map(),calls=[];
 global.localStorage={getItem:k=>store.get(k)||null,setItem:(k,v)=>{if(options.failStore)throw Error('unavailable');store.set(k,v);}};
 global.crypto=require('node:crypto').webcrypto;
 const defaults={pricedApprovalForSource:()=>null,pricedPreview:proposal,pricedApprove:approved,pricedApproval:()=>null,pricedCancellation:()=>null};
 const api=new Proxy({}, {get:(_,name)=>async(...args)=>{calls.push([name,...args]);assert.ok(name in defaults||handlers[name],`unexpected IPC ${name}`);return (handlers[name]||defaults[name])(...args);}});
 const exports={};new Function('require','exports',transpile(script.content))(name=>{
  if(name==='vue')return vue;if(name==='../recovery/api')return {protectionRecoveryApi:api};
  if(name==='../recovery/contract')return contract;if(name==='../recovery/priced')return priced;throw Error(name);
 },exports);
 let b;const props=vue.reactive({scanId:'old',selection:clone(selection),newScope:clone(scope),disabled:false});
 const child={...exports.default,setup(p,c){b=exports.default.setup(p,c);return ()=>rendered.render({},[],{},vue.proxyRefs(b),{},{});}};
 const app=renderer.createApp({setup(){return ()=>vue.h(child,props);}});app.mount({});
 return {get b(){return b;},props,calls,store,close:()=>app.unmount()};
}
async function ready(h){await tick();for(const [key,value] of Object.entries({usd:'5.000000001',revision:'fixture-v2',inputRate:'2',cacheRate:'0.5',outputRate:'4',requestRate:'0.01'}))h.b[key].value=value;
 await h.b.review();assert.ok(h.b.proposal.value);h.b.consent.value=true;}
const writes=h=>h.calls.filter(([name])=>['pricedApprove','pricedCancel'].includes(name));

test('actual component freezes exact strings before write and lost ACK keeps one UUID',async()=>{
 let n=0;const h=harness({pricedApprove:op=>{assert.equal(h.store.size,1);if(++n===1)throw Error('lost ACK');return approved(op);}});
 try{await ready(h);await h.b.approve();const op=clone(h.b.operation.value);assert.equal(op.pricing.maxBudgetUsd,'5.000000001');assert.equal(op.pricing.cachedInput,'500000000');
 await h.b.approve();assert.deepEqual(writes(h).map(v=>v[1].requestId),[op.requestId,op.requestId]);assert.equal(h.b.approval.value.executionAllowed,false);assert.equal(h.b.locked.value,true);
 }finally{h.close();}
});
test('failed storage cannot write approval or replace a pending operation',async()=>{
 const h=harness({}, {failStore:true});try{await ready(h);await h.b.approve();assert.equal(writes(h).length,0);assert.equal(h.b.operation.value,undefined);}finally{h.close();}
 const g=harness();try{await ready(g);await g.b.approve();const original=g.b.operation.value;g.store.clear();await g.b.load();assert.equal(g.b.blocked.value,true);assert.equal(g.b.operation.value,original);assert.equal(writes(g).length,1);}finally{g.close();}
});
test('explicit revoke lost ACK is recovered by readonly original receipt lookup',async()=>{
 let op,cancellation;const h=harness({pricedApprove:value=>{op=value;return approved(op);},pricedApproval:()=>op?approved(op):null,
 pricedCancellation:()=>cancellation||null,pricedCancel:(request,id)=>{assert.equal(request,op.requestId);assert.equal(id,approved(op).approvalId);cancellation=revoked(op,approved(op));throw Error('lost revoke ACK');}});
 try{await ready(h);await h.b.approve();await h.b.revoke();assert.equal(writes(h).length,1,'revoke requires separate consent');
 h.b.revokeConsent.value=true;await h.b.revoke();assert.equal(h.b.cancellation.value,undefined);const n=writes(h).length;await h.b.load();assert.deepEqual(h.b.cancellation.value,cancellation);await h.b.approve();await h.b.revoke();assert.equal(writes(h).length,n);assert.equal(h.store.size,1);
 }finally{h.close();}
});
test('reload validates saved v2 operation and performs only two history reads',async()=>{
 const first=harness();let op;try{await ready(first);await first.b.approve();op=clone(first.b.operation.value);}finally{first.close();}
 const h=harness({pricedApproval:()=>approved(op),pricedCancellation:()=>revoked(op,approved(op))},{store:new Map([[`oviraptor.protection-recovery.priced.v2:old`,JSON.stringify(op)]])});
 try{await tick();assert.deepEqual(h.calls.map(v=>v[0]),['pricedApproval','pricedCancellation']);assert.ok(h.b.cancellation.value);assert.equal(writes(h).length,0);assert.equal(h.b.usd.value,'5.000000001');assert.equal(h.b.inputRate.value,'2');assert.equal(h.b.cacheRate.value,'0.5');assert.equal(h.b.requestRate.value,'0.01');}finally{h.close();}
 const old=harness({}, {store:new Map([[`oviraptor.protection-recovery.priced.v2:old`,JSON.stringify({...op,version:1})]])});
 try{await tick();assert.equal(old.b.blocked.value,true);assert.equal(old.calls.length,0);}finally{old.close();}
});
test('invalid prices and multiple URLs refuse before preview while parent changes invalidate consent',async()=>{
 const h=harness();try{await ready(h);const n=h.calls.length;h.b.inputRate.value='1e2';await h.b.review();assert.equal(h.calls.length,n);
 h.b.inputRate.value='2';h.props.newScope.urls.push('https://second.test/');await h.b.review();assert.equal(h.calls.length,n);
 h.props.newScope.urls.pop();await h.b.review();h.b.consent.value=true;h.props.newScope.instruction='Changed scope';assert.equal(h.b.proposal.value,undefined);assert.equal(h.b.consent.value,false);await h.b.approve();assert.equal(writes(h).length,0);
 }finally{h.close();}
});
test('unaffordable fixed call is explained without approval writes and can be corrected',async()=>{
 let reject=true;
 const h=harness({pricedPreview:(...args)=>{
  if(reject)throw Error('protection_recovery_fixed_call_exceeds_budget');
  return proposal(...args);
 }});
 try{
  await tick();
  for(const [key,value] of Object.entries({usd:'5.000000001',revision:'r1',inputRate:'2',cacheRate:'0.5',outputRate:'4',requestRate:'6'}))h.b[key].value=value;
  await h.b.review();
  assert.equal(h.b.proposal.value,undefined);
  assert.match(h.b.error.value,/单次固定请求费用超过全部费用预算/);
  assert.equal(writes(h).length,0);
  assert.equal(h.store.size,0);
  reject=false;
  h.b.requestRate.value='0.01';
  await h.b.review();
  assert.ok(h.b.proposal.value);
  assert.equal(h.b.error.value,'');
  assert.equal(writes(h).length,0);
 }finally{h.close();}
});
test('in-flight preview cannot accept changed parent fields or changed prices',async()=>{
 let release;const h=harness({pricedPreview:(...args)=>new Promise(resolve=>{release=()=>resolve(proposal(...args));})});
 try{await tick();for(const [key,value] of Object.entries({usd:'5',revision:'r1',inputRate:'2',cacheRate:'0.5',outputRate:'4',requestRate:'0'}))h.b[key].value=value;
 const pending=h.b.review();h.props.newScope.taskName='Changed while loading';release();await pending;assert.equal(h.b.proposal.value,undefined);assert.ok(h.b.error.value);assert.equal(writes(h).length,0);
 }finally{h.close();}
});
test('wrong grants and wrong cancellation source cannot be advertised as verified',async()=>{
 const h=harness({pricedApprove:op=>({...approved(op),executionAllowed:true})});try{await ready(h);await h.b.approve();assert.equal(h.b.approval.value,undefined);assert.ok(h.b.operation.value);}finally{h.close();}
 const g=harness({pricedCancel:()=>({executionAllowed:false,authorityRevoked:true})});try{await ready(g);await g.b.approve();g.b.revokeConsent.value=true;await g.b.revoke();assert.equal(g.b.cancellation.value,undefined);assert.ok(g.b.error.value);}finally{g.close();}
});
test('scan switch discards old ACK view but retains original persisted UUID',async()=>{
 let release;const h=harness({pricedApprove:op=>new Promise(resolve=>{release=()=>resolve(approved(op));})});
 try{await ready(h);const pending=h.b.approve();const op=clone(h.b.operation.value);h.props.scanId='other';await tick();release();await pending;assert.equal(h.b.approval.value,undefined);assert.equal(h.b.operation.value,undefined);assert.equal(h.store.size,1);assert.equal(JSON.parse(h.store.values().next().value).requestId,op.requestId);}finally{h.close();}
});


test('parent scope unavailable blocks new approval but keeps explicit original revocation usable',async()=>{
 const h=harness();try{await ready(h);await h.b.approve();const op=clone(h.b.operation.value);
 h.props.disabled=true;const n=writes(h).length;await h.b.review();await h.b.approve();assert.equal(writes(h).length,n);
 h.b.revokeConsent.value=true;
 h.calls.length=0;
 // Backend mock only receives this original request/approval, no current parent scope.
 const apiResult=revoked(op,approved(op));
 // Use a fresh read of the persisted operation with a cancellation handler.
 const g=harness({pricedApproval:()=>approved(op),pricedCancel:(request,id)=>{assert.equal(request,op.requestId);assert.equal(id,approved(op).approvalId);return apiResult;}},{store:h.store});
 try{await tick();g.props.disabled=true;g.props.newScope=null;g.props.selection=undefined;g.b.revokeConsent.value=true;await g.b.revoke();assert.deepEqual(g.b.cancellation.value,apiResult);assert.deepEqual(writes(g).map(v=>v[0]),['pricedCancel']);}finally{g.close();}
 }finally{h.close();}
});


test('approval preserves another window operation and refuses a missing original retry record',async()=>{
 const key='oviraptor.protection-recovery.priced.v2:old';
 const h=harness();try{
  await ready(h);
  const foreign=JSON.stringify({version:2,requestId:'original-other-window',evidence:'retain exact bytes'});
  h.store.set(key,foreign);await h.b.approve();
  assert.equal(writes(h).length,0);assert.equal(h.store.get(key),foreign);
  assert.equal(h.b.blocked.value,true);assert.equal(h.b.operation.value,undefined);
 }finally{h.close();}
 for(const change of ['missing','changed']){
  const g=harness({pricedApprove:()=>{throw Error('lost ACK');}});try{
   await ready(g);await g.b.approve();const original=clone(g.b.operation.value),n=writes(g).length;
   if(change==='missing')g.store.delete(key);else g.store.set(key,JSON.stringify({...original,requestId:'different-request'}));
   const saved=g.store.get(key);await g.b.approve();
   assert.equal(writes(g).length,n);assert.deepEqual(clone(g.b.operation.value),original);
   assert.equal(g.store.get(key),saved);assert.equal(g.b.blocked.value,true);
  }finally{g.close();}
 }
});


test('explicit source recovery restores the signed original UUID and cancellation without approving again',async()=>{
 const first=harness();let op;try{await ready(first);await first.b.approve();op=clone(first.b.operation.value);}finally{first.close();}
 const a=approved(op),c=revoked(op,a);
 const h=harness({pricedApprovalForSource:selected=>{assert.deepEqual(selected,selection);return a;},pricedCancellation:request=>{assert.equal(request,op.requestId);return c;}});
 try{
  await tick();assert.equal(h.calls.length,0,'source recovery is explicit');
  h.props.disabled=true;h.props.newScope=null;await h.b.recoverOriginal();
  assert.deepEqual(clone(h.b.operation.value),op);assert.deepEqual(h.b.cancellation.value,c);
  assert.equal(JSON.parse(h.store.values().next().value).requestId,op.requestId);
  assert.deepEqual(h.calls.map(v=>v[0]),['pricedApprovalForSource','pricedCancellation']);assert.equal(writes(h).length,0);
 }finally{h.close();}
 for(const kind of ['foreign-cache','wrong-source','fake-grant','failed-storage']){
  const g=harness({pricedApprovalForSource:()=>kind==='wrong-source'?{...a,proposal:{...a.proposal,selection:{...selection,rootRunId:'other'}}}:kind==='fake-grant'?{...a,executionAllowed:true}:a},{failStore:kind==='failed-storage'});
  try{await tick();const foreign='retain foreign bytes';if(kind==='foreign-cache')g.store.set('oviraptor.protection-recovery.priced.v2:old',foreign);
   await g.b.recoverOriginal();assert.equal(g.b.operation.value,undefined);assert.equal(g.b.approval.value,undefined);assert.equal(writes(g).length,0);
   if(kind==='foreign-cache')assert.equal(g.store.values().next().value,foreign);
  }finally{g.close();}
 }
});

test('source recovery discards replies after the selected source or scan changes',async()=>{
 const first=harness();let op;try{await ready(first);await first.b.approve();op=clone(first.b.operation.value);}finally{first.close();}
 for(const change of ['source','scan']){
  let release;const h=harness({pricedApprovalForSource:()=>new Promise(resolve=>{release=()=>resolve(approved(op));})});
  try{await tick();const pending=h.b.recoverOriginal();
   if(change==='scan')h.props.scanId='other';else h.props.selection.rootRunId='other';
   await tick();release();await pending;assert.equal(h.b.operation.value,undefined);assert.equal(h.store.size,0);assert.equal(writes(h).length,0);
  }finally{h.close();}
 }
});
