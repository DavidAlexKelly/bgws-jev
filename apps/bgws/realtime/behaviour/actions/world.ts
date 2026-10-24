// ── behaviour/actions/world.ts ───────────────────────────────────────────
// The world action and fact vocabulary: every verb a template may offer at a
// world decision point, and every fact its conditions may test
// (docs/plan/MULTI_DOMAIN_PLAN.md W3–W11, DECISION_FLOW_GRAPHS.md §3).
//
// DATA ONLY. The handlers — what each verb does to the game — live in
// realtime/world/actions.ts, keyed by the same names; a test checks the two
// lists are the same. Kept apart so the template validator (and the
// Behaviour Manager) can know the vocabulary without importing the engine.
//
// Each verb names the trigger it answers in the plan's decision tables.

import type { FactInfo } from "../facts";

export interface WorldActionInfo {
  /** The domain whose system raises it. */
  domain: "land" | "fires" | "c2" | "drones" | "ew" | "air" | "sea" | "space" | "cyber" | "logistics" | "medical" | "killweb";
  /** What it does, as the Behaviour Manager and Jev read it. */
  label: string;
}

const a = (domain: WorldActionInfo["domain"], label: string): WorldActionInfo => ({ domain, label });

export const WORLD_ACTION_INFO = {
  // ── W3 land: fire requests, counter-battery, obstacles, dismounts, stocks ──
  "fires.fireNow": a("fires", "Fire the mission now"),
  "fires.fireWhenLaid": a("fires", "Lay on the target carefully, then fire"),
  "fires.refuse": a("fires", "Refuse the mission (out of range, out of ammunition)"),
  "fires.fireThenScoot": a("fires", "Fire, then move at once (shoot and scoot)"),
  "fires.displaceNow": a("fires", "Displace now: the position may be located"),
  "fires.finishThenDisplace": a("fires", "Finish the mission, then displace"),
  "fires.stay": a("fires", "Stay in position"),
  "land.breach": a("land", "Breach the obstacle"),
  "land.bypass": a("land", "Find a way round the obstacle"),
  "land.callEngineers": a("land", "Halt and call the engineers"),
  "land.waitAtObstacle": a("land", "Wait at the obstacle"),
  "land.pushThroughLane": a("land", "Push through on a lane"),
  "land.reverseOut": a("land", "Reverse out of the minefield"),
  "land.dismountHere": a("land", "Dismount the infantry here"),
  "land.dismountInCover": a("land", "Dismount in cover close by"),
  "land.stayMounted": a("land", "Stay mounted"),
  "land.remount": a("land", "Remount the infantry"),
  "land.requestResupply": a("land", "Ask for resupply"),
  "land.conserve": a("land", "Conserve: fire only in self-defence"),
  "land.withdrawToResupply": a("land", "Withdraw to resupply"),
  "land.digIn": a("land", "Dig in here"),

  // ── W4 C2 ──
  "c2.carryOnPhase": a("c2", "Carry on with the current phase"),
  "c2.actOnIntent": a("c2", "Act on the commander's intent: go on to the next phase"),
  "c2.holdAndReestablish": a("c2", "Hold and try to re-establish comms"),
  "c2.moveToRegainComms": a("c2", "Move to regain comms with HQ"),
  "c2.fireFirstRequest": a("c2", "Fire the most urgent request first"),
  "c2.commitReserve": a("c2", "Commit the reserve"),
  "c2.holdReserve": a("c2", "Hold the reserve"),
  "c2.reinforce": a("c2", "Reinforce the unit in trouble"),
  "c2.swapWithReserve": a("c2", "Swap it with the reserve"),
  "c2.withdrawIt": a("c2", "Withdraw it"),
  "c2.carryOn": a("c2", "Carry on as planned"),
  "c2.displacePost": a("c2", "Displace the command post"),
  "c2.alternateBearer": a("c2", "Switch to the alternate bearer"),
  "c2.acceptRisk": a("c2", "Accept the risk and stay on the air"),

  // ── W5 drones ──
  "drones.reportAndWatch": a("drones", "Report it and keep watching"),
  "drones.callFire": a("drones", "Call fire on it"),
  "drones.handOff": a("drones", "Hand it off to a strike team"),
  "drones.strike": a("drones", "Strike it"),
  "drones.pushOn": a("drones", "Push on (risk losing the drone)"),
  "drones.climbForLink": a("drones", "Climb to regain the link"),
  "drones.returnHome": a("drones", "Bring it home"),
  "drones.goAutonomous": a("drones", "Switch to autonomous terminal guidance"),
  "drones.onePass": a("drones", "One more pass"),
  "drones.ditchOnTarget": a("drones", "Ditch it on the target"),
  "drones.switchToFibre": a("drones", "Switch to fibre drones"),
  "drones.relocate": a("drones", "Relocate the team"),
  "drones.callFireOnJammer": a("drones", "Call fire on the jammer"),
  "drones.wait": a("drones", "Wait it out"),
  "drones.finishThenRelocate": a("drones", "Finish the flight, then relocate"),
  "drones.stay": a("drones", "Stay put"),
  "drones.freezeAndHide": a("drones", "Freeze and hide from the drone"),
  "drones.shootItDown": a("drones", "Shoot it down"),
  "drones.scatter": a("drones", "Scatter"),
  "drones.moveToCover": a("drones", "Move to cover"),
  "drones.callCounterDrone": a("drones", "Call counter-drone"),
  "drones.engageTarget": a("drones", "Engage this target"),
  "drones.holdForBetter": a("drones", "Hold for a better target"),

  // ── W6 EW ──
  "ew.locate": a("ew", "Locate the emitter (takes time)"),
  "ew.jam": a("ew", "Jam it"),
  "ew.reportForStrike": a("ew", "Report it for a strike"),
  "ew.ignore": a("ew", "Ignore it"),
  "ew.goSilent": a("ew", "Go silent"),
  "ew.relocateAfter": a("ew", "Relocate after this transmission"),
  "ew.carryOn": a("ew", "Carry on transmitting"),
  "ew.switchBearer": a("ew", "Switch bearer"),
  "ew.jamSector": a("ew", "Jam the sector"),
  "ew.protectAdvance": a("ew", "Move up behind the advance and protect it"),
  "ew.fireAnyway": a("ew", "Fire anyway"),
  "ew.switchMunition": a("ew", "Switch to non-GNSS munitions"),
  "ew.waitForPnt": a("ew", "Wait for PNT to recover"),
  "ew.requestDroneGuidance": a("ew", "Ask for drone guidance onto the target"),

  // ── W7 air ──
  "air.defend": a("air", "Defend: notch, chaff and flares"),
  "air.pressAttack": a("air", "Press the attack"),
  "air.abort": a("air", "Abort the mission"),
  "air.shootNow": a("air", "Shoot now"),
  "air.closeForPk": a("air", "Close for a surer shot"),
  "air.holdFire": a("air", "Hold fire (not identified, or the rules forbid it)"),
  "air.rtb": a("air", "Return to base"),
  "air.tanker": a("air", "Go to the tanker"),
  "air.onePass": a("air", "Stay for one more pass"),
  "air.goAround": a("air", "Route around the threat"),
  "air.goLow": a("air", "Go low under the radar"),
  "air.requestSead": a("air", "Request SEAD"),
  "air.engageCheapest": a("air", "Engage with the cheapest interceptor that will do"),
  "air.engageSurest": a("air", "Engage with the surest interceptor"),
  "air.holdTight": a("air", "Hold: weapons tight, not identified"),
  "air.goSilentLetPass": a("air", "Go silent and let it pass"),
  "air.shutDown": a("air", "Shut the radar down"),
  "air.stayUp": a("air", "Stay up and engage"),
  "air.relocateRadar": a("air", "Relocate the radar"),
  "air.acceptNineLine": a("air", "Accept the 9-line"),
  "air.acceptWithDelay": a("air", "Accept, with delay"),
  "air.declineNineLine": a("air", "Decline (threat or fuel)"),

  // ── W8 sea ──
  "sea.engageLong": a("sea", "Engage with the long-range layer"),
  "sea.engageMedium": a("sea", "Engage with the medium layer"),
  "sea.engageShort": a("sea", "Leave it to point defence"),
  "sea.decoyManoeuvre": a("sea", "Decoys and manoeuvre"),
  "sea.unmask": a("sea", "Turn to unmask more launchers"),
  "sea.leaveToReload": a("sea", "Leave station to reload"),
  "sea.conserve": a("sea", "Stay and conserve: point defence only"),
  "sea.requestEscort": a("sea", "Request an escort"),
  "sea.engageBeyondHorizon": a("sea", "Engage beyond the horizon"),
  "sea.closeToIdentify": a("sea", "Close to identify"),
  "sea.shadow": a("sea", "Shadow it"),
  "sea.avoid": a("sea", "Avoid it"),
  "sea.attack": a("sea", "Attack"),
  "sea.track": a("sea", "Track it"),
  "sea.evade": a("sea", "Evade: go deep, sprint and drift"),
  "sea.goSilent": a("sea", "Go silent"),
  "sea.gunsAndHelo": a("sea", "Engage with guns and helicopter"),
  "sea.withdraw": a("sea", "Withdraw"),

  // ── W9 space and cyber ──
  "space.imageArea": a("space", "Image this area on the pass"),
  "space.skipPass": a("space", "Hold the pass for a better area"),
  "cyber.executeNow": a("cyber", "Execute the access now"),
  "cyber.holdForMoment": a("cyber", "Hold for a better moment"),
  "cyber.abandon": a("cyber", "Abandon it: it may be detected"),

  // ── W10 logistics and medical ──
  "logistics.convoy": a("logistics", "Send a convoy"),
  "logistics.ugv": a("logistics", "Send a ground robot"),
  "logistics.drone": a("logistics", "Send it by drone"),
  "logistics.waitForDark": a("logistics", "Wait for dark"),
  "logistics.pushOn": a("logistics", "Push on"),
  "logistics.alternateRoute": a("logistics", "Take the alternate route"),
  "logistics.scatter": a("logistics", "Scatter"),
  "logistics.takeCover": a("logistics", "Take cover"),
  "logistics.relocateNode": a("logistics", "Relocate the node back"),
  "logistics.disperseStocks": a("logistics", "Disperse the stocks"),
  "logistics.repairAndStay": a("logistics", "Repair and stay"),
  "medical.evacuateNow": a("medical", "Evacuate now under threat"),
  "medical.waitForDark": a("medical", "Wait for dark"),
  "medical.requestUgv": a("medical", "Ask for a robot or drone to evacuate"),

  // ── W11 kill web ──
  "killweb.useShooter": a("killweb", "Task this shooter"),
  "killweb.defer": a("killweb", "Defer the target"),
  "killweb.restrike": a("killweb", "Strike again"),
  "killweb.lookAgain": a("killweb", "Look again before deciding"),
  "killweb.moveOn": a("killweb", "Move on to the next target"),
  "killweb.holdGuns": a("killweb", "Hold the guns"),
  "killweb.shiftAircraft": a("killweb", "Shift the aircraft"),
  "killweb.acceptRisk": a("killweb", "Accept the risk"),
} as const satisfies Record<string, WorldActionInfo>;

