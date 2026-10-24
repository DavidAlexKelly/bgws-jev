// picture.ts: what units have been told, stored once per side (SIDE_PICTURE.md).
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";
import type { GameState } from "../../lib/state";
import { NO_REPORTS, ReportWriter, reportsHeldBy, sideReports, trackedBy, trackFor, withReport, type PictureSource } from "./picture";
import { elementRank } from "./rank";
import type { RtState, RtUnit, Track } from "./types";

const track = (seenAt: number): Track => ({ level: "partial", at: { lat: 0, lng: 0 }, seenAt, receivedAt: seenAt, from: "x", errorM: 0, moving: false });

describe("picture readers", () => {
  const fe = (id: string, side: "blue" | "red") => ({ id, side, combatStrength: 5 });
  const game = {
    forceElements: { u1: fe("u1", "blue"), u2: fe("u2", "blue"), e1: fe("e1", "red"), e2: fe("e2", "red") },
  } as unknown as GameState;
  let state = { game, units: { u1: {} as RtUnit, u2: {} as RtUnit }, reports: NO_REPORTS } as unknown as RtState;
  state = withReport(state, "u1", "e2", track(2));
  state = withReport(state, "u1", "e1", track(1));
  const source: PictureSource = state;

  it("trackFor: the unit's report, or nothing", () => {
    expect(trackFor(source, "u1", "e1")?.seenAt).toBe(1);
    expect(trackFor(source, "u1", "e9")).toBeUndefined();
    expect(trackFor(source, "u2", "e1")).toBeUndefined();
    expect(trackFor(source, "nobody", "e1")).toBeUndefined();
  });

  it("trackedBy and reportsHeldBy: every report, in the order held", () => {
    const seen: string[] = [];
    trackedBy(source, "u1", (enemyId) => seen.push(enemyId));
    expect(seen).toEqual(["e2", "e1"]);
    expect(reportsHeldBy(source, "u1").map(([id]) => id)).toEqual(["e2", "e1"]);
    expect(reportsHeldBy(source, "u2")).toEqual([]);
  });

  it("ReportWriter: listeners share a version; a unit holds one version per enemy; the old record is untouched", () => {
    const before = state;
    const frozen = JSON.stringify(before.reports, (_k, v: unknown) => (v instanceof Uint8Array ? [...v] : v));
    const writer = new ReportWriter(before.reports, elementRank(game));
    const fresh = track(5);
    writer.hear("blue", "u1", "e1", fresh);
    writer.hear("blue", "u2", "e1", { ...fresh });
    const after = { ...before, reports: writer.reports };
    // One version, both listeners; u1 left its old one, which nobody holds now and is gone.
    expect(after.reports.blue.e1).toHaveLength(1);
    expect(trackFor(after, "u1", "e1")?.seenAt).toBe(5);
    expect(trackFor(after, "u2", "e1")?.seenAt).toBe(5);
    // The previous record is exactly as it was: replays and worker patches depend on it.
    expect(JSON.stringify(before.reports, (_k, v: unknown) => (v instanceof Uint8Array ? [...v] : v))).toBe(frozen);
    expect(trackFor(before, "u1", "e1")?.seenAt).toBe(1);
    expect(trackFor(before, "u2", "e1")).toBeUndefined();
  });

  it("ReportWriter.forget drops versions for everyone who held them", () => {
    const writer = new ReportWriter(state.reports, elementRank(game));
    writer.forget("blue", (_enemyId, t) => t.seenAt > 1);
    const after = { ...state, reports: writer.reports };
    expect(trackFor(after, "u1", "e1")).toBeUndefined();
    expect(trackFor(after, "u1", "e2")?.seenAt).toBe(2);
    expect(after.reports.blue.e1).toBeUndefined();
  });

  it("sideReports: the newest version any living unit holds, and how many hold it", () => {
    const writer = new ReportWriter(state.reports, elementRank(game));
    writer.hear("blue", "u2", "e1", track(9));
    const heard = sideReports({ ...state, reports: writer.reports }, "blue");
    expect(heard.find((h) => h.enemyId === "e1")).toMatchObject({ holders: 2, track: { seenAt: 9 } });
    expect(heard.find((h) => h.enemyId === "e2")).toMatchObject({ holders: 1 });
  });
});

describe("pictures live only in the side record", () => {
  // SIDE_PICTURE.md: a unit has no picture of its own any more. Nothing may
  // bring one back: what a unit was told is read and written through picture.ts.
  const root = join(__dirname, "..", "..");
  const allowed = new Map<string, number>();
  const files = (dir: string): string[] =>
    readdirSync(dir).flatMap((name) => {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) return name === "node_modules" || name === "__golden__" ? [] : files(path);
      return /\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) ? [path] : [];
    });

  it("no code mentions a unit picture", () => {
    const offenders: string[] = [];
    for (const path of files(root)) {
      const rel = relative(root, path).split("\\").join("/");
      const code = readFileSync(path, "utf8")
        .split("\n")
        .filter((line) => !line.trim().startsWith("//") && !line.trim().startsWith("*"));
      const uses = code.filter((line) => /\.picture\b/.test(line)).length;
      if (uses > (allowed.get(rel) ?? 0)) offenders.push(`${rel}: ${uses}`);
    }
    expect(offenders).toEqual([]);
  });
});
