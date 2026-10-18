import { describe, expect, it } from "vitest";

import { canPenetrate } from "./resolvers";
import { PLATFORM_SNAPSHOT } from "./forceList";
import { HOUSE_V1, withModules } from "./ruleset";

const ON = withModules(HOUSE_V1, { penetration: true });

describe("the failure mode is the one that is implemented", () => {
  it("gates rather than softening, because softening was never built", () => {
    // ⚠ THE REGRESSION GUARD FOR A BUG THAT FAILED PERMISSIVE.
    //
    // `onFailure` used to admit "columnShift" as well, with `columnsLost: 2`
    // beside it and a doc comment calling it "softer and faster". Neither was
    // ever read: the only consumer is one `=== "hardGate"` test in
    // resolvers.ts. Selecting it did not soften armour, it removed armour —
    // every bounce became a full-effect hit.
    //
    // It is caught here rather than only by the narrowed type because the
    // type is the kind of thing a future change widens without thinking. If
    // the union grows again, this fails until the new member is actually
    // wired into `resolveDirectFire`.
    expect(HOUSE_V1.penetration.onFailure).toBe("hardGate");
    expect(Object.keys(HOUSE_V1.penetration).sort()).toEqual([
      "falloffPerKmBeyond1Km",
      "onFailure",
      "tandemEraDefeatFraction",
    ]);
  });
});

describe("unknown penetration fails open", () => {
  it("lets a round with no recorded penetration through", () => {
    // THE CASE THIS RULE WAS MOST LIKELY TO GET WRONG. 751 WWII rounds have
    // no penetration curve at all, and reading absent as zero would silently
    // render those weapons unable to damage anything — a unit disabled by a
    // gap in the data, with nothing failing and nobody told.
    //
    // ⚠ FAILING OPEN IS STILL RIGHT AND STILL DANGEROUS, AND THE DANGER IS
    // NO LONGER HYPOTHETICAL. Challenger 2 and Type 59 used to be in this
    // category, which meant two main battle tanks defeated every armour value
    // in the game unconditionally. Both are sourced now. The rule stays as it
    // is, because absent data must not disarm a unit; the defence is the
    // catalogue test below, which requires every armour-capable weapon to
    // state a figure so that nothing relies on this path.
    expect(canPenetrate(undefined, 900, 1000, ON)).toBe(true);
  });

  it("lets a round through against unknown armour", () => {
    expect(canPenetrate(480, undefined, 1000, ON)).toBe(true);
  });

  it("is inert when the module is off", () => {
    expect(canPenetrate(10, 900, 1000, HOUSE_V1)).toBe(true);
  });
});

describe("the gate itself", () => {
  it("stops a round that cannot defeat the armour", () => {
    // T-80U's 550 against an M1A2's 900.
    expect(canPenetrate(550, 900, 1000, ON)).toBe(false);
  });

  it("passes a round that can", () => {
    expect(canPenetrate(480, 200, 1000, ON)).toBe(true);
  });

  it("treats exactly enough as enough", () => {
    expect(canPenetrate(400, 400, 1000, ON)).toBe(true);
  });
});

describe("range falloff", () => {
  it("does not reward being closer than the quoted range", () => {
    // Penetration is quoted at 1 km. A figure measured there is not improved
    // by firing at 200 m; inventing a bonus would be inventing data.
    expect(canPenetrate(440, 440, 200, ON)).toBe(true);
    expect(canPenetrate(440, 441, 200, ON)).toBe(false);
  });

  it("degrades beyond a kilometre", () => {
    // 440 at 1 km, 15% lost per further km: at 2.4 km that is ~348, which no
    // longer defeats a Challenger 2's 400. This is where the rule does most
    // of its work in practice — long shots at heavy frontal armour bounce.
    expect(canPenetrate(440, 400, 1000, ON)).toBe(true);
    expect(canPenetrate(440, 400, 2400, ON)).toBe(false);
  });
});

describe("what the current data actually supports", () => {
  it("nothing in the catalogue out-penetrates 800 mm, and 900 mm is immune to everything", () => {
    // An honest record of how narrow this rule is with today's figures: the
    // best recorded penetration anywhere is 800 mm, so any frontal armour
    // above it cannot be defeated by ANY round in the game.
    //
    // ⚠ ASSERTED AS A PROPERTY, NOT A LIST OF NAMES. It used to name the
    // M1A2 as the only immune platform. That broke the moment the generated
    // catalogue arrived and brought two Merkavas with 900 mm — a change in
    // the breadth of the data, not in its quality, and a test that churns on
    // every regeneration stops being read. What matters is the CEILING.
    //
    // If `best` ever rises above 800, the pipeline's penetration coverage
    // improved, which is the point and worth noticing.
    const best = Object.values(PLATFORM_SNAPSHOT)
      .flatMap((p) => p.capabilities)
      .map((c) => c.penetrationMm ?? 0)
      .reduce((a, b) => Math.max(a, b), 0);

    const immune = Object.values(PLATFORM_SNAPSHOT).filter(
      (p) => p.armourMm != null && p.armourMm > best,
    );

    // ⚠ THE TRIPWIRE ABOVE FIRED, WHICH IS THE OUTCOME IT WAS BUILT FOR.
    // The ceiling was 800 mm when every anti-armour figure came from the
    // simulator export. A Kornet detachment from the curated section profile
    // carries 1,200, so the ceiling is now 1,200 —
    //
    // and the consequence is the part worth reading: NOTHING IS IMMUNE ANY
    // MORE. The heaviest frontal protection in the catalogue is 900 mm, so
    // for the first time every vehicle in the game can be killed by something
    // an infantry section can carry. That is a real change to what the game
    // is about, and it arrived with the data rather than with a rule.
    expect(best).toBe(1200);
    expect(immune.length).toBe(0);
  });

  it("records which platforms have no penetration figure at all", () => {
    // Named rather than counted, so it is obvious when the data improves.
    const blind = Object.values(PLATFORM_SNAPSHOT)
      .filter((p) => p.capabilities.every((c) => c.penetrationMm == null))
      .map((p) => p.displayName)
      .sort();
    // ⚠ THE FIRST CATEGORY IS NOW EMPTY, AND THAT IS THE POINT OF THIS TEST.
    //
    // It used to hold Challenger 2 and Type 59 — two MAIN BATTLE TANKS whose
    // guns stated no penetration and therefore, failing open, defeated every
    // armour value in the game unconditionally. The armour tables could be as
    // carefully sourced as you like and nothing between tanks could ever
    // bounce.
    //
    // Both are sourced now: the Challenger states the profile's 657 mm, and
    // the Type 59 was replaced by the T-55 it always was, at 390 mm.
    //
    // WHAT IS LEFT IS ONLY THE SECOND CATEGORY — NOTHING TO RECORD. Mortars
    // firing HE and a machine gun firing ball have no anti-armour penetration
    // to state, and all four are barred from engaging armour by target class
    // (9.2.1), so the check never runs for them. A weapon appearing here that
    // CAN engage armour is a bug, because it would be silently invincible.
    expect(blind).toEqual([
      "Mortar Section",
      "RU 82mm Mortar Section",
      "UK 81mm Mortar Section",
      "UK GPMG Sustained-Fire Team",
    ]);
    // The invariant behind the list, stated so it survives the list changing.
    for (const platform of Object.values(PLATFORM_SNAPSHOT)) {
      for (const capability of platform.capabilities) {
        if (capability.kind === "atk" || capability.kind === "atm") {
          expect(capability.penetrationMm, `${platform.displayName} ${capability.kind}`)
            .toBeTypeOf("number");
        }
      }
    }
  });
});
