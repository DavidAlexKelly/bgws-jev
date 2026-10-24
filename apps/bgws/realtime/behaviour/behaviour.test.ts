import { describe, expect, it } from "vitest";
import { buildRegistry, bundledTemplateFiles, templateIdFromPath } from "./load";
import { validateRegistry, validateTemplate } from "./schema";
import type { FactTable, Template } from "./types";
import { evaluate, resolveTemplate, walk } from "./walk";

interface Ctx {
  ammo: number;
  rangeM: number | null;
  suppressed: boolean;
}
const facts: FactTable<Ctx> = {
  "ammo.main": (c) => c.ammo,
  "target.rangeM": (c) => c.rangeM,
  suppressed: (c) => c.suppressed,
};
const ctx = (over: Partial<Ctx> = {}): Ctx => ({ ammo: 10, rangeM: 1500, suppressed: false, ...over });

const TANK: Template = {
  id: "core/tank",
  version: 1,
  archetype: "tank",
  doctrine: "A tank crew fights from cover and keeps its distance from infantry.",
  triggers: { sighted: "contact", underFire: "underFire" },
  nodes: {
    underFire: {
      kind: "branch",
      cases: [{ when: { fact: "suppressed", op: "==", value: true }, next: "smoke" }],
      else: "contact",
    },
    smoke: { kind: "do", action: "popSmokeReverse" },
    contact: {
      kind: "choose",
      title: "Enemy in sight",
      question: "What does the crew do?",
      options: [
        {
          id: "fire",
          action: "fire",
          when: { all: [{ fact: "ammo.main", op: ">", value: 0 }, { fact: "target.rangeM", op: "<=", value: 3000 }] },
        },
        { id: "hull-down", action: "takeHullDown" },
        { id: "wait", action: "holdObserve", bias: -1 },
        { id: "pull-back", action: "withdraw" },
      ],
      fallback: "fire",
    },
  },
};
const STAND_FAST: Template = { id: "custom/tank-stand-fast", version: 1, parent: "core/tank", remove: ["pull-back"], bias: { wait: 1 } };
const registry = new Map([TANK, STAND_FAST].map((t) => [t.id, t]));

describe("evaluate", () => {
  it("compares, combines and negates", () => {
    expect(evaluate({ fact: "ammo.main", op: ">", value: 0 }, facts, ctx())).toBe(true);
    expect(evaluate({ fact: "ammo.main", op: ">", value: 0 }, facts, ctx({ ammo: 0 }))).toBe(false);
    expect(evaluate({ any: [{ fact: "suppressed", op: "==", value: true }, { fact: "ammo.main", op: "==", value: 10 }] }, facts, ctx())).toBe(true);
    expect(evaluate({ not: { fact: "suppressed", op: "==", value: false } }, facts, ctx())).toBe(false);
  });
  it("treats an ordering comparison with a missing value as false, not as zero", () => {
    expect(evaluate({ fact: "target.rangeM", op: "<", value: 5000 }, facts, ctx({ rangeM: null }))).toBe(false);
  });
  it("refuses a fact it has not been given", () => {
    expect(() => evaluate({ fact: "fuel", op: ">", value: 0 }, facts, ctx())).toThrow(/Unknown fact "fuel"/);
  });
});

describe("walk", () => {
  const tank = resolveTemplate("core/tank", registry);

  it("offers the options whose conditions hold, keeping the fallback", () => {
    const r = walk(tank, "sighted", facts, ctx());
    expect(r.kind).toBe("choose");
    if (r.kind !== "choose") return;
    expect(r.options.map((o) => o.id)).toEqual(["fire", "hull-down", "wait", "pull-back"]);
    expect(r.fallback).toBe("fire");
    expect(r.options.find((o) => o.id === "wait")?.bias).toBe(-1);
    expect(r.doctrine).toEqual([TANK.doctrine]);
    expect(r.path).toEqual(["contact"]);
  });

  it("drops an option whose condition fails, and falls back to the first that remains", () => {
    const r = walk(tank, "sighted", facts, ctx({ ammo: 0 }));
    if (r.kind !== "choose") throw new Error("expected choose");
    expect(r.options.map((o) => o.id)).toEqual(["hull-down", "wait", "pull-back"]);
    expect(r.fallback).toBe("hull-down");
  });

  it("follows branches, ending at a drill or a question", () => {
    expect(walk(tank, "underFire", facts, ctx({ suppressed: true }))).toEqual({
      kind: "do", nodeId: "smoke", action: "popSmokeReverse", params: {}, path: ["underFire", "smoke"],
    });
    const r = walk(tank, "underFire", facts, ctx());
    expect(r.kind === "choose" && r.path).toEqual(["underFire", "contact"]);
  });

  it("returns none for a trigger the template does not handle", () => {
    expect(walk(tank, "lostComms", facts, ctx())).toEqual({ kind: "none", path: [] });
  });

  it("stops a branch cycle instead of hanging", () => {
    const loop: Template = {
      id: "core/loop", version: 1, triggers: { sighted: "a" },
      nodes: {
        a: { kind: "branch", cases: [{ when: { fact: "suppressed", op: "==", value: false }, next: "b" }], else: "b" },
        b: { kind: "branch", cases: [{ when: { fact: "suppressed", op: "==", value: false }, next: "a" }], else: "a" },
      },
    };
    const t = resolveTemplate("core/loop", new Map([[loop.id, loop]]));
    expect(() => walk(t, "sighted", facts, ctx())).toThrow(/exceeded 32 steps/);
  });
});

