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
  qa.addMessage = (id, content, role='assistant') => {
    const s=qa.snapshots[id], sequence=(s.messages.at(-1)?.sequence || 0)+1;
    const message={id:`synthetic-${id}-${sequence}`,conversationId:id,sequence,role,content,replyToMessageId:null,createdAt:now};
    s.messages.push(message);s.conversation.messageCount=s.messages.length;
    return message;
  };
  qa.completeReply = id => { const s=qa.snapshots[id]; qa.addMessage(id, 'Synthetic saved reply after completion');s.latestReply.status='completed'; };
  for(const snapshot of Object.values(qa.snapshots)) {
    if(options.available) snapshot.replyAvailability={available:true,code:'synthetic_ready',message:'Synthetic model transport only',model:'synthetic-qa-model'};
    if(options.markdown) snapshot.messages[1].content=options.markdown;
    if(options.long) snapshot.messages[1].content='## Long synthetic reply\n\n' + 'A'.repeat(4000) + '\n\n```text\n'+'B'.repeat(3000)+'\n```\n\n| Column one | Column two |\n| --- | --- |\n| '+ 'C'.repeat(500)+' | synthetic |';
    if(options.replyStatus) snapshot.latestReply={id:'synthetic-reply-1',userMessageId:snapshot.messages[0].id,status:options.replyStatus,code:'synthetic',message:'Synthetic active reply; no provider call',assistantMessageId:null,outcomeUnknown:false,createdAt:now,updatedAt:now};
    if(options.earlier){snapshot.messages.forEach(m=>m.sequence+=100);snapshot.hasMore=true;}
  }
  const second=clone(qa.conversations[0]);
  if(second){second.id='chat-1-second';second.title='Synthetic second conversation';qa.conversations.push(second);qa.snapshots[second.id]={...clone(qa.snapshots['chat-1']),conversation:second,messages:[{id:'synthetic-second-1',conversationId:second.id,sequence:1,role:'assistant',content:'Synthetic second conversation history',replyToMessageId:null,createdAt:now}]};}
  Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async value=>{if(qa.clipboardFail)throw Error('Synthetic clipboard rejected');qa.clipboard.push(value)}}});
  window.__TAURI_INTERNALS__.invoke=async (cmd,args={})=>{
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
      const c={id:'synthetic-new-chat',projectId:input.projectId,title:'New synthetic conversation',archived:false,revision:1,messageCount:0,lastMessagePreview:'',createdAt:now,updatedAt:now};qa.conversations.unshift(c);qa.snapshots[c.id]={conversation:c,messages:[],hasMore:false,links:[],replyAvailability:{available:!!options.available,code:'synthetic',message:'Synthetic fixture',model:options.available?'synthetic-qa-model':null},latestReply:null};return clone(c);
    }
    if(cmd==='update_conversation'){const s=qa.snapshots[input.conversationId];Object.assign(s.conversation,{...(input.title?{title:input.title}:{}),...(typeof input.archived==='boolean'?{archived:input.archived}:{}),revision:s.conversation.revision+1});return clone(s.conversation);}
  };
  window.fetch=async (...args)=>{qa.network.push(String(args[0]));throw Error('Synthetic QA blocks fetch');};
};
