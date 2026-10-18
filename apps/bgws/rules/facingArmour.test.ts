// ── bgws/rules/facingArmour.test.ts ────────────────────────────────────────
//
// Armour by aspect, and penetration by munition.
//
// These two changes exist because the engine was already doing most of the
// work and spending it on nothing. It has computed a flank aspect for a long
// time and only ever paid it out as a dice modifier: the shot still met the
// glacis, so manoeuvring onto a flank improved the roll and changed nothing
// about whether the round got through. And it applied one kinetic falloff to
// every round in the game, including shaped charges, which do not lose
// penetration with range at all.
//
// The numbers below are the curated Challenger 2: 700 mm frontal and 140 mm
// side against kinetic, 1,000 and 400 against shaped charge, 30 mm of roof.

import { describe, expect, it } from "vitest";

import type { ForceElement, Side } from "../lib/state";
import { armourFacing, canPenetrate, type FireContext } from "./resolvers";
import { HOUSE_V1, withModules } from "./ruleset";

const FACING_ON = withModules(HOUSE_V1, { penetration: true, facingArmour: true });
const FACING_OFF = withModules(HOUSE_V1, { penetration: true, facingArmour: false });

function challenger(overrides: Partial<ForceElement> = {}): ForceElement {
  return {
    id: "cr2",
    side: "blue" as Side,
    label: "Challenger 2",
    sidc: "SFGPUCA-------",
    moveType: "T",
    targetClass: "armoured_vehicle",
    capabilities: [{ kind: "atk", maxRangeM: 3000, shortRangeM: 1500 }],
    troopQuality: 4,
    combatStrength: 8,
    combatStrengthStart: 8,
    morale: "good",
    markers: [],
    concealed: false,
    isDummy: false,
    position: { lat: 54.71, lng: 20.51 },
    armourMm: 700,
    armour: {
      frontKeMm: 700,
      frontCeMm: 1000,
      sideKeMm: 140,
      sideCeMm: 400,
      roofKeMm: 30,
      roofCeMm: 30,
    },
    ...overrides,
  };
}

describe("which armour a shot meets", () => {
  it("meets the glacis from the front", () => {
    expect(armourFacing(challenger(), {}, FACING_ON)).toBe(700);
  });

  it("meets the side from a flank — which is the point of flanking", () => {
    // ⚠ THE CHANGE THAT MAKES MANOEUVRE MECHANICAL. Before this, going round
    // the side bought a dice modifier and the round still hit 700 mm.
    expect(armourFacing(challenger(), { flank: true }, FACING_ON)).toBe(140);
  });

  it("meets the roof when the missile attacks from above", () => {
    // A Javelin is not fighting the same tank a sabot round is.
    const context: FireContext = { topAttack: true, munition: "ceTandem" };
    expect(armourFacing(challenger({ eraFitted: false }), context, FACING_ON)).toBe(30);
  });

  it("meets chemical protection when the round is a shaped charge", () => {
    // Composite armour resists a jet far better than a penetrator: 400 mm at
    // the side against HEAT, 140 against sabot.
    expect(armourFacing(challenger(), { flank: true, munition: "ce" }, FACING_ON)).toBe(400);
  });
});

describe("tandem warheads and reactive armour", () => {
  it("strips the reactive armour it was built to strip", () => {
    // ⚠ CALIBRATED AGAINST THE CURATED MATRIX, which resolves this exact
    // shot at 240 mm. If this number moves, the rule has stopped agreeing
    // with the reference implementation.
    const tandem: FireContext = { flank: true, munition: "ceTandem" };
    expect(armourFacing(challenger({ eraFitted: true }), tandem, FACING_ON)).toBe(240);
  });

  it("leaves a plain shaped charge to face the full thickness", () => {
    // Which is why tandem warheads exist at all.
    const plain: FireContext = { flank: true, munition: "ce" };
    expect(armourFacing(challenger({ eraFitted: true }), plain, FACING_ON)).toBe(400);
  });
});

describe("falling back", () => {
  it("uses the single frontal figure when the module is off", () => {
    expect(armourFacing(challenger(), { flank: true }, FACING_OFF)).toBe(700);
  });

  it("uses the single frontal figure when the element has no facings", () => {
    // Every element built from the L6 profile, and every hand-declared
    // platform, is in this case. They must keep working unchanged.
    const flat = challenger({ armour: undefined });
    expect(armourFacing(flat, { flank: true }, FACING_ON)).toBe(700);
  });

  it("still knows nothing when nothing is known", () => {
    const unknown = challenger({ armour: undefined, armourMm: undefined });
    expect(armourFacing(unknown, { flank: true }, FACING_ON)).toBeUndefined();
  });
});

describe("penetration and range", () => {
  it("a kinetic round loses penetration beyond a kilometre", () => {
    // 500 mm at 1 km, 15% lost per further km: 425 at 2 km, which no longer
    // defeats 450.
    expect(canPenetrate(500, 450, 1000, FACING_ON, "ke")).toBe(true);
    expect(canPenetrate(500, 450, 2000, FACING_ON, "ke")).toBe(false);
  });

  it("a shaped charge does not", () => {
    // ⚠ THE CORRECTION. A Kornet at 5 km defeats what it defeats at 500 m.
    // Under the old single-falloff model this shot failed, which quietly
    // nerfed every guided anti-tank weapon at the ranges they exist for.
    expect(canPenetrate(500, 450, 5000, FACING_ON, "ce")).toBe(true);
    expect(canPenetrate(500, 450, 5000, FACING_ON, "ceTandem")).toBe(true);
  });

  it("treats an unspecified munition as kinetic, as the engine always did", () => {
    expect(canPenetrate(500, 450, 2000, FACING_ON, undefined)).toBe(false);
  });

  it("fails open on unknowns rather than punishing gaps in the data", () => {
    // A rule that punishes missing data rather than weak guns silently
    // disabled the Challenger 2 once already.
    expect(canPenetrate(undefined, 450, 1000, FACING_ON, "ke")).toBe(true);
    expect(canPenetrate(500, undefined, 1000, FACING_ON, "ke")).toBe(true);
  });
});

describe("the two flags together", () => {
  it("a flank shot that bounces off the front gets through the side", () => {
    // The whole change in one assertion: same round, same range, same tank,
    // different aspect, different outcome.
    const round = 300;
    const frontal = armourFacing(challenger(), {}, FACING_ON);
    const flanking = armourFacing(challenger(), { flank: true }, FACING_ON);

    expect(canPenetrate(round, frontal, 1000, FACING_ON, "ke")).toBe(false);
    expect(canPenetrate(round, flanking, 1000, FACING_ON, "ke")).toBe(true);
  });
});
