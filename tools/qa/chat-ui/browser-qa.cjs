// Standalone synthetic browser acceptance. No credentials, providers or scans.
const {chromium}=require('playwright');
const crypto=require('node:crypto'),fs=require('node:fs'),path=require('node:path'),http=require('node:http'),assert=require('node:assert/strict');
const base=require('./base-fixture.cjs'),enhance=require('./fixture.cjs');
const repo=path.resolve(process.env.OVIRAPTOR_REPO_ROOT||process.cwd());
const output=path.resolve(process.env.OVIRAPTOR_QA_OUTPUT||path.join(repo,'qa-artifacts/chat-ui'));
const bundle=path.join(output,'bundle'),artifacts=path.join(output,'screenshots');
const scenarioFilter=process.env.OVIRAPTOR_QA_SCENARIOS?new RegExp(process.env.OVIRAPTOR_QA_SCENARIOS):null;
fs.mkdirSync(artifacts,{recursive:true});
const markdown=`# A clear plan\n\nSynthetic assistant reply with **bold**, *emphasis*, \`inline code\`, and a [safe link](https://example.com/reference).\n\n- Review the history\n- Keep one composer\n\n> Synthetic quoted context\n\n| Area | Outcome |\n| --- | --- |\n| Chat | Local-only fixture |\n| Rendering | Real Vue components |\n\n\`\`\`typescript\nconst greeting: string = "hello";\nconsole.log(greeting);\n\`\`\`\n\n<script>window.__unsafe = true</script>\n[unsafe](javascript:alert(1))`;
const results=[],contexts=[],captureLayouts=[];
let browser,server,origin;
async function pageFixture(options={},viewport={width:1440,height:1000}){
 const context=await browser.newContext({viewport,deviceScaleFactor:1});contexts.push(context);
 const errors=[],blocked=[];
 await context.route('**/*',route=>{const url=new URL(route.request().url());if(url.origin===origin)return route.continue();blocked.push(url.href);return route.abort();});
 await context.addInitScript({content:`(${base.toString()})(${JSON.stringify({project:1,...options})});(${enhance.toString()})(${JSON.stringify(options)});`});
 const page=await context.newPage();page.setDefaultTimeout(15000);page.on('pageerror',e=>errors.push(String(e)));
 await page.goto(origin);await page.locator('.conversation-workspace').waitFor();
 const fixture={page,context,errors,blocked};page.__qaFixture=fixture;return fixture;
}
async function toggleFor(page){const desktop=page.locator('.sidebar-toggle').first();return await desktop.isVisible()?desktop:page.locator('.main-sidebar-toggle').first();}
async function chooseChat(page){const toggle=await toggleFor(page);if(await toggle.getAttribute('aria-expanded')==='false')await toggle.click();await page.locator('.conversation-row').first().click();await page.locator('.conversation-composer textarea').waitFor();}
async function collapse(page){const toggle=await toggleFor(page);if(await toggle.getAttribute('aria-expanded')==='true')await toggle.click();}
async function openAccess(page){const panel=page.locator('.conversation-access-panel');await panel.waitFor();if(!await panel.evaluate(el=>el.open))await panel.locator('summary').click();await panel.locator('.access-mode').waitFor();return panel;}
async function selectHistory(page,title){const toggle=await toggleFor(page);if(await toggle.getAttribute('aria-expanded')==='false')await toggle.click();await page.locator('.conversation-row').getByText(title,{exact:true}).click();await page.locator('.conversation-composer textarea').waitFor();}
async function reloadAccess(page){await selectHistory(page,'Synthetic second conversation');await selectHistory(page,'Workspace 1 conversation');return openAccess(page);}
async function settleCapture(page){
 // Vue updates, loaded fonts and finite CSS transitions must settle before
 // judging geometry. Do not wait for the intentional infinite reply pulse.
 await page.evaluate(async()=>{
   await document.fonts.ready;
   await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
   const finite=document.getAnimations().filter(animation=>Number.isFinite(animation.effect?.getComputedTiming().endTime));
   await Promise.all(finite.map(animation=>animation.finished.catch(()=>{})));
   await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
 });
}
async function captureGeometry(page){return page.evaluate(()=>{
 const rect=selector=>document.querySelector(selector)?.getBoundingClientRect().toJSON();
 const sidebar=document.querySelector('.sidebar');
 return {viewport:{width:innerWidth,height:innerHeight},mobile:matchMedia('(max-width:760px)').matches,collapsed:document.querySelector('.app-shell')?.classList.contains('sidebar-collapsed'),sidebar:rect('.sidebar'),sidebarVisibility:sidebar?getComputedStyle(sidebar).visibility:null,main:rect('.main-area'),composer:rect('.conversation-composer')};
});}
async function shot(page,name){
 await settleCapture(page);
 const geometry=await captureGeometry(page);
 // Capture even if a subsequent geometry assertion fails, for diagnosis.
 await page.screenshot({path:path.join(artifacts,name+'.png'),fullPage:true,animations:'disabled'});
 captureLayouts.push({name,...geometry});
 fs.writeFileSync(path.join(output,'screenshot-layouts.json'),JSON.stringify(captureLayouts,null,2));
 const expectedMainLeft=geometry.mobile?0:geometry.collapsed?64:276;
 assert(Math.abs(geometry.main.x-expectedMainLeft)<1,`${name}: settled main x ${geometry.main.x}, expected ${expectedMainLeft}`);
 if(!geometry.mobile)assert(Math.abs(geometry.sidebar.width-expectedMainLeft)<1,`${name}: settled sidebar width ${geometry.sidebar.width}, expected ${expectedMainLeft}`);
 else if(!geometry.collapsed)assert(Math.abs(geometry.sidebar.x)<1,`${name}: mobile drawer still translated ${geometry.sidebar.x}`);
}

