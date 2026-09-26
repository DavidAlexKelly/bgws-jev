// ── bgws/realtime/engine/jevDecider.ts ─────────────────────────────────────
// Jev in command, in real time.
//
// When units on one side need a decision at the same moment, they are asked
// together: ONE request, one `choice` question per unit, answered in parallel.
// Jev chooses from the options the rules generated, so it cannot give an
// order that is not possible.
//
// "Carry on" is always an option. When Jev is unsure the RULES decide — not
// "carry on", which is how a unit used to trade misses for ever.
//
// Jev is stateless: each question is a fresh request. So the state carries
// the memory — each unit's fire record, the fire it is taking, its last
// decisions and what came of them — and the side's picture of who is
// fighting whom, and every option says what it would really achieve (the
// chance per minute of doing damage, not of a round striking). That is what
// lets Jev weigh a trade-off instead of repeating the last answer.
//
// Every decision is printed to the browser console, like the turn game's.

import { distanceM } from "../../lib/board";
import { projectForSide } from "../../lib/fogOfWar";
import { inCover } from "../../lib/proceduralTerrain";
import { forceElementsOf, opposing, sightingOf, type Side } from "../../lib/state";
import {
  JEV_MAX_CHOICES,
  JEV_MODEL,
  choiceOf,
  keyed,
  withDeadline,
  type JevCall,
  type JevQuestion,
  type JevResponse,
} from "../../rules/jev";
import { printDecision } from "../../rules/jevConsole";
import type { TacticalTrace } from "../../rules/tactical";
import { canHit, describeOrder, onMission } from "./engine";
import { damageEffect, describeEffect, engagedBy, knownDamage } from "./options";
import { compass } from "./geometry";
import { ruleChoice, ruleTrace, type RtDecider, type RtDecision } from "./deciders";
import { PINNED_AT, SUPPRESSED_AT, clock } from "./timing";
import type { RtConfig, RtState } from "./types";

export interface JevRealtimeOptions {
  side: Side;
  call: JevCall;
  /** Doctrine or temperament, put in front of every question. */
  directive?: string;
  /** How long to wait before the rules decide. Sim time waits for this, not the other way round. */
  timeoutMs?: number;
  /** Below this, Jev's answer is not taken on its own and the unit carries on. */
  minConfidence?: number;
  /** Print decisions to the console. On unless turned off. */
  log?: boolean;
}

/**
 * What this side knows, right now, for Jev.
 *
 * Fog of war applied: enemies only as sighted, unidentified ones unnamed,
 * and their odds against us only when identified (their weapons are what
 * nobody knows yet).
 */
