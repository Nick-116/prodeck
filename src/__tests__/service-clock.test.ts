import { describe, expect, it } from "vitest";
import { servicePhase, displayServiceTimeId, serviceTimesOnly, type ClockTime } from "../lib/serviceClock";

// This church's real Sunday, as PCO returns it (UTC): a Thursday rehearsal,
// a 7:00 call time, then services at 8:00, 9:30, 11:00 and 12:30 local.
const T = (iso: string) => Date.parse(iso);
const TIMES: ClockTime[] = [
  { id: "reh", name: "Rehearsal", ts: T("2026-09-10T22:30:00Z"), type: "rehearsal" },
  { id: "call", name: "Call time/Sound Check", ts: T("2026-09-13T11:00:00Z"), type: "rehearsal" },
  { id: "s1", name: "", ts: T("2026-09-13T12:00:00Z"), type: "service" },
  { id: "s2", name: "", ts: T("2026-09-13T13:30:00Z"), type: "service" },
  { id: "s3", name: "", ts: T("2026-09-13T15:00:00Z"), type: "service" },
  { id: "s4", name: "", ts: T("2026-09-13T16:30:00Z"), type: "service" },
];

describe("service clock", () => {
  it("ignores rehearsal and call times", () => {
    expect(serviceTimesOnly(TIMES).map((t) => t.id)).toEqual(["s1", "s2", "s3", "s4"]);
  });

  it("counts down to the first service during sound check, not to the sound check", () => {
    const p = servicePhase(TIMES, T("2026-09-13T11:20:00Z"));
    expect(p.phase).toBe("before");
    if (p.phase === "before") {
      expect(p.time.id).toBe("s1");
      expect(p.secondsUntil).toBe(40 * 60);
    }
  });

  it("is live from a service's start until the next one begins", () => {
    let p = servicePhase(TIMES, T("2026-09-13T12:45:00Z"));
    expect(p.phase).toBe("live");
    if (p.phase === "live") expect(p.time.id).toBe("s1");
    // 9:30 has started: it is now the live one, whatever the 8:00 was doing.
    p = servicePhase(TIMES, T("2026-09-13T13:30:00Z"));
    if (p.phase === "live") expect(p.time.id).toBe("s2");
  });

  it("after the last service, stops being live once the window passes", () => {
    expect(servicePhase(TIMES, T("2026-09-13T17:30:00Z")).phase).toBe("live"); // 12:30 + 1h
    expect(servicePhase(TIMES, T("2026-09-13T19:00:00Z")).phase).toBe("done"); // 12:30 + 2.5h
  });

  it("gives Show Flow the service happening now, not the one selected at 7am", () => {
    // Booth left the 8:00 selected all morning — the reported bug.
    expect(displayServiceTimeId(TIMES, "s1", T("2026-09-13T13:45:00Z"))).toBe("s2");
    expect(displayServiceTimeId(TIMES, "s1", T("2026-09-13T15:05:00Z"))).toBe("s3");
    // Before anything has started, the first service.
    expect(displayServiceTimeId(TIMES, "s3", T("2026-09-13T11:30:00Z"))).toBe("s1");
  });

  it("falls back to the selection when the plan has no timed services", () => {
    expect(displayServiceTimeId([], "s2", T("2026-09-13T13:45:00Z"))).toBe("s2");
    expect(servicePhase([{ id: "x", name: "", ts: 0 }], 1).phase).toBe("done");
  });
});
