const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');

function contract(relative) {
  const calls = [];
  const source = fs.readFileSync(path.join(__dirname, '..', relative), 'utf8');
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
  const exports = {};
  new Function('require', 'exports', compiled)(name => {
    assert.equal(name, '@tauri-apps/api/core');
    return { invoke: (command, args) => { calls.push({ command, args }); return Promise.resolve(); } };
  }, exports);
  return { exports, calls };
}

test('Skills catalog exposes instruction and knowledge lifecycles without Scanning producers or rule repositories', () => {
  const { exports } = contract('src/features/skills/catalogApi.ts');
  for (const owner of [exports.instructionCatalogApi, exports.knowledgeCatalogApi, exports.skillCatalogApi]) {
    for (const forbidden of ['generateAgentLearningCandidate', 'aggregateAgentKnowledge', 'listSecurityRulePacks', 'syncSecurityRulePack']) {
      assert.equal(owner[forbidden], undefined, `${forbidden} must remain with Scanning`);
    }
  }
  assert.deepEqual(Object.keys(exports.skillCatalogApi).sort(), [...Object.keys(exports.instructionCatalogApi), ...Object.keys(exports.knowledgeCatalogApi)].sort());
});

test('Instruction CRUD and portable imports preserve existing IPC names and selected input', async () => {
  const { exports: { instructionCatalogApi: api }, calls } = contract('src/features/skills/catalogApi.ts');
  const input = { id: 7, name: 'Synthetic instructions', description: 'Reference only', instructions: 'Read granted data.', enabled: false };
  await api.saveAgentSkill(input); await api.importAgentSkills('/synthetic/instructions.json'); await api.exportAgentSkills();
  assert.deepEqual(calls, [{ command: 'save_agent_instruction', args: { input } }, { command: 'import_agent_instructions', args: { path: '/synthetic/instructions.json' } }, { command: 'export_agent_instructions', args: undefined }]);
});

test('Knowledge reads retain scan provenance filters and review/application stay separate explicit requests', async () => {
  const { exports: { knowledgeCatalogApi: api }, calls } = contract('src/features/skills/catalogApi.ts');
  await api.listAgentKnowledge('source:synthetic-hash'); await api.listAgentLearningCandidates('pending', 'scan-synthetic');
  await api.reviewAgentLearningCandidate(11, 'accepted', 7); await api.applyAgentLearningCandidate(11);
  assert.deepEqual(calls, [{ command: 'list_agent_knowledge', args: { scanId: 'source:synthetic-hash' } }, { command: 'list_agent_learning_candidates', args: { status: 'pending', scanId: 'scan-synthetic' } }, { command: 'review_agent_learning_candidate', args: { candidateId: 11, decision: 'accepted', targetSkillId: 7 } }, { command: 'apply_agent_learning_candidate', args: { candidateId: 11 } }]);
});

test('Scanning owns generation and aggregate production with unchanged IPC inputs', async () => {
  const { exports: { scanningLearningApi: api }, calls } = contract('src/features/sentinel/learningApi.ts');
  await api.generateAgentLearningCandidate('scan-synthetic'); await api.aggregateAgentKnowledge('web');
  assert.deepEqual(calls, [{ command: 'generate_agent_learning_candidate', args: { scanId: 'scan-synthetic' } }, { command: 'aggregate_agent_knowledge', args: { scanType: 'web' } }]);
  assert.equal(api.reviewAgentLearningCandidate, undefined);
});

test('Scanning rule-pack synchronization remains explicit and preserves repository reference', async () => {
  const { exports: { scanningRulePacksApi: api }, calls } = contract('src/features/sentinel/rulePacksApi.ts');
  const input = { key: 'synthetic-pack', name: 'Synthetic', engine: 'semgrep', repository: 'https://example.invalid/rules', reference: 'reviewed-revision', enabled: false };
  await api.listSecurityRulePacks(); await api.saveSecurityRulePack(input); await api.syncSecurityRulePack(3); await api.deleteSecurityRulePack(3);
  assert.deepEqual(calls, [{ command: 'list_security_rule_packs', args: undefined }, { command: 'save_security_rule_pack', args: { input } }, { command: 'sync_security_rule_pack', args: { packId: 3 } }, { command: 'delete_security_rule_pack', args: { packId: 3 } }]);
});