export type WorldAction = keyof typeof WORLD_ACTION_INFO;
export const WORLD_ACTIONS = Object.keys(WORLD_ACTION_INFO) as WorldAction[];

/**
 * Facts a world template may test. Every world question supplies all of
 * them (world/facts.ts); one that does not apply to the agent is null, which
 * every ordering comparison treats as false.
 */
export const WORLD_FACT_INFO = {
  "agent.domain": { label: "It is a", kind: "text", values: ["land", "air", "surface", "subsurface", "space", "node"] },
  "agent.archetype": { label: "Its archetype", kind: "text" },
  "agent.strengthPct": { label: "Its strength left", kind: "number", unit: "%" },
  "agent.ammoPct": { label: "Its ammunition left", kind: "number", unit: "%" },
  "agent.fuelPct": { label: "Its fuel left", kind: "number", unit: "%" },
  "agent.batteryPct": { label: "Its battery left", kind: "number", unit: "%" },
  "agent.magazinePct": { label: "Its magazine left", kind: "number", unit: "%" },
  "agent.onAirS": { label: "Time on the air", kind: "number", unit: "s" },
  "agent.inCover": { label: "It is in cover", kind: "boolean" },
  "agent.carriesDismounts": { label: "It carries dismounts", kind: "boolean" },
  "agent.hasFibreDrones": { label: "It has fibre drones", kind: "boolean" },
  "agent.armed": { label: "It is armed", kind: "boolean" },
  "agent.canAutonomous": { label: "It has autonomous terminal guidance", kind: "boolean" },
  "link.quality": { label: "Link quality", kind: "number" },
  "pnt.quality": { label: "GNSS quality here", kind: "number" },
  "target.rangeKm": { label: "Range to the target", kind: "number", unit: "km" },
  "target.inRange": { label: "The target is in range", kind: "boolean" },
  "target.identified": { label: "The target is identified", kind: "boolean" },
  "target.costUsd": { label: "The target's cost", kind: "number", unit: "$" },
  "threat.adWithinKm": { label: "Nearest known air defence", kind: "number", unit: "km" },
  "threat.located": { label: "Its own position may be located", kind: "boolean" },
  "env.night": { label: "It is dark", kind: "boolean" },
  "env.badWeather": { label: "The weather is bad", kind: "boolean" },
  "roe.weaponsFree": { label: "Weapons free", kind: "boolean" },
} as const satisfies Record<string, FactInfo>;

export type WorldFact = keyof typeof WORLD_FACT_INFO;
export const WORLD_FACTS = Object.keys(WORLD_FACT_INFO) as WorldFact[];