async function fit(page){return page.evaluate(()=>({viewport:innerWidth,document:document.documentElement.scrollWidth,composer:document.querySelector('.conversation-composer')?.getBoundingClientRect().toJSON(),transcript:document.querySelector('.conversation-transcript')?.getBoundingClientRect().toJSON(),bodyHeight:document.body.scrollHeight,viewportHeight:innerHeight}));}
async function fixtureState(page){return page.evaluate(()=>({unknown:window.__QA__.unknown,network:window.__QA__.network,calls:window.__QA__.calls,mutations:window.__QA__.mutations}));}
async function scenario(name,fn){if(scenarioFilter&&!scenarioFilter.test(name))return;let m;try{m=await fn();assert.deepEqual(m.errors,[],'Browser runtime errors');assert.deepEqual(m.blocked,[],'Non-local network attempted');const state=await fixtureState(m.page);assert.deepEqual(state.unknown,[],'Unimplemented IPC');assert.deepEqual(state.network,[],'External fetch attempted');results.push({name,status:'passed',layout:await fit(m.page),errors:m.errors,blocked:m.blocked,state});console.log('PASS',name);}catch(error){const page=m?.page||contexts.at(-1)?.pages()[0];m??=page?.__qaFixture;if(page)await shot(page,name+'-failure').catch(()=>{});results.push({name,status:'failed',error:String(error),errors:m?.errors??[],blocked:m?.blocked??[],state:page?await fixtureState(page).catch(()=>null):null});console.error('FAIL',name,error);}}
(async()=>{
 const styles=fs.readdirSync(bundle).filter(x=>x.endsWith('.css')).map(x=>`<link rel="stylesheet" href="/${x}">`).join('');
 server=http.createServer((request,response)=>{const pathname=new URL(request.url,'http://localhost').pathname;if(pathname==='/'){response.setHeader('Content-Type','text/html');return response.end(`<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">${styles}</head><body><div id="app"></div><script src="/app.iife.js"></script></body></html>`);}const name=path.basename(pathname);if(name!==pathname.slice(1)||!fs.existsSync(path.join(bundle,name))){response.statusCode=404;return response.end();}response.setHeader('Content-Type',name.endsWith('.css')?'text/css':'text/javascript');response.end(fs.readFileSync(path.join(bundle,name)));});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));origin=`http://127.0.0.1:${server.address().port}`;
 browser=await chromium.launch({headless:true,...(process.env.OVIRAPTOR_QA_CHROMIUM?{executablePath:process.env.OVIRAPTOR_QA_CHROMIUM}:{})});
 await scenario('00-default-home',async()=>{const m=await pageFixture({project:null});await shot(m.page,'00-default-home');assert.equal(await m.page.locator('.sidebar-toggle').first().getAttribute('aria-expanded'),'false');return m;});
 for(const saved of [{module:'asset',view:'assets',label:'Assets',selector:'.asset-plugin-workspace',read:'dashboard_stats'},{module:'sentinel',view:'sentinel',label:'Scanning',selector:'.scanning-plugin-workspace',read:'list_sentinel_scans'}]){
  await scenario(`startup-${saved.module}-restores-chat-only`,async()=>{
   const m=await pageFixture({route:{module:saved.module,view:saved.view},hackerOne:true}),p=m.page;
   await p.waitForFunction(()=>window.__QA__.calls.some(call=>call.cmd==='list_conversations'));
   assert.equal(await p.locator('.asset-plugin-workspace,.scanning-plugin-workspace,.hackerone-plugin-workspace').count(),0,'No plugin workspace is mounted at startup');
   assert.deepEqual(await p.evaluate(()=>({module:localStorage.getItem('oviraptor-module'),view:localStorage.getItem('oviraptor-view'),project:localStorage.getItem('oviraptor-project')})),{module:'chat',view:'chat',project:'1'});
   const pluginReads=await p.evaluate(()=>window.__QA__.calls.filter(call=>/^(dashboard_stats|list_assets|list_runs|list_asset_events|list_sentinel_|sentinel_overview_stats|investigation_overview|list_hackerone_|get_agent_dialog_selection|list_worker_nodes|sync_worker_node)/.test(call.cmd)));
   assert.deepEqual(pluginReads,[],'No plugin data initialization before explicit selection, including configured HackerOne');
   await chooseChat(p);await settleCapture(p);
   const historyText=await p.locator('#conversation-history').textContent();
   const order=await p.evaluate(()=>{
    const rect=selector=>document.querySelector(selector).getBoundingClientRect().toJSON();
    const chat=document.querySelector('.workspace-chat-module'),history=document.querySelector('#conversation-history'),plugins=document.querySelector('.workspace-plugin-group'),workspace=document.querySelector('.sidebar-project');
    const before=(a,b)=>Boolean(a.compareDocumentPosition(b)&Node.DOCUMENT_POSITION_FOLLOWING);
    return {dom:before(chat,history)&&before(history,plugins)&&before(plugins,workspace),chat:rect('.workspace-chat-module'),history:rect('#conversation-history'),plugins:rect('.workspace-plugin-group'),workspace:rect('.sidebar-project'),entries:[...plugins.querySelectorAll('.workspace-plugin-modules button')].map(button=>button.getAttribute('aria-label'))};
   });
   assert(order.dom,'DOM order is Chat, history, bottom plugins, workspace');
   assert(order.chat.bottom<=order.history.top+1&&order.history.bottom<=order.plugins.top+1&&order.plugins.bottom<=order.workspace.top+1,JSON.stringify(order));
   assert.deepEqual(order.entries,['Assets','Scanning','HackerOne','Skills']);
   assert(order.workspace.top-order.plugins.bottom<=14,'Plugin group sits directly above workspace');
   await shot(p,`18-startup-${saved.module}-chat-history-order`);
   await p.locator('.workspace-plugin-modules').getByRole('button',{name:saved.label,exact:true}).click();
   await p.locator(saved.selector).waitFor();
   await p.waitForFunction(read=>window.__QA__.calls.some(call=>call.cmd===read),saved.read);
   assert.equal(await p.evaluate(()=>localStorage.getItem('oviraptor-module')),saved.module,'Only the clicked plugin becomes active');
   assert.equal(await p.locator('.asset-plugin-workspace,.scanning-plugin-workspace,.hackerone-plugin-workspace').count(),1);
   await shot(p,`19-explicit-${saved.module}-workspace`);
   assert.equal(await p.locator('.conversation-workspace').isVisible(),false,'Chat page stays mounted but hidden behind the explicit plugin');
   assert.equal(await p.locator('#conversation-history').isVisible(),true,'Saved chat history remains visible under Chat while a plugin is active');
   assert.equal(await p.locator('#conversation-history').textContent(),historyText);
   await p.locator('.conversation-row').first().click();
   await p.locator('.conversation-workspace').waitFor();await p.locator('.conversation-composer').waitFor();
   assert.equal(await p.evaluate(()=>localStorage.getItem('oviraptor-module')),'chat','Clicking history returns to Chat');
   assert.equal(await p.locator('#conversation-history').textContent(),historyText,'Explicit plugin navigation preserves saved conversation history');
   assert.equal(await p.locator('.asset-plugin-workspace,.scanning-plugin-workspace,.hackerone-plugin-workspace').count(),0);
   assert.equal(await p.evaluate(()=>window.__QA__.mutations.length),0,'Navigation never starts scans or changes business data');
   return m;
  });
 }
 await scenario('01-wide-chat-shell',async()=>{const m=await pageFixture({markdown}),p=m.page;assert.equal(await p.locator('.sidebar-toggle').first().getAttribute('aria-expanded'),'false');await shot(p,'01-wide-empty');await chooseChat(p);await p.locator('.conversation-composer textarea').fill('Synthetic unsent draft');const transcript=await p.locator('.conversation-transcript').textContent();await shot(p,'02-wide-expanded');await collapse(p);assert.equal(await p.locator('.conversation-composer textarea').inputValue(),'Synthetic unsent draft');assert.equal(await p.locator('.conversation-transcript').textContent(),transcript);assert.equal(await p.locator('.conversation-composer').count(),1);assert.equal(await p.locator('.conversation-transcript table').count(),1);assert.equal(await p.locator('.conversation-transcript pre').count(),1);assert.equal(await p.locator('.conversation-transcript script, .conversation-transcript iframe, .conversation-transcript img').count(),0);assert.equal(await p.locator('.conversation-transcript a[href^="javascript:"]').count(),0);await shot(p,'03-wide-rich-chat');await p.getByRole('button',{name:'Copy code',exact:true}).click();assert.equal(await p.evaluate(()=>window.__QA__.clipboard[0]),'const greeting: string = "hello";\nconsole.log(greeting);\n');await p.locator('.conversation-message.assistant .message-actions').getByRole('button',{name:'Copy message',exact:true}).click();assert.equal(await p.evaluate(()=>window.__QA__.clipboard[1]),markdown);await shot(p,'11-copy-confirmations');await p.locator('.conversation-transcript a').first().click();assert.equal(await p.evaluate(()=>window.__QA__.openedLinks[0]),'https://example.com/reference');const layout=await fit(p);assert(layout.document<=layout.viewport+1,JSON.stringify(layout));assert(layout.composer.width<1050,'Composer should retain readable width');return m;});
 await scenario('02-keyboard-save-retry-stop',async()=>{const m=await pageFixture({available:true,failNextSend:true}),p=m.page;await chooseChat(p);await collapse(p);const input=p.locator('.conversation-composer textarea');await input.fill('Synthetic keyboard send');await input.press('Shift+Enter');assert.equal(await p.evaluate(()=>window.__QA__.mutations.length),0);await input.fill('Synthetic keyboard send');await input.dispatchEvent('keydown',{key:'Enter',isComposing:true});assert.equal(await p.evaluate(()=>window.__QA__.mutations.length),0);await input.press('Enter');await p.getByRole('button',{name:'Retry send',exact:true}).waitFor();assert.equal(await input.inputValue(),'Synthetic keyboard send');await p.getByRole('button',{name:'Retry send',exact:true}).click();await p.getByRole('button',{name:'Stop reply',exact:true}).waitFor();const sends=await p.evaluate(()=>window.__QA__.mutations.filter(c=>c.cmd==='send_conversation_message'));assert.equal(sends.length,2);assert.deepEqual(sends[0].args,sends[1].args);assert.equal(await p.locator('.conversation-composer').count(),1);await p.getByRole('button',{name:'Stop reply',exact:true}).click();await p.waitForFunction(()=>document.querySelector('.composer-submit.is-stop')?.disabled);await shot(p,'04-reply-stopping');return m;});
 await scenario('03-narrow-rich-chat',async()=>{const m=await pageFixture({markdown},{width:390,height:844}),p=m.page;await chooseChat(p);const mobileToggle=await toggleFor(p);if(await mobileToggle.getAttribute('aria-expanded')==='false')await mobileToggle.click();await shot(p,'05-narrow-sidebar');await collapse(p);await shot(p,'06-narrow-rich-chat');const layout=await fit(p);assert(layout.document<=layout.viewport+1,JSON.stringify(layout));assert(layout.composer.x>=0&&layout.composer.x+layout.composer.width<=layout.viewport+1,'Composer within narrow viewport');assert.equal(await p.locator('.conversation-composer').count(),1);return m;});
 await scenario('04-long-content-and-scroll',async()=>{const m=await pageFixture({long:true}),p=m.page;await chooseChat(p);await collapse(p);await shot(p,'07-wide-long-content');assert((await fit(p)).document<=(await fit(p)).viewport+1,'Long content cannot overflow document');await p.setViewportSize({width:390,height:844});await shot(p,'08-narrow-long-content');const layout=await fit(p);assert(layout.document<=layout.viewport+1,JSON.stringify(layout));const scroll=await p.locator('.conversation-transcript').evaluate(el=>{el.scrollTop=el.scrollHeight;return {height:el.clientHeight,scrollHeight:el.scrollHeight,top:el.scrollTop}});assert(scroll.scrollHeight>scroll.height,'Long transcript should overflow its independent scroll box');await p.locator('.conversation-transcript').evaluate(el=>{el.scrollTop=100;el.dispatchEvent(new Event('scroll'))});await p.evaluate(()=>window.__QA__.addMessage('chat-1','Synthetic incoming while reading older text'));await p.locator('.conversation-actions summary').click();await p.getByRole('button',{name:'Refresh history',exact:true}).click();await p.getByRole('button',{name:'New messages',exact:true}).waitFor();assert((await p.locator('.conversation-transcript').evaluate(el=>el.scrollTop))<120,'Incoming reply must preserve older reading position');await p.getByRole('button',{name:'New messages',exact:true}).click();assert((await p.locator('.conversation-transcript').evaluate(el=>el.scrollHeight-el.scrollTop-el.clientHeight))<5,'Jump should reach latest message');return m;});
 await scenario('05-dark-theme',async()=>{const m=await pageFixture({markdown,theme:'codex'});await chooseChat(m.page);await collapse(m.page);await shot(m.page,'10-dark-rich-chat');assert.equal(await m.page.locator('.theme-codex').count(),1);return m;});
 await scenario('06-save-only-new-chat',async()=>{const m=await pageFixture(),p=m.page;await chooseChat(p);await collapse(p);await p.locator('.conversation-composer textarea').fill('Synthetic local-only save');await p.getByRole('button',{name:'Save message',exact:true}).click();await p.waitForFunction(()=>window.__QA__.mutations.some(c=>c.cmd==='send_conversation_message'));const send=await p.evaluate(()=>window.__QA__.mutations.find(c=>c.cmd==='send_conversation_message'));assert.equal(send.args.input.requestReply,false);await shot(p,'09-save-only');return m;});
 await scenario('07-steer-queued-next-turn',async()=>{const m=await pageFixture({available:true,replyStatus:'running'}),p=m.page;await chooseChat(p);await collapse(p);await p.locator('.conversation-composer textarea').fill('Synthetic direction: focus the next reply on tests and give a concise summary.');await p.locator('.composer-steer').click();await p.locator('.conversation-steer-queue').waitFor();assert.equal(await p.evaluate(()=>window.__QA__.mutations.length),0,'Queued Steer must not send before completion');assert.match(await p.locator('.conversation-steer-queue').textContent(),/current reply won’t change/);assert.equal(await p.locator('.conversation-composer').count(),1);await shot(p,'12-steer-queued-wide');await p.setViewportSize({width:390,height:560});await shot(p,'13-steer-queued-narrow-short');const layout=await fit(p);assert(layout.document<=layout.viewport+1,JSON.stringify(layout));assert(layout.composer.y+layout.composer.height<=layout.viewportHeight+1,'Composer remains visible in short viewport');await p.setViewportSize({width:1440,height:1000});await p.evaluate(()=>window.__QA__.completeReply('chat-1'));await p.locator('.conversation-actions summary').click();await p.getByRole('button',{name:'Refresh history',exact:true}).click();await p.waitForFunction(()=>!document.querySelector('.conversation-steer-queue')&&window.__QA__.mutations.some(c=>c.cmd==='send_conversation_message'));const sends=await p.evaluate(()=>window.__QA__.mutations.filter(c=>c.cmd==='send_conversation_message'));assert.equal(sends.length,1);assert.match(sends[0].args.input.content,/focus the next reply/);await shot(p,'14-steer-followup-saved');return m;});
 await scenario('08-stop-pauses-steer',async()=>{const m=await pageFixture({available:true,replyStatus:'running'}),p=m.page;await chooseChat(p);await collapse(p);await p.locator('.conversation-composer textarea').fill('Synthetic direction retained after stop');await p.locator('.composer-steer').click();await p.getByRole('button',{name:'Stop reply',exact:true}).click();await p.getByText('Steer paused',{exact:true}).waitFor();assert.equal(await p.getByRole('button',{name:'Send steer',exact:true}).isDisabled(),true);assert.equal(await p.evaluate(()=>window.__QA__.mutations.filter(c=>c.cmd==='send_conversation_message').length),0);await shot(p,'15-steer-paused-after-stop');await p.evaluate(()=>{window.__QA__.snapshots['chat-1'].latestReply.status='cancelled'});await p.locator('.conversation-actions summary').click();await p.getByRole('button',{name:'Refresh history',exact:true}).click();await p.waitForFunction(()=>!document.querySelector('.composer-submit.is-stop'));assert.equal(await p.evaluate(()=>window.__QA__.mutations.filter(c=>c.cmd==='send_conversation_message').length),0);await p.getByRole('button',{name:'Send steer',exact:true}).click();await p.waitForFunction(()=>window.__QA__.mutations.some(c=>c.cmd==='send_conversation_message'));return m;});
 await scenario('09-two-window-steer-lock',async()=>{const m=await pageFixture({available:true,replyStatus:'running',preserveStorage:true}),p=m.page;assert.equal(await p.evaluate(()=>typeof navigator.locks?.request),'function','Native Web Locks are available on localhost');const second=await m.context.newPage();second.on('pageerror',e=>m.errors.push(String(e)));await second.goto(origin);await second.locator('.conversation-workspace').waitFor();await chooseChat(p);await chooseChat(second);await collapse(p);await collapse(second);await p.locator('.conversation-composer textarea').fill('Synthetic direction from window A');await second.locator('.conversation-composer textarea').fill('Synthetic direction from window B');await Promise.all([p.locator('.composer-steer').click(),second.locator('.composer-steer').click()]);await p.waitForFunction(()=>!!localStorage.getItem('oviraptor.conversation.steer.v1:1:chat-1'));const accepted=await p.locator('.conversation-steer-queue').count()+await second.locator('.conversation-steer-queue').count();assert.equal(accepted,1,'Exactly one competing direction is accepted');const saved=await p.evaluate(()=>JSON.parse(localStorage.getItem('oviraptor.conversation.steer.v1:1:chat-1')));const loser=(await p.locator('.conversation-steer-queue').count())?second:p;const losingText=await loser.locator('.conversation-composer textarea').inputValue();assert(losingText&&losingText!==saved.content,'Unaccepted direction remains editable');assert.equal(await p.evaluate(()=>window.__QA__.mutations.length),0);assert.equal(await second.evaluate(()=>window.__QA__.mutations.length),0);await shot(p,'16-steer-concurrent-window-a');await shot(second,'17-steer-concurrent-window-b');await second.close();return m;});
 await scenario('10-project-switch-isolates-chat-and-grants',async()=>{
  const m=await pageFixture({activity:true}),p=m.page;await chooseChat(p);let panel=await openAccess(p);
  await panel.locator('.access-disclosure input').check();await panel.getByRole('switch',{name:'Allow reading project assets',exact:true}).click();
  await p.waitForFunction(()=>window.__QA__.access['chat-1'].readAssets);
  await p.locator('.conversation-composer textarea').fill('Synthetic draft must not leak to another project');
  await p.getByRole('combobox',{name:'Choose workspace',exact:true}).selectOption('2');
  await p.locator('.conversation-row').getByText('Workspace 2 conversation',{exact:true}).waitFor();
  assert.equal(await p.locator('.conversation-row').count(),1,'History lists only the selected project');
  await selectHistory(p,'Workspace 2 conversation');panel=await openAccess(p);
  assert.match(await p.locator('.conversation-heading').textContent(),/Boreal/);
  assert.match(await p.locator('.conversation-transcript').textContent(),/history for 2/);
  assert.doesNotMatch(await p.locator('.conversation-transcript').textContent(),/history for 1|receipt for project 1/);
  assert.equal(await p.locator('.conversation-composer textarea').inputValue(),'','The previous project draft is never carried into a new scope');
  assert.equal(await panel.getByRole('switch',{name:'Allow reading project assets',exact:true}).getAttribute('aria-checked'),'false');
  await p.locator('.conversation-tool-activity summary').click();await shot(p,'20-project-two-scoped-activity');
  await p.getByRole('combobox',{name:'Choose workspace',exact:true}).selectOption('1');await selectHistory(p,'Workspace 1 conversation');panel=await openAccess(p);
  assert.equal(await panel.getByRole('switch',{name:'Allow reading project assets',exact:true}).getAttribute('aria-checked'),'true','Original project grant is restored from its backend response');
  const grants=await p.evaluate(()=>window.__QA__.mutations.filter(call=>call.cmd==='set_conversation_access'));
  assert.equal(grants.length,1);assert.equal(grants[0].args.input.projectId,1);assert.equal(grants[0].args.input.conversationId,'chat-1');return m;
 });
 await scenario('11-access-explicit-directory-and-live-revocation',async()=>{
  const m=await pageFixture({available:true}),p=m.page;await chooseChat(p);await collapse(p);const panel=await openAccess(p),assets=panel.getByRole('switch',{name:'Allow reading project assets',exact:true}),scans=panel.getByRole('switch',{name:'Allow reading saved scan results',exact:true});
  assert.equal(await assets.isDisabled(),true);assert.equal(await scans.isDisabled(),true);
  await panel.locator('.access-disclosure input').check();await assets.click();await p.waitForFunction(()=>window.__QA__.access['chat-1'].readAssets);
  await panel.locator('.access-skill-row').filter({hasText:'Synthetic saved skill'}).locator('input').check();
  await panel.getByRole('button',{name:'Choose directory',exact:true}).click();await panel.locator('.access-directory-confirm').waitFor();
  assert.equal(await p.evaluate(()=>window.__QA__.mutations.filter(call=>call.cmd==='grant_conversation_directory').length),0,'Native chooser only stages a directory');
  await panel.locator('.access-directory-confirm').getByRole('button',{name:'Cancel',exact:true}).click();
  await panel.getByRole('button',{name:'Choose directory',exact:true}).click();await panel.getByRole('button',{name:'Allow read-only access',exact:true}).click();
  await panel.getByRole('button',{name:'Revoke directory approved-project',exact:true}).waitFor();await shot(p,'21-explicit-tools-and-directory');
  await p.locator('.conversation-composer textarea').fill('Synthetic reply with scoped read-only tools');await p.getByRole('button',{name:'Send',exact:true}).click();await p.getByRole('button',{name:'Stop reply',exact:true}).waitFor();
  assert.equal(await scans.isDisabled(),true,'A running reply cannot gain scan access');assert.equal(await panel.getByRole('button',{name:'Choose directory',exact:true}).isDisabled(),true);
  assert.equal(await assets.isDisabled(),false,'A running reply may lose access');await assets.click();await panel.getByRole('button',{name:'Revoke directory approved-project',exact:true}).click();
  await p.waitForFunction(()=>!window.__QA__.access['chat-1'].readAssets&&!window.__QA__.access['chat-1'].directories.length);
  const mutations=await p.evaluate(()=>window.__QA__.mutations);
  assert.equal(mutations.filter(call=>call.cmd==='grant_conversation_directory').length,1);
  for(const call of mutations.filter(call=>['set_conversation_access','grant_conversation_directory'].includes(call.cmd))){assert.equal(call.args.input.providerIdentity,'synthetic-recipient-a');assert.equal(typeof call.args.input.expectedSkillRevision,'string');}
  await p.setViewportSize({width:390,height:844});await shot(p,'22-narrow-permissions-revoked-during-reply');assert((await fit(p)).document<=(await fit(p)).viewport+1);return m;
 });
 await scenario('12-provider-and-skill-change-requires-ui-review',async()=>{
  const m=await pageFixture(),p=m.page;await chooseChat(p);let panel=await openAccess(p);
  await panel.locator('.access-disclosure input').check();await panel.getByRole('switch',{name:'Allow reading project assets',exact:true}).click();
  await panel.locator('.access-skill-row').filter({hasText:'Synthetic saved skill'}).locator('input').check();
  await p.evaluate(()=>window.__QA__.changeProvider('synthetic-recipient-b','https://provider-b.example.invalid'));
  panel=await reloadAccess(p);
  assert.equal(await panel.locator('.access-disclosure input').isChecked(),false,'A different recipient clears consent in the actual mounted panel');
  assert.match(await panel.locator('.access-disclosure').textContent(),/provider-b\.example\.invalid/);
  assert.equal(await panel.getByRole('switch',{name:'Allow reading project assets',exact:true}).getAttribute('aria-checked'),'true','Saved grants remain visible while consent is inactive');
  assert.equal(await panel.getByRole('switch',{name:'Allow reading saved scan results',exact:true}).isDisabled(),true);
  await panel.locator('.access-disclosure input').check();await panel.getByRole('button',{name:'Confirm recipient and current skill content',exact:true}).click();
  await p.waitForFunction(()=>window.__QA__.access['chat-1'].acceptedProvider==='synthetic-recipient-b');
  await p.evaluate(()=>window.__QA__.changeSkill(1));panel=await reloadAccess(p);
  assert.equal(await panel.locator('.access-disclosure input').isChecked(),false,'Changed skill content also clears consent');
  assert.match(await panel.locator('.access-disclosure').textContent(),/Selected skills changed or were disabled/);
  await panel.locator('.access-disclosure input').check();await panel.getByRole('button',{name:'Confirm recipient and current skill content',exact:true}).click();
  await p.waitForFunction(()=>window.__QA__.access['chat-1'].acceptedSkills[1]===window.__QA__.skillVersions[1]);
  await p.evaluate(()=>window.__QA__.changeSkill(1,false));panel=await reloadAccess(p);
  const selected=panel.locator('.access-skill-row').filter({hasText:'Synthetic saved skill'}).locator('input');assert.equal(await selected.isChecked(),true);assert.equal(await selected.isDisabled(),false,'Disabled selected skills remain removable');
  await selected.uncheck();await p.waitForFunction(()=>window.__QA__.access['chat-1'].selectedSkillIds.length===0);
  await shot(p,'23-recipient-and-skill-review');
  const changes=await p.evaluate(()=>window.__QA__.mutations.filter(call=>call.cmd==='set_conversation_access'));
  assert(changes.some(call=>call.args.input.providerIdentity==='synthetic-recipient-b'&&call.args.input.providerDisclosureAccepted),'Explicit UI confirmation binds the replacement recipient');return m;
 });
 await scenario('13-plain-chat-and-saved-activity',async()=>{
  const m=await pageFixture({available:true,activity:true,access:{agentEnabled:false,readAssets:true,providerDisclosureAccepted:true}}),p=m.page;await chooseChat(p);await collapse(p);const panel=await openAccess(p);
  assert.equal(await panel.getByRole('button',{name:'Plain chat',exact:true}).getAttribute('aria-pressed'),'true');assert.match(await p.locator('.composer-mode').textContent(),/Plain chat/);
  assert.equal(await panel.getByRole('switch',{name:'Allow reading saved scan results',exact:true}).isDisabled(),true);
  await p.locator('.conversation-tool-activity summary').click();const saved=await p.locator('.conversation-tool-activity').textContent();
  assert.match(saved,/Synthetic scoped assets receipt for project 1/);await selectHistory(p,'Synthetic second conversation');assert.equal(await p.locator('.conversation-tool-activity').count(),0);
  await selectHistory(p,'Workspace 1 conversation');await p.locator('.conversation-tool-activity summary').click();assert.equal(await p.locator('.conversation-tool-activity').textContent(),saved,'Backend-saved activity is restored when returning to the conversation');
  await p.locator('.conversation-composer textarea').fill('Synthetic ordinary chat remains usable with Agent disabled');await p.getByRole('button',{name:'Send',exact:true}).click();await p.getByRole('button',{name:'Stop reply',exact:true}).waitFor();
  assert.equal(await p.evaluate(()=>window.__QA__.mutations.filter(call=>call.cmd==='send_conversation_message').length),1);assert.equal(await p.locator('.conversation-title-source').count(),0,'Plain chat does not display an AI-generated title');
  await shot(p,'24-plain-chat-saved-activity');return m;
 });
 await scenario('14-skills-import-preview-confirm-error-and-resource',async()=>{
  const m=await pageFixture(),p=m.page;await chooseChat(p);await p.locator('.workspace-plugin-modules').getByRole('button',{name:'Skills',exact:true}).click();await p.locator('.skills-workspace').waitFor();await p.getByRole('button',{name:'Import local package',exact:true}).click();
  const preview=p.locator('.skills-import-preview');await preview.waitFor();assert.match(await preview.textContent(),/Synthetic review methods|Import saves local files only/);
  assert.equal(await p.evaluate(()=>window.__QA__.mutations.length),0,'Selecting and previewing a folder cannot import or grant access');await shot(p,'25-skills-package-import-preview');
  await p.evaluate(()=>window.__QA__.failNextSkillImport=true);await preview.getByRole('button',{name:'Import package',exact:true}).click();await p.getByRole('alert').filter({hasText:'Synthetic import failed'}).waitFor();assert.equal(await preview.isVisible(),true,'An import error preserves the explicit preview for review');
  await preview.getByRole('button',{name:'Import package',exact:true}).click();await p.locator('.skills-library-card').waitFor();assert.equal(await preview.count(),0);assert.match(await p.locator('.skills-package').textContent(),/Synthetic imported skill/);
  await p.getByRole('button',{name:'Instructions & resources',exact:true}).click();await p.locator('.skills-resource-panel pre').waitFor();assert.match(await p.locator('.skills-resource-panel pre').textContent(),/Read project evidence/);
  await p.locator('.skills-resource-files').getByRole('button',{name:/scripts\/example\.js/}).click();await p.locator('.skills-script-notice').waitFor();assert.match(await p.locator('.skills-resource-panel pre').textContent(),/Nothing is executed/);
  await p.getByRole('checkbox',{name:'Enable Synthetic imported skill',exact:true}).uncheck();await p.waitForFunction(()=>window.__QA__.skillPackages[0].skills[0].enabled===false);await shot(p,'26-skills-imported-read-only-resource');
  const mutations=await p.evaluate(()=>window.__QA__.mutations);assert.deepEqual(mutations.map(call=>call.cmd),['import_skill_package','import_skill_package','set_packaged_skill_enabled']);assert.equal(mutations[0].args.input.expectedDigest,'a'.repeat(64));
  await selectHistory(p,'Workspace 1 conversation');assert.equal(await p.locator('.conversation-workspace').isVisible(),true,'Skills manager returns to the preserved conversation');return m;
 });
})().catch(error=>{console.error(error);results.push({name:'setup',status:'failed',error:String(error)});}).finally(async()=>{const browserVersion=browser?.version()??null;for(const context of contexts)await context.close().catch(()=>{});await browser?.close();if(server)await new Promise(resolve=>server.close(resolve));const manifestPath=path.join(output,'source-before.sha256');if(fs.existsSync(manifestPath)){const original=fs.readFileSync(manifestPath,'utf8');const after=original.trim().split('\n').map(line=>{const file=line.slice(66);return crypto.createHash('sha256').update(fs.readFileSync(path.join(repo,file))).digest('hex')+'  '+file}).join('\n')+'\n';fs.writeFileSync(path.join(output,'source-after-browser.sha256'),after);if(original!==after)results.push({name:'stable-source-inputs',status:'failed',error:'Application source changed during browser QA'});}
const counts={passed:results.filter(r=>r.status==='passed').length,failed:results.filter(r=>r.status==='failed').length,runtimeErrors:results.reduce((sum,r)=>sum+(r.errors?.length??0),0),blockedRequests:results.reduce((sum,r)=>sum+(r.blocked?.length??0),0),unknownIpc:results.reduce((sum,r)=>sum+(r.state?.unknown?.length??0),0),externalFetch:results.reduce((sum,r)=>sum+(r.state?.network?.length??0),0)};
fs.writeFileSync(path.join(output,'browser-results.json'),JSON.stringify({commit:process.env.GITHUB_SHA||null,browserLaunched:!!browser,browserVersion,chromiumExecutable:process.env.OVIRAPTOR_QA_CHROMIUM||'Playwright managed',scenarioFilter:scenarioFilter?.source??null,counts,captureLayouts,screenshots:fs.readdirSync(artifacts).filter(name=>name.endsWith('.png')),mode:'Actual Chromium pixels; actual Vue app; synthetic-only IPC; no provider/scans/credentials',results},null,2));if(results.some(r=>r.status==='failed'))process.exitCode=1;console.log(`${counts.passed}/${results.length} scenarios passed`);});
