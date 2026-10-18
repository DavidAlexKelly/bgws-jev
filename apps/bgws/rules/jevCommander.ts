// ── bgws/rules/jevCommander.ts ─────────────────────────────────────────────
// Jev as the whole commander, with no language model above it.
//
// Two shapes, for the two sequences of play:
//
//   jevCommander        the per-activation Commander (alternating activation,
//                       rules/turnLoop.ts). One choice question per
//                       activation: which of the offered options.
//   jevOrdersCommander  the OrdersCommander (orders phase, rules/orders.ts).
//                       One request per turn with a choice per element — the
//                       questions run in parallel, so a whole force costs one
//                       round trip.
//
// WHAT IS LOST WITHOUT A PLANNER ABOVE IT
// ---------------------------------------
// Jev chooses; it does not form intent. Each element's choice is judged on
// its own merits against the same picture, so there is no "main effort" in
// the sense the LLM commander's `plan` names one. That is a real weakness and
// the point of measuring it: "LLM plans, Jev executes" is the pairing the
// integration plan recommends, and this is the control that says whether the
// planner earns its cost.

import { opposing, type Side } from "../lib/state";
import type { SideView } from "../lib/fogOfWar";
import type { ActionOption, Commander } from "./commander";
import {
  JEV_MAX_CHOICES,
  JEV_MODEL,
  choiceOf,
  keyed,
  noulOf,
  withDeadline,
  type JevCall,
  type JevQuestion,
  type JevResponse,
} from "./jev";
import { describeOption, pruneOptions, sideState } from "./jevState";
import type {
  CounteractionOrders,
  Intent,
  Orders,
  OrdersCommander,
  StandingOrder,
} from "./orders";
import type { StandingEngagement } from "./ruleset";
import type { PhaseConfig } from "./turnLoop";

export interface JevCommanderOptions {
  side: Side;
  call: JevCall;
  /** Doctrine, mission or temperament, prepended to every question. */
  directive?: string;
  name?: string;
  /**
   * Terrain and rules, for describing where a move ends up. Optional: without
   * it options are described by the engine's summary alone.
   */
  config?: Pick<PhaseConfig, "terrain" | "ruleset">;
  timeoutMs?: number;
}

const NONE = "uncommitted";

function preamble(side: Side, directive?: string): string {
  return (
    `You command ${side.toUpperCase()} in a turn-based battlegroup wargame, ` +
    `against ${opposing(side).toUpperCase()}. ` +
    (directive?.trim() ? `${directive.trim()} ` : "")
  );
}

// ── Per activation ─────────────────────────────────────────────────────────

/**
 * A per-activation Commander backed by Jev, falling back to another.
 *
 * The fallback is required rather than optional: an activation MUST return an
 * option, and the engine's own default — the first one — is a worse answer
 * to an outage than the heuristic's.
 */
export function jevCommander(
  options: JevCommanderOptions & { fallback: Commander },
): Commander {
  const timeoutMs = options.timeoutMs ?? 4000;
  const config = options.config as PhaseConfig | undefined;

  return {
    kind: "jev",
    name: options.name ?? `${JEV_MODEL}-${options.side}`,
    async decide(view: SideView, offered: ActionOption[], question: string) {
      const pruned = keyed(pruneOptions(offered, JEV_MAX_CHOICES), "o");
      const criteria: Record<string, string> = {};
      for (const { key, item } of pruned) criteria[key] = describeOption(item, view, config);

      try {
        const response = await withDeadline(
          options.call({
            state: sideState(view, config),
            questions: {
              action: {
                type: "choice",
                instructions:
                  `${preamble(options.side, options.directive)}Decide ${question} ` +
                  "Exactly one element acts now; the enemy then replies. Prefer actions " +
                  "with good odds that keep your force together and out of the open.",
                criteria,
              },
            },
          }),
          timeoutMs,
        );
        const answer = choiceOf(response.answers, "action");
        const chosen = pruned.find(({ key }) => key === answer?.choice)?.item;
        if (chosen) {
          return {
            optionId: chosen.id,
            rationale: `Jev ${Math.round((answer?.probabilities[answer.choice] ?? 0) * 100)}% ` +
              `(confidence ${(answer?.confidence ?? 0).toFixed(2)})`,
          };
        }
      } catch {
        // Falls through to the fallback, which is the point of having one.
      }
      const fallen = await options.fallback.decide(view, offered, question);
      return { ...fallen, rationale: `Jev unavailable — ${options.fallback.name} decided` };
    },
  };
}

