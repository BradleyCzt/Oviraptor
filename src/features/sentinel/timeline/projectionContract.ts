import type { AgentTimelineItem } from "../types";
import { chatDecisionDisplay } from "./rootDecisionContract";

// Identity is event-scoped. Delimiters can occur in either field, so all
// cache, reading and rendering consumers must encode the tuple losslessly.
export function timelineIdentity(item: Pick<AgentTimelineItem, "eventType" | "id">): string {
  return JSON.stringify([item.eventType, item.id]);
}

const optionalTextFields = [
  "threadKey", "targetKey", "correlationId", "assignmentId", "fromRole",
  "toRole", "fromRunId", "toRunId", "messageKind", "deliveryState", "ackState", "status",
  "workerId", "sliceId", "sourceUserDirectiveId",
] as const;

// Shared by live snapshots and historical pages. This checks the message
// envelope consumed by rendering/reading, not nested receipts or authorization.
// Timestamp order and event-scoped IDs remain valid; timestamps need not be ISO.
export function validTimelineRows(items: AgentTimelineItem[]): boolean {
  if (!Array.isArray(items)) return false;
  const sequences = new Set<number>();
  const identities = new Set<string>();
  for (const item of items) {
    if (!item || typeof item !== "object" || Array.isArray(item)
      || typeof item.id !== "string" || !item.id.trim()
      || typeof item.eventType !== "string" || !item.eventType.trim()
      || !Number.isSafeInteger(item.sequence) || item.sequence <= 0
      || typeof item.timestamp !== "string" || typeof item.summary !== "string"
      || optionalTextFields.some((field) => item[field] != null && typeof item[field] !== "string")) return false;
    for (const values of [item.reasonCodes, item.requiredApprovals]) {
      if (values != null && (!Array.isArray(values) || values.some((value) => typeof value !== "string"))) return false;
    }
    if (item.eventType === "root_decision" && !chatDecisionDisplay(item)) return false;
    if (item.workerId != null && !/^wrk-[0-9a-f]+$/.test(item.workerId)) return false;
    if (item.sourceUserDirectiveId != null && (item.eventType !== "homogeneous_guidance"
      || !/^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/i.test(item.sourceUserDirectiveId))) return false;
    if (item.eventType === "homogeneous_guidance" && (!/^dir-[0-9a-f]+$/.test(item.id)
      || !["received", "rule_screened", "awaiting_coordinator", "relayed", "rewritten", "rejected", "applied"].includes(item.status ?? ""))) return false;
    if (item.sliceId != null && !/^slc-[0-9a-f]+$/.test(item.sliceId)) return false;
    if ([item.modelRequests, item.totalTokens].some(value => value != null && (!Number.isSafeInteger(value) || value < 0))) return false;
    if (item.zeroIntent != null && (typeof item.zeroIntent !== "object"
      || !["covered_by_inflight", "blocked_on_inflight_output"].includes(item.zeroIntent.reason)
      || !Array.isArray(item.zeroIntent.evidence_assignment_ids) || !item.zeroIntent.evidence_assignment_ids.length
      || item.zeroIntent.evidence_assignment_ids.some(id => typeof id !== "string" || !/^asg-[0-9a-f]+$/.test(id))
      || typeof item.zeroIntent.note_zh !== "string" || Array.from(item.zeroIntent.note_zh).length > 80)) return false;
    const identity = timelineIdentity(item);
    if (sequences.has(item.sequence) || identities.has(identity)) return false;
    sequences.add(item.sequence);
    identities.add(identity);
  }
  return true;
}
