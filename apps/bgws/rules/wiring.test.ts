/**
 * THE GUARD. Fails when a rule is declared but nothing consults it.
 *
 * WHY THIS FILE EXISTS
 * --------------------
 * A module sweep once reported all ten mechanics as having "no measurable
 * effect — a candidate for deletion". Every one of those readings was false.
 * Seven of the ten flags were never read anywhere in the codebase, so the
 * sweep was not measuring mechanics that did nothing; it was measuring
 * mechanics that did not exist. Those two findings need OPPOSITE fixes, and
 * the harness cannot tell them apart.
 *
 * So the distinction is enforced here instead. A flag or a modifier that
 * nothing consults fails CI, which means the sweep's "no effect" verdict can
 * always be trusted to mean what it says.
 *
 * HOW TO MAKE A FAILURE GO AWAY
 * -----------------------------
 * Two legitimate ways, and no third:
 *
 *   1. Wire it up.
 *   2. Delete it, and delete its flag.
 *
 * Adding it to an allowlist is only legitimate with a reason that says what
 * would have to exist before it could fire — see NOT_YET_REACHABLE. An entry
 * there is a debt with a name, not an excuse.
 */

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { proceduralTerrain, STANDARD_GROUND } from "../lib/proceduralTerrain";
import { advanceTurn, startGame } from "../lib/liveGame";
import { scenarioFactory } from "../lib/forceBuilder";
import { createRng } from "./dice";
import { EventLog } from "./events";
import { heuristicOrdersCommander } from "./orders";
import { FORCE_LISTS } from "./forceList";
import { declaredModifiers, runBatch } from "./harness";
import { CORE_MODULES, HOUSE_V1, type ModuleFlags, type RuleSet } from "./ruleset";

const BGWS_ROOT = join(process.cwd(), "src/apps/bgws");

/** Every non-test source file under bgws, except the one that DECLARES rules. */
function ruleConsumingSource(): string {
  const chunks: string[] = [];

  const walk = (dir: string): void => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(path);
        continue;
      }
      if (!entry.name.endsWith(".ts") && !entry.name.endsWith(".tsx")) continue;
      if (entry.name.endsWith(".test.ts") || entry.name.endsWith(".test.tsx")) continue;
      // ruleset.ts declares the flags; referencing them there proves nothing.
      if (entry.name === "ruleset.ts") continue;
      // harness.ts toggles every flag generically, which would also prove
      // nothing about whether a resolver acts on it.
      if (entry.name === "harness.ts") continue;
      chunks.push(readFileSync(path, "utf8"));
    }
  };

  walk(BGWS_ROOT);
  return chunks.join("\n");
}

describe("every module flag is consulted by something", () => {
  const source = ruleConsumingSource();
  const modules = Object.keys(CORE_MODULES) as (keyof ModuleFlags)[];

  it.each(modules)("%s is read outside ruleset.ts", (module) => {
    // `modules.<name>` is how a resolver asks whether a mechanic is on.
    const consulted = source.includes(`modules.${module}`);

    expect(
      consulted,
      `ModuleFlags.${module} is declared but nothing reads \`modules.${module}\`.\n` +
        `A declared-but-inert flag makes the harness report "no measurable effect" ` +
        `for a mechanic that was never implemented, which is the one failure mode ` +
        `this platform cannot tolerate.\n` +
        `Either wire it into a resolver, or delete the flag from ModuleFlags.`,
    ).toBe(true);
  });
});

/**
 * THE GUARD'S OWN BLIND SPOT, now partly covered.
 *
 * The two guards above sweep MODULE FLAGS and MODIFIERS. A RuleSet also
 * carries TABLES, and nothing swept those — which is how `movement` came to
 * sit on every ruleset for weeks, fully implemented in lib/movement.ts, with
 * no sequence of play consulting it. `optionsFor` offered a destination 40% of
 * the way towards the enemy and the loop put the element there, across water,
 * up a slope and through a forest at the same rate.
 *
 * Sweeping every table generically is harder than it looks: several are read
 * through accessors declared in ruleset.ts rather than as `ruleset.<field>`,
 * so a naive text search reports correctly-wired tables as dead. This checks
 * the ones that decide where an element ENDS UP, which is the family that was
 * actually broken. Extend it rather than adding another table nobody reads.
 */
describe("the tables that decide movement are consulted", () => {
  const source = ruleConsumingSource();

  it.each(["movement"])("ruleset.%s is read outside ruleset.ts", (field) => {
    expect(
      source.includes(`ruleset.${field}`),
      `RuleSet.${field} is declared on every ruleset and nothing reads ` +
        `\`ruleset.${field}\`. A table no sequence of play consults is a rule ` +
        `that cannot decide anything and cannot be measured deciding anything.`,
    ).toBe(true);
  });
});

/**
 * Modifiers that cannot fire yet, each with what would have to exist first.
 *
 * Every entry is a debt. Emptying this list is a project goal.
 */
