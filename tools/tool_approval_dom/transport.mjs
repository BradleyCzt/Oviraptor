import { reactive } from 'vue';
export const fixture = reactive({ mode: 'normal', calls: [], pending: 0 });
const approvalId = '11111111-1111-4111-8111-111111111111';
const waits = [];
const inventory = fence => ({ schemaVersion: 1, approvalId, toolId: 'mock-inventory-parser', version: '1.0',
  platform: 'darwin-arm64', packageDigest: `sha256:${'a'.repeat(64)}`, runtimeAdapter: 'static-parser',
  runtimeDigest: 'b'.repeat(64), approvalHash: 'c'.repeat(64), status: 'approved_inventory_only',
  executionAllowed: false, preparationFencePresent: fence });
function decode(frame) {
  if (!(frame instanceof Uint8Array) || frame.byteLength < 20 || new TextDecoder().decode(frame.subarray(0, 8)) !== 'OVTA0001') throw 'mock_binary_frame_invalid';
  const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
  const sizes = [view.getUint32(8, true), view.getUint32(12, true), view.getUint32(16, true)];
  if (sizes.some(n => n === 0) || sizes[0] > 16384 || sizes[1] > 16384 || sizes[2] > 67108864 || 20 + sizes.reduce((a, b) => a + b, 0) !== frame.byteLength) throw 'mock_binary_frame_invalid';
  return sizes;
}
export async function invoke(command, args) {
  const mode = fixture.mode;
  if (command === 'approve_tool_inventory') {
    const sizes = decode(args);
    fixture.calls.push({ command, sizes, bytes: args.byteLength });
    if (mode === 'pending') {
      fixture.pending++;
      try { await new Promise((resolve, reject) => waits.push({ resolve, reject })); }
      finally { fixture.pending--; }
    }
    if (mode === 'saved-error') throw `tool_administrator_receipt_saved_preparation_incomplete:${approvalId}:mock-fence-remains`;
    if (mode === 'unconfigured') throw 'tool_administrator_trust_unconfigured';
    return inventory(mode === 'fence');
  }
  if (command === 'read_tool_approval_inventory') {
    fixture.calls.push({ command, approvalId: args.approvalId });
    return args.approvalId === approvalId ? inventory(mode === 'fence') : null;
  }
  if (command === 'revoke_tool_inventory') {
    fixture.calls.push({ command, bytes: args.decision.length });
    if (mode === 'revoke-error') throw 'tool_administrator_revocation_saved_preparation_incomplete:mock-fence-remains';
    return { schemaVersion: 1, revocationId: '22222222-2222-4222-8222-222222222222',
      status: 'revoked_inventory_only', executionAllowed: false, preparationFencePresent: mode === 'fence' };
  }
  throw new Error(`fixture_native_command_forbidden:${command}`);
}
export function release() { const wait = waits.shift(); if (wait) wait.resolve(); }
export function reset() { while (waits.length) waits.shift().reject('mock_fixture_reset'); fixture.calls = []; fixture.mode = 'normal'; }
