/**
 * Firing and assaulting together (Core Rules 9.2.1 COMBINED, 9.3 COMBINED).
 *
 * ⚠ WHY THIS MATTERS MORE THAN A MISSING RULE USUALLY WOULD.
 *
 * `resolveDirectFire` has always taken an ARRAY of firers, summed their
 * Combat Strength and applied the worst modifier among them. Every call site
 * passed exactly one element, and `optionsFor` could only ever generate
 * one-element options, so no sequence of play could produce a combined shot.
 *
 * Meanwhile HOUSE_V1's fire columns are spaced, in the ruleset's own words,
 * "wide enough to make concentration of force the obviously correct play,
 * because a wargame in which massing does not pay teaches the wrong lesson".
 *
 * So the table was calibrated around a manoeuvre the game could not perform,
 * and that untested claim sat underneath every number in reports/. These
 * tests are the claim being made testable.
 */

import { describe, expect, it } from "vitest";

import { projectForSide } from "../lib/fogOfWar";
import { flatTerrain } from "../lib/lineOfSight";
import type { ForceElement, GameState, Side } from "../lib/state";
import { heuristicCommander } from "./commander";
import { createRng } from "./dice";
import { EventLog, type ResolutionEvent } from "./events";
import { HOUSE_V1, withModules } from "./ruleset";
import { optionsFor, resolveAction, type PhaseConfig } from "./turnLoop";

const COMBINED = withModules(HOUSE_V1, { combinedFire: true });

function fe(overrides: Partial<ForceElement> & { id: string; side: Side }): ForceElement {
  return {
    label: overrides.id,
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
    ...overrides,
  };
}

function board(elements: ForceElement[]): GameState {
  const sighting: GameState["sighting"] = { blue: {}, red: {} };
  for (const element of elements) {
    const viewer: Side = element.side === "blue" ? "red" : "blue";
    sighting[viewer][element.id] = "full";
  }
  return {
    gameId: "cf",
    scenarioId: "cf",
    turn: 1,
    phase: "arcAction",
    initiative: "blue",
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements: Object.fromEntries(elements.map((f) => [f.id, f])),
    sighting,
    rng: { seed: "cf", cursor: 0 },
  };
}

function phaseConfig(ruleset = COMBINED, seed = "cf"): PhaseConfig {
  return {
    ruleset,
    terrain: flatTerrain(),
    rng: createRng(seed),
    log: new EventLog(),
    maxTurns: 40,
  };
}

/**
 * Two blue elements 100 m apart, one red a kilometre away that both can see.
 * `apart` pushes the second element outside the 250 m co-location radius.
 */
function troop(options: { apart?: boolean; second?: Partial<ForceElement> } = {}): GameState {
  return board([
    fe({ id: "B1", side: "blue", position: { lat: 54.71, lng: 20.51 } }),
    fe({
      id: "B2",
      side: "blue",
      position: { lat: options.apart ? 54.716 : 54.7109, lng: 20.51 },
      ...options.second,
    }),
    fe({ id: "R1", side: "red", position: { lat: 54.719, lng: 20.51 } }),
  ]);
}

const combinedFireOption = (state: GameState, config = phaseConfig()) =>
  optionsFor(state, state.forceElements.B1, config).find(
    (option) => option.kind === "fire" && (option.actorIds?.length ?? 1) > 1,
  );

const shots = (log: EventLog) =>
  log.all().filter((e): e is ResolutionEvent => e.type === "resolution" && e.kind === "directFire");

// ── 9.2.1 Combined Direct Fire ─────────────────────────────────────────────

describe("combined direct fire is offered (9.2.1)", () => {
  it("when two co-located elements can both reach the target", () => {
    const option = combinedFireOption(troop());
    expect(option).toBeDefined();
    expect(option!.actorIds).toEqual(["B1", "B2"]);
  });

  it("never when the module is off", () => {
    // The whole point of the flag: the sweep has to be able to play the game
    // both ways to find out whether massing pays.
    expect(combinedFireOption(troop(), phaseConfig(HOUSE_V1))).toBeUndefined();
  });

  it("not to elements further apart than the Glossary's 250 m", () => {
    // "Co-located — An FE within 250m of another Friendly FE is Co-located
    // with it."
    expect(combinedFireOption(troop({ apart: true }))).toBeUndefined();
  });

  it("not to an element that has already fired", () => {
    // 9.2.1: "No participating FE can already have a FIRED marker."
    expect(combinedFireOption(troop({ second: { markers: ["fired"] } }))).toBeUndefined();
  });

  it("not to a broken element", () => {
    expect(combinedFireOption(troop({ second: { morale: "broken" } }))).toBeUndefined();
  });

  it("alongside the individual shot, never instead of it", () => {
    // Massing is a CHOICE. Replacing the single-element option with the
    // combined one would make it compulsory, which is a different game and
    // an unmeasurable one.
    const state = troop();
    const fire = optionsFor(state, state.forceElements.B1, phaseConfig()).filter(
      (option) => option.kind === "fire",
    );
    expect(fire.length).toBe(2);
    expect(fire.filter((option) => (option.actorIds?.length ?? 1) === 1).length).toBe(1);
  });
});

