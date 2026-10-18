// ── bgws/rules/jev.ts ──────────────────────────────────────────────────────
// The shape of a Jev decision call, and nothing that makes one.
//
// Jev (TypeSafe, on OpenRouter as `typesafe/jev-1.13`) is a DECISION model,
// not a text model. It is handed a state and a map of named questions, and
// answers each one with a typed value:
//
//   choice  one of up to 255 described options, with a probability for every
//           option and a confidence for the whole distribution
//   noul    yes or no, as the probability of yes
//   score   a position on a described scale
//
// All questions in one request are answered in parallel, so asking about six
// reactors costs one round trip, not six. That property is what makes
// in-the-moment decisions affordable, and the deciders lean on it.
//
// As with llmCommander.ts, THE CALL IS INJECTED. Everything that builds a
// question or reads an answer is pure and tested with a fake; the network
// lives in data/jevClient.ts.

/** The pinned model. `-latest` would make yesterday's experiment unrepeatable. */
export const JEV_MODEL = "typesafe/jev-1.13";

/** Jev's hard limit on the options in one choice question. */
export const JEV_MAX_CHOICES = 255;

export type JevQuestion =
  | {
      type: "choice";
      instructions: string;
      /** Option key → what choosing it means, in words. Keys are what comes back. */
      criteria: Record<string, string>;
    }
  | {
      type: "noul";
      instructions: string;
      /** What a "yes" means, when the instructions alone leave it open. */
      criteria?: string;
    }
  | {
      type: "score";
      instructions: string;
      /** Level key → what that level means, lowest first. */
      criteria: Record<string, string>;
    };

export interface JevRequest {
  /** Anything JSON: Jev reads structure as readily as prose. */
  state: unknown;
  questions: Record<string, JevQuestion>;
}

export interface JevChoiceAnswer {
  type: "choice";
  choice: string;
  probabilities: Record<string, number>;
  confidence: number;
}

export interface JevNoulAnswer {
  type: "noul";
  /** Probability that the answer is yes, 0–1. */
  noul: number;
}

export interface JevScoreAnswer {
  type: "score";
  score: number;
  probabilities: Record<string, number>;
  confidence: number;
}

export type JevAnswer = JevChoiceAnswer | JevNoulAnswer | JevScoreAnswer;

export interface JevResponse {
  answers: Record<string, JevAnswer>;
  latencyMs: number;
  /** What OpenRouter says the call cost, where it says. */
  costUsd?: number;
}

/** The only impure thing: send a request, get typed answers back. */
export type JevCall = (request: JevRequest) => Promise<JevResponse>;

export class JevTimeoutError extends Error {
  constructor(ms: number) {
    super(`Jev did not answer within ${ms} ms`);
    this.name = "JevTimeoutError";
  }
}

/**
 * Race a call against a deadline.
 *
 * The engine is in the middle of an action when it asks, so a decider that
 * waits forever stalls the whole turn. A late answer is thrown away and the
 * rule decides instead.
 */
export async function withDeadline<T>(promise: Promise<T>, ms: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new JevTimeoutError(ms)), ms);
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function numberRecord(value: unknown): Record<string, number> {
  if (!isRecord(value)) return {};
  const out: Record<string, number> = {};
  for (const [key, entry] of Object.entries(value)) {
    if (typeof entry === "number" && Number.isFinite(entry)) out[key] = entry;
  }
  return out;
}

/**
 * Read the answers out of a Decisions API response body.
 *
 * TOLERANT, BECAUSE THE ENDPOINT IS ALPHA. The documented shape is answers
 * keyed by question id, each tagged with its type. This also accepts the
 * answers at the top level, an answer that omits its tag (the question it
 * answers says what it must be), and a noul given as a bare boolean. What it
 * will not do is invent an answer to a question that was not answered — a
 * missing key stays missing and the decider falls back for that question.
 */
export function parseJevAnswers(
  body: unknown,
  questions: Record<string, JevQuestion>,
): Record<string, JevAnswer> {
  if (!isRecord(body)) return {};
  const container = isRecord(body.answers)
    ? body.answers
    : isRecord(body.decisions)
      ? body.decisions
      : body;

  const answers: Record<string, JevAnswer> = {};
  for (const [id, question] of Object.entries(questions)) {
    const raw = container[id];
    if (raw == null) continue;

    if (question.type === "noul") {
      const value = isRecord(raw) ? (raw.noul ?? raw.probability ?? raw.value) : raw;
      const p =
        typeof value === "number" ? value : typeof value === "boolean" ? (value ? 1 : 0) : NaN;
      if (Number.isFinite(p)) answers[id] = { type: "noul", noul: Math.min(1, Math.max(0, p)) };
      continue;
    }

    if (!isRecord(raw)) {
      if (question.type === "choice" && typeof raw === "string" && raw in question.criteria) {
        answers[id] = { type: "choice", choice: raw, probabilities: {}, confidence: 0 };
      }
      continue;
    }

    const probabilities = numberRecord(raw.probabilities);
    const confidence = typeof raw.confidence === "number" ? raw.confidence : 0;

    if (question.type === "choice") {
      const choice = typeof raw.choice === "string" ? raw.choice : undefined;
      // Only an option that was offered. Jev guarantees this; the check is
      // for a proxy or a schema change that does not.
      if (choice && choice in question.criteria) {
        answers[id] = { type: "choice", choice, probabilities, confidence };
      }
      continue;
    }

    if (typeof raw.score === "number") {
      answers[id] = { type: "score", score: raw.score, probabilities, confidence };
    }
  }
  return answers;
}

/**
 * Short, stable keys for things whose real ids are long or awkward.
 *
 * Option ids carry colons and coordinates, element ids whatever the force
 * list chose. The API does not need to see either — it needs keys it can hand
 * back unchanged — so every question and criterion is keyed `prefix0`,
 * `prefix1`, … and mapped back here.
 */
export function keyed<T>(items: readonly T[], prefix: string): { key: string; item: T }[] {
  return items.map((item, index) => ({ key: `${prefix}${index}`, item }));
}

export function choiceOf(answers: Record<string, JevAnswer>, id: string): JevChoiceAnswer | null {
  const answer = answers[id];
  return answer?.type === "choice" ? answer : null;
}

export function noulOf(answers: Record<string, JevAnswer>, id: string): number | null {
  const answer = answers[id];
  return answer?.type === "noul" ? answer.noul : null;
}
