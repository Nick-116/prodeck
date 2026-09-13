import type { PlanItem } from "../pcoStore";
import { serviceLengthSec, servicePlannedEnd } from "./planTimes";

/**
 * "Keys to the stage" — the worship team's cue.
 *
 * While an item is live, count down to the end time Planning Center publishes for it. When the items that
 * follow it are songs and the countdown is inside the lead time, it's a CALL:
 * the team should be walking, and what they need to know is the keys. The
 * classic case is the closing set after the sermon, but it is deliberately
 * generic — any run of songs after any timed item — so it also fires before
 * the opening set if a pre-service countdown item is live.
 *
 * Pure so it can be tested against a clock we control.
 */

export type StageCallPhase = "idle" | "waiting" | "call" | "over";

export interface StageCallState {
  phase: StageCallPhase;
  /** The live item, if any. */
  live: PlanItem | null;
  /** Seconds until Planning Center says the live item ends; negative = over. null = PCO hasn't said. */
  remaining: number | null;
  /** The songs coming up next (the contiguous run after the live item). */
  next: PlanItem[];
}

export function nextSongsAfter(items: PlanItem[], fromIdx: number): PlanItem[] {
  const out: PlanItem[] = [];
  for (let i = fromIdx + 1; i < items.length; i++) {
    const t = items[i].type;
    if (t === "header") continue; // section headings carry no time and no key
    if (t !== "song") break;
    out.push(items[i]);
  }
  return out;
}

/**
 * `endsAt` is Planning Center's own end time for the live item — the clock the
 * LIVE screen counts down from, published as `live_end_at`. It is the one
 * source that matches what the person driving PCO actually did, and it is the
 * same on the booth and on a kiosk that just switched on. Null means PCO
 * hasn't said (nobody holds LIVE control, or the item has no length), and
 * the answer is then "no countdown", never a guess.
 */
export function stageCallState(
  items: PlanItem[],
  liveItemId: string | null,
  endsAt: number | null,
  now: number,
  leadSec: number,
): StageCallState {
  const liveIdx = liveItemId ? items.findIndex((i) => i.id === liveItemId) : -1;
  if (liveIdx < 0) {
    // Nothing live: show the first songs of the plan so the team still sees
    // their keys while they wait.
    const first = items.findIndex((i) => i.type === "song");
    const next = first >= 0 ? nextSongsAfter(items, first - 1) : [];
    return { phase: "idle", live: null, remaining: null, next };
  }
  const live = items[liveIdx];
  const next = nextSongsAfter(items, liveIdx);
  const remaining = endsAt !== null ? (endsAt - now) / 1000 : null;
  if (next.length === 0 || remaining === null) {
    return { phase: "waiting", live, remaining, next };
  }
  if (remaining < 0) return { phase: "over", live, remaining, next };
  if (remaining <= leadSec) return { phase: "call", live, remaining, next };
  return { phase: "waiting", live, remaining, next };
}

export function fmtClock(sec: number): string {
  const s = Math.max(0, Math.round(Math.abs(sec)));
  const m = Math.floor(s / 60);
  return `${m}:${String(s % 60).padStart(2, "0")}`;
}

/**
 * The closing set: the last contiguous run of songs in the plan. This is what
 * the team is being called to when nobody is driving PCO LIVE (or the live
 * item isn't followed by songs), so the keys still show.
 */
export function closingSongs(items: PlanItem[]): PlanItem[] {
  let end = items.length - 1;
  while (end >= 0 && items[end].type !== "song") end--;
  if (end < 0) return [];
  let start = end;
  while (start - 1 >= 0 && (items[start - 1].type === "song" || items[start - 1].type === "header")) start--;
  return items.slice(start, end + 1).filter((i) => i.type === "song");
}

/**
 * Planned length of the service proper, seconds. Only items that run DURING
 * the service count: a rehearsal or countdown video marked pre-service in PCO
 * happens before the start time and must not push the end out (it once put
 * this church's planned end 57 minutes late).
 */
export function planLengthSec(items: PlanItem[]): number {
  return serviceLengthSec(items);
}

/**
 * When the service is planned to END: its PCO start time plus the during
 * items. Dynamic — re-timing or adding an item during the morning moves it.
 * Null when the plan has no timed service to anchor to.
 */
export function serviceEndsAt(items: PlanItem[], serviceStartTs: number | null): number | null {
  return servicePlannedEnd(items, serviceStartTs);
}

/**
 * The cue keyed to the SERVICE ending, not the live item ending.
 *
 * For a church running services back to back, the service end is the hard
 * constraint and the sermon is what flexes — so "five minutes before the
 * service ends" is when the team needs to be walking, however long the
 * message ran. Independent of whether anyone advanced PCO LIVE: it fires off
 * the clock. The songs shown are the next ones after the live item when that
 * is known and musical, else the closing set.
 */
export function stageCallServiceState(
  items: PlanItem[],
  liveItemId: string | null,
  endsAt: number | null,
  now: number,
  leadSec: number,
): StageCallState {
  const liveIdx = liveItemId ? items.findIndex((i) => i.id === liveItemId) : -1;
  const live = liveIdx >= 0 ? items[liveIdx] : null;
  const after = liveIdx >= 0 ? nextSongsAfter(items, liveIdx) : [];
  const next = after.length > 0 ? after : closingSongs(items);
  const remaining = endsAt !== null ? (endsAt - now) / 1000 : null;
  if (remaining === null) return { phase: live ? "waiting" : "idle", live, remaining, next };
  if (remaining < 0) return { phase: "over", live, remaining, next };
  if (remaining <= leadSec) return { phase: "call", live, remaining, next };
  return { phase: live ? "waiting" : "idle", live, remaining, next };
}

