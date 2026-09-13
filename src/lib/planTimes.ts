import type { PlanItem } from "../pcoStore";

/**
 * Where each plan item falls on the clock, the way Planning Center lays it out.
 *
 * PCO anchors a plan on the SERVICE START, not on the first item. Items marked
 * `service_position: "pre"` (rehearsal, pre-service slides, the countdown
 * video) are stacked BACKWARDS from the start time; "during" items run forward
 * from it; "post" items follow the end. Stacking everything forward from the
 * start time — what ProDeck did — put a 45-minute rehearsal at 8:00 and the
 * first song at 8:56 on a plan where PCO shows the first song at 8:00. That
 * was "the times from Planning Center are not accurate".
 *
 * Verified against this church's plan: Rehearsal 7:03:30, Count Down Video
 * 7:58:30, Praise 8:00:00, Sermon 8:26:06, Benediction 9:04:06, end 9:04:36.
 */

type Pos = "pre" | "during" | "post";
const posOf = (it: PlanItem): Pos =>
  it.position === "pre" || it.position === "post" ? it.position : "during";
const lenMs = (it: PlanItem) => (it.type === "header" ? 0 : Math.max(0, it.length || 0) * 1000);

/** Planned length of the service proper (during items only), seconds. */
export function serviceLengthSec(items: PlanItem[]): number {
  return items.filter((i) => posOf(i) === "during").reduce((a, i) => a + lenMs(i), 0) / 1000;
}

/** When the service is planned to END: start + the during items. */
export function servicePlannedEnd(items: PlanItem[], serviceStartTs: number | null): number | null {
  if (!serviceStartTs) return null;
  const len = serviceLengthSec(items);
  return len > 0 ? serviceStartTs + len * 1000 : null;
}

/** Start time of every item, keyed by id. Headers get the time of what follows them. */
export function itemStartTimes(items: PlanItem[], serviceStartTs: number): Map<string, number> {
  const out = new Map<string, number>();
  const pre = items.filter((i) => posOf(i) === "pre");
  const during = items.filter((i) => posOf(i) === "during");
  const post = items.filter((i) => posOf(i) === "post");
  // Pre-service: the LAST pre item ends exactly at the service start.
  let t = serviceStartTs - pre.reduce((a, i) => a + lenMs(i), 0);
  for (const i of pre) {
    out.set(i.id, t);
    t += lenMs(i);
  }
  t = serviceStartTs;
  for (const i of during) {
    out.set(i.id, t);
    t += lenMs(i);
  }
  for (const i of post) {
    out.set(i.id, t);
    t += lenMs(i);
  }
  return out;
}