// ── Orders phase ───────────────────────────────────────────────────────────

const ENGAGEMENTS: Record<StandingEngagement, string> = {
  never: "Hold fire and stay concealed whatever happens; keep the ammunition.",
  ifFiredUpon: "Stay quiet until the enemy engages your side, then answer.",
  withinShortRange: "Answer anything that closes inside short range.",
  always: "Answer anything that can be reached. Overwatch.",
};

/**
 * An OrdersCommander backed by Jev.
 *
 * One request per turn:
 *   - a choice per element: one of its legal options, or stay uncommitted
 *   - a choice per element: its rules of engagement
 *   - a yes/no per element: hold it in reserve (when reserves are on)
 *
 * Command capacity is then spent on the elements Jev was MOST sure of, which
 * is how "most important first" gets expressed without a planner to rank it.
 */
export function jevOrdersCommander(
  options: JevCommanderOptions & { fallback?: OrdersCommander },
): OrdersCommander {
  const timeoutMs = options.timeoutMs ?? 8000;
  const config = options.config as PhaseConfig | undefined;
  const lead = preamble(options.side, options.directive);

  const call = (state: unknown, questions: Record<string, JevQuestion>): Promise<JevResponse> =>
    withDeadline(options.call({ state, questions }), timeoutMs);

  return {
    kind: "jev",
    name: options.name ?? `${JEV_MODEL}-${options.side}`,

    async planTurn(request): Promise<Orders> {
      const elements = keyed(Object.entries(request.optionsByElement), "e");
      const questions: Record<string, JevQuestion> = {};
      const optionKeys = new Map<string, { key: string; item: ActionOption }[]>();

      for (const { key, item: [actorId, offered] } of elements) {
        const pruned = keyed(
          pruneOptions(
            offered.filter((option) => option.kind !== "hold"),
            JEV_MAX_CHOICES - 1,
          ),
          "o",
        );
        optionKeys.set(key, pruned);
        const criteria: Record<string, string> = {
          [NONE]:
            "Leave this element uncommitted this turn: it stays where it is, " +
            "watching, and does not cost command capacity.",
        };
        for (const option of pruned) {
          criteria[option.key] = describeOption(option.item, request.view, config);
        }
        questions[`${key}_order`] = {
          type: "choice",
          instructions:
            `${lead}What does your element ${actorId} do this turn? Only ` +
            `${request.activationBudget === Number.POSITIVE_INFINITY ? "all" : request.activationBudget} ` +
            "element(s) can be committed; the ones you are surest about get the " +
            "capacity. Engage where the odds favour you, keep elements that can " +
            "support each other together, and do not walk into the open under " +
            "enemy guns without a reason.",
          criteria,
        };
        questions[`${key}_roe`] = {
          type: "choice",
          instructions:
            `${lead}Rules of engagement for ${actorId} this turn: when does it snap-fire ` +
            "at an enemy acting in its arc? A reacting element cannot act again this " +
            "turn and reveals itself.",
          criteria: { ...ENGAGEMENTS },
        };
        if (request.reserveLimit > 0) {
          questions[`${key}_reserve`] = {
            type: "noul",
            instructions:
              `${lead}Should ${actorId} be held in reserve this turn? A reserve does not ` +
              "fire or assault in the first round; after both sides have acted it may " +
              "move up to 1,000 m to where it is needed and then fire or assault. " +
              `At most ${request.reserveLimit} element(s) may be held back.`,
          };
        }
      }

      if (elements.length === 0) {
        return { side: request.side, intents: [], plan: "no element has a decision to make" };
      }

      let response: JevResponse;
      try {
        response = await call(sideState(request.view, config), questions);
      } catch (err) {
        const reason = err instanceof Error ? err.message : String(err);
        if (options.fallback) {
          const fallen = await options.fallback.planTurn(request);
          return {
            ...fallen,
            plan: `Jev unreachable — ${options.fallback.name} planned (${reason})`,
            failure: `unreachable: ${reason}`,
          };
        }
        return {
          side: request.side,
          intents: [],
          plan: `Jev unreachable — held position (${reason})`,
          failure: `unreachable: ${reason}`,
        };
      }

      // One candidate order per element, ranked by how sure Jev was.
      const candidates: { intent: Intent; certainty: number; kind: string }[] = [];
      const standingOrders: StandingOrder[] = [];
      const reserveVotes: { actorId: string; p: number }[] = [];

      for (const { key, item: [actorId] } of elements) {
        const order = choiceOf(response.answers, `${key}_order`);
        const picked = optionKeys.get(key)?.find(({ key: k }) => k === order?.choice)?.item;
        if (order && picked) {
          const p = order.probabilities[order.choice] ?? order.confidence;
          candidates.push({
            intent: {
              actorId,
              optionId: picked.id,
              rationale: `Jev ${Math.round(p * 100)}%`,
            },
            certainty: p,
            kind: picked.kind,
          });
        }

        const roe = choiceOf(response.answers, `${key}_roe`);
        if (roe && roe.choice in ENGAGEMENTS) {
          standingOrders.push({ actorId, engage: roe.choice as StandingEngagement });
        }

        const reserve = noulOf(response.answers, `${key}_reserve`);
        if (reserve != null && reserve >= 0.5) reserveVotes.push({ actorId, p: reserve });
      }

      candidates.sort((a, b) => b.certainty - a.certainty);
      const intents = candidates
        .slice(0, request.activationBudget)
        .map((candidate) => candidate.intent);

      // A reserve that was also ordered to fire would fire nothing — the round
      // forbids it — so only elements NOT ordered to engage are held back.
      const engaging = new Set(
        candidates
          .filter((candidate) => candidate.kind === "fire" || candidate.kind === "assault")
          .map((candidate) => candidate.intent.actorId),
      );
      const reserves = reserveVotes
        .filter((vote) => !engaging.has(vote.actorId))
        .sort((a, b) => b.p - a.p)
        .slice(0, request.reserveLimit)
        .map((vote) => vote.actorId);

      const uncommitted = elements.length - intents.length;
      return {
        side: request.side,
        intents,
        standingOrders,
        reserves,
        plan:
          `Jev: ${intents.length} committed, most certain first` +
          (uncommitted > 0 ? `; ${uncommitted} uncommitted` : "") +
          (reserves.length ? `; reserve ${reserves.join(", ")}` : ""),
      };
    },

    async planCounteraction(request): Promise<CounteractionOrders> {
      const offered = keyed(
        [...Object.values(request.reserveMoves), ...Object.values(request.fireOptions)].flat(),
        "c",
      );
      if (offered.length === 0) return { side: request.side, optionIds: [] };

      const questions: Record<string, JevQuestion> = {};
      for (const { key, item } of offered) {
        questions[key] = {
          type: "noul",
          instructions:
            `${lead}The first round of the turn has been fought. Take this now? ` +
            `${describeOption(item, request.view, config)}. A second-round shot is worse ` +
            "than a first-round one; passing is final for the round.",
        };
      }

      try {
        const response = await call(sideState(request.view, config), questions);
        const taken = offered
          .map(({ key, item }) => ({ id: item.id, p: noulOf(response.answers, key) ?? 0 }))
          .filter((entry) => entry.p >= 0.5)
          .sort((a, b) => b.p - a.p);
        return {
          side: request.side,
          optionIds: taken.map((entry) => entry.id),
          plan: taken.length ? undefined : "Jev passed the counteraction round",
        };
      } catch (err) {
        return {
          side: request.side,
          optionIds: [],
          plan: `Jev unreachable — passed (${err instanceof Error ? err.message : String(err)})`,
        };
      }
    },
  };
}