describe("inheritance", () => {
  it("removes options and re-biases them for the child only", () => {
    const r = walk(resolveTemplate("custom/tank-stand-fast", registry), "sighted", facts, ctx());
    if (r.kind !== "choose") throw new Error("expected choose");
    expect(r.options.map((o) => o.id)).toEqual(["fire", "hull-down", "wait"]);
    expect(r.options.find((o) => o.id === "wait")?.bias).toBe(1);
    const base = walk(resolveTemplate("core/tank", registry), "sighted", facts, ctx());
    expect(base.kind === "choose" && base.options.map((o) => o.id)).toContain("pull-back");
  });

  it("reports unknown parents and inheritance loops", () => {
    const orphan: Template = { id: "custom/x", version: 1, parent: "core/nope" };
    expect(() => resolveTemplate("custom/x", new Map([[orphan.id, orphan]]))).toThrow(/Unknown template "core\/nope"/);
    const a: Template = { id: "custom/a", version: 1, parent: "custom/b" };
    const b: Template = { id: "custom/b", version: 1, parent: "custom/a" };
    expect(() => resolveTemplate("custom/a", new Map([[a.id, a], [b.id, b]]))).toThrow(/inheritance loop/);
  });
});

describe("validation", () => {
  const vocab = {
    facts: new Set(Object.keys(facts)),
    actions: new Set(["fire", "takeHullDown", "holdObserve", "withdraw", "popSmokeReverse"]),
  };

  it("accepts the example templates", () => {
    expect(validateTemplate(TANK, vocab)).toEqual([]);
    expect(validateTemplate(STAND_FAST, vocab)).toEqual([]);
    expect(validateRegistry(registry)).toEqual([]);
  });

  it("lists every problem in a broken template, with where it is", () => {
    const broken = {
      id: "core/broken", version: 1, triggers: { sighted: "nowhere" },
      nodes: {
        c: {
          kind: "choose", title: "T", question: "Q", fallback: "missing",
          options: [
            { id: "a", action: "teleport" },
            { id: "a", action: "fire", when: { fact: "morale", op: "~", value: 1 } },
          ],
        },
      },
    };
    const errors = validateTemplate(broken, vocab);
    expect(errors).toEqual(
      expect.arrayContaining([
        expect.stringContaining(`triggers.sighted: no node "nowhere"`),
        expect.stringContaining(`unknown action "teleport"`),
        expect.stringContaining(`duplicate option id "a"`),
        expect.stringContaining(`unknown fact "morale"`),
        expect.stringContaining(`"op" must be one of`),
        expect.stringContaining(`fallback "missing" is not one of its options`),
      ]),
    );
  });

  it("refuses a child that removes the fallback it inherits", () => {
    const bad: Template = { id: "custom/no-fire", version: 1, parent: "core/tank", remove: ["fire"] };
    expect(validateRegistry(new Map([...registry, [bad.id, bad]]))).toEqual([
      expect.stringContaining(`removes "fire", which is this decision's fallback`),
    ]);
  });
});

describe("loading", () => {
  it("derives ids from paths", () => {
    expect(templateIdFromPath("./templates/core/tank.json")).toBe("core/tank");
    expect(templateIdFromPath("./templates/custom/tank-stand-fast.json")).toBe("custom/tank-stand-fast");
    expect(templateIdFromPath("./other/tank.json")).toBeNull();
  });

  it("builds a registry, and rejects a file whose id does not match its path", () => {
    const reg = buildRegistry({
      "./templates/core/tank.json": TANK,
      "./templates/custom/tank-stand-fast.json": STAND_FAST,
    });
    expect([...reg.keys()]).toEqual(["core/tank", "custom/tank-stand-fast"]);
    expect(() => buildRegistry({ "./templates/core/ifv.json": TANK })).toThrow(/path says "core\/ifv"/);
  });

  it("validates every template file shipped in the app", () => {
    expect(() => buildRegistry(bundledTemplateFiles())).not.toThrow();
  });
});
