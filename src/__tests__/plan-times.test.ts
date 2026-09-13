import { describe, expect, it } from "vitest";
import { itemStartTimes, servicePlannedEnd, serviceLengthSec } from "../lib/planTimes";
import type { PlanItem } from "../pcoStore";

// This church's real 8:00 plan, positions as Planning Center returns them.
const mk = (id: string, type: string, len: number, position: "pre" | "during" | "post" = "during"): PlanItem =>
  ({ id, title: id, sequence: 0, length: len, type, description: "", key: "", leader: "", position }) as PlanItem;
const PLAN: PlanItem[] = [
  mk("h-reh", "header", 0),
  mk("Rehearsal Time", "item", 2700, "pre"),
  mk("h-cd", "header", 0),
  mk("Pre-Service Slides", "item", 600, "pre"),
  mk("Count Down Video", "item", 90, "pre"),
  mk("h-w", "header", 0),
  mk("Welcome", "item", 0),
  mk("Praise", "song", 296),
  mk("Thank God I'm Free", "song", 301),
  mk("Living Hope", "song", 153),
  mk("No Longer Slaves", "song", 366),
  mk("Worship Moment", "item", 240),
  mk("Giving", "item", 120),
  mk("h-s", "header", 0),
  mk("Video Announcements", "item", 60),
  mk("Sermon Bumper", "item", 30),
  mk("Sermon", "item", 2100),
  mk("Salvation moment", "item", 180),
  mk("Benediction", "item", 30),
];
const EIGHT = Date.parse("2026-09-13T12:00:00Z"); // 8:00 AM Eastern
const at = (h: number, m: number, s = 0) => EIGHT + ((h - 8) * 3600 + m * 60 + s) * 1000;

describe("plan times, laid out like Planning Center", () => {
  const starts = itemStartTimes(PLAN, EIGHT);

  it("puts the first song at the service start, not the rehearsal", () => {
    expect(starts.get("Praise")).toBe(at(8, 0));
    expect(starts.get("Welcome")).toBe(at(8, 0));
  });

  it("stacks pre-service items backwards so the countdown ends at the start", () => {
    expect(starts.get("Rehearsal Time")).toBe(at(7, 3, 30));
    expect(starts.get("Pre-Service Slides")).toBe(at(7, 48, 30));
    expect(starts.get("Count Down Video")).toBe(at(7, 58, 30));
  });

  it("matches PCO's own times through the sermon", () => {
    expect(starts.get("Thank God I'm Free")).toBe(at(8, 4, 56));
    expect(starts.get("No Longer Slaves")).toBe(at(8, 12, 30));
    expect(starts.get("Sermon")).toBe(at(8, 26, 6));
    expect(starts.get("Salvation moment")).toBe(at(9, 1, 6));
    expect(starts.get("Benediction")).toBe(at(9, 4, 6));
  });

  it("ends the service after the during items only — the rehearsal is before it", () => {
    expect(serviceLengthSec(PLAN)).toBe(64 * 60 + 36);
    expect(servicePlannedEnd(PLAN, EIGHT)).toBe(at(9, 4, 36));
    // …so the team is called at 8:59:36, five minutes before, while the sermon closes.
    expect(servicePlannedEnd(PLAN, EIGHT)! - 300_000).toBe(at(8, 59, 36));
  });

  it("treats an item with no position as during, so older data still works", () => {
    const legacy = PLAN.map((i) => ({ ...i, position: undefined })) as PlanItem[];
    // Everything forward from 8:00: the old behaviour, only when PCO gave no positions.
    expect(itemStartTimes(legacy, EIGHT).get("Rehearsal Time")).toBe(at(8, 0));
  });

  it("has no end without a start", () => {
    expect(servicePlannedEnd(PLAN, null)).toBeNull();
    expect(servicePlannedEnd([], EIGHT)).toBeNull();
  });
});
