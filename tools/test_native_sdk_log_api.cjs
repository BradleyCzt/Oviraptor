const assert=require('node:assert/strict'),test=require('node:test');
const {loadSentinelApi}=require('./sentinel_api_harness.cjs');
test('SDK log actual public API dispatches dedicated IPC with original scope and cursor only',async()=>{
 const calls=[],exports={sentinelApi:loadSentinelApi(async(...args)=>{calls.push(args);return {real:'transport-fixture'};})};
 assert.equal(typeof exports.sentinelApi.readNativeSdkLog,'function','existing public API must expose the separate committed SDK reader');
 const value=await exports.sentinelApi.readNativeSdkLog('A',2,2,19,100,'a'.repeat(64));
 assert.deepEqual(calls,[['read_native_sdk_log',{scanId:'A',attempt:2,cursorAttempt:2,ownerId:'a'.repeat(64),afterSequence:19,limit:100}]]);assert.deepEqual(value,{real:'transport-fixture'});
});
