const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const ts=require('typescript');
const vue=require('vue');
const {parse,compileScript,compileTemplate}=require('@vue/compiler-sfc');
const {renderToString}=require('@vue/server-renderer');
const {transpile}=require('./shell_module_harness.cjs');
const source=relative=>fs.readFileSync(path.join(__dirname,'..',relative),'utf8');
const flush=async()=>{for(let i=0;i<30;i++)await vue.nextTick();};
const deferred=()=>{let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return{promise,resolve,reject};};
function compile(relative){
 const filename=path.join(__dirname,'..',relative),id=relative;
 const {descriptor}=parse(source(relative),{filename});
 const script=compileScript(descriptor,{id,fs:{fileExists:ts.sys.fileExists,readFile:ts.sys.readFile}});
 const template=compileTemplate({source:descriptor.template.content,filename,id,compilerOptions:{bindingMetadata:script.bindings}});assert.deepEqual(template.errors,[]);
 const rendered={};new Function('require','exports',transpile(template.code))(require,rendered);
 return{script:transpile(script.content),render:rendered.render};
}
const renderer=vue.createRenderer({createElement:()=>({}),createText:()=>({}),createComment:()=>({}),insert(){},remove(){},setText(){},setElementText(){},patchProp(){},parentNode:()=>null,nextSibling:()=>null});
const project=(id=91)=>({id,name:`Project ${id}`,status:'active'});
function mountWorkspace(t,kind='asset',overrides={},initialProps={}){
 const paths={asset:'src/features/assets/workspace/AssetsPluginWorkspace.vue',sentinel:'src/features/sentinel/workspace/ScanningPluginWorkspace.vue',hackerone:'src/features/hackerone/workspace/HackerOnePluginWorkspace.vue'};
 const compiled=compile(paths[kind]),calls=[],events=[],subscriptions=[];
 const defaults={dashboardStats:async()=>({assetCount:0,runningJobs:0,blockedCount:0}),listRuns:async()=>[],listEvents:async()=>[],listSentinelScans:async()=>[],listSentinelScanAttempts:async()=>[],listHackerOneEvents:async()=>[],startJob:async()=>7,resumeJob:async()=>7,acknowledgeInterruptedRun:async()=>{},cancelJob:async()=>{}};
 const api=new Proxy({}, {get(_,name){if(!(name in defaults)&&!(name in overrides))return undefined;return(...args)=>{calls.push([name,...args]);return(overrides[name]||defaults[name])(...args);};},ownKeys(){return [...new Set([...Object.keys(defaults),...Object.keys(overrides)])];},getOwnPropertyDescriptor(){return {configurable:true,enumerable:true};}});
 const logState={snapshot:vue.ref(),loading:vue.ref(false),readFailed:vue.ref(false),connectionUnavailable:vue.ref(false),refresh:async()=>{calls.push(['refreshRunnerLog']);}};
 const imports=name=>{
  if(name==='vue')return{...vue,defineAsyncComponent:()=>()=>null};
  if(name==='@lucide/vue')return new Proxy({}, {get:()=>()=>null});
  if(name==='@tauri-apps/api/event')return{listen:overrides.listen||(async(event,callback)=>{const sub={event,callback,stopped:false};subscriptions.push(sub);return()=>{sub.stopped=true;};})};
  if(name==='../api')return{assetApi:api,sentinelApi:api,hackerOneApi:api};
  if(name==='../../../shell/api')return{shellApi:api};
  if(name==='../useAssetLogPanel')return{useAssetLogPanel:()=>({logs:vue.ref([]),loading:vue.ref(false),readFailed:vue.ref(false),connectionUnavailable:vue.ref(false),refresh:async()=>{calls.push(['refreshAssetLogs']);}})};
  if(name==='../execution/useRunnerLogPanel')return{useRunnerLogPanel:()=>logState};
  if(name==='../presentation')return{humanizeScanCheckpoint:value=>value};
  if(name==='../../../i18n')return{useI18n:()=>({t:vue.ref({refresh:'Refresh',newQuery:'New query'}),tr:(_zh,en)=>en})};
  if(name.endsWith('.vue'))return{default:()=>null};
  throw Error(`Unexpected plugin import ${name}`);
 };
 const exports={};new Function('require','exports',compiled.script)(imports,exports);
 const props=vue.reactive({projects:[project(),project(92)],profiles:[{id:1,isDefault:true,settings:{}}],projectId:91,view:kind==='asset'?'dashboard':kind==='sentinel'?'sentinel':'hackerone',refreshToken:0,scanningEnabled:true,...initialProps});
 let b;
 const component={...exports.default,setup(p,ctx){b=exports.default.setup(p,ctx);return()=>null;}};
 const listeners={};for(const name of ['navigate','selectProject','createProject','restoreProject','projectsChanged','notify','metadata','intentConsumed','interruptedResumed'])listeners[`on${name[0].toUpperCase()+name.slice(1)}`]=(...args)=>events.push([name,...args]);
 const app=renderer.createApp({render:()=>vue.h(component,{...props,...listeners})});app.mount({});if(t)t.after(()=>app.unmount());
 return{b,props,api,calls,events,subscriptions,logState,unmount:()=>app.unmount(),render:()=>renderToString(vue.createSSRApp({render:()=>compiled.render({},[],{},vue.proxyRefs(b),{}, {})}))};
}
module.exports={assert,vue,flush,deferred,project,mountWorkspace,compile,source};
function mountLegacyAssets(t,overrides={}){
 const compiled=compile('src/features/assets/components/AssetWorkspace.vue'),calls=[],events=[];
 const api=new Proxy({}, {get:(_,name)=>(...args)=>{calls.push([name,...args]);if(overrides[name])return overrides[name](...args);if(name==='listAssets')return Promise.resolve({items:[],total:0});if(name==='createSentinelScan')return Promise.resolve('scan');return Promise.resolve(1);}});
 const exports={},storage={getItem:()=>null,setItem(){}};
 new Function('require','exports','localStorage','sessionStorage','window','document',compiled.script)(name=>{
  if(name==='vue')return{...vue,onMounted(){}};
  if (name === '../api') return { assetApi: api };
    if (name === '../../sentinel/api') return { sentinelApi: api };
  if(name==='../../../i18n')return{useI18n:()=>({tr:(_zh,en)=>en})};
  if(name==='@lucide/vue')return new Proxy({}, {get:()=>()=>null});
  if(name==='@tauri-apps/plugin-opener')return{openUrl:async()=>{}};
  if(name.endsWith('.vue'))return{default:()=>null};
  if(name.endsWith('.css'))return{};
  throw Error(`Unexpected asset import ${name}`);
 },exports,storage,storage,{addEventListener(){},removeEventListener(){},getSelection:()=>null},{elementFromPoint:()=>null});
 const props=vue.reactive({projects:[project(91),project(92)],selectedProjectId:undefined,scanningEnabled:true});let b;
 const component={...exports.default,setup(p,ctx){b=exports.default.setup(p,ctx);return()=>null;}};
 const app=renderer.createApp({render:()=>vue.h(component,{...props,onNotify:(...args)=>events.push(args)})});app.mount({});if(t)t.after(()=>app.unmount());
 b.toggleAsset({id:1,projectId:91},true);b.toggleAsset({id:2,projectId:92},true);
 return{b,props,calls,events,unmount:()=>app.unmount()};
}
module.exports.mountLegacyAssets=mountLegacyAssets;