describe("combined direct fire resolves as one shot (9.2.1)", () => {
  it("sums Combat Strength into a higher column", () => {
    // Two CS 8 elements are CS 16 together, which is two columns up the
    // ladder — "CS 6-9" becomes "CS 16+". This is the assertion that the
    // array passed to resolveDirectFire is actually being used.
    const state = troop();
    const config = phaseConfig();
    const option = combinedFireOption(state, config)!;

    resolveAction(state, option, config, 1);
    const [event] = shots(config.log);

    expect(event.actorIds).toEqual(["B1", "B2"]);
    expect(event.table).toBe("fire:CS 16+");
  });

  it("and a single element fires two columns lower", () => {
    // The control for the test above: same elements, same target, one firer.
    const state = troop();
    const config = phaseConfig();
    const single = optionsFor(state, state.forceElements.B1, config).find(
      (option) => option.kind === "fire" && (option.actorIds?.length ?? 1) === 1,
    )!;

    resolveAction(state, single, config, 1);
    expect(shots(config.log)[0].table).toBe("fire:CS 6-9");
  });

  it("marks every firer, not just the one that led", () => {
    // 9.2.1.2 step 7: "Add a FIRED marker to the Firing FE(s)."
    const state = troop();
    const config = phaseConfig();
    const next = resolveAction(state, combinedFireOption(state, config)!, config, 1);

    expect(next.forceElements.B1.markers).toContain("fired");
    expect(next.forceElements.B2.markers).toContain("fired");
  });

  it("spends a round for every firer", () => {
    // Step 6: "Reduce the Ammo of all firing FE by 1." Charging one round for
    // a combined shot would make massing free as well as effective.
    const ruleset = withModules(HOUSE_V1, { combinedFire: true, ammunition: true });
    const state = troop();
    const config = phaseConfig(ruleset);
    const next = resolveAction(state, combinedFireOption(state, config)!, config, 1);

    const rounds = ruleset.logistics.roundsPerCapability;
    expect(next.forceElements.B1.ammo?.atk).toBe(rounds - 1);
    expect(next.forceElements.B2.ammo?.atk).toBe(rounds - 1);
  });

  it("takes the WORST firer's state, not the best", () => {
    // "For Combined DirF use the most detrimental modifiers to the Firing
    // side to determine DRMs." Otherwise a fresh element could launder a
    // suppressed one's fire, and suppression would stop meaning anything.
    const state = troop({ second: { morale: "suppressed2" } });
    const config = phaseConfig();
    const option = combinedFireOption(state, config)!;

    resolveAction(state, option, config, 1);
    const [event] = shots(config.log);
    expect(event.modifiers.some((m) => m.source === "firerSuppressed")).toBe(true);
  });
});

// ── 9.3 Combined Assault ───────────────────────────────────────────────────

describe("combined assault (9.3)", () => {
  /** Two blue elements in contact with one red. */
  function contact(second: Partial<ForceElement> = {}): GameState {
    return board([
      fe({ id: "B1", side: "blue", position: { lat: 54.71, lng: 20.51 } }),
      fe({ id: "B2", side: "blue", position: { lat: 54.7101, lng: 20.51 }, ...second }),
      fe({ id: "R1", side: "red", position: { lat: 54.7102, lng: 20.51 } }),
    ]);
  }

  const combinedAssaultOption = (state: GameState, config = phaseConfig()) =>
    optionsFor(state, state.forceElements.B1, config).find(
      (option) => option.kind === "assault" && (option.actorIds?.length ?? 1) > 1,
    );

  it("is offered when both can reach the objective", () => {
    const option = combinedAssaultOption(contact());
    expect(option).toBeDefined();
    expect(option!.actorIds).toEqual(["B1", "B2"]);
  });

  it("is not offered to a disrupted element", () => {
    // 9.3.4: attacking FEs must "not be Disrupted/Broken".
    expect(combinedAssaultOption(contact({ morale: "disrupted" }))).toBeUndefined();
  });

  it("puts both attackers into the odds ratio and into the melee", () => {
    const state = contact();
    const config = phaseConfig();
    const next = resolveAction(state, combinedAssaultOption(state, config)!, config, 1);

    const assault = config.log
      .all()
      .find((e): e is ResolutionEvent => e.type === "resolution" && e.kind === "assault")!;
    expect(assault.actorIds).toEqual(["B1", "B2"]);

    for (const id of ["B1", "B2"]) {
      expect(next.forceElements[id].markers).toContain("fired");
    }
  });
});

// ── The commanders have to actually use it ─────────────────────────────────

describe("the baseline commander concentrates when it can", () => {
  it("prefers the combined shot over the single one", async () => {
    // A mechanic only a model ever uses cannot be A/B'd against the baseline,
    // which is the whole purpose of having a baseline. The real commander is
    // asked, through the real fog-of-war projection, with the real options.
    const state = troop();
    const config = phaseConfig();
    const fire = optionsFor(state, state.forceElements.B1, config).filter(
      (option) => option.kind === "fire",
    );
    const combined = fire.find((option) => (option.actorIds?.length ?? 1) > 1);
    expect(combined).toBeDefined();

    const commander = heuristicCommander("blue", createRng("pick"));
    const { optionId } = await commander.decide(
      projectForSide(state, "blue"),
      fire,
      "which?",
    );

    expect(optionId).toBe(combined!.id);
  });

  it("still fires alone when nobody is close enough to join in", async () => {
    const state = troop({ apart: true });
    const config = phaseConfig();
    const fire = optionsFor(state, state.forceElements.B1, config).filter(
      (option) => option.kind === "fire",
    );

    const commander = heuristicCommander("blue", createRng("pick"));
    const { optionId } = await commander.decide(
      projectForSide(state, "blue"),
      fire,
      "which?",
    );

    expect(fire.find((option) => option.id === optionId)?.actorIds).toBeUndefined();
  });
});
