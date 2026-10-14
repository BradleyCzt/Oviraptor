module.exports = function fixture({ project = null, long = false, empty = false, hackerOne = false, enabled = null, route = null, preference = null, preserveStorage = false } = {}) {
  if(!preserveStorage)localStorage.clear(); localStorage.setItem('oviraptor-locale', 'en');
  if(enabled) localStorage.setItem('oviraptor-builtin-plugins-v1',JSON.stringify({version:1,enabled}));
  if(preference) localStorage.setItem('oviraptor-builtin-plugins-v1',preference);
  if(route){localStorage.setItem('oviraptor-module',route.module);localStorage.setItem('oviraptor-view',route.view)}
  if(project) localStorage.setItem('oviraptor-project', String(project));
  const now = '2026-10-13T11:00:00Z';
  const projects = empty ? [] : [
    {id:1,name:long ? 'A'.repeat(160) : 'Atlas · Synthetic workspace',description:'Synthetic QA data only',status:'active'},
    {id:2,name:long ? '项目'.repeat(80) : 'Boreal · Separate workspace',description:'Separate synthetic workspace',status:'active'},
    {id:3,name:'Archived synthetic workspace',description:'Read-only QA',status:'archived'},
  ].map(p => ({assetCount:0,pendingCount:0,targetCount:0,assetRunCount:0,scanCount:0,vulnerabilityCount:0,validationCount:0,activeFuseCount:0,createdAt:now,updatedAt:now,...p}));
  const profiles = [{id:1,name:'Synthetic local fixture',description:'No providers configured; no network execution',isDefault:true,settings:{priorityRate:1,workers:1,hackerOneUsername:hackerOne?'synthetic-qa':'',hackerOneToken:hackerOne?'nonfunctional-synthetic-value':''},createdAt:now,updatedAt:now}];
  const makeConversation = p => ({id:`chat-${p}`,projectId:p,title:long?'VeryLongConversationTitle'.repeat(7):`Workspace ${p} conversation`,archived:false,revision:1,messageCount:2,lastMessagePreview:'Synthetic local-only history',createdAt:now,updatedAt:now});
  const conversations = projects.map(p => makeConversation(p.id));
  const snapshots = Object.fromEntries(conversations.map(c => [c.id,{conversation:c,messages:[{id:`m-${c.id}-1`,conversationId:c.id,sequence:1,role:'user',content:long?'https://example.invalid/'+ 'unbroken'.repeat(150):'This is synthetic conversation history for '+c.projectId,replyToMessageId:null,createdAt:now},{id:`m-${c.id}-2`,conversationId:c.id,sequence:2,role:'assistant',content:'Saved local conversation. QA does not contact a model provider.',replyToMessageId:`m-${c.id}-1`,createdAt:now}],hasMore:false,links:c.projectId===1?[{kind:'asset',targetId:'11',label:long?'LongAssetLabel'.repeat(30):'Synthetic asset 11',available:true},{kind:'scan',targetId:'scan-1',label:long?'LongScanLabel'.repeat(30):'Synthetic scan 1',available:true},{kind:'scan',targetId:'missing-scan',label:'Unavailable scan',available:false}]:[],replyAvailability:{available:false,code:'model_unavailable',message:'Synthetic QA: no model provider configured',model:null},latestReply:null}]));
  const callbacks = {}, listeners = {}; let next=0;
  window.__QA__={delays:{},calls:[],unknown:[],mutations:[],projects,profiles,conversations,snapshots,emit(event,payload){for(const l of Object.values(listeners))if(l.event===event) callbacks[l.handler]?.({event,payload,id:l.handler})}};
  window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(_event,id){delete listeners[id]}};
  window.__TAURI_INTERNALS__={transformCallback(fn){callbacks[++next]=fn;return next},unregisterCallback(id){delete callbacks[id]},convertFileSrc(){return ''},async invoke(cmd,args={}){
    window.__QA__.calls.push({cmd,args});
    if(window.__QA__.delays[cmd]) await new Promise(r=>setTimeout(r,window.__QA__.delays[cmd]));
    if(cmd==='plugin:event|listen'){listeners[++next]=args;return next}
    if(cmd==='plugin:event|unlisten')return;
    switch(cmd){
      case 'list_projects':return projects;
      case 'list_config_profiles':return profiles;
      case 'get_app_settings':return {reminderDays:7,customIcon:false,deduplicatedAssets:0};
      case 'startup_status':return {reminderDays:7,staleProjects:[],interruptedJobs:[]};
      case 'dashboard_stats':return {projectCount:projects.length,assetCount:0,aliveCount:0,pendingCount:0,newCount:0,changedCount:0,blockedCount:0,runningJobs:0};
      case 'get_app_icon_data_url':return '';
      case 'list_worker_nodes':case 'list_hackerone_events':case 'list_runs':case 'list_asset_events':case 'list_hackerone_programs':case 'list_targets':case 'list_sentinel_scans':return [];
      case 'list_conversations':{const items=conversations.filter(c=>c.projectId===args.input.projectId && (!args.input.query || c.title.includes(args.input.query))); return {items,total:items.length}}
      case 'read_conversation':return snapshots[args.input.conversationId];
      case 'sentinel_overview_stats':return Object.fromEntries('taskCount urlCount fingerprintCount apiCount endpointCount vulnerabilityCount highRiskCount reviewerConfirmedCount reviewerHighRiskCount sourceReviewerConfirmedCount sourceReviewAuditedTaskCount sourceReviewUnavailableTaskCount sourceReviewUnverifiedTaskCount otherVulnerabilityCount validatedCount pendingVulnerabilityCount vulnerableUrlCount activeFuseCount opportunityCount readyOpportunityCount'.split(' ').map(x=>[x,0]));
      case 'investigation_overview':return Object.fromEntries('targetCount nodeCount edgeCount apiCount parameterCount hypothesisCount readyHypothesisCount identityDiffCount tokenWorthyCount averageInformationGain factCount promotedStrategyCount'.split(' ').map(x=>[x,0]));
      case 'list_environment_install_logs':return {rows:[],more:false,olderRows:0,earliestId:0};
      case 'list_logs':case 'list_sentinel_targets':case 'list_sentinel_vulnerability_scan_ids':case 'list_sentinel_opportunities':case 'list_sentinel_fuse_zone':case 'list_sentinel_validation_work_items':return [];
      case 'list_assets':{const q=args.query;const items=q.pageSize===50 && q.search!=='empty'?[{id:q.projectId*100+1,projectId:q.projectId,isDeleted:false,title:long?'LongAssetCandidate'.repeat(30):`Workspace ${q.projectId} synthetic asset`,host:'synthetic.example.invalid',link:'https://synthetic.example.invalid',assetKey:'synthetic'}]:[];return {items,total:items.length,page:q.page||1,pageSize:q.pageSize||100};}
      case 'search_sentinel_scan_page':return args.limit===50 && args.search!=='empty'?[{id:`scan-candidate-${args.projectId}`,projectId:args.projectId,updatedAt:now,taskName:long?'LongScanCandidate'.repeat(30):`Workspace ${args.projectId} synthetic scan`,projectName:'Synthetic workspace',scanType:'code',status:'completed'}]:[];
      case 'set_conversation_links':{window.__QA__.mutations.push({cmd,args});const v=snapshots[args.input.conversationId];if(v.conversation.projectId!==args.input.projectId||v.conversation.revision!==args.input.expectedRevision)throw Error('synthetic_revision_conflict');v.conversation.revision++;v.links=args.input.links.map(l=>({...l,label:`Synthetic ${l.kind} ${l.targetId}`,available:true}));return {...v.conversation};}

      default:window.__QA__.unknown.push({cmd,args});if(!/^(list_|get_|read_|page_|count_|dashboard_|exposure_|plugin:event)/.test(cmd))window.__QA__.mutations.push({cmd,args});throw new Error('QA fixture blocks unimplemented IPC: '+cmd);
    }
  }};
};
