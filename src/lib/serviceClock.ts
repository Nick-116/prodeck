/**
 * Which Planning Center service time a screen should be counting to, right now.
 *
 * The Service Countdown used to count to a time typed by hand, and Show Flow
 * printed start times from whichever service time was SELECTED — on a
 * four-service Sunday that meant every later service showed the first one's
 * times unless someone remembered to switch. Both now ask this instead.
 *
 * Only service-type times count: a "Call time / Sound check" is a rehearsal
 * time in PCO and nobody wants "ON AIR IN" counting to it. Pure, so it can be
 * tested against a clock we control.
 */

export interface ClockTime {
  id: string;
  name: string;
  ts: number; // epoch ms; 0 = unknown
  type?: string; // PCO time_type: "service" | "rehearsal" | "other"
}

export type ServicePhase =
  | { phase: "before"; time: ClockTime; secondsUntil: number }
  | { phase: "live"; time: ClockTime; secondsSince: number }
  | { phase: "done"; last: ClockTime | null };

/** How long after its start a service still counts as "the one happening". */
export const LIVE_WINDOW_MS = 2 * 60 * 60 * 1000;

export function serviceTimesOnly(times: ClockTime[]): ClockTime[] {
  return times
    .filter((t) => t.ts > 0 && (t.type ?? "service") === "service")
    .sort((a, b) => a.ts - b.ts);
}

/**
 * The service in progress (started, and either the next one hasn't started or
 * it's within the live window), else the next upcoming, else done for today.
 */
export function servicePhase(times: ClockTime[], now: number): ServicePhase {
  const svc = serviceTimesOnly(times);
  if (svc.length === 0) return { phase: "done", last: null };
  let live: ClockTime | null = null;
  for (let i = 0; i < svc.length; i++) {
    const t = svc[i];
    const next = svc[i + 1];
    const ended = next ? now >= next.ts : now - t.ts >= LIVE_WINDOW_MS;
    if (now >= t.ts && !ended) live = t;
  }
  if (live) return { phase: "live", time: live, secondsSince: Math.floor((now - live.ts) / 1000) };
  const upcoming = svc.find((t) => t.ts > now);
  if (upcoming) return { phase: "before", time: upcoming, secondsUntil: Math.ceil((upcoming.ts - now) / 1000) };
  return { phase: "done", last: svc[svc.length - 1] };
}

/**
 * The service time whose start Show Flow should print item times from: the one
 * happening now, else the next one; the operator's selection wins only while it
 * IS the one happening. Falls back to the selection when PCO has no timed
 * services (a plan with no times set).
 */
export function displayServiceTimeId(times: ClockTime[], selectedId: string | null, now: number): string | null {
  const p = servicePhase(times, now);
  if (p.phase === "live") return p.time.id;
  if (p.phase === "before") return p.time.id;
  return p.last?.id ?? selectedId;
}
