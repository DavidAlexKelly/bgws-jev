// ── bgws/rules/jevDecider.ts ───────────────────────────────────────────────
// Jev making a side's in-the-moment calls.
//
// Implements TacticalDecider (rules/tactical.ts): the engine offers a moment
// — an enemy crossing an arc, a move running into contact — with the rules
// having already worked out who is ABLE to act, and this asks Jev who
// SHOULD.
//
// THREE THINGS JEV IS NOT TRUSTED WITH
// ------------------------------------
//   - Legality. It answers about candidates the rules produced. Anything it
//     returns that is not one is impossible by construction (keys map back to
//     a fixed list) and would be dropped by the engine anyway.
//   - "Never". An element whose declared rule of engagement is `never` is
//     concealed by the commander's choice, and a tactical decider does not get
//     to overrule that. It is not asked about.
//   - Availability. A timeout, an error or a missing answer falls back to the
//     declared rule for that element, and the log says so.

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
import {
  contactState,
  describeOption,
  observerState,
  optionState,
  pruneOptions,
  reactionState,
} from "./jevState";
import { projectForSide } from "../lib/fogOfWar";
import type { Side } from "../lib/state";
import type {
  ContactVerdict,
  ObserverMoment,
  OptionMoment,
  ReactionVerdict,
  TacticalDecider,
  TacticalTrace,
} from "./tactical";

export interface JevDeciderOptions {
  side: Side;
  call: JevCall;
  /** Shown in logs. */
  name?: string;
  /** P(yes) at or above which a reactor fires. */
  reactThreshold?: number;
  /** Below this confidence a contact decision falls back to the commander's preset. */
  minContactConfidence?: number;
  /** How long the engine will wait, mid-action, before the rule decides. */
  timeoutMs?: number;
  /** Below this confidence an observer or option choice falls back to the rule. */
  minChoiceConfidence?: number;
  /** Doctrine or temperament, prepended to every question's instructions. */
  directive?: string;
}

const DEFAULTS = {
  reactThreshold: 0.5,
  minContactConfidence: 0.3,
  minChoiceConfidence: 0.2,
  timeoutMs: 2500,
};

function failureOf(err: unknown): "timeout" | "error" {
  return err instanceof Error && err.name === "JevTimeoutError" ? "timeout" : "error";
}

