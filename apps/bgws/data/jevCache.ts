// ── bgws/data/jevCache.ts ──────────────────────────────────────────────────
// Jev's answers, remembered across page loads.
//
// A wrapper around ANY JevCall — the direct OpenRouter client or a Foundry
// function — so the cache does not care how the question is sent.
//
// WHY IT PERSISTS
// ---------------
// The same moment asked twice should get the same answer: a replayed game, a
// trial re-run to check a result, a turn discarded and planned again. An
// in-memory cache gives that only until the tab closes, which is exactly when
// an experiment gets interesting. Stored in localStorage, keyed by a hash of
// the full request, and bounded so it cannot grow without limit.
//
// ⚠ STORAGE CAN VANISH OR REFUSE. Private windows, cleared site data and full
// quotas all happen. Every read and write is guarded, and a cache that cannot
// be used is simply a cache miss — never an error in the middle of a turn.

import type { JevCall, JevResponse } from "../rules/jev";

const STORAGE_KEY = "bgws-jev-cache-v1";
const DEFAULT_LIMIT = 2000;

type Entries = Record<string, { at: number; response: Omit<JevResponse, "latencyMs"> }>;

/** FNV-1a over the request text: short keys, no crypto needed. */
export function hashRequest(text: string): string {
  let h1 = 0x811c9dc5;
  let h2 = 0x01000193;
  for (let i = 0; i < text.length; i += 1) {
    const c = text.charCodeAt(i);
    h1 = Math.imul(h1 ^ c, 16777619);
    h2 = Math.imul(h2 ^ c, 2246822519);
  }
  return `${(h1 >>> 0).toString(36)}${(h2 >>> 0).toString(36)}${text.length.toString(36)}`;
}

export interface PersistentCacheOptions {
  /** Part of every key, so switching model does not serve the old model's answers. */
  namespace: string;
  /** Most entries kept; the oldest go first. */
  limit?: number;
  /** For tests: anything shaped like localStorage. */
  storage?: Pick<Storage, "getItem" | "setItem">;
}

function defaultStorage(): Pick<Storage, "getItem" | "setItem"> | undefined {
  try {
    return typeof localStorage !== "undefined" ? localStorage : undefined;
  } catch {
    return undefined;
  }
}

export function withPersistentCache(call: JevCall, options: PersistentCacheOptions): JevCall {
  const storage = options.storage ?? defaultStorage();
  const limit = options.limit ?? DEFAULT_LIMIT;
  let entries: Entries | null = null;

  const load = (): Entries => {
    if (entries) return entries;
    try {
      entries = JSON.parse(storage?.getItem(STORAGE_KEY) ?? "{}") as Entries;
    } catch {
      entries = {};
    }
    return entries;
  };

  const save = () => {
    if (!storage || !entries) return;
    try {
      const keys = Object.keys(entries);
      if (keys.length > limit) {
        keys
          .sort((a, b) => entries![a].at - entries![b].at)
          .slice(0, keys.length - limit)
          .forEach((key) => delete entries![key]);
      }
      storage.setItem(STORAGE_KEY, JSON.stringify(entries));
    } catch {
      // Quota or a refused write: carry on uncached.
    }
  };

  return async (request) => {
    const key = hashRequest(`${options.namespace}\n${JSON.stringify(request)}`);
    const hit = load()[key];
    // No cost on a hit: it was paid for once, and counting it again would
    // make a replayed trial look twice as expensive as it was.
    if (hit) return { answers: hit.response.answers, latencyMs: 0 };

    const response = await call(request);
    load()[key] = {
      at: Date.now(),
      response: { answers: response.answers, costUsd: response.costUsd },
    };
    save();
    return response;
  };
}
