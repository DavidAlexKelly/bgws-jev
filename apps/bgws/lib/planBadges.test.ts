import { describe, expect, it } from "vitest";

import { PLAN_BADGE_LEGEND, badgeFor, planBadgesFor } from "./planBadges";
import type { PlanGeometry, PlanLine } from "./planLines";

const at = (lat: number, lng: number) => ({ lat, lng });

function line(overrides: Partial<PlanLine> = {}): PlanLine {
  return {
    actorId: "blue-1",
    side: "blue",
    kind: "move",
    optionId: "blue-1:move:0",
    points: [at(54.7, 20.5), at(54.72, 20.52)],
    label: "advance north",
    ...overrides,
  };
}

const geometry = (over: Partial<PlanGeometry> = {}): PlanGeometry => ({
  lines: [],
  badges: [],
  ...over,
});

describe("one badge per thing the engine can order", () => {
  it("distinguishes the three kinds of movement", () => {
    // Each maps to a field on ActionOption — see the module header. A glyph
    // that is not traceable to a rule is a glyph that is making something up.
    expect(badgeFor(line()).label).toBe("advance");
    expect(badgeFor(line({ onContact: "press" })).label).toBe("press on through contact");
    expect(badgeFor(line({ retreat: true })).label).toBe("withdraw");
  });

  it("puts a withdrawal before a press-on when both are set", () => {
    // A retreat that presses on through contact is still a retreat; reading
    // it as an advance would invert the plan on the map.
    expect(badgeFor(line({ retreat: true, onContact: "press" })).label).toBe("withdraw");
  });

  it("separates direct fire, indirect fire and smoke", () => {
    expect(badgeFor(line({ kind: "fire" })).label).toBe("fire");
    expect(badgeFor(line({ kind: "fire", indirect: true })).label).toBe("indirect fire");
    expect(badgeFor(line({ kind: "fire", smoke: true })).label).toBe("smoke");
  });

  it("marks an assault as its own thing", () => {
    expect(badgeFor(line({ kind: "assault" })).label).toBe("assault");
  });

  it("uses a distinct glyph for every badge", () => {
    // Two orders sharing a glyph would be indistinguishable on the counter,
    // which is the only place these are ever read.
    const glyphs = PLAN_BADGE_LEGEND.map((badge) => badge.glyph);
    expect(new Set(glyphs).size).toBe(glyphs.length);
  });

  it("uses single characters that a system font will have", () => {
    // ⚠ A GLYPH NOBODY CAN SEE IS WORSE THAN A PLAIN ARROW. Anything outside
    // the Basic Multilingual Plane renders as a tofu box on some machines,
    // and a demo is exactly where that gets noticed.
    for (const badge of PLAN_BADGE_LEGEND) {
      expect([...badge.glyph]).toHaveLength(1);
      expect(badge.glyph.codePointAt(0)!).toBeLessThan(0x10000);
    }
  });
});

describe("collecting badges per element", () => {
  it("gives an element ordered to move and shoot both badges, movement first", () => {
    const badges = planBadgesFor(
      geometry({
        lines: [
          line({ kind: "fire", optionId: "blue-1:fire:red-1" }),
          line({ kind: "move", optionId: "blue-1:move:0" }),
        ],
      }),
    );
    expect(badges.get("blue-1")!.badges.map((one) => one.label)).toEqual([
      "advance",
      "fire",
    ]);
  });

  it("reads a combined fire mission as one shot, not several", () => {
    // Combined fire produces one line per participant. The lead element
    // appearing twice in its own badge strip would be noise.
    const badges = planBadgesFor(
      geometry({
        lines: [
          line({ kind: "fire", combined: true, optionId: "o1" }),
          line({ kind: "fire", combined: true, optionId: "o2" }),
        ],
      }),
    );
    expect(badges.get("blue-1")!.badges).toHaveLength(1);
  });

  it("badges an element that was told to stand still", () => {
    // ⚠ THE CASE THAT JUSTIFIES THE WHOLE FEATURE. A held element and an
    // element nobody mentioned look identical on the board; only the badge
    // tells them apart.
    const badges = planBadgesFor(
      geometry({
        badges: [
          { actorId: "blue-9", side: "blue", optionId: "hold", label: "hold", kind: "hold" },
        ],
      }),
    );
    expect(badges.get("blue-9")!.badges.map((one) => one.label)).toEqual(["hold"]);
  });

  it("tells a held element apart from one that was never committed", () => {
    // ⚠ THE TWO USED TO BE THE SAME PICTURE — an empty one. A hold is a
    // decision that cost an activation to make; an uncommitted element is one
    // the commander kept in hand, which under a command cap is the usual and
    // correct way to leave a unit alone. A model's own plan text says so
    // ("fe-1 and fe-2 hold pending contact reports") while the map said
    // nothing about either.
    const badges = planBadgesFor(
      geometry({
        badges: [
          { actorId: "blue-9", side: "blue", optionId: "hold", label: "hold", kind: "hold" },
          { actorId: "blue-8", side: "blue", label: "not committed", kind: "uncommitted" },
        ],
      }),
    );
    const held = badges.get("blue-9")!.badges[0];
    const spare = badges.get("blue-8")!.badges[0];
    expect(held.label).toBe("hold");
    expect(spare.label).toBe("not committed this turn");
    expect(spare.glyph).not.toBe(held.glyph);
  });

  it("invents nothing from an empty plan", () => {
    // Renamed from "gives an unordered element nothing at all", which stopped
    // being true: an unordered element now arrives as an `uncommitted` badge
    // in the geometry. What this still guards is that this module adds
    // nothing of its own — every badge traces to something planLines emitted,
    // which is what keeps the fog-of-war gate in one place.
    expect(planBadgesFor(geometry()).size).toBe(0);
  });

  it("keeps each element's own side", () => {
    // The strip is coloured by side so an umpire watching both plans can tell
    // whose intention a badge is.
    const badges = planBadgesFor(
      geometry({ lines: [line({ actorId: "red-2", side: "red", kind: "assault" })] }),
    );
    expect(badges.get("red-2")!.side).toBe("red");
  });
});
