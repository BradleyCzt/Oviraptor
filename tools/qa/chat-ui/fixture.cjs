module.exports = function enhanceFixture(options = {}) {
  if(options.theme) localStorage.setItem('oviraptor-theme',options.theme);
  for(const [key,value] of Object.entries(options.storageSeed||{}))localStorage.setItem(key,value);
  const qa = window.__QA__, oldInvoke = window.__TAURI_INTERNALS__.invoke;
  const clone = value => JSON.parse(JSON.stringify(value));
  const now = '2026-10-13T12:00:00Z';
  qa.failNextSend = options.failNextSend || false;
  qa.failNextRead = false;
  qa.receipts = {};
  qa.network = [];
  qa.openedLinks = [];
  qa.clipboard = [];
  qa.chatReads = 0;
  qa.access = {};
  qa.nextDirectory = '/synthetic/approved-project';
  qa.failNextAccessWrite = false;
  qa.failNextAccessRead = false;
  qa.accessSkills = [{id:1,name:'Synthetic saved skill',description:'Metadata only until explicitly selected and read',enabled:true},{id:2,name:'Disabled synthetic skill',description:'Not available for selection',enabled:false}];
  const newAccess = c => ({version:1,projectId:c.projectId,conversationId:c.id,revision:1,agentEnabled:true,directorySupported:true,readAssets:false,readScans:false,selectedSkillIds:[],directories:[],providerDisclosureAccepted:false,...(options.access||{})});
  qa.addToolActivity = (id, userMessageId, name, status, summary) => {
    const s=qa.snapshots[id];s.toolActivity??=[];
    s.toolActivity.push({id:`synthetic-tool-${id}-${s.toolActivity.length+1}`,replyId:s.latestReply?.id||'synthetic-history-reply',userMessageId,round:1,callId:`synthetic-call-${s.toolActivity.length+1}`,name,status,summary,createdAt:now});
  };
  qa.addMessage = (id, content, role='assistant') => {
    const s=qa.snapshots[id], sequence=(s.messages.at(-1)?.sequence || 0)+1;
    const message={id:`synthetic-${id}-${sequence}`,conversationId:id,sequence,role,content,replyToMessageId:null,createdAt:now};
    s.messages.push(message);s.conversation.messageCount=s.messages.length;
    return message;
  };
  qa.completeReply = id => {
    const s=qa.snapshots[id], userId=s.latestReply?.userMessageId; const answer=qa.addMessage(id, 'Synthetic saved reply after completion'); answer.replyToMessageId=userId;
    s.latestReply.status='completed';s.latestReply.assistantMessageId=answer.id;s.conversation.revision++;
    if(s.conversation.titleSource==='pending'&&qa.access[id]?.agentEnabled){s.conversation.title='Synthetic AI-generated title';s.conversation.titleSource='ai';qa.addToolActivity(id,userId,'conversation_set_title','completed','A title was proposed and saved with this reply');}
    const row=qa.conversations.find(c=>c.id===id);if(row)Object.assign(row,s.conversation);
  };
  for(const snapshot of Object.values(qa.snapshots)) {
    snapshot.toolActivity=[];
    snapshot.conversation.titleSource=options.pendingTitle?'pending':'legacy';
    qa.access[snapshot.conversation.id]=newAccess(snapshot.conversation);
    if(options.activity)qa.addToolActivity(snapshot.conversation.id,snapshot.messages[0].id,'assets_search','completed',`Synthetic scoped assets receipt for project ${snapshot.conversation.projectId}`);
    if(options.available) snapshot.replyAvailability={available:true,code:'synthetic_ready',message:'Synthetic model transport only',model:'synthetic-qa-model'};
    if(options.markdown) snapshot.messages[1].content=options.markdown;
    if(options.long) snapshot.messages[1].content='## Long synthetic reply\n\n' + 'A'.repeat(4000) + '\n\n```text\n'+'B'.repeat(3000)+'\n```\n\n| Column one | Column two |\n| --- | --- |\n| '+ 'C'.repeat(500)+' | synthetic |';
    if(options.replyStatus) snapshot.latestReply={id:'synthetic-reply-1',userMessageId:snapshot.messages[0].id,status:options.replyStatus,code:'synthetic',message:'Synthetic active reply; no provider call',assistantMessageId:null,outcomeUnknown:false,createdAt:now,updatedAt:now};
    if(options.earlier){snapshot.messages.forEach(m=>m.sequence+=100);snapshot.hasMore=true;}
  }
  const second=clone(qa.conversations[0]);
  if(second){second.id='chat-1-second';second.title='Synthetic second conversation';qa.conversations.push(second);qa.snapshots[second.id]={...clone(qa.snapshots['chat-1']),conversation:second,messages:[{id:'synthetic-second-1',conversationId:second.id,sequence:1,role:'assistant',content:'Synthetic second conversation history',replyToMessageId:null,createdAt:now}]};}
  if(second)qa.access[second.id]=newAccess(second);
  Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async value=>{if(qa.clipboardFail)throw Error('Synthetic clipboard rejected');qa.clipboard.push(value)}}});
  window.__TAURI_INTERNALS__.invoke=async (cmd,args={})=>{
    if(cmd==='plugin:dialog|open'){qa.calls.push({cmd,args:clone(args)});if(qa.delays[cmd])await new Promise(r=>setTimeout(r,qa.delays[cmd]));return qa.nextDirectory;}
    if(['get_conversation_access','set_conversation_access','grant_conversation_directory','revoke_conversation_directory','list_conversation_access_skills'].includes(cmd)){
      qa.calls.push({cmd,args:clone(args)});const input=args.input||{},s=qa.snapshots[input.conversationId];
      if(!s||s.conversation.projectId!==input.projectId)throw Error('conversation_scope_mismatch');
      if(qa.delays[cmd])await new Promise(r=>setTimeout(r,qa.delays[cmd]));
      const value=qa.access[input.conversationId]??(qa.access[input.conversationId]=newAccess(s.conversation));
      if(cmd==='get_conversation_access'){if(qa.failNextAccessRead){qa.failNextAccessRead=false;throw Error('Synthetic permissions read failed');}return clone(value);}
      if(cmd==='list_conversation_access_skills')return clone(qa.accessSkills);
      qa.mutations.push({cmd,args:clone(args)});
      if(qa.failNextAccessWrite){qa.failNextAccessWrite=false;throw Error('Synthetic access save failed');}
      if(input.expectedRevision!==value.revision)throw Error('conversation_access_revision_conflict');
      const active=s.latestReply&&['queued','running','cancel_requested'].includes(s.latestReply.status);
      if(cmd==='set_conversation_access'){
        const expanding=input.agentEnabled&&!value.agentEnabled||input.readAssets&&!value.readAssets||input.readScans&&!value.readScans||input.selectedSkillIds.some(id=>!value.selectedSkillIds.includes(id));
        if(expanding&&active)throw Error('conversation_reply_active');
        if(expanding&&!input.providerDisclosureAccepted)throw Error('conversation_provider_disclosure_required');
        Object.assign(value,{agentEnabled:input.agentEnabled,readAssets:input.readAssets,readScans:input.readScans,selectedSkillIds:[...input.selectedSkillIds],providerDisclosureAccepted:input.providerDisclosureAccepted,revision:value.revision+1});
      }
      if(cmd==='grant_conversation_directory'){
        if(active)throw Error('conversation_reply_active');if(!input.providerDisclosureAccepted)throw Error('conversation_provider_disclosure_required');
        value.directories.push({id:`synthetic-grant-${value.revision}`,label:input.path.split('/').at(-1),path:input.path});value.providerDisclosureAccepted=true;value.revision++;
      }
      if(cmd==='revoke_conversation_directory'){value.directories=value.directories.filter(item=>item.id!==input.grantId);value.revision++;}
      return clone(value);
    }
    if(cmd==='plugin:opener|open_url'){qa.calls.push({cmd,args:clone(args)});qa.openedLinks.push(args.url);return;}

    if(!['list_conversations','read_conversation','send_conversation_message','cancel_conversation_reply','create_conversation','update_conversation'].includes(cmd))return clone(await oldInvoke(cmd,args) ?? null);
    qa.calls.push({cmd,args:clone(args)});
    const delayMs=cmd==='read_conversation'&&args.input?.beforeSequence?qa.delays.read_earlier:qa.delays[cmd];
    if(delayMs)await new Promise(r=>setTimeout(r,delayMs));
    const input=args.input || {};
    if(cmd==='list_conversations'){
      const items=qa.conversations.filter(c=>c.projectId===input.projectId&&(!input.query||c.title.includes(input.query))&&(input.includeArchived||!c.archived));return clone({items,total:items.length});
    }
    if(cmd==='read_conversation'){
      qa.chatReads++;
      if(qa.failNextRead){qa.failNextRead=false;throw Error('Synthetic read failed');}
      const s=clone(qa.snapshots[input.conversationId]);
      if(input.beforeSequence){s.messages=Array.from({length:10},(_,i)=>({id:`synthetic-older-${i}`,conversationId:input.conversationId,sequence:input.beforeSequence-10+i,role:'assistant',content:`Synthetic older message ${i}`,replyToMessageId:null,createdAt:now}));s.hasMore=false;}
      return s;
    }
    qa.mutations.push({cmd,args:clone(args)});
    if(cmd==='send_conversation_message'){
      if(qa.failNextSend){qa.failNextSend=false;throw Error('Synthetic uncertain send failure');}
      if(qa.receipts[input.operationId])return clone(qa.receipts[input.operationId]);
      const s=qa.snapshots[input.conversationId];
      if(input.expectedRevision!==s.conversation.revision)throw Error('conversation_revision_conflict');
      const userMessage=qa.addMessage(input.conversationId,input.content,'user');s.conversation.revision++;
      s.latestReply=input.requestReply?{id:'synthetic-reply-'+userMessage.id,userMessageId:userMessage.id,status:'running',code:'synthetic_running',message:'Synthetic reply fixture only',assistantMessageId:null,outcomeUnknown:false,createdAt:now,updatedAt:now}:null;
      const result={conversation:s.conversation,userMessage,reply:s.latestReply};qa.receipts[input.operationId]=clone(result);return clone(result);
    }
    if(cmd==='cancel_conversation_reply'){const s=qa.snapshots[input.conversationId];s.latestReply.status='cancel_requested';return clone(s.latestReply);}
    if(cmd==='create_conversation'){
      const c={id:'synthetic-new-chat',projectId:input.projectId,title:'New synthetic conversation',titleSource:'pending',archived:false,revision:1,messageCount:0,lastMessagePreview:'',createdAt:now,updatedAt:now};qa.conversations.unshift(c);qa.snapshots[c.id]={conversation:c,messages:[],hasMore:false,links:[],toolActivity:[],replyAvailability:{available:!!options.available,code:'synthetic',message:'Synthetic fixture',model:options.available?'synthetic-qa-model':null},latestReply:null};qa.access[c.id]=newAccess(c);return clone(c);
    }
    if(cmd==='update_conversation'){const s=qa.snapshots[input.conversationId];Object.assign(s.conversation,{...(input.title?{title:input.title,titleSource:'manual'}:{}),...(typeof input.archived==='boolean'?{archived:input.archived}:{}),revision:s.conversation.revision+1});return clone(s.conversation);}
  };
  window.fetch=async (...args)=>{qa.network.push(String(args[0]));throw Error('Synthetic QA blocks fetch');};
};