export function realtimeSideState(
  state: RtState,
  side: Side,
  config: RtConfig,
  asked: readonly string[],
  recent: readonly { time: number; text: string }[] = [],
) {
  const view = projectForSide(state.game, side);
  const objective = state.game.objectives?.[side];

  // Units being asked, and every unit in the same fights, get the full
  // picture — memory, fire record, friends around them. The rest are
  // summarised, so a big battle does not crowd out the part being decided.
  const fightOf = (id: string) => {
    const unit = state.units[id];
    const out = new Set<string>();
    if (unit?.engagement && state.time - unit.engagement.lastShotAt <= 60) out.add(unit.engagement.targetId);
    for (const [enemyId, fire] of Object.entries(unit?.incoming ?? {})) if (state.time - fire.last <= 60) out.add(enemyId);
    return out;
  };
  const askedFights = new Set(asked.flatMap((id) => [...fightOf(id)]));
  const involved = new Set(view.own.map((fe) => fe.id).filter((id) => [...fightOf(id)].some((e) => askedFights.has(e))));

  const units = view.own.map((self) => {
    const unit = state.units[self.id];
    const threats = forceElementsOf(state.game, opposing(side))
      .filter((enemy) => enemy.combatStrength > 0 && sightingOf(state.game, side, enemy.id) === "full")
      .filter((enemy) => canHit(enemy, self, self.position, config))
      .map((enemy) => ({
        from: enemy.id,
        rangeM: Math.round(distanceM(enemy.position, self.position)),
        itsFireOnYou: describeEffect(damageEffect(enemy, self, state, config), "you"),
      }));
    const detailed = unit != null && (asked.includes(self.id) || involved.has(self.id));
    const memory = detailed ? unitMemory(state, self.id) : {};
    const friends = view.own
      .filter((other) => other.id !== self.id && distanceM(other.position, self.position) <= FRIENDS_M)
      .map((other) => ({
        id: other.id,
        rangeM: Math.round(distanceM(other.position, self.position)),
        doing: state.units[other.id] ? describeOrder(state.units[other.id].order) : "unknown",
      }));
    const seesNow = unit
      ? Object.keys(unit.ownSeen).filter((enemyId) => state.game.forceElements[enemyId]?.combatStrength > 0)
      : [];
    return {
      id: self.id,
      unit: self.label,
      strength: `${self.combatStrength}/${self.combatStrengthStart}`,
      ...(unit ? { vehicles: `${unit.vehicles.fit} of ${unit.vehicles.total} fighting` } : {}),
      ...(unit && unit.posture === "hullDown" ? { hullDown: true } : {}),
      troopQuality: self.troopQuality,
      ...(unit
        ? {
            cohesion: unit.cohesion,
            suppression: `${Math.round(unit.suppression)}/100${
              unit.suppression >= PINNED_AT ? " (pinned: cannot advance)" : unit.suppression >= SUPPRESSED_AT ? " (suppressed)" : ""
            }`,
            posture: unit.posture,
            mission: `${unit.mission.task}: ${unit.mission.purpose}${
              unit.mission.at
                ? ` (${Math.round(distanceM(self.position, unit.mission.at))} m ${compass(self.position, unit.mission.at)})`
                : ""
            }`,
            onMission: onMission(unit, self),
          }
        : { morale: self.morale }),
      doing: unit ? describeOrder(unit.order) : "unknown",
      rulesOfEngagement: unit?.roe,
      terrain: config.terrain.classify(self.position),
      inCover: inCover(config.terrain, self.position),
      ...(seesNow.length ? { seesItself: seesNow } : {}),
      ...(objective
        ? {
            objective: `${Math.round(distanceM(self.position, objective))} m ${compass(self.position, objective)}`,
          }
        : {}),
      ...(threats.length ? { threatsToYou: threats } : {}),
      ...memory,
      ...(detailed && friends.length ? { friendsWithin1500m: friends } : {}),
      beingAskedNow: asked.includes(self.id),
    };
  });

  const contacts = view.contacts.map((contact) => ({
    id: contact.id,
    identified: contact.sighting === "full",
    unit: contact.label ?? "unidentified",
    seen: contact.observedMarkers,
    // Who is fighting whom, so units can be coordinated: one covering while
    // another moves, or fire spread over targets that are being ignored.
    engagedBy: engagedBy(state, side, contact.id),
    firingOn: view.own
      .filter((fe) => {
        const fire = state.units[fe.id]?.incoming[contact.id];
        return fire != null && state.time - fire.last <= 60;
      })
      .map((fe) => fe.id),
    damageYouHaveDoneToIt: knownDamage(state, side, contact.id),
    // Knocked-out vehicles are seen to burn; only when it is identified.
    ...(contact.sighting === "full" && state.units[contact.id]
      ? { vehiclesStillFighting: `${state.units[contact.id].vehicles.fit} of ${state.units[contact.id].vehicles.total}` }
      : {}),
    ...(contact.sighting === "full" && state.units[contact.id]?.cohesion !== "steady"
      ? { visiblyBreaking: state.units[contact.id]?.cohesion === "broken" ? "falling back" : "shaken" }
      : {}),
    nearestOfYoursM: view.own.length
      ? Math.round(Math.min(...view.own.map((fe) => distanceM(fe.position, contact.position))))
      : null,
  }));
  const inSight = new Set(view.contacts.map((contact) => contact.id));
  const lastKnown = Object.entries(state.lastKnown?.[side] ?? {})
    .filter(([id]) => !inSight.has(id))
    .map(([id, seen]) => ({
      id,
      unit: seen.label ?? "unidentified",
      lastSeen: `${clock(state.time - seen.time)} ago`,
      nearestOfYoursM: view.own.length
        ? Math.round(Math.min(...view.own.map((fe) => distanceM(fe.position, seen.at))))
        : null,
    }));

  return {
    clock: clock(state.time),
    you: side,
    ...(state.plan[side] ? { commandersPlan: state.plan[side] } : {}),
    yourUnits: units,
    knownEnemies: contacts,
    ...(lastKnown.length ? { lostContacts: lastKnown } : {}),
    recent: recent.slice(-8),
    note:
      "Real time: every unit acts at once. Enemies you have not sighted are not " +
      "listed; their absence is not evidence that they are not there. Lost " +
      "contacts are where an enemy was last seen, not where it is. Shaken and " +
      "broken units are not yours to order until they rally.",
  };
}

