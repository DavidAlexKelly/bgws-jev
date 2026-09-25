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
  JEV_MODEL,
  choiceOf,
  keyed,
  noulOf,
  withDeadline,
  type JevCall,
  type JevQuestion,
  type JevResponse,
} from "./jev";
import { contactState, reactionState } from "./jevState";
import type { Side } from "../lib/state";
import type {
  ContactVerdict,
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
  /** Doctrine or temperament, prepended to every question's instructions. */
  directive?: string;
}

const DEFAULTS = {
  reactThreshold: 0.5,
  minContactConfidence: 0.3,
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
            fallback: confident ? undefined : answer ? "lowConfidence" : "error",
          },
        ],
      };
    },
  };
}
