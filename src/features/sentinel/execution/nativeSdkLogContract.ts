// SDK-only constants and original physical owner. Process claim IDs never enter.
export type SdkDomain = "root" | "specialist" | "source_round" | "mapper_loop";
export type SdkStage = "prepared" | "sent" | "response_received" | "cost_saved" | "validated" | "terminal";
export interface SdkScope { scanId: string; attempt: number; ownerId?: string }
export interface SdkOwner {
 ownerId: string; domain: SdkDomain; dispatchKey: string; scanId: string; attempt: number;
 rootRunId: string; runId: string; assignmentId: string | null; leaseAttemptId: string;
 workerId: string | null; round: number; requestHash: string;
}
export interface SdkLogRow extends SdkOwner { sequence: number; ordinal: number; stage: SdkStage; costPhase: "" | "received" | "uncertain" | "unsent"; terminalState: "" | "returned" | "withheld" | "uncertain" | "unsent" | "failed"; time: string }
export interface SdkLogGap { ownerId: string; afterOrdinal: number; failedStage: SdkStage; code: "stage_write_failed"; time: string }
export interface NativeSdkLogPage {
 schemaVersion: 2; channel: "native_sdk"; scanId: string; attempt: number; ownerId: string | null;
 requestedAfterSequence: number; resetCursor: boolean; afterSequence: number; latestSequence: number;
 more: boolean; available: boolean; rows: SdkLogRow[]; owners: SdkOwner[]; ownersTruncated: boolean;
 gaps: SdkLogGap[]; gapsTruncated: boolean; incompleteOwners: string[]; incompleteOwnersTruncated: boolean;
}
export interface SdkReplay {
 attempt?: number; cursor: number; rows: SdkLogRow[]; owners: SdkOwner[]; gaps: SdkLogGap[];
 incompleteOwners: string[]; metadataTruncated: boolean; hasGap: boolean; evicted: number; more: boolean; available: boolean;
}
export const emptySdkReplay = (): SdkReplay => ({cursor:0,rows:[],owners:[],gaps:[],incompleteOwners:[],metadataTruncated:false,hasGap:false,evicted:0,more:false,available:false});
const integer=(v:unknown):v is number=>Number.isSafeInteger(v)&&(v as number)>=0;
const hash=(v:unknown):v is string=>typeof v==="string"&&/^[a-f0-9]{64}$/i.test(v);
const id=(v:unknown):v is string=>typeof v==="string"&&/^[a-z0-9_:.-]{1,128}$/i.test(v);
const uuid=(v:unknown):v is string=>typeof v==="string"&&/^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i.test(v);
const stages=new Set(["prepared","sent","response_received","cost_saved","validated","terminal"]);
const ownerFields=["ownerId","domain","dispatchKey","scanId","attempt","rootRunId","runId","assignmentId","leaseAttemptId","workerId","round","requestHash"] as const;
const pickOwner=(o:SdkOwner):SdkOwner=>Object.fromEntries(ownerFields.map(k=>[k,o[k]])) as unknown as SdkOwner;
const binding=(o:SdkOwner)=>JSON.stringify(ownerFields.map(k=>o[k]));
function ownerValid(o:SdkOwner,scope:SdkScope):boolean {
 return !!o&&hash(o.ownerId)&&hash(o.dispatchKey)&&hash(o.requestHash)&&o.scanId===scope.scanId
 &&integer(o.attempt)&&o.attempt>0&&(scope.attempt===0||o.attempt===scope.attempt)&&(!scope.ownerId||o.ownerId===scope.ownerId)
 &&id(o.rootRunId)&&id(o.runId)&&uuid(o.leaseAttemptId)&&integer(o.round)&&o.round>0
 &&(o.domain==="root"?o.runId===o.rootRunId&&o.workerId===null&&o.assignmentId===null
 :["specialist","source_round","mapper_loop"].includes(o.domain)&&o.runId!==o.rootRunId&&uuid(o.workerId)&&typeof o.assignmentId==="string"&&/^asg-[a-f0-9]{24}$/i.test(o.assignmentId)&&(o.domain!=="specialist"||o.round===1)&&(o.domain!=="mapper_loop"||o.round<=2));
}
export function acceptsSdkHint(raw:unknown,scope:SdkScope|undefined):boolean {
 if(!scope||!raw||typeof raw!=="object")return false;
 const h=raw as Partial<SdkLogRow>;
 return h.scanId===scope.scanId&&integer(h.attempt)&&h.attempt>0&&(scope.attempt===0||h.attempt===scope.attempt)
 &&hash(h.ownerId)&&(!scope.ownerId||h.ownerId===scope.ownerId)&&hash(h.dispatchKey)&&id(h.runId)
 &&integer(h.sequence)&&h.sequence>0&&(h.domain==="root"?h.workerId===null:["specialist","source_round","mapper_loop"].includes(h.domain??"")&&uuid(h.workerId)&&(h.domain!=="mapper_loop"||(integer(h.round)&&h.round>=1&&h.round<=2)));
}
function rowValid(r:SdkLogRow,scope:SdkScope):boolean {
 if(!ownerValid(r,scope)||!integer(r.sequence)||r.sequence<1||!integer(r.ordinal)||r.ordinal<1||r.ordinal>6
 ||!stages.has(r.stage)||!["","received","uncertain","unsent"].includes(r.costPhase)
 ||!["","returned","withheld","uncertain","unsent","failed"].includes(r.terminalState)||typeof r.time!=="string"||r.time.length>128)return false;
 if(r.stage==="terminal")return !!r.terminalState;
 if(r.terminalState)return false;
 return r.stage==="cost_saved"?!!r.costPhase:r.costPhase==="";
}
function transitionValid(last:SdkLogRow|undefined,row:SdkLogRow):boolean {
 if(!last)return row.ordinal===1?row.stage==="prepared":true;
 switch(row.stage){
  case "prepared":return false;
  case "sent":return last.stage==="prepared";
  case "response_received":return last.stage==="sent";
  case "cost_saved":return row.costPhase==="received"?last.stage==="response_received":row.costPhase==="unsent"?last.stage==="prepared":["sent","response_received"].includes(last.stage);
  case "validated":return last.stage==="cost_saved"&&last.costPhase==="received";
  case "terminal":return row.terminalState==="returned"?last.stage==="validated"&&row.costPhase==="received"
   :row.terminalState==="failed"?["prepared","sent","response_received"].includes(last.stage)&&row.costPhase===""
   :last.stage==="cost_saved"&&last.costPhase===row.costPhase&&(row.terminalState==="withheld"?row.costPhase==="received":row.terminalState===row.costPhase);
 }
}
// The authoritative IPC is re-read; event payloads are never rows or authority.
export function acceptSdkPage(raw:unknown,scope:SdkScope,state:SdkReplay):SdkReplay {
 if(!raw||typeof raw!=="object")throw Error("sdk_log_contract");const p=raw as NativeSdkLogPage;
 if(p.schemaVersion!==2||p.channel!=="native_sdk"||p.scanId!==scope.scanId||!integer(p.attempt)||p.attempt<1
 ||(scope.attempt>0&&p.attempt!==scope.attempt)||p.ownerId!==(scope.ownerId??null)||p.requestedAfterSequence!==state.cursor
 ||![p.resetCursor,p.more,p.available,p.ownersTruncated,p.gapsTruncated,p.incompleteOwnersTruncated].every(v=>typeof v==="boolean")
 ||!integer(p.afterSequence)||!integer(p.latestSequence)||![p.rows,p.owners,p.gaps,p.incompleteOwners].every(v=>Array.isArray(v)&&v.length<=300))throw Error("sdk_log_contract");
 const switched=state.attempt!==undefined&&state.attempt!==p.attempt;
 if(p.resetCursor!==switched||(switched&&scope.attempt!==0))throw Error("sdk_log_attempt_cursor");
 const old=switched?emptySdkReplay():state;const bound={...scope,attempt:p.attempt};const bindings=new Map<string,string>();
 for(const o of [...old.owners,...old.rows])bindings.set(o.ownerId,binding(o));
 const seenOwners=new Set<string>();
 const owners=p.owners.map(o=>{if(seenOwners.has(o.ownerId))throw Error("sdk_log_owner_duplicate");seenOwners.add(o.ownerId);if(!ownerValid(o,bound))throw Error("sdk_log_owner");const value=binding(o);
  if(bindings.has(o.ownerId)&&bindings.get(o.ownerId)!==value)throw Error("sdk_log_owner_rebound");bindings.set(o.ownerId,value);return pickOwner(o);});
 const rows=[...old.rows],ordinals=new Map<string,number>();for(const r of rows)ordinals.set(r.ownerId,r.ordinal);
 let cursor=old.cursor;for(const r of p.rows){
  if(!rowValid(r,bound)||r.sequence<=cursor||r.sequence>p.latestSequence)throw Error("sdk_log_row");const value=binding(r);
  if(bindings.has(r.ownerId)&&bindings.get(r.ownerId)!==value)throw Error("sdk_log_owner_rebound");bindings.set(r.ownerId,value);
  const previous=[...rows].reverse().find(row=>row.ownerId===r.ownerId);
  if(!transitionValid(previous,r))throw Error("sdk_log_stage_order");
  const last=ordinals.get(r.ownerId);if(last!==undefined&&r.ordinal!==last+1)throw Error("sdk_log_ordinal");
  if(last===undefined&&old.cursor===0&&r.ordinal!==1)throw Error("sdk_log_prefix");
  rows.push({...pickOwner(r),sequence:r.sequence,ordinal:r.ordinal,stage:r.stage,costPhase:r.costPhase,terminalState:r.terminalState,time:r.time});ordinals.set(r.ownerId,r.ordinal);cursor=r.sequence;
 }
 if(p.afterSequence!==cursor||(p.more&&(!p.rows.length||cursor>=p.latestSequence))||(!p.available&&(p.rows.length||p.owners.length||p.gaps.length||p.incompleteOwners.length)))throw Error("sdk_log_page_cursor");
 const gaps=p.gaps.map(g=>{if(!hash(g.ownerId)||(!scope.ownerId?false:g.ownerId!==scope.ownerId)||!integer(g.afterOrdinal)||g.afterOrdinal>6||!stages.has(g.failedStage)||g.code!=="stage_write_failed"||typeof g.time!=="string"||g.time.length>128)throw Error("sdk_log_gap");return {ownerId:g.ownerId,afterOrdinal:g.afterOrdinal,failedStage:g.failedStage,code:g.code,time:g.time};});
 const incompleteOwners=p.incompleteOwners.map(v=>{if(!hash(v)||(scope.ownerId&&v!==scope.ownerId))throw Error("sdk_log_incomplete_owner");return v;});
 const evicted=Math.max(0,rows.length-300);
 return {attempt:p.attempt,cursor,rows:rows.slice(-300),owners,gaps,incompleteOwners,
 metadataTruncated:p.ownersTruncated||p.gapsTruncated||p.incompleteOwnersTruncated,
 hasGap:old.hasGap||gaps.length>0,evicted:old.evicted+evicted,more:p.more,available:p.available};
}