/** Units in this much range of each other are "nearby" for coordination. */
const FRIENDS_M = 1500;

/**
 * What a unit remembers: its fire on its target, the fire it is taking, and
 * its last few decisions with what came of each. Jev is stateless — every
 * question is a fresh request — so this is its only memory.
 */
export function unitMemory(state: RtState, id: string) {
  const unit = state.units[id];
  const fe = state.game.forceElements[id];
  if (!unit || !fe) return {};
  const e = unit.engagement;
  const firing = e && state.time - e.lastShotAt <= 60 ? e : null;
  const incoming = Object.entries(unit.incoming)
    .filter(([, fire]) => state.time - fire.last <= 120)
    .map(([from, fire]) => ({
      from,
      for: clock(fire.last - fire.since),
      shots: fire.shots,
      damageToYou: fire.damage,
    }));
  const decisions = unit.history.map((memory) => ({
    ago: `${clock(state.time - memory.time)} ago`,
    chose: memory.chose,
    by: memory.by,
    because: memory.because,
    since: `lost ${memory.strength - fe.combatStrength} strength, did ${unit.dealt - memory.dealt} damage`,
  }));
  return {
    ...(firing
      ? {
          yourFire: {
            target: firing.targetId,
            for: clock(state.time - firing.since),
            shots: firing.shots,
            struck: firing.hits,
            damageDone: firing.damage,
          },
        }
      : {}),
    ...(incoming.length ? { fireOnYou: incoming } : {}),
    ...(decisions.length ? { lastDecisions: decisions } : {}),
  };
}

/** One line on a unit's fire and last decision, for its question. */
function briefing(state: RtState, id: string): string {
  const unit = state.units[id];
  const e = unit?.engagement;
  const parts: string[] = [];
  if (e && state.time - e.lastShotAt <= 60) {
    parts.push(
      `It has been firing on ${e.targetId} for ${clock(state.time - e.since)}: ${e.shots} shots, ` +
        `${e.hits} struck, ${e.damage} damage done.`,
    );
  }
  const last = unit?.history[unit.history.length - 1];
  if (last) {
    const fe = state.game.forceElements[id];
    parts.push(
      `Last decision (${clock(state.time - last.time)} ago, by ${last.by}): ${last.chose} — since then it has ` +
        `lost ${last.strength - (fe?.combatStrength ?? last.strength)} strength and done ${(unit?.dealt ?? 0) - last.dealt} damage.`,
    );
  }
  return parts.length ? `${parts.join(" ")} ` : "";
}

