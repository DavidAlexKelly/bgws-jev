import { describe, expect, it } from "vitest";

import { describeForPrompt, projectForSide } from "./fogOfWar";
import type { ForceElement, GameState, Side, SightingLevel } from "./state";

function fe(overrides: Partial<ForceElement> & { id: string; side: Side }): ForceElement {
  return {
    label: `${overrides.id} label`,
    sidc: "SFGPUCA-------",
    moveType: "T",
    targetClass: "armoured_vehicle",
    capabilities: [{ kind: "atk", maxRangeM: 3000, shortRangeM: 1500 }],
    troopQuality: 5,
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

function game(
  forceElements: ForceElement[],
  sighting: Partial<Record<Side, Record<string, SightingLevel>>> = {},
): GameState {
  return {
    gameId: "g1",
    scenarioId: "s1",
    turn: 3,
    phase: "arcAction",
    initiative: "blue",
    sides: {
      blue: { transmissions: 2, transmissionsLastTurn: 4, chitsHeld: 1, eliminatedLastTurn: 0 },
      red: { transmissions: 7, transmissionsLastTurn: 5, chitsHeld: 3, eliminatedLastTurn: 1 },
    },
    forceElements: Object.fromEntries(forceElements.map((f) => [f.id, f])),
    sighting: { blue: sighting.blue ?? {}, red: sighting.red ?? {} },
    rng: { seed: "test", cursor: 0 },
  };
}

describe("what a side is given", () => {
  it("omits an unsighted enemy entirely — not as an 'unknown' entry", () => {
    // A list of placeholders still leaks the count, which is most of what an
    // opponent wants to know.
    const state = game([fe({ id: "B1", side: "blue" }), fe({ id: "R1", side: "red" })]);
    const view = projectForSide(state, "blue");
    expect(view.contacts).toHaveLength(0);
    expect(JSON.stringify(view)).not.toContain("R1");
  });

  it("gives a partial contact a position and nothing else", () => {
    const state = game(
      [fe({ id: "B1", side: "blue" }), fe({ id: "R1", side: "red", troopQuality: 7 })],
      { blue: { R1: "partial" } },
    );
    const view = projectForSide(state, "blue");
    expect(view.contacts).toHaveLength(1);
    expect(view.contacts[0].position).toBeDefined();
    expect(view.contacts[0].label).toBeUndefined();
    expect(view.contacts[0].sidc).toBeUndefined();
    // None of the opponent's private state may appear on the CONTACT. The
    // view as a whole legitimately contains combat strengths — its own.
    const serialisedContacts = JSON.stringify(view.contacts);
    expect(serialisedContacts).not.toContain("troopQuality");
    expect(serialisedContacts).not.toContain("combatStrength");
    expect(serialisedContacts).not.toContain("morale");
    expect(serialisedContacts).not.toContain("ammo");
  });

  it("identifies a fully sighted contact", () => {
    const state = game(
      [fe({ id: "B1", side: "blue" }), fe({ id: "R1", side: "red", label: "3 PL" })],
      { blue: { R1: "full" } },
    );
    const view = projectForSide(state, "blue");
    expect(view.contacts[0].label).toBe("3 PL");
    expect(view.contacts[0].sidc).toBeDefined();
  });

  it("hides a concealed FE again even if it was sighted before", () => {
    const state = game(
      [fe({ id: "B1", side: "blue" }), fe({ id: "R1", side: "red", concealed: true })],
      { blue: { R1: "partial" } },
    );
    expect(projectForSide(state, "blue").contacts).toHaveLength(0);
  });

  it("projects a Dummy exactly like a real FE", () => {
    // The moment a dummy is distinguishable — even by a missing field — it
    // has stopped being a dummy.
    const state = game(
      [
        fe({ id: "R1", side: "red", isDummy: true, label: "2 PL" }),
        fe({ id: "R2", side: "red", label: "2 PL" }),
      ],
      { blue: { R1: "full", R2: "full" } },
    );
    const view = projectForSide(state, "blue");
    const [dummy, real] = [
      view.contacts.find((c) => c.id === "R1")!,
      view.contacts.find((c) => c.id === "R2")!,
    ];
    expect(Object.keys(dummy).sort()).toEqual(Object.keys(real).sort());
    expect(JSON.stringify(view)).not.toContain("isDummy");
  });

  it("keeps its own force complete", () => {
    const state = game([fe({ id: "B1", side: "blue", troopQuality: 6 })]);
    const view = projectForSide(state, "blue");
    expect(view.own[0].troopQuality).toBe(6);
    expect(view.own[0].combatStrength).toBe(8);
  });

  it("does not reveal the opponent's transmissions or chits", () => {
    const state = game([fe({ id: "B1", side: "blue" })]);
    const view = projectForSide(state, "blue");
    expect(view.transmissions).toBe(2);
    expect(view.chitsHeld).toBe(1);
    // Red holds 3 chits and sent 7 — neither may be inferable.
    expect(JSON.stringify(view)).not.toContain("\"chitsHeld\":3");
  });

  it("drops eliminated and mounted FEs from contacts", () => {
    const state = game(
      [
        fe({ id: "R1", side: "red", combatStrength: 0 }),
        fe({ id: "R2", side: "red", mountedIn: "R3" }),
        fe({ id: "R3", side: "red" }),
      ],
      { blue: { R1: "full", R2: "full", R3: "full" } },
    );
    const view = projectForSide(state, "blue");
    expect(view.contacts.map((c) => c.id)).toEqual(["R3"]);
  });
});

describe("the prompt view", () => {
  it("contains no trace of an unsighted enemy", () => {
    const state = game([
      fe({ id: "B1", side: "blue", label: "1 TP" }),
      fe({ id: "R1", side: "red", label: "ENEMY RECCE" }),
    ]);
    const prompt = describeForPrompt(projectForSide(state, "blue"));
    expect(prompt).toContain("1 TP");
    expect(prompt).not.toContain("ENEMY RECCE");
    expect(prompt).toContain("none sighted");
  });

  it("says that absence is not evidence, because a model will assume it is", () => {
    const prompt = describeForPrompt(projectForSide(game([]), "blue"));
    expect(prompt.toLowerCase()).toContain("absence is not evidence");
  });
});