function describe(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function jevTacticalDecider(options: JevDeciderOptions): TacticalDecider {
  const settings = { ...DEFAULTS, ...options };
  const directive = options.directive?.trim() ? `${options.directive.trim()}\n\n` : "";

  const ask = async (
    state: unknown,
    questions: Record<string, JevQuestion>,
  ): Promise<JevResponse> =>
    withDeadline(settings.call({ state, questions }), settings.timeoutMs);

  return {
    name: options.name ?? `${JEV_MODEL}-${options.side}-tactics`,

    async decideReactions(moment): Promise<ReactionVerdict> {
      const asked = keyed(
        moment.candidates.filter((candidate) => candidate.engage !== "never"),
        "r",
      );
      const byRule = moment.candidates
        .filter((candidate) => candidate.ruleSaysReact)
        .map((candidate) => candidate.reactorId);
      if (asked.length === 0) return { reactorIds: byRule, traces: [] };

      const state = reactionState(moment);
      const questions: Record<string, JevQuestion> = {};
      for (const { key, item } of asked) {
        questions[key] = {
          type: "noul",
          instructions:
            `${directive}Should your element ${item.reactorId} open fire on enemy ` +
            `${moment.actorId} right now, as a snap shot? Read ${item.reactorId}'s entry ` +
            `in "reactors": its odds, its exposure, its orders and its rules of ` +
            `engagement, which are your commander's default for this element. Fire ` +
            `when the shot is worth giving up this element's action for the rest of ` +
            `the turn and possibly its concealment; hold when the odds are poor, when ` +
            `it has a more important job, or when revealing it costs more than the ` +
            `shot gains. At most ${moment.maxReactors} element(s) will fire.`,
          criteria: `Yes means ${item.reactorId} fires now. No means it holds fire.`,
        };
      }

      let response: JevResponse;
      try {
        response = await ask(state, questions);
      } catch (err) {
        return {
          reactorIds: byRule,
          traces: [
            {
              actorId: moment.actorId,
              question: `reactive fire at ${moment.actorId}`,
              options: asked.map(({ item }) => ({ id: item.reactorId, summary: item.engage })),
              chosenId: byRule.join(",") || "none",
              chosenBy: "heuristic",
              rationale: `Jev unavailable — rules of engagement decided (${describe(err)})`,
              fallback: failureOf(err),
            },
          ],
        };
      }

      const scored: { reactorId: string; p: number }[] = [];
      const traces: TacticalTrace[] = [];
      for (const { key, item } of asked) {
        const p = noulOf(response.answers, key);
        const fire = p == null ? item.ruleSaysReact : p >= settings.reactThreshold;
        if (fire) scored.push({ reactorId: item.reactorId, p: p ?? settings.reactThreshold });
        traces.push({
          actorId: item.reactorId,
          question: `snap-fire at ${moment.actorId}?`,
          options: [
            { id: "fire", summary: `fire at ${moment.actorId}, ${item.rangeM} m` },
            { id: "hold", summary: "hold fire" },
          ],
          chosenId: fire ? "fire" : "hold",
          chosenBy: p == null ? "heuristic" : "jev",
          rationale:
            `ROE ${item.engage} would ${item.ruleSaysReact ? "fire" : "hold"}` +
            (p == null ? "; Jev gave no answer for this element" : ""),
          probabilities: p == null ? undefined : { fire: p, hold: 1 - p },
          latencyMs: response.latencyMs,
          costUsd: traces.length === 0 ? response.costUsd : undefined,
          fallback: p == null ? "error" : undefined,
        });
      }

      scored.sort((a, b) => b.p - a.p);
      return { reactorIds: scored.map((entry) => entry.reactorId), traces };
    },

    async decideContact(moment): Promise<ContactVerdict> {
      const byPreset = moment.preferred === "press";
      const question = `contact while moving: press on or halt?`;
      const options = [
        { id: "halt", summary: "halt here and go to ground" },
        { id: "press", summary: `press on the remaining ${Math.round(moment.remainingM)} m` },
      ];

      let response: JevResponse;
      try {
        response = await ask(contactState(moment), {
          decision: {
            type: "choice",
            instructions:
              `${directive}Your element ${moment.actorId} has just made contact with ` +
              `${moment.newContacts.join(", ") || "the enemy"} while moving and has halted. ` +
              `Weigh its orders and your commander's plan against what it now faces: the ` +
              `new contacts' range and whether they can see it, the cover here versus at ` +
              `the destination, and its own strength and morale.`,
            criteria: {
              halt: "Stop here. Stay where it is, take whatever cover this ground offers, and do not complete the move.",
              press: "Keep going to the ordered destination despite the contact, accepting the exposure.",
            },
          },
        });
      } catch (err) {
        return {
          press: byPreset,
          traces: [
            {
              actorId: moment.actorId,
              question,
              options,
              chosenId: byPreset ? "press" : "halt",
              chosenBy: "heuristic",
              rationale: `Jev unavailable — commander's preset decided (${describe(err)})`,
              fallback: failureOf(err),
            },
          ],
        };
      }

      const answer = choiceOf(response.answers, "decision");
      const confident = answer != null && answer.confidence >= settings.minContactConfidence;
      const press = confident ? answer.choice === "press" : byPreset;

      return {
        press,
        traces: [
          {
            actorId: moment.actorId,
            question,
            options,
            chosenId: press ? "press" : "halt",
            chosenBy: confident ? "jev" : "heuristic",
            rationale: confident
              ? `commander's preset was ${moment.preferred}`
              : `Jev ${answer ? "was unsure" : "gave no answer"} — commander's preset (${moment.preferred}) decided`,
            probabilities: answer?.probabilities,
            confidence: answer?.confidence,
            latencyMs: response.latencyMs,
            costUsd: response.costUsd,
            fallback: confident ? undefined : answer ? "lowConfidence" : "error",
          },
        ],
      };
    },

    async chooseObserver(moment: ObserverMoment) {
      const nearest = moment.observers[0]?.observerId ?? null;
      const question = `who tries to make out ${moment.actorId}?`;
      const candidates = keyed(moment.observers, "w");
      const options = moment.observers.map((o) => ({
        id: o.observerId,
        summary: `${o.observerId} at ${o.rangeM} m${o.recce ? " (recce)" : ""}`,
      }));
      const criteria: Record<string, string> = {};
      for (const { key, item } of candidates) {
        criteria[key] =
          `${item.observerId} looks: ${item.rangeM} m away` +
          (item.recce ? ", a reconnaissance element" : "");
      }

      try {
        const response = await ask(observerState(moment), {
          observer: {
            type: "choice",
            instructions:
              `${directive}A concealed enemy element has just given itself away by acting. ` +
              "Exactly one of your elements may try to identify it. Pick the one most " +
              "likely to succeed — closer is easier, cover on the target makes it " +
              "harder, reconnaissance elements are better at it.",
            criteria,
          },
        });
        const answer = choiceOf(response.answers, "observer");
        const picked = candidates.find(({ key }) => key === answer?.choice)?.item.observerId;
        const confident =
          picked != null && (answer?.confidence ?? 0) >= settings.minChoiceConfidence;
        const chosenId = confident ? picked : nearest;
        return {
          observerId: chosenId,
          traces: [
            {
              actorId: chosenId ?? undefined,
              question,
              options,
              chosenId: chosenId ?? "none",
              chosenBy: confident ? "jev" : "heuristic",
              probabilities: answer
                ? Object.fromEntries(
                    candidates.map(({ key, item }) => [item.observerId, answer.probabilities[key] ?? 0]),
                  )
                : undefined,
              confidence: answer?.confidence,
              latencyMs: response.latencyMs,
              costUsd: response.costUsd,
              fallback: confident ? undefined : answer ? "lowConfidence" : "error",
            } satisfies TacticalTrace,
          ],
        };
      } catch (err) {
        return {
          observerId: nearest,
          traces: [
            {
              actorId: nearest ?? undefined,
              question,
              options,
              chosenId: nearest ?? "none",
              chosenBy: "heuristic",
              rationale: `Jev unavailable — nearest observer looked (${describe(err)})`,
              fallback: failureOf(err),
            } satisfies TacticalTrace,
          ],
        };
      }
    },

    async chooseOption(moment: OptionMoment) {
      const view = projectForSide(moment.state, moment.side);
      const offered = keyed(pruneOptions(moment.options, JEV_MAX_CHOICES - 1), "o");
      const criteria: Record<string, string> = {};
      for (const { key, item } of offered) {
        criteria[key] = describeOption(item, view, moment.config);
      }
      if (moment.allowPass) criteria.pass = "Do none of these.";
      const options = [
        ...moment.options.map((o) => ({ id: o.id, summary: o.summary })),
        ...(moment.allowPass ? [{ id: "pass", summary: "do none of these" }] : []),
      ];

      try {
        const response = await ask(optionState(moment), {
          option: {
            type: "choice",
            instructions: `${directive}Decide: ${moment.question}`,
            criteria,
          },
        });
        const answer = choiceOf(response.answers, "option");
        const confident = answer != null && answer.confidence >= settings.minChoiceConfidence;
        if (!confident) {
          return {
            optionId: undefined,
            traces: [
              {
                question: moment.question,
                options,
                chosenId: "rules",
                chosenBy: "heuristic",
                rationale: `Jev ${answer ? "was unsure" : "gave no answer"} — the engine's default decided`,
                confidence: answer?.confidence,
                latencyMs: response.latencyMs,
                costUsd: response.costUsd,
                fallback: answer ? "lowConfidence" : "error",
              } satisfies TacticalTrace,
            ],
          };
        }
        const picked = offered.find(({ key }) => key === answer.choice)?.item;
        return {
          optionId: picked?.id ?? null,
          traces: [
            {
              actorId: picked?.actorId,
              question: moment.question,
              options,
              chosenId: picked?.id ?? "pass",
              chosenBy: "jev",
              probabilities: Object.fromEntries(
                Object.entries(answer.probabilities).map(([key, p]) => [
                  offered.find((entry) => entry.key === key)?.item.id ?? key,
                  p,
                ]),
              ),
              confidence: answer.confidence,
              latencyMs: response.latencyMs,
              costUsd: response.costUsd,
            } satisfies TacticalTrace,
          ],
        };
      } catch (err) {
        return {
          optionId: undefined,
          traces: [
            {
              question: moment.question,
              options,
              chosenId: "rules",
              chosenBy: "heuristic",
              rationale: `Jev unavailable — the engine's default decided (${describe(err)})`,
              fallback: failureOf(err),
            } satisfies TacticalTrace,
          ],
        };
      }
    },
  };
}
