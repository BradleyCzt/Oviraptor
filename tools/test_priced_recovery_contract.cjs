const test = require('node:test'), assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), ts = require('typescript');
const root = path.join(__dirname, '..');
const compile = relative => ts.transpileModule(fs.readFileSync(path.join(root, relative), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
const legacy = {}, priced = {};
new Function('exports', compile('src/features/sentinel/recovery/contract.ts'))(legacy);
new Function('require', 'exports', compile('src/features/sentinel/recovery/priced.ts'))(
  name => { assert.equal(name, './contract'); return legacy; }, priced);
const hash = c => c.repeat(64);
const selection = { scanId:'scan-a', attemptNumber:1, rootRunId:'root-a', protectionReceiptId:'protection-a' };
const scope = { projectId:1, taskName:'New explicit task', urls:['https://fixture.invalid/'], allowedMethods:['GET'],
  authSessionIds:[], authSessionScopeId:'', model:'fixture-model', proxyBindingHash:null, orchestrationMode:'multi',
  scanMode:'standard', closure:'proof', skillIds:[], instruction:'Only this new scope', maxBudgetUsd:5,
  limits:[60000,60000,60000,20,80,0,0,0,0,600000], deadlineUtc:'2099-01-01T00:00:00Z' };
const pricing = { schemaVersion:1, maxBudgetUsd:'5', priceRevision:'fixture-v2',
  uncachedInput:'2000000000', cachedInput:'500000000', output:'4000000000', perRequest:'10000000' };
const op = { version:2, requestId:'11111111-1111-4111-8111-111111111111', selection, newScope:scope, pricing, proposalHash:hash('a') };
const proposal = () => ({ schemaVersion:2, purpose:'explicit_priced_new_target_work_after_original_protection_stop',
  selection, newScope:scope, pricingInput:pricing, proposalHash:hash('a'), budgetDimensionNames:legacy.recoveryDimensions,
  approval:false, executionAllowed:false, targetRequestsGranted:0, oldStopRetained:true, oldCostsRetained:true,
  oldScopeRenewed:false, unknownOriginalWorkReplayAllowed:false, freshBirthRequired:true, legacyScopeBudgetUsdIsDisplayOnly:true,
  pricingBasis:'approved_rate_card_not_provider_dollar_invoice',
  modelBinding:{ model:scope.model, endpointBinding:hash('b'), deployment:'cloud', credentialBinding:hash('c'),
    proxyBinding:hash('d'), maxContextTokens:64000, maxOutputTokens:null, temperature:0 },
  financialTerms:{ schemaVersion:1, maxNanoUsd:'5000000000', pricing:{ schemaVersion:1, basis:'approved_rate_card',
    tokenRateUnit:'nano_usd_per_million_tokens', cachedSemantics:'subset_of_input', rounding:'ceil_total_per_model_call',
    modelBindingHash:hash('e'), priceRevision:pricing.priceRevision, uncachedInput:pricing.uncachedInput,
    cachedInput:pricing.cachedInput, output:pricing.output, perRequest:pricing.perRequest } } });
const receipt = () => ({ schemaVersion:2, purpose:'explicit_priced_new_target_work_approval', requestId:op.requestId,
  proposalHash:op.proposalHash, approvalId:hash('f'), proposal:proposal(),
  consent:'approve_exact_priced_new_scope_preserve_old_stop_and_costs', status:'approved_pending_priced_fresh_birth',
  executionAllowed:false, targetRequestsGranted:0, oldScopeRenewed:false });

test('decimal and nano strings preserve exact bounded amounts and reject numeric coercion', () => {
  assert.equal(priced.decimalUsdNanos('0.000000001'),1n);
  assert.equal(priced.decimalUsdNanos('9223372036.854775807'),9223372036854775807n);
  for (const value of ['', '1e2','+5','05','5 ','0.0000000001','9223372036.854775808',5,NaN,null])
    assert.equal(priced.decimalUsdNanos(value),undefined,String(value));
  assert.equal(priced.validRecoveryPricing(pricing),true);
  assert.equal(priced.validRecoveryPricing({ ...pricing, output:'9223372036854775807' }),true);
  for (const patch of [{output:2},{output:'9223372036854775808'},{output:'01'},{output:'-1'},
    {maxBudgetUsd:'0'},{maxBudgetUsd:'10000.000000001'},{schemaVersion:2},{priceRevision:'unsafe revision'},
    {executionAllowed:false}]) assert.equal(priced.validRecoveryPricing({...pricing,...patch}),false,JSON.stringify(patch));
});
test('v2 operation and approved-rate receipt remain non-executable and separate from v1', () => {
  assert.equal(priced.validPricedRecoveryOperation(op,'scan-a'),true);
  assert.equal(priced.pricedProposalMatches(proposal(),selection,scope,pricing),true);
  assert.equal(priced.pricedApprovalMatches(receipt(),op),true);
  const unspecified = proposal(); unspecified.modelBinding.maxContextTokens = 0;
  assert.equal(priced.pricedProposalMatches(unspecified,selection,scope,pricing),true,'unknown bound remains pending and non-executable');
  assert.equal(legacy.validOperation(op,'scan-a'),false);
  assert.equal(legacy.approvalMatches(receipt(),{...op,version:1}),false);
  for (const patch of [{version:1},{requestId:'bad'},{newScope:{...scope,maxBudgetUsd:6}},
    {newScope:{...scope,urls:[...scope.urls,'https://second.invalid/']}},{proposalHash:'bad'},
    {executionAllowed:false}]) assert.equal(priced.validPricedRecoveryOperation({...op,...patch},'scan-a'),false);
  assert.equal(priced.pricedApprovalMatches(receipt(),{}),false);
});
test('changed rate semantics scope fees model and authority flags are rejected', () => {
  for (const patch of [{schemaVersion:1},{executionAllowed:true},{targetRequestsGranted:1},
    {pricingBasis:'provider_invoice'},{oldScopeRenewed:true},{freshBirthRequired:false},
    {newScope:{...scope,model:'different'}},{pricingInput:{...pricing,output:'5000000000'}},
    {modelBinding:{...proposal().modelBinding,credentialBinding:'secret'}},
    {modelBinding:{...proposal().modelBinding,maxContextTokens:-1}},
    {modelBinding:{...proposal().modelBinding,maxContextTokens:Number.MAX_SAFE_INTEGER+1}}, {modelBinding:undefined}]) {
    assert.equal(priced.pricedProposalMatches({...proposal(),...patch},selection,scope,pricing),false,JSON.stringify(patch));
  }
  for (const patch of [{uncachedInput:'3000000000'},{basis:'provider_invoice'},{rounding:'nearest'},
    {cachedSemantics:'additional_input'},{modelBindingHash:'bad'},{schemaVersion:2},{executionAllowed:false}]) {
    const p=proposal();p.financialTerms.pricing={...p.financialTerms.pricing,...patch};
    assert.equal(priced.pricedProposalMatches(p,selection,scope,pricing),false,JSON.stringify(patch));
  }
  for (const patch of [{maxNanoUsd:5000000000},{maxNanoUsd:'5000000001'},{schemaVersion:2}]) {
    const p=proposal();p.financialTerms={...p.financialTerms,...patch};
    assert.equal(priced.pricedProposalMatches(p,selection,scope,pricing),false);
  }
  for (const patch of [{schemaVersion:1},{executionAllowed:true},{status:'running'},
    {consent:'approve_exact_new_scope_preserve_old_stop_and_costs'},{requestId:'other'},{targetRequestsGranted:1}])
    assert.equal(priced.pricedApprovalMatches({...receipt(),...patch},op),false);
});
test('production IPC uses the new consent and forwards exact strings without legacy draft or execution calls', async () => {
  const calls=[], api={};
  new Function('require','exports',compile('src/features/sentinel/recovery/api.ts'))(name => {
    assert.equal(name,'@tauri-apps/api/core');return {invoke:async (...args) => {calls.push(args);return null;}};
  },api);
  await api.protectionRecoveryApi.pricedPreview(selection,scope,pricing);
  await api.protectionRecoveryApi.pricedApprove(op);
  await api.protectionRecoveryApi.pricedApproval(op.requestId);
  await api.protectionRecoveryApi.pricedApprovalForSource(selection);
  assert.deepEqual(calls,[['preview_native_priced_protection_recovery',{selection,newScope:scope,pricing}],
    ['approve_native_priced_protection_recovery',{selection,newScope:scope,pricing,requestId:op.requestId,
      proposalHash:op.proposalHash,consent:'approve_exact_priced_new_scope_preserve_old_stop_and_costs'}],
    ['get_native_priced_protection_recovery_approval',{requestId:op.requestId}],
    ['find_native_priced_protection_recovery_approval',{selection}]]);
  const source=fs.readFileSync(path.join(root,'src-tauri/src/lib.rs'),'utf8');
  for (const [name] of calls) assert.ok(source.includes(`scanning::${name},`),name);
});

if (process.env.OVIRAPTOR_PRICED_RECOVERY_PUBLIC_FIXTURE_FILE) {
  test('actual Rust localhost public receipt crosses the frontend contract unchanged', () => {
    const actual=JSON.parse(fs.readFileSync(process.env.OVIRAPTOR_PRICED_RECOVERY_PUBLIC_FIXTURE_FILE,'utf8'));
    const p=actual.proposal;
    const actualOp={version:2,requestId:actual.requestId,selection:p.selection,newScope:p.newScope,
      pricing:p.pricingInput,proposalHash:actual.proposalHash};
    assert.equal(priced.validPricedRecoveryOperation(actualOp,actualOp.selection.scanId),true);
    assert.equal(priced.pricedProposalMatches(p,actualOp.selection,actualOp.newScope,actualOp.pricing),true);
    assert.equal(priced.pricedApprovalMatches(actual,actualOp),true);
    assert.equal(legacy.approvalMatches(actual,{...actualOp,version:1}),false);
  });
}


test('priced cancellation has its own matched non-executable history and explicit IPC', async () => {
  const approved={...receipt(),originalModeReceiptId:'original-private-mode'};
  const value={schemaVersion:1,purpose:'cancel_unconsumed_priced_recovery_approval',cancellationId:hash('c'),
    requestId:op.requestId,approvalId:approved.approvalId,approvalHash:hash('d'),originalModeReceiptId:approved.originalModeReceiptId,
    reason:'user_cancelled_before_priced_fresh_birth',authorityRevoked:true,executionAllowed:false,
    targetRequestsGranted:0,oldStopRetained:true,oldCostsRetained:true};
  assert.equal(priced.pricedCancellationMatches(value,op,approved),true);
  for(const patch of [{requestId:'different'},{approvalId:hash('a')},{authorityRevoked:false},
    {executionAllowed:true},{targetRequestsGranted:1},{oldCostsRetained:false},{originalModeReceiptId:'foreign'},
    {purpose:'cancel_unconsumed_protection_recovery_draft'},{approvalHash:'invalid'},{schemaVersion:2},{extra:false}])
    assert.equal(priced.pricedCancellationMatches({...value,...patch},op,approved),false,JSON.stringify(patch));
  assert.equal(priced.pricedCancellationMatches(value,{...op,version:1},approved),false);
  const calls=[],api={};
  new Function('require','exports',compile('src/features/sentinel/recovery/api.ts'))(name=>{
    assert.equal(name,'@tauri-apps/api/core');return {invoke:async (...args)=>{calls.push(args);return null;}};
  },api);
  await api.protectionRecoveryApi.pricedCancel(op.requestId,approved.approvalId);
  await api.protectionRecoveryApi.pricedCancellation(op.requestId);
  assert.deepEqual(calls,[['cancel_native_priced_protection_recovery_approval',{requestId:op.requestId,
    approvalId:approved.approvalId,consent:'revoke_exact_pending_priced_approval_preserve_original_history'}],
    ['get_native_priced_protection_recovery_cancellation',{requestId:op.requestId}]]);
  const source=fs.readFileSync(path.join(root,'src-tauri/src/lib.rs'),'utf8');
  for(const [name] of calls)assert.ok(source.includes(`scanning::${name},`),name);
});
