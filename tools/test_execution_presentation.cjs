const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const ts = require('typescript');

const modulePath = path.join(__dirname, '../src/features/sentinel/execution/presentation.ts');
const boardPath = path.join(__dirname, '../src/features/sentinel/components/SentinelBoard.vue');

test('historical backend is a display-only label, never a selectable native backend', () => {
  const compiled = ts.transpileModule(fs.readFileSync(modulePath, 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  }).outputText;
  const presentation = {};
  new Function('exports', compiled)(presentation);
  assert.equal(presentation.agentBackendLabel('native'), '原生 Agent');
  assert.equal(presentation.agentBackendLabel('legacy_backend_removed'), '历史封存（只读）');
  for (const value of [undefined, '', 'other', 'Native', 'legacy_backend_removed_active', 'strix']) {
    assert.equal(presentation.agentBackendLabel(value), '未记录');
  }
  assert.equal(presentation.agentModeLabel('deep'), '深度');
  assert.equal(presentation.agentModeLabel(undefined), '未记录');
  const source = fs.readFileSync(modulePath, 'utf8');
  assert.doesNotMatch(source, /strix/i);
});

test('attempt plan summary identifies only an exact native backend', () => {
  const summaryPath = path.join(__dirname, '../src/features/sentinel/presentation.ts');
  const compiled = ts.transpileModule(fs.readFileSync(summaryPath, 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  }).outputText;
  const presentation = {};
  new Function('exports', compiled)(presentation);
  const plan = engine => presentation.attemptBackendSummary({
    backendPlanJson: JSON.stringify({ targets: [{ backend: engine }] }), status: 'completed',
  }).backend;
  assert.equal(plan('native'), 'Native');
  for (const value of ['strix', 'notnative', 'Native', 'native-extra', 'unregistered']) {
    assert.equal(plan(value), '未识别的执行计划（只读）');
  }
  assert.equal(presentation.attemptBackendSummary({ llmRequests: 1 }).backend, '后端计划未记录（只读）');
  assert.equal(presentation.attemptBackendSummary({ llmRequests: 0 }).backend, '尚未进入后端执行');
  assert.doesNotMatch(fs.readFileSync(summaryPath, 'utf8'), /strix/i);
});

test('page container only consumes isolated presentation, without historical backend literals', () => {
  const board = fs.readFileSync(boardPath, 'utf8');
  const details = fs.readFileSync(path.join(__dirname, '../src/features/sentinel/components/SentinelExecutionDetails.vue'), 'utf8');
  assert.match(board, /from "\.\/SentinelExecutionDetails\.vue"/);
  assert.match(board, /:scope="agentExecutionScope"/);
  assert.match(details, /from "\.\.\/execution\/presentation"/);
  assert.match(details, /agentBackendLabel\(String\(execution\.backend\)\)/);
  assert.doesNotMatch(board, /strix/i);
});