const NOT_YET_REACHABLE: Record<string, string> = {
  throughSmoke:
    "needs Attempt Sighting to happen DURING activations for ordinary elements, " +
    "not just for concealed ones (10.0). runSighting sweeps every pair at the top " +
    "of the turn, smoke is laid after that sweep, and clean-up removes it at the " +
    "end of the same turn (8.0) — so a cloud never exists at the moment anybody " +
    "looks. The DirF half of the same rule (`smoke`) fires fine.",
  // Four came OFF this list rather than being excused better, which is the
  // only direction it is supposed to move. Every one had been filed as a
  // COMMANDER or CONTENT problem when it was a WIRING problem, and that is
  // the pattern to watch for when adding an entry here:
  //
  //   defenderIsVehicleOnly   excused as needing a mixed force list. It did —
  //                           but `targetClass` was also read by NO rule at
  //                           all, while a comment in ruleset.ts claimed it
  //                           "gated WHETHER you could engage". The content
  //                           was half the problem; the other half was a
  //                           documented lie.
  //
  //   attackerSurprise        excused as "needs a commander that assaults out
  //                           of concealment". The rulebook rolls for Surprise
  //                           on EVERY assault (9.3.1) and nothing was rolling.
  //   attackerAlreadyInMelee  excused as needing ongoing melee. True, but the
  //                           blocker was clean-up wiping the MELEE marker
  //                           that 9.3.8 says must survive.
  //   flank                   excused as "needs a commander that manoeuvres
  //                           for aspect rather than firing frontally". There
  //                           was no facing on a Force Element and no code
  //                           computing aspect, so the cleverest commander
  //                           alive would not have made it fire once.
  //
  // An entry here should name a thing that does not EXIST, not a thing a
  // commander does not DO — a commander can always be replaced.
};

/**
 * Every module ON.
 *
 * The question this guard asks is "CAN this modifier ever fire", not "does it
 * fire under the default ruleset". Several modifiers are gated behind a
 * module — `targetConcealed` needs `concealment` — and `CORE_MODULES` has
 * almost everything off, so running the guard on the defaults would report a
 * correctly-wired modifier as dead and send someone to delete it.
 */
const EVERYTHING_ON: RuleSet = {
  ...HOUSE_V1,
  id: "guard-everything-on",
  modules: Object.fromEntries(
    Object.keys(CORE_MODULES).map((key) => [key, true]),
  ) as unknown as ModuleFlags,
};

/**
 * Every modifier that fires anywhere, under EITHER sequence of play.
 *
 * ⚠ THERE ARE TWO SEQUENCES AND THE GUARD MUST SWEEP BOTH.
 *
 * `runTurn` uses alternating activation and is what the harness measures.
 * `runOrdersTurn` uses an orders phase and is what the play screen and the
 * LLM commanders use. They share every resolver and, since ARC was finished,
 * both ARC rounds as well — but they differ in who chooses and when, so they
 * do not exercise the same things in the same proportions.
 *
 * Sweeping only `runTurn` made `snapShot` look dead the moment it was added —
 * the guard's own blind spot, one level up from the one it was built to
 * catch. Both are swept because a mechanic that lives in one sequence is
 * still wired, and because the day one sequence drifts from the other is the
 * day this guard is the only thing that will notice.
 */
async function modifiersThatFire(seeds: string[]): Promise<Set<string>> {
  const fired = new Set<string>();
  const terrain = proceduralTerrain(STANDARD_GROUND);

  for (const list of Object.values(FORCE_LISTS)) {
    // Sequence one: alternating activation, via the harness.
    const result = await runBatch({
      scenario: scenarioFactory(list, EVERYTHING_ON),
      ruleset: EVERYTHING_ON,
      seeds,
      terrain,
      maxTurns: 40,
    });
    for (const name of Object.keys(result.modifierUsage)) fired.add(name);

    // Sequence two: the orders phase, which is the only place ARC happens.
    for (const seed of seeds.slice(0, 8)) {
      const log = new EventLog();
      let game = startGame(scenarioFactory(list, EVERYTHING_ON)());
      const config = {
        ruleset: EVERYTHING_ON,
        terrain,
        commanders: {
          blue: heuristicOrdersCommander("blue"),
          red: heuristicOrdersCommander("red"),
        },
        rng: createRng(`${seed}:dice`),
        log,
        maxTurns: 40,
      };
      while (!game.over) game = await advanceTurn(game, config);
      for (const event of log.all()) {
        if (event.type !== "resolution") continue;
        for (const modifier of event.modifiers) fired.add(modifier.source);
      }
    }
  }

  return fired;
}

describe("every declared modifier can actually fire", () => {
  const seeds = Array.from({ length: 40 }, (_, index) => `guard${index}`);

  it("fires every modifier that is not a named debt", async () => {
    const fired = await modifiersThatFire(seeds);

    const declared = declaredModifiers(EVERYTHING_ON);
    const silent = declared.filter((name) => !fired.has(name) && !(name in NOT_YET_REACHABLE));

    expect(
      silent,
      `These modifiers are declared, are not on the NOT_YET_REACHABLE list, and ` +
        `never fired in ${seeds.length} games per force list:\n  ${silent.join(", ")}\n` +
        `Either make them reachable, delete them, or add them to ` +
        `NOT_YET_REACHABLE with what would have to exist first.`,
    ).toEqual([]);
  }, 60_000);

  it("has no stale entries on the debt list", async () => {
    // The opposite failure: a modifier that now fires but is still excused.
    // Left alone, the list becomes decoration and stops being a to-do.
    const fired = await modifiersThatFire(seeds);

    const stale = Object.keys(NOT_YET_REACHABLE).filter((name) => fired.has(name));

    expect(
      stale,
      `These modifiers fire now but are still listed as unreachable: ` +
        `${stale.join(", ")}. Remove them from NOT_YET_REACHABLE.`,
    ).toEqual([]);
  }, 60_000);

  it("only excuses modifiers that are actually declared", () => {
    // A typo in the debt list would silently excuse nothing at all.
    const declared = new Set(declaredModifiers(HOUSE_V1));
    for (const name of Object.keys(NOT_YET_REACHABLE)) {
      expect(declared.has(name), `${name} is excused but not declared — typo?`).toBe(true);
    }
  });
});
