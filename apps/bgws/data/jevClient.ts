// ── bgws/data/jevClient.ts ─────────────────────────────────────────────────
// The one impure function behind every Jev decision.
//
// rules/jev.ts defines the request and parses the answer, both pure. This
// sends it: straight from the browser to OpenRouter's Decisions API.
//
// ⚠ THE KEY IS BUILT INTO THE BUNDLE. Vite inlines `import.meta.env.VITE_*`
// at build time, so anyone who can load this app can read the key out of its
// JavaScript. That is acceptable for a key scoped to this app with a spend
// limit on it, and not otherwise. The alternative — a Foundry function with
// the key held as a source secret, called through the OSDK like
// `bgwsCommanderTurn` — is a drop-in replacement: anything that returns a
// JevCall will do.
//
// ⚠ THE ENDPOINT IS ALPHA. Every assumption about its wire format is in this
// file and in `parseJevAnswers`, and nowhere else.

import {
  JEV_MODEL,
  parseJevAnswers,
  type JevCall,
  type JevRequest,
  type JevResponse,
} from "../rules/jev";

export const OPENROUTER_DECISIONS_URL = "https://openrouter.ai/api/alpha/decisions";

/** Models the picker offers. The pinned one first: it is what experiments should use. */
export const JEV_MODELS = [JEV_MODEL, "~typesafe/jev-latest"] as const;

export type JevModelName = (typeof JEV_MODELS)[number];

/** Read once, here, so there is exactly one line to change if the key moves. */
const OPENROUTER_API_KEY: string | undefined = import.meta.env.VITE_OPENROUTER_API_KEY;

export class JevCallError extends Error {
  constructor(
    message: string,
    /** HTTP status, where there was a response at all. */
    readonly status?: number,
  ) {
    super(message);
    this.name = "JevCallError";
  }
}

/** True when a key has been configured. The play screen uses it to say why Jev is off. */
export function jevConfigured(): boolean {
  return Boolean(OPENROUTER_API_KEY);
}

export interface OpenRouterJevOptions {
  model?: JevModelName;
  /**
   * Remember answers to identical requests for the life of the page.
   *
   * On by default. The same moment asked twice — a discarded and regenerated
   * turn, a replay — should get the same answer, both because it is cheaper
   * and because a decider that changes its mind about an unchanged situation
   * makes a game impossible to reason about.
   */
  cache?: boolean;
}

const CACHE_LIMIT = 500;

/**
 * A JevCall that goes to OpenRouter.
 *
 * Deadlines are the caller's business (see `withDeadline`) because the right
 * one differs: a reaction is waited for mid-action and wants a short one, a
 * whole turn's orders can afford longer. A late answer is simply ignored.
 */
export function openRouterJevCall(options: OpenRouterJevOptions = {}): JevCall {
  const model = options.model ?? JEV_MODEL;
  const cache = options.cache === false ? null : new Map<string, JevResponse>();

  return async (request: JevRequest): Promise<JevResponse> => {
    if (!OPENROUTER_API_KEY) {
      throw new JevCallError(
        "No OpenRouter key. Set VITE_OPENROUTER_API_KEY in the Vite environment.",
      );
    }

    const body = JSON.stringify({ model, state: request.state, questions: request.questions });
    const cached = cache?.get(body);
    if (cached) return { ...cached, latencyMs: 0 };

    const started = performance.now();
    let response: Response;
    try {
      response = await fetch(OPENROUTER_DECISIONS_URL, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${OPENROUTER_API_KEY}`,
          "Content-Type": "application/json",
          // OpenRouter's attribution headers. Optional; they label the usage.
          "X-Title": "BGWS",
          ...(typeof location !== "undefined" ? { "HTTP-Referer": location.origin } : {}),
        },
        body,
      });
    } catch (err) {
      // A CSP or egress block surfaces here as a bare TypeError, which reads
      // like a bug. Say what it probably is.
      throw new JevCallError(
        `Could not reach OpenRouter (${err instanceof Error ? err.message : String(err)}). ` +
          "If this is a network or CSP block, openrouter.ai must be allowed for this app.",
      );
    }

    const text = await response.text();
    let parsed: unknown = undefined;
    try {
      parsed = text ? JSON.parse(text) : undefined;
    } catch {
      /* reported below with the raw text */
    }

    if (!response.ok) {
      const detail =
        (parsed as { error?: { message?: string } } | undefined)?.error?.message ??
        text.slice(0, 300);
      throw new JevCallError(`OpenRouter ${response.status}: ${detail}`, response.status);
    }

    const answers = parseJevAnswers(parsed, request.questions);
    if (Object.keys(answers).length === 0) {
      // Every question unanswered is a schema drift, not a decision. Throwing
      // makes the decider fall back, and says so, rather than reading silence
      // as "no" to every question.
      throw new JevCallError(`OpenRouter returned no readable answers: ${text.slice(0, 300)}`);
    }

    const usage = (parsed as { usage?: { cost?: number } } | undefined)?.usage;
    const result: JevResponse = {
      answers,
      latencyMs: Math.round(performance.now() - started),
      costUsd: typeof usage?.cost === "number" ? usage.cost : undefined,
    };

    if (cache) {
      if (cache.size >= CACHE_LIMIT) {
        const oldest = cache.keys().next().value;
        if (oldest !== undefined) cache.delete(oldest);
      }
      cache.set(body, result);
    }
    return result;
  };
}
