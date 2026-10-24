import { describe, expect, it } from "vitest";
import { createRng } from "../../rules/dice";
import { applyPatch, diff } from "./statePatch";

/** Same content AND same key order, all the way down. */
const exact = (a: unknown) => JSON.stringify(a);

describe("statePatch", () => {
  it("nothing changed: no patch, and the base comes back as it is", () => {
    const state = { a: { b: 1 } };
    expect(diff(state, state)).toBeNull();
    expect(applyPatch(state, null)).toBe(state);
  });

  it("walks only what changed, and shares the rest", () => {
    const shared = { big: [1, 2, 3] };
    const prev = { units: { u1: { picture: { e1: { at: 1 }, e2: { at: 2 } }, hp: 3 }, u2: shared }, time: 1 };
    const next = { ...prev, units: { ...prev.units, u1: { ...prev.units.u1, picture: { ...prev.units.u1.picture, e2: { at: 5 } } } }, time: 2 };
    const patch = diff(prev, next)!;
    expect(patch).toEqual({
      set: { time: 2 },
      sub: { units: { sub: { u1: { sub: { picture: { sub: { e2: { set: { at: 5 } } } } } } } } },
    });
    // Below PATCH_DEPTH levels a changed value goes whole.
    expect(diff(prev, next, 3)).toEqual({ set: { time: 2 }, sub: { units: { sub: { u1: { set: { picture: next.units.u1.picture } } } } } });
    const out = applyPatch(prev, patch);
    expect(exact(out)).toBe(exact(next));
    expect(out.units.u2).toBe(shared);
    expect(prev.units.u1.picture.e2.at).toBe(2); // the base is untouched
  });

  it("deletions, additions, undefined values, and arrays sent whole", () => {
    const prev: Record<string, unknown> = { a: 1, b: { x: 1 }, c: [1], d: undefined };
    const next: Record<string, unknown> = { a: 1, c: [1, 2], d: 4, e: undefined, f: { y: 2 } };
    const out = applyPatch(prev, diff(prev, next));
    expect(exact(out)).toBe(exact(next));
    expect(Object.keys(out)).toEqual(Object.keys(next));
    expect("e" in out).toBe(true);
  });

  it("a key deleted and added back moves to the end: the object is sent whole, so order is kept", () => {
    const prev = { lastKnown: { e1: 1, e2: 2, e3: 3 } };
    const { e1: _gone, ...rest } = prev.lastKnown;
    void _gone;
    const next = { lastKnown: { ...rest, e1: 9 } };
    const patch = diff(prev, next)!;
    expect(patch.sub?.lastKnown.whole).toBeDefined();
    const out = applyPatch(prev, patch);
    expect(Object.keys(out.lastKnown)).toEqual(["e2", "e3", "e1"]);
  });

  it("random copy-on-write edits: the patched replica always serialises exactly like the original", () => {
    const rng = createRng("patch-fuzz");
    type Tree = Record<string, unknown>;
    const make = (depth: number): Tree => {
      const out: Tree = {};
      for (let i = 0; i < 4; i++) out[`k${i}`] = depth > 0 && rng.int(2) ? make(depth - 1) : rng.int(100);
      return out;
    };
    /** One immutable edit at a random path: set, delete, re-add (moves to the end), or replace a subtree. */
    const edit = (tree: Tree, depth: number): Tree => {
      const keys = Object.keys(tree);
      const key = keys.length ? keys[rng.int(keys.length)] : "n";
      const child = tree[key];
      const roll = rng.int(6);
      if (roll === 0 && depth > 0 && child && typeof child === "object") return { ...tree, [key]: edit(child as Tree, depth - 1) };
      if (roll === 1) {
        const { [key]: _drop, ...rest } = tree;
        void _drop;
        return rest;
      }
      if (roll === 2) {
        const { [key]: value, ...rest } = tree;
        return { ...rest, [key]: value };
      }
      if (roll === 3) return { ...tree, [`n${rng.int(50)}`]: rng.int(100) };
      if (roll === 4) return { ...tree, [key]: make(1) };
      return { ...tree, [key]: rng.int(100) };
    };
    let original = make(4);
    let replica: Tree = structuredClone(original);
    for (let round = 0; round < 500; round++) {
      let next = original;
      for (let i = 0; i < 1 + rng.int(5); i++) next = edit(next, 4);
      replica = applyPatch(replica, structuredClone(diff(original, next)));
      original = next;
      expect(exact(replica)).toBe(exact(original));
    }
  });
});
