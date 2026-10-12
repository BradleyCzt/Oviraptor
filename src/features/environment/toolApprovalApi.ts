import { invoke } from "@tauri-apps/api/core";

export const TOOL_APPROVAL_JSON_LIMIT = 16 * 1024;
export const TOOL_APPROVAL_PACKAGE_LIMIT = 64 * 1024 * 1024;

export interface ToolApprovalInventory {
  schemaVersion: 1;
  approvalId: string;
  toolId: string;
  version: string;
  platform: string;
  packageDigest: string;
  runtimeAdapter: string;
  runtimeDigest: string;
  approvalHash: string;
  status: "approved_inventory_only";
  executionAllowed: false;
  preparationFencePresent: boolean;
}
export interface ToolRevocationInventory {
  schemaVersion: 1;
  revocationId: string;
  status: "revoked_inventory_only";
  executionAllowed: false;
  preparationFencePresent: boolean;
}
function requireBounded(bytes: Uint8Array, limit: number, label: string): void {
  if (bytes.byteLength === 0 || bytes.byteLength > limit) {
    throw new Error(`${label}: 文件为空或超出 ${limit} 字节上限`);
  }
}
function approvalFrame(decision: Uint8Array, manifest: Uint8Array, packageBytes: Uint8Array): Uint8Array {
  // Check every segment before allocating or copying the binary transport frame.
  // Raw transport still buffers this full frame; this is not a transport-wide memory cap.
  requireBounded(decision, TOOL_APPROVAL_JSON_LIMIT, "批准决定");
  requireBounded(manifest, TOOL_APPROVAL_JSON_LIMIT, "工具清单");
  requireBounded(packageBytes, TOOL_APPROVAL_PACKAGE_LIMIT, "工具包");
  const frame = new Uint8Array(20 + decision.byteLength + manifest.byteLength + packageBytes.byteLength);
  frame.set([0x4f, 0x56, 0x54, 0x41, 0x30, 0x30, 0x30, 0x31]); // OVTA0001
  const header = new DataView(frame.buffer, frame.byteOffset, 20);
  header.setUint32(8, decision.byteLength, true);
  header.setUint32(12, manifest.byteLength, true);
  header.setUint32(16, packageBytes.byteLength, true);
  frame.set(decision, 20);
  frame.set(manifest, 20 + decision.byteLength);
  frame.set(packageBytes, 20 + decision.byteLength + manifest.byteLength);
  return frame;
}
export async function readToolApprovalFile(file: File, limit: number): Promise<Uint8Array> {
  if (file.size === 0 || file.size > limit) throw new Error(`${file.name}: 文件为空或超出 ${limit} 字节上限`);
  const bytes = new Uint8Array(await file.arrayBuffer());
  if (bytes.byteLength === 0 || bytes.byteLength > limit) throw new Error(`${file.name}: 读取内容超出限制`);
  return bytes;
}
function requireInventory<T extends { schemaVersion: number; executionAllowed: boolean; status: string; preparationFencePresent: boolean }>(
  inventory: T, status: string,
): T {
  if (!inventory || inventory.schemaVersion !== 1 || inventory.executionAllowed !== false || inventory.status !== status || typeof inventory.preparationFencePresent !== "boolean") {
    throw new Error("tool_administrator_inventory_response_invalid");
  }
  return inventory;
}
export const toolApprovalApi = {
  async approve(decision: Uint8Array, manifest: Uint8Array, packageBytes: Uint8Array): Promise<ToolApprovalInventory> {
    return requireInventory(await invoke<ToolApprovalInventory>("approve_tool_inventory",
      approvalFrame(decision, manifest, packageBytes)), "approved_inventory_only");
  },
  async revoke(decision: Uint8Array): Promise<ToolRevocationInventory> {
    requireBounded(decision, TOOL_APPROVAL_JSON_LIMIT, "撤销决定");
    return requireInventory(await invoke<ToolRevocationInventory>("revoke_tool_inventory", {
      decision: Array.from(decision),
    }), "revoked_inventory_only");
  },
  async read(approvalId: string): Promise<ToolApprovalInventory | null> {
    const inventory = await invoke<ToolApprovalInventory | null>("read_tool_approval_inventory", { approvalId });
    return inventory === null ? null : requireInventory(inventory, "approved_inventory_only");
  },
};