function describe(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function jevRealtimeDecider(
  options: JevRealtimeOptions,
  /** The last few things that happened, as this side saw them. Supplied by the runner. */
  recent: () => { time: number; text: string }[] = () => [],
): RtDecider {
  const minConfidence = options.minConfidence ?? 0.25;
  const timeoutMs = options.timeoutMs ?? 4000;
  const directive = options.directive?.trim() ? `${options.directive.trim()}\n\n` : "";
  const print = (trace: TacticalTrace, stateSent: unknown) => {
    if (options.log !== false) printDecision(options.side, trace, stateSent);
  };

  return {
    name: `${JEV_MODEL}-${options.side}-realtime`,

    async decide(state, side, requests, config): Promise<RtDecision[]> {
      const asked = keyed(requests, "u");
      const stateSent = realtimeSideState(state, side, config, requests.map((r) => r.unitId), recent());
      const keysFor = new Map<string, { key: string; id: string; summary: string }[]>();
      const questions: Record<string, JevQuestion> = {};

      const team = requests.map((request) => ({
        id: request.unitId,
        doing: describeOrder(state.units[request.unitId].order),
      }));
      for (const { key, item } of asked) {
        const choices = item.options.slice(0, JEV_MAX_CHOICES).map((option, index) => ({
          key: option.id === "keep" ? "keep" : `o${index}`,
          id: option.id,
          summary: option.summary,
        }));
        keysFor.set(key, choices);
        const label = state.game.forceElements[item.unitId]?.label ?? item.unitId;
        questions[key] = {
          type: "choice",
          instructions:
            `${directive}Your unit ${item.unitId} (${label}) is ${describeOrder(state.units[item.unitId].order)}. ` +
            `What just happened: ${item.events.map((event) => event.detail).join("; ")}. ` +
            briefing(state, item.unitId) +
            (team.length > 1
              ? `Also being decided now, so choose them to work together: ${team
                  .filter((one) => one.id !== item.unitId)
                  .map((one) => `${one.id} (${one.doing})`)
                  .join(", ")}. `
              : "") +
            "The crew has already run its drill (returned fire, taken the nearest cover). " +
            "Choose its order from now on. Weigh its mission and your commander's plan " +
            "against the threats to it, its odds, the cover around it, its strength " +
            "and its suppression. How it moves matters: a road march does not fire; " +
            "tactical movement fires within its ROE; an assault fires at anything and " +
            "closes to point-blank, which is how ground is taken; bounding is slow and " +
            "covered. Once in contact, halting to engage, taking cover or flanking is " +
            "usually better than driving on into the enemy; a quiet unit off its " +
            "mission should resume it; an enemy that breaks can be pursued. " +
            "Judge fire by what it has actually done (yourFire, lastDecisions) and the " +
            "damage odds per minute in each option, not by how often rounds strike: " +
            "if a long exchange is doing nothing, change something — close to effective " +
            "range, flank, shift to a target you can hurt, or break contact. When a friend " +
            "is already firing on a target, it can cover you while you move. " +
            "Carry on only when what it is doing is working or nothing better is on offer.",
          criteria: Object.fromEntries(choices.map((choice) => [choice.key, choice.summary])),
        };
      }

      let response: JevResponse;
      try {
        response = await withDeadline(options.call({ state: stateSent, questions }), timeoutMs);
      } catch (err) {
        return requests.map((request) => {
          const optionId = ruleChoice(state, request);
          const trace = ruleTrace(
            request,
            optionId,
            `Jev unavailable (${describe(err)}) — the rules decided`,
            err instanceof Error && err.name === "JevTimeoutError" ? "timeout" : "error",
          );
          print(trace, stateSent);
          return { unitId: request.unitId, optionId, trace };
        });
      }

      return asked.map(({ key, item }, index) => {
        const choices = keysFor.get(key) ?? [];
        const answer = choiceOf(response.answers, key);
        const picked = choices.find((choice) => choice.key === answer?.choice);
        const confident = answer != null && picked != null && answer.confidence >= minConfidence;
        // Unsure is not "carry on": with twenty options on the table even a
        // clear preference can have low confidence, and defaulting to "carry
        // on" is how a unit kept trading misses for ever. The rules decide.
        const optionId = confident ? picked.id : ruleChoice(state, item);
        const trace: TacticalTrace = {
          actorId: item.unitId,
          question: item.events.map((event) => event.kind).join(" + "),
          options: choices.map((choice) => ({ id: choice.id, summary: choice.summary })),
          chosenId: optionId,
          chosenBy: confident ? "jev" : "heuristic",
          rationale: confident
            ? item.events.map((event) => event.detail).join("; ")
            : `Jev ${answer ? `was unsure (${picked?.id ?? answer.choice} at ${Math.round((answer.confidence ?? 0) * 100)}% confidence)` : "gave no answer"} — the rules chose`,
          probabilities: answer
            ? Object.fromEntries(choices.map((choice) => [choice.id, answer.probabilities[choice.key] ?? 0]))
            : undefined,
          confidence: answer?.confidence,
          latencyMs: response.latencyMs,
          costUsd: index === 0 ? response.costUsd : undefined,
          fallback: confident ? undefined : answer ? "lowConfidence" : "error",
          mark: choices.find((choice) => choice.id === optionId)?.summary,
        };
        print(trace, stateSent);
        return { unitId: item.unitId, optionId, trace };
      });
    },
  };
}
