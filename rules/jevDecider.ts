// ── bgws/rules/jevDecider.ts ───────────────────────────────────────────────
// Jev making a side's in-the-moment calls.
//
// Implements TacticalDecider (rules/tactical.ts): the engine offers a moment
// — an enemy crossing an arc, a move running into contact, an activation to
// take — with the rules having already worked out what is POSSIBLE, and this
// asks Jev what SHOULD happen.
//
// ONE PIPELINE FOR EVERY QUESTION
// -------------------------------
// Every decision here is a single Jev `choice` over described options, and
// every one goes through `decide`:
//
//   1. ask Jev (or use an answer prefetched for exactly this moment)
//   2. confident?  → take its choice, or SAMPLE from its probabilities when a
//                    seeded generator is configured
//   3. unsure?     → ESCALATE to the side's language model when one is
//                    configured; otherwise the rule decides
//   4. unreachable → the rule decides
//   5. print it to the console, and hand the engine a trace for the log
//
// Keeping that in one place is what makes the five kinds of question behave
// the same way — the same thresholds, the same fallbacks, the same log line.
//
// THREE THINGS JEV IS NOT TRUSTED WITH
// ------------------------------------
//   - Legality. It chooses from options the rules produced; its keys map back
//     to a fixed list, so it cannot name anything else.
//   - "Never". An element whose declared rule of engagement is `never` is
//     concealed by the commander's choice, and is not offered as a reactor.
//   - Availability. A timeout or error is the rule's decision, and says so.

import type { Rng } from "./dice";
import {
  JEV_MAX_CHOICES,
  JEV_MODEL,
  choiceOf,
  keyed,
  withDeadline,
  type JevCall,
  type JevChoiceAnswer,
  type JevQuestion,
} from "./jev";
import { printDecision } from "./jevConsole";
import {
  activationState,
  contactState,
  describeOption,
  observerState,
  optionState,
  pruneOptions,
  reactionState,
} from "./jevState";
import type { ModelCall } from "./llmCommander";
import { projectForSide } from "../lib/fogOfWar";
import type { Side } from "../lib/state";
import type {
  ActivationMoment,
  ContactVerdict,
  ObserverMoment,
  OptionMoment,
  ReactionMoment,
  ReactionVerdict,
  TacticalDecider,
  TacticalTrace,
} from "./tactical";

export interface JevDeciderOptions {
  side: Side;
  call: JevCall;
  /** Shown in logs. */
  name?: string;
  /** Doctrine or temperament, prepended to every question's instructions. */
  directive?: string;
  /** How long the engine will wait, mid-action, before the rule decides. */
  timeoutMs?: number;
  /** Below this confidence Jev's answer is not taken on its own. */
  minConfidence?: number;
  /**
   * The side's language model, asked when Jev is unsure.
   *
   * Slow — seconds, not milliseconds — so it is kept for the calls Jev itself
   * says are hard. Absent, an unsure Jev defers to the rule instead.
   */
  escalate?: ModelCall;
  escalateTimeoutMs?: number;
  /**
   * Sample from Jev's probabilities instead of always taking its top choice.
   *
   * Seeded, so a game still replays exactly. Two games from the same position
   * then differ the way two commanders would, in proportion to how close the
   * call was — a 90/10 decision almost never flips, a 55/45 one often does.
   */
  sampleRng?: Rng;
  /** Print every decision to the console. On unless turned off. */
  log?: boolean;
}

const DEFAULTS = {
  timeoutMs: 2500,
  minConfidence: 0.25,
  escalateTimeoutMs: 20_000,
};

/** Reactor combinations are listed up to this many candidates, nearest first. */
const MAX_REACTOR_CANDIDATES = 10;
/** How many prefetched moments go into one request. */
const PREFETCH_BATCH = 8;

// ── The pipeline ───────────────────────────────────────────────────────────

interface Choice {
  key: string;
  /** What the engine gets back. */
  id: string;
  /** For the log and the console. */
  summary: string;
  /** For Jev: what choosing this means. */
  criterion: string;
  /** For the map marker. */
  mark: string;
}

interface Ask {
  question: string;
  actorId?: string;
  state: unknown;
  instructions: string;
  choices: Choice[];
  /** The rule's answer. Undefined means "the engine's own default". */
  fallbackId?: string;
  /** What the rule was, in words, for the log when it decides. */
  fallbackWhy: string;
  /** Used instead of a call when this exact moment was asked ahead of time. */
  prefetched?: { answer: JevChoiceAnswer; latencyMs: number };
}

interface Decided {
  id?: string;
  trace: TacticalTrace;
}

