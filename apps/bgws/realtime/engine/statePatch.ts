// ── statePatch.ts ────────────────────────────────────────────────────────
// Step 3 of docs/plan/WORKER_AND_SCHEDULER.md: send the main thread what
// CHANGED, not the whole state.
//
// The plan was a slimmed "frame view". Measuring first changed it: at 600
// units the state is ~13 MB (each unit's picture of the enemy is most of
// it), a structured clone costs ~250 ms — on the main thread too, where the
// message is read — yet one simulated second changes only ~5% of it. And a
// frame view would have left the inspector, the commander and Jev, which read
// far more than the map, needing the full state anyway.
//
// So the worker sends a patch and the main thread keeps an exact replica.
// The engine never mutates a state it has handed out (tick is pure;
// comms.ts only mutates copies it made in the same step), so "changed" is
// simply "a different object": unchanged parts are the same references, and
// the diff only walks what changed.

/** Changes to one object: keys set to new values, keys removed, and keys whose value changed only in part. */
export interface Patch {
  /** The object as a whole, sent when its keys changed order (see `diff`). Excludes the rest. */
  whole?: Record<string, unknown>;
  set?: Record<string, unknown>;
  del?: string[];
  sub?: Record<string, Patch>;
}

/** How deep the diff goes before it sends a value whole: state → units → id → picture → enemy → track. */
export const PATCH_DEPTH = 5;

function isPlain(value: unknown): value is Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  const proto = Object.getPrototypeOf(value);
  return proto === Object.prototype || proto === null;
}

/**
 * What turns `prev` into `next`, by identity: a value that is the same object
 * is unchanged and not looked into. Plain objects are compared key by key up
 * to `depth`; anything else that differs (arrays, numbers, deeper objects) is
 * sent whole. Null when nothing changed.
 */
export function diff(prev: unknown, next: unknown, depth = PATCH_DEPTH): Patch | null {
  if (prev === next) return null;
  if (!isPlain(prev) || !isPlain(next)) throw new Error("diff: the roots must be plain objects");
  return diffObject(prev, next, depth);
}

/**
 * Key order is part of the state: the engine and the deciders iterate
 * objects, and the replica must iterate them the same way. A patch keeps the
 * order only if the keys both objects share are in the same order and new
 * keys come after them (as a spread-and-set produces). A key deleted and
 * added back moves to the end; then the object is sent whole.
 */
function diffObject(prev: Record<string, unknown>, next: Record<string, unknown>, depth: number): Patch | null {
  let patch: Patch | null = null;
  const prevKeys = Object.keys(prev);
  let cursor = 0;
  let added = false;
  for (const key in next) {
    if (key in prev) {
      while (cursor < prevKeys.length && !(prevKeys[cursor] in next)) cursor++;
      if (added || prevKeys[cursor] !== key) return { whole: next };
      cursor++;
    } else {
      added = true;
    }
    const before = prev[key];
    const after = next[key];
    if (before === after && (after !== undefined || key in prev)) continue;
    patch ??= {};
    if (depth > 1 && key in prev && isPlain(before) && isPlain(after)) {
      const inner = diffObject(before, after, depth - 1);
      if (inner) (patch.sub ??= {})[key] = inner;
    } else {
      (patch.set ??= {})[key] = after;
    }
  }
  for (const key in prev) {
    if (!(key in next)) ((patch ??= {}).del ??= []).push(key);
  }
  return patch;
}

/**
 * `base` with `patch` applied, as new objects along the changed paths: the
 * rest is shared with `base`, which is left as it was. Key order matches what
 * `diff` saw — changed keys keep their place, new keys go at the end — so the
 * result serialises exactly like the state it was made from.
 */
export function applyPatch<T>(base: T, patch: Patch | null): T {
  if (!patch) return base;
  return applyObject(base as Record<string, unknown>, patch) as T;
}

function applyObject(base: Record<string, unknown>, patch: Patch): Record<string, unknown> {
  if (patch.whole) return patch.whole;
  const out: Record<string, unknown> = { ...base };
  if (patch.set) for (const key in patch.set) out[key] = patch.set[key];
  if (patch.sub) {
    for (const key in patch.sub) {
      const inner = out[key];
      if (!isPlain(inner)) throw new Error(`applyPatch: "${key}" is not an object to patch`);
      out[key] = applyObject(inner, patch.sub[key]);
    }
  }
  if (patch.del) for (const key of patch.del) delete out[key];
  return out;
}
