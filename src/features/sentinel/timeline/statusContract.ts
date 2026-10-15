import type { NativeScanStatus } from "../types";
import { validTimelineRows } from "./projectionContract";

// IPC types do not validate runtime values. Keep the received event window
// distinct from the larger, locally merged message cache.
export function validStatusCursor(result: NativeScanStatus, after?: number, attempt?: number): boolean {
  const integer = (value: number) => Number.isSafeInteger(value) && value >= 0;
  const optionalBoolean = (value: boolean | undefined) => value === undefined || typeof value === "boolean";
  const first = result.timelineBeforeSequence;
  return integer(result.attemptNumber) && integer(result.latestSequence)
    && typeof result.isIncremental === "boolean"
    && optionalBoolean(result.hasEarlierTimeline) && optionalBoolean(result.timelineHasMore)
    && (first === undefined || (integer(first) && first <= result.latestSequence))
    && (!result.hasEarlierTimeline || (first ?? 0) > 0)
    && (!result.timelineHasMore || result.isIncremental)
    && (!result.isIncremental || (after !== undefined && attempt === result.attemptNumber
      && result.attemptNumber > 0 && result.latestSequence >= after
      && (!first || first > after)
      && (!result.timelineHasMore || result.latestSequence > after)));
}

export function validStatusTimeline(result: NativeScanStatus, after?: number): boolean {
  const first = result.timelineBeforeSequence;
  return Array.isArray(result.timeline) && result.timeline.length <= 100 && validTimelineRows(result.timeline)
    && (!result.timeline.length || first === undefined || first > 0)
    // Bounds refer to persisted events, not only projected messages. Empty or
    // sparse projections and timestamp ordering must not imply missing events.
    && result.timeline.every((item) => item.sequence <= result.latestSequence
      && (first === undefined || item.sequence >= first)
      && (!result.isIncremental || (after !== undefined && item.sequence > after)));
}