function failureOf(err: unknown): "timeout" | "error" {
  return err instanceof Error && err.name === "JevTimeoutError" ? "timeout" : "error";
}

function describe(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** Pick a key in proportion to its probability. Falls back to `fallback`. */
function sample(
  probabilities: Record<string, number>,
  keys: readonly string[],
  rng: Rng,
  fallback: string,
): string {
  const weights = keys.map((key) => Math.max(0, probabilities[key] ?? 0));
  const total = weights.reduce((sum, w) => sum + w, 0);
  if (total <= 0) return fallback;
  let roll = (rng.int(1_000_000) / 1_000_000) * total;
  for (const [index, key] of keys.entries()) {
    roll -= weights[index];
    if (roll < 0) return key;
  }
  return keys[keys.length - 1];
}

/** The escalation prompt: the same question, put to a model that can read. */
export function escalationPrompt(side: Side, directive: string, ask: Ask): string {
  const state = JSON.stringify(ask.state);
  return [
    `You command ${side.toUpperCase()} in a turn-based battlegroup wargame.`,
    directive.trim(),
    "A fast decision model was unsure about the call below, so it comes to you.",
    "",
    ask.instructions,
    "",
    "SITUATION (JSON, fog of war applied — this is all you know):",
    state.length > 12_000 ? `${state.slice(0, 12_000)}…` : state,
    "",
    "OPTIONS (reply with the key):",
    ...ask.choices.map((choice) => `  ${choice.key} — ${choice.criterion}`),
    "",
    'JSON only: {"choice":"<key>","why":"<one sentence>"}',
  ]
    .filter((line, index) => line !== "" || index > 0)
    .join("\n");
}

/** Read a key out of a model's reply. Only a key that was offered counts. */
export function parseEscalation(
  reply: string,
  keys: readonly string[],
): { key?: string; why?: string } {
  const choice = /"choice"\s*:\s*"([^"]+)"/.exec(reply)?.[1];
  const why = /"why"\s*:\s*"([^"]*)"/.exec(reply)?.[1];
  if (choice && keys.includes(choice)) return { key: choice, why };
  // A bare key on its own is an answer too.
  const bare = reply.trim().replace(/^["'`]|["'`]$/g, "");
  if (keys.includes(bare)) return { key: bare };
  return {};
}

export function jevTacticalDecider(options: JevDeciderOptions): TacticalDecider {
  const settings = { ...DEFAULTS, ...options };
  const side = options.side;
  const directive = options.directive?.trim() ? `${options.directive.trim()}\n\n` : "";
  const prefetched = new Map<string, { answer: JevChoiceAnswer; latencyMs: number }>();

  async function decide(ask: Ask): Promise<Decided> {
    const keys = ask.choices.map((choice) => choice.key);
    const byKey = new Map(ask.choices.map((choice) => [choice.key, choice]));
    const listed = ask.choices.map((choice) => ({ id: choice.id, summary: choice.summary }));
    const probabilitiesById = (answer?: JevChoiceAnswer) =>
      answer
        ? Object.fromEntries(
            ask.choices.map((choice) => [choice.id, answer.probabilities[choice.key] ?? 0]),
          )
        : undefined;

    const finish = (decided: Decided): Decided => {
      if (options.log !== false) printDecision(side, decided.trace, ask.state);
      return decided;
    };

    const byRule = (fallback: TacticalTrace["fallback"], why: string, extra: Partial<TacticalTrace> = {}) => {
      const choice = ask.choices.find((one) => one.id === ask.fallbackId);
      return finish({
        id: ask.fallbackId,
        trace: {
          actorId: ask.actorId,
          question: ask.question,
          options: listed,
          chosenId: ask.fallbackId ?? "engine default",
          chosenBy: "heuristic",
          rationale: `${why} — ${ask.fallbackWhy}`,
          fallback,
          mark: choice?.mark,
          ...extra,
        },
      });
    };

    // 1. Ask, unless the answer is already waiting.
    let answer: JevChoiceAnswer | null;
    let latencyMs: number;
    let costUsd: number | undefined;
    if (ask.prefetched) {
      answer = ask.prefetched.answer;
      latencyMs = ask.prefetched.latencyMs;
    } else {
      try {
        const response = await withDeadline(
          settings.call({
            state: ask.state,
            questions: {
              decision: {
                type: "choice",
                instructions: `${directive}${ask.instructions}`,
                criteria: Object.fromEntries(ask.choices.map((choice) => [choice.key, choice.criterion])),
              },
            },
          }),
          settings.timeoutMs,
        );
        answer = choiceOf(response.answers, "decision");
        latencyMs = response.latencyMs;
        costUsd = response.costUsd;
      } catch (err) {
        return byRule(failureOf(err), `Jev unavailable (${describe(err)})`);
      }
    }

    const common = {
      probabilities: probabilitiesById(answer ?? undefined),
      confidence: answer?.confidence,
      latencyMs,
      costUsd,
    };

    // 2. Confident: take it, or sample it.
    if (answer && answer.confidence >= settings.minConfidence && byKey.has(answer.choice)) {
      const key = settings.sampleRng
        ? sample(answer.probabilities, keys, settings.sampleRng, answer.choice)
        : answer.choice;
      const choice = byKey.get(key)!;
      return finish({
        id: choice.id,
        trace: {
          actorId: ask.actorId,
          question: ask.question,
          options: listed,
          chosenId: choice.id,
          chosenBy: "jev",
          rationale:
            (ask.prefetched ? "answered ahead of time; " : "") +
            (key !== answer.choice ? `sampled (Jev's top choice was ${byKey.get(answer.choice)?.id})` : "Jev's choice"),
          mark: choice.mark,
          ...common,
        },
      });
    }

    // 3. Unsure: escalate, if there is anyone to escalate to.
    const unsure = answer ? `Jev unsure (confidence ${answer.confidence.toFixed(2)})` : "Jev gave no answer";
    if (settings.escalate) {
      try {
        const reply = await withDeadline(
          settings.escalate(escalationPrompt(side, options.directive ?? "", ask)),
          settings.escalateTimeoutMs,
        );
        const { key, why } = parseEscalation(reply, keys);
        const choice = key ? byKey.get(key) : undefined;
        if (choice) {
          return finish({
            id: choice.id,
            trace: {
              actorId: ask.actorId,
              question: ask.question,
              options: listed,
              chosenId: choice.id,
              chosenBy: "llm",
              rationale: `${unsure}; escalated to the commander${why ? `: ${why}` : ""}`,
              mark: choice.mark,
              ...common,
            },
          });
        }
      } catch {
        // Falls through to the rule, which is the answer of last resort.
      }
    }

    return byRule(answer ? "lowConfidence" : "error", unsure, common);
  }

  // ── Reactive fire, as one coordinated choice ─────────────────────────────

  /**
   * Who fires, as ONE question over every combination.
   *
   * Asking each reactor "fire?" separately let three tanks each say yes to a
   * target worth one shot, or each say no expecting another to take it. A
   * single choice over "none", each reactor alone, and each pair (up to the
   * cap) makes the trade explicit: the answer is a fire plan, not a vote.
   */
  function reactionAsk(moment: ReactionMoment): Ask {
    const asked = moment.candidates
      .filter((candidate) => candidate.engage !== "never")
      .sort((a, b) => a.rangeM - b.rangeM)
      .slice(0, MAX_REACTOR_CANDIDATES);
    const state = reactionState(moment);
    const oddsOf = new Map(
      (state.reactors as { id: string; oddsIfFiring?: { pHit: number } }[]).map((reactor) => [
        reactor.id,
        reactor.oddsIfFiring?.pHit,
      ]),
    );
    const describeReactor = (id: string) => {
      const candidate = asked.find((one) => one.reactorId === id)!;
      const p = oddsOf.get(id);
      return `${id} (${candidate.rangeM} m${p != null ? `, ${Math.round(p * 100)}% to hit` : ""}, ROE ${candidate.engage})`;
    };

    const combos: string[][] = [];
    const cap = Math.max(1, Math.min(moment.maxReactors, asked.length));
    const grow = (start: number, current: string[]) => {
      if (current.length > 0) combos.push(current);
      if (current.length === cap) return;
      for (let i = start; i < asked.length; i += 1) grow(i + 1, [...current, asked[i].reactorId]);
    };
    grow(0, []);
    combos.sort((a, b) => a.length - b.length);

    const choices: Choice[] = [
      {
        key: "none",
        id: "none",
        summary: "hold fire",
        criterion: `Nobody fires at ${moment.actorId}. Every element keeps its action and its concealment.`,
        mark: `holds fire on ${moment.actorId}`,
      },
      ...combos.slice(0, JEV_MAX_CHOICES - 1).map((combo, index) => ({
        key: `f${index}`,
        id: combo.join("+"),
        summary: `${combo.join(" and ")} fire${combo.length === 1 ? "s" : ""}`,
        criterion:
          `${combo.map(describeReactor).join(" and ")} ${combo.length === 1 ? "fires" : "fire"} at ` +
          `${moment.actorId} now; ${combo.length === 1 ? "it is" : "they are"} spent for the turn.`,
        mark: `fires on ${moment.actorId}`,
      })),
    ];

    const byRule = moment.candidates
      .filter((candidate) => candidate.ruleSaysReact)
      .map((candidate) => candidate.reactorId);
    const ruleId =
      byRule.length === 0
        ? "none"
        : (choices.find((choice) => choice.id === byRule.slice(0, cap).join("+"))?.id ??
          byRule.slice(0, cap).join("+"));

    return {
      question: `who fires at ${moment.actorId}?`,
      actorId: asked[0]?.reactorId,
      state,
      instructions:
        `Enemy ${moment.actorId} is acting inside your arcs. Decide which of your ` +
        `elements, if any, snap-fire at it now — at most ${moment.maxReactors}. Weigh ` +
        "each shot's odds against what firing costs that element: its action for the " +
        "rest of the turn, its concealment, and the threats it faces. Rules of " +
        "engagement are the commander's default for each element. A mover that is " +
        "Disrupted or Broken by this fire does not complete its move.",
      choices,
      fallbackId: ruleId,
      fallbackWhy: "rules of engagement decided",
    };
  }

  function reactionKey(moment: ReactionMoment): string {
    const actor = moment.state.forceElements[moment.actorId];
    const at = actor ? `${actor.position.lat.toFixed(5)},${actor.position.lng.toFixed(5)}` : "?";
    return [
      moment.turn,
      moment.round,
      moment.actorId,
      at,
      moment.wasFiredUpon,
      moment.candidates.map((candidate) => `${candidate.reactorId}:${candidate.engage}`).join(","),
    ].join("|");
  }

  return {
    name: options.name ?? `${JEV_MODEL}-${side}-tactics`,

    async decideReactions(moment): Promise<ReactionVerdict> {
      const asked = moment.candidates.filter((candidate) => candidate.engage !== "never");
      if (asked.length === 0) {
        return {
          reactorIds: moment.candidates.filter((c) => c.ruleSaysReact).map((c) => c.reactorId),
          traces: [],
        };
      }
      const key = reactionKey(moment);
      const ask = { ...reactionAsk(moment), prefetched: prefetched.get(key) };
      prefetched.delete(key);

      const { id, trace } = await decide(ask);
      const reactorIds = !id || id === "none" ? [] : id.split("+");
      return { reactorIds, traces: [trace] };
    },

    async prefetchReactions(moments) {
      const asks = moments
        .filter((moment) => moment.candidates.some((candidate) => candidate.engage !== "never"))
        .map((moment) => ({ key: reactionKey(moment), ask: reactionAsk(moment) }));

      for (let start = 0; start < asks.length; start += PREFETCH_BATCH) {
        const batch = keyed(asks.slice(start, start + PREFETCH_BATCH), "m");
        const questions: Record<string, JevQuestion> = {};
        const state: Record<string, unknown> = {};
        for (const { key, item } of batch) {
          state[key] = item.ask.state;
          questions[key] = {
            type: "choice",
            instructions:
              `${directive}This is about moment "${key}" in the state: a possible ` +
              `enemy action later this turn. ${item.ask.instructions}`,
            criteria: Object.fromEntries(item.ask.choices.map((choice) => [choice.key, choice.criterion])),
          };
        }
        try {
          const response = await withDeadline(
            settings.call({ state: { moments: state }, questions }),
            settings.timeoutMs * 2,
          );
          for (const { key, item } of batch) {
            const answer = choiceOf(response.answers, key);
            if (answer) prefetched.set(item.key, { answer, latencyMs: 0 });
          }
          if (options.log !== false && typeof console !== "undefined") {
            console.info?.(
              `[Jev ${side}] prefetched ${batch.length} likely reaction(s) in one call ` +
                `(${response.latencyMs} ms)`,
            );
          }
        } catch {
          // Prefetching is an optimisation. A failure means asking live later.
        }
      }
    },

    async decideContact(moment): Promise<ContactVerdict> {
      const { id, trace } = await decide({
        question: "contact while moving: press on or halt?",
        actorId: moment.actorId,
        state: contactState(moment),
        instructions:
          `Your element ${moment.actorId} has just made contact with ` +
          `${moment.newContacts.join(", ") || "the enemy"} while moving and has halted. ` +
          "Weigh its orders, the objective and your commander's plan against what it " +
          "now faces: the threats to it here, the cover here versus at the " +
          "destination, and its own strength and morale.",
        choices: [
          {
            key: "halt",
            id: "halt",
            summary: "halt here and go to ground",
            criterion:
              "Stop here. Stay where it is, take whatever cover this ground offers, and do not complete the move.",
            mark: "halts on contact",
          },
          {
            key: "press",
            id: "press",
            summary: `press on the remaining ${Math.round(moment.remainingM)} m`,
            criterion: "Keep going to the ordered destination despite the contact, accepting the exposure.",
            mark: "presses on",
          },
        ],
        fallbackId: moment.preferred,
        fallbackWhy: `commander's preset (${moment.preferred}) decided`,
      });
      return { press: id === "press", traces: [trace] };
    },

    async chooseObserver(moment: ObserverMoment) {
      const nearest = moment.observers[0]?.observerId ?? null;
      const { id, trace } = await decide({
        question: `who tries to make out ${moment.actorId}?`,
        state: observerState(moment),
        instructions:
          "A concealed enemy element has just given itself away by acting. Exactly one " +
          "of your elements may try to identify it. Pick the one most likely to succeed " +
          "— closer is easier, cover on the target makes it harder, reconnaissance " +
          "elements are better at it.",
        choices: keyed(moment.observers, "w").map(({ key, item }) => ({
          key,
          id: item.observerId,
          summary: `${item.observerId} at ${item.rangeM} m${item.recce ? " (recce)" : ""}`,
          criterion:
            `${item.observerId} looks: ${item.rangeM} m away` +
            (item.recce ? ", a reconnaissance element" : ""),
          mark: "watching",
        })),
        fallbackId: nearest ?? undefined,
        fallbackWhy: "the nearest observer looked",
      });
      return {
        observerId: id ?? nearest,
        traces: [{ ...trace, actorId: id ?? nearest ?? undefined }],
      };
    },

    async chooseOption(moment: OptionMoment) {
      const view = projectForSide(moment.state, moment.side);
      const offered = keyed(pruneOptions(moment.options, JEV_MAX_CHOICES - 1), "o");
      const { id, trace } = await decide({
        question: moment.question,
        actorId: moment.options[0]?.actorId,
        state: optionState(moment),
        instructions: `Decide: ${moment.question}`,
        choices: [
          ...offered.map(({ key, item }) => ({
            key,
            id: item.id,
            summary: item.summary,
            criterion: describeOption(item, view, moment.config),
            mark: item.kind,
          })),
          ...(moment.allowPass
            ? [{ key: "pass", id: "pass", summary: "do none of these", criterion: "Do none of these.", mark: "passes" }]
            : []),
        ],
        // No rule of its own: the engine's heuristic is the default.
        fallbackId: undefined,
        fallbackWhy: "the engine's default decided",
      });
      return { optionId: id === undefined ? undefined : id === "pass" ? null : id, traces: [trace] };
    },

    async chooseActivation(moment: ActivationMoment) {
      const view = projectForSide(moment.state, moment.side);
      // Each element keeps its ordered option, every engagement and a hold;
      // moves are pruned first when the list is long. Budget shared across
      // elements so no single element crowds the others out.
      const perElement = Math.max(
        3,
        Math.floor((JEV_MAX_CHOICES - 1) / Math.max(1, moment.candidates.length)),
      );
      const entries = moment.candidates.flatMap((candidate) => {
        const ordered = candidate.options.find((o) => o.id === candidate.orderedOptionId);
        const rest = pruneOptions(
          candidate.options.filter((o) => o.id !== candidate.orderedOptionId),
          perElement - (ordered ? 1 : 0),
        );
        return [...(ordered ? [ordered] : []), ...rest].map((option) => ({
          actorId: candidate.actorId,
          option,
          ordered: option.id === candidate.orderedOptionId,
        }));
      });

      const choices = keyed(entries.slice(0, JEV_MAX_CHOICES), "c").map(({ key, item }) => ({
        key,
        id: `${item.actorId}::${item.option.id}`,
        summary: item.option.summary + (item.ordered ? " [as ordered]" : ""),
        criterion:
          `${item.actorId} acts now: ${describeOption(item.option, view, moment.config)}` +
          (item.ordered ? ". THIS IS WHAT THE COMMANDER ORDERED." : ""),
        mark: item.ordered ? "as ordered" : "adapted",
      }));

      const { id, trace } = await decide({
        question: "which element acts now, and how?",
        state: activationState(moment),
        instructions:
          "Choose the next activation. The commander's orders are the plan: carry them " +
          "out unless what has happened since makes a different action clearly better " +
          "— a target that is gone, a new threat, a better shot. Order matters too: act " +
          "first with the element whose action most shapes what follows.",
        choices,
        fallbackId: undefined,
        fallbackWhy: "the next order was carried out as written",
      });

      if (!id) return { traces: [trace] };
      const [actorId, optionId] = id.split("::");
      return { pick: { actorId, optionId }, traces: [{ ...trace, actorId }] };
    },
  };
}
