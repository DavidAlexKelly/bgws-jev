// ── realtime/world/figures.ts ────────────────────────────────────────────
// Every hand-set number the world layer runs on, with its unit, the module
// that reads it, what it means, where it came from, and the range it could
// plausibly take (docs/plan/MULTI_DOMAIN_PLAN.md W0 step 4, W12).
//
// The engine's own figures are in engine/figures.ts; this is the same
// registry for the systems that came after, kept beside them so the land
// engine's file and its test are untouched. `ALL_FIGURES` joins the two for
// the figures table and the sensitivity report.
//
// `range` is what the sensitivity analysis sweeps (W12): the low and high
// ends of what the value could be, not its error bars.

import { FIGURES, type Figure } from "../engine/figures";

export interface WorldFigure extends Figure {
  /** Plausible low and high values, for the sensitivity sweep. */
  range: [number, number];
}

const declared = { kind: "declared" } as const;
const ref = (r: string) => ({ kind: "ref", ref: r }) as const;

export const WORLD_FIGURES = {
  // ── Effects on land units (world/landEffects) ──
  "world/landEffects.SPREAD_M": { value: 60, unit: "m", module: "world/landEffects", source: declared, range: [30, 120], note: "How far a platoon's vehicles are spread about its centre, for blast and area fire." },
  "world/landEffects.COVER_FACTOR": { value: 0.6, unit: "×", module: "world/landEffects", source: declared, range: [0.4, 0.8], note: "A unit in woods or buildings takes this share of the blast it would in the open." },
  "world/landEffects.DUG_IN_FACTOR": { value: 0.6, unit: "×", module: "world/landEffects", source: declared, range: [0.4, 0.8], note: "A fully dug-in position cuts blast casualties by this much (scaled by how dug in)." },
  "world/landEffects.BLAST_VS_ARMOUR": { value: 0.12, unit: "×", module: "world/landEffects", source: declared, range: [0.05, 0.25], note: "Blast that cannot penetrate still disables an armoured vehicle this share of the time (tracks, optics)." },
  "world/landEffects.BLAST_VS_LIGHT": { value: 0.5, unit: "×", module: "world/landEffects", source: declared, range: [0.3, 0.8], note: "Blast against a light vehicle, as a share of its effect on a soft one." },
  "world/landEffects.SUPPRESSION_NEAR": { value: 35, module: "world/landEffects", source: declared, range: [20, 50], note: "Suppression a round adds to a unit it lands on; it falls off with distance." },
  "world/landEffects.SUPPRESSION_FALLOFF_M": { value: 200, unit: "m", module: "world/landEffects", source: declared, range: [100, 400], note: "Distance over which a round's suppression falls to a third." },
  "world/landEffects.KILLED_SHARE": { value: 0.3, unit: "×", module: "world/landEffects", source: declared, range: [0.2, 0.4], note: "Of the people in a knocked-out vehicle or team, the share killed; the rest are wounded." },
  "world/landEffects.CREW_HURT_SHARE": { value: 0.5, unit: "×", module: "world/landEffects", source: declared, range: [0.3, 0.8], note: "Of a knocked-out vehicle's crew, the share hurt at all." },

  // ── The step itself (world/step) ──
  "world/step.LOG_KEEP": { value: 300, module: "world/step", source: declared, range: [300, 300], note: "World feed lines kept in the state (the full feed streams out as it is made)." },

  // ── Environment (world/environment) ──
  "world/environment.NIGHT_DETECT": { value: 0.35, unit: "×", module: "world/environment", source: declared, range: [0.2, 0.5], note: "Unaided eyes at night see this share of what they see by day; thermal sights are not affected." },
  "world/environment.CROSSOVER_THERMAL": { value: 0.6, unit: "×", module: "world/environment", source: ref("R4.1"), range: [0.4, 0.8], note: "Thermal sights near dawn and dusk crossover, when targets and ground are the same temperature." },
  "world/environment.DRONE_MAX_WIND_MS": { value: 12, unit: "m/s", module: "world/environment", source: declared, range: [9, 15], note: "Small multirotor drones cannot fly in more wind than this." },

  // ── PNT (world/pnt) ──
  "world/pnt.JAM_DEPTH": { value: 0.9, unit: "×", module: "world/pnt", source: declared, range: [0.7, 0.98], note: "At a GNSS jammer's centre, GNSS quality falls by this share; less towards the edge of its footprint." },
  "world/pnt.INERTIAL_CEP_M": { value: 150, unit: "m", module: "world/pnt", source: declared, range: [80, 300], note: "A GNSS-guided round with no GNSS at all falls back on inertial guidance: its circular error grows to about this." },

  // ── Munitions in flight (world/munitions) ──
  "world/munitions.FUZE_M": { value: 15, unit: "m", module: "world/munitions", source: declared, range: [5, 30], note: "A guided round passing its target this close (closest approach within the second) has reached it; then its kill probability is rolled." },
  "world/munitions.KEEP_SPENT_S": { value: 60, unit: "s", module: "world/munitions", source: declared, range: [60, 60], note: "A spent round stays in the world this long, for the map, then is removed." },

  // ── World sensors (world/sensing) ──
  "world/sensing.OPTIC_RATE": { value: 0.5, unit: "/s", module: "world/sensing", source: declared, range: [0.2, 1], note: "An optical or thermal sensor's detection hazard per second against a target at no range, falling to zero at its reach." },
  "world/sensing.SONAR_NOISE_DB": { value: 70, unit: "dB", module: "world/sensing", source: declared, range: [60, 80], note: "Ambient sea noise in the passive sonar equation (NL)." },
  "world/sensing.SONAR_DI_DB": { value: 15, unit: "dB", module: "world/sensing", source: declared, range: [10, 20], note: "Directivity index of a hull or towed array (DI)." },
  "world/sensing.SONAR_DT_DB": { value: 10, unit: "dB", module: "world/sensing", source: declared, range: [5, 15], note: "Detection threshold (DT)." },
  "world/sensing.TRACK_DROP_S": { value: 300, unit: "s", module: "world/sensing", source: declared, range: [120, 900], note: "A track nothing has updated for this long is dropped." },

  // ── Indirect fire (world/domains/fires) ──
  "world/domains/fires.RECALL_S": { value: 120, unit: "s", module: "world/domains/fires", source: declared, range: [60, 300], note: "A unit does not call for fire on the same enemy again within this long." },
  "world/domains/fires.GIVE_UP_S": { value: 300, unit: "s", module: "world/domains/fires", source: declared, range: [120, 600], note: "A call for fire no shooter can take for this long is dropped." },
  "world/domains/fires.LAY_MORTAR_S": { value: 45, unit: "s", module: "world/domains/fires", source: ref("P24 (default)"), range: [30, 120], note: "A mortar section lays on a new target in about this long; call-for-fire to rounds is 3–15 min with the chain." },
  "world/domains/fires.LAY_GUN_S": { value: 90, unit: "s", module: "world/domains/fires", source: ref("P24 (default)"), range: [60, 240], note: "A howitzer or rocket section lays on a new target in about this long." },
  "world/domains/fires.SALVOS": { value: 2, module: "world/domains/fires", source: declared, range: [1, 4], note: "Salvos per tube in a standard mission, when the request does not say." },
  "world/domains/fires.CAREFUL_CEP": { value: 0.6, unit: "×", module: "world/domains/fires", source: declared, range: [0.4, 0.8], note: "A carefully laid mission spreads this much less (it takes twice as long to lay)." },
  "world/domains/fires.OFF_BOARD_M": { value: 15000, unit: "m", module: "world/domains/fires", source: declared, range: [8000, 25000], note: "Abstracted fires come from this far behind the side's start line." },
  "world/domains/fires.CB_ERROR_M": { value: 50, unit: "m", module: "world/domains/fires", source: declared, range: [20, 150], note: "A counter-battery radar's back-plotted gun position is off by about this." },
  "world/domains/fires.NEAR_MISS_M": { value: 300, unit: "m", module: "world/domains/fires", source: declared, range: [150, 500], note: "Rounds landing this close to a gun tell its crew it has been found." },
  "world/domains/fires.DISPLACE_M": { value: 500, unit: "m", module: "world/domains/fires", source: ref("P23 (default)"), range: [300, 1500], note: "How far a gun moves when it displaces; the survival window against counter-battery fire is 2–3 minutes." },

  // ── Land combined arms (world/domains/landExt) ──
  "world/domains/landExt.AMMO_LOW": { value: 0.25, unit: "×", module: "world/domains/landExt", source: declared, range: [0.15, 0.4], note: "Below this share of its basic load, a unit's leader is asked about stocks." },
  "world/domains/landExt.FUEL_LOW": { value: 0.2, unit: "×", module: "world/domains/landExt", source: declared, range: [0.1, 0.3], note: "Below this share of its fuel, a unit's leader is asked about stocks." },
  "world/domains/landExt.LANE_SLACK_M": { value: 10, unit: "m", module: "world/domains/landExt", source: declared, range: [0, 20], note: "A vehicle this far either side of a cleared lane's edge still counts as on it." },
  "world/domains/landExt.LANE_WIDTH_M": { value: 6, unit: "m", module: "world/domains/landExt", source: ref("R4.5, P13"), range: [4, 8], note: "Width of a lane cleared through a minefield." },
  "world/domains/landExt.BREACH_S": { value: 5400, unit: "s", module: "world/domains/landExt", source: ref("R4.5, P13"), range: [2700, 10800], note: "Time to clear one lane through a minefield of standard density (about 1.5 h)." },
  "world/domains/landExt.NON_ENGINEER_SLOW": { value: 1.6, unit: "×", module: "world/domains/landExt", source: declared, range: [1.2, 3], note: "A tank with a plough or a unit without engineers breaches this much slower." },
  "world/domains/landExt.UNDER_FIRE_SLOW": { value: 0.5, unit: "×", module: "world/domains/landExt", source: declared, range: [0.2, 1], note: "Seconds added to a breach for every second the engineers are suppressed." },
  "world/domains/landExt.SPOT_MINES_PER_S": { value: 0.05, unit: "/s", module: "world/domains/landExt", source: declared, range: [0.01, 0.2], note: "Chance per second that a unit close enough, with sight of it, notices a minefield before striking it." },
  "world/domains/landExt.STRIKE_PER_100M": { value: 0.35, unit: "/100 m", module: "world/domains/landExt", source: declared, range: [0.15, 0.6], note: "Chance a vehicle platoon strikes a mine for every 100 m it drives into a standard-density field (0.5 mines per metre of front)." },
  "world/domains/landExt.MINE_KILL": { value: 0.5, unit: "×", module: "world/domains/landExt", source: declared, range: [0.3, 0.8], note: "A mine strike knocks the vehicle out this share of the time; otherwise it is a mobility kill waiting for recovery." },
  "world/domains/landExt.LOOK_AHEAD_M": { value: 150, unit: "m", module: "world/domains/landExt", source: declared, range: [50, 300], note: "A moving unit stops this far short of an obstacle it knows of." },
  "world/domains/landExt.DISMOUNT_AT_M": { value: 2000, unit: "m", module: "world/domains/landExt", source: declared, range: [800, 3000], note: "An IFV is asked whether to dismount once an enemy it knows is this close, or as soon as it is fired on." },
  "world/domains/landExt.DISMOUNT_ASK_S": { value: 120, unit: "s", module: "world/domains/landExt", source: declared, range: [60, 300], note: "An IFV that stayed mounted is not asked again for this long." },
  "world/domains/landExt.DIG_IN_S": { value: 7200, unit: "s", module: "world/domains/landExt", source: declared, range: [3600, 14400], note: "Time to prepare a position fully." },
  "world/domains/landExt.SETTLE_TO_DIG_S": { value: 600, unit: "s", module: "world/domains/landExt", source: declared, range: [300, 1800], note: "A unit still for this long is asked whether to dig in." },

  // ── Support services (world/domains/support) ──
  "world/domains/support.ISR_RADIUS_M": { value: 4000, unit: "m", module: "world/domains/support", source: declared, range: [2000, 8000], note: "An abstracted ISR overflight reports enemy within this distance of the side's own units." },

  // ── Networks (world/net) ──
  "world/net.VHF_RANGE_M": { value: 8000, unit: "m", module: "world/net", source: ref("R2, P4"), range: [5000, 10000], note: "Combat net radio on high power, between two vehicles with line of sight." },
  "world/net.VHF_POST_RANGE_M": { value: 25000, unit: "m", module: "world/net", source: ref("R2, P4"), range: [10000, 40000], note: "Combat net radio with an amplifier and a mast at a command post." },
  "world/net.VHF_NO_LOS": { value: 0.4, unit: "×", module: "world/net", source: declared, range: [0.2, 0.6], note: "Without line of sight between the two antennas, VHF reaches this share of its range." },
  "world/net.VHF_HOP_RESIST": { value: 0.5, unit: "×", module: "world/net", source: declared, range: [0.2, 0.8], note: "How much frequency hopping resists jamming (0 none, 1 immune)." },
  "world/net.HF_RANGE_M": { value: 450000, unit: "m", module: "world/net", source: ref("R2, P5"), range: [300000, 600000], note: "HF NVIS reach." },
  "world/net.HF_TRANSMIT_S": { value: 30, unit: "s", module: "world/net", source: declared, range: [15, 60], note: "Sending one message on HF, slow and repeated." },
  "world/net.VOICE_COMPOSE_S": { value: 15, unit: "s", module: "world/net", source: declared, range: [5, 30], note: "Composing a voice message." },
  "world/net.VOICE_TRANSMIT_S": { value: 10, unit: "s", module: "world/net", source: declared, range: [5, 20], note: "Sending a voice message on combat net radio." },
  "world/net.DATALINK_RANGE_M": { value: 300000, unit: "m", module: "world/net", source: declared, range: [150000, 500000], note: "A Link-16-like data link's line-of-sight reach between high terminals." },
  "world/net.DATALINK_S": { value: 1, unit: "s", module: "world/net", source: ref("R2, P6"), range: [0.5, 12], note: "Data link delivery: slots of 7.8 ms, a frame a few seconds at worst." },
  "world/net.DATALINK_RESIST": { value: 0.8, unit: "×", module: "world/net", source: declared, range: [0.6, 0.95], note: "Spread-spectrum data link resistance to jamming." },
  "world/net.SATCOM_MIL_S": { value: 5, unit: "s", module: "world/net", source: declared, range: [2, 15], note: "GEO military SATCOM: compose, queue and the 0.5 s round trip." },
  "world/net.SATCOM_LEO_S": { value: 2, unit: "s", module: "world/net", source: ref("R2, P7"), range: [1, 5], note: "Commercial LEO SATCOM: 25–100 ms latency, plus handling." },
  "world/net.MESH_HOP_M": { value: 2000, unit: "m", module: "world/net", source: declared, range: [1000, 4000], note: "A mesh radio reaches the next friend this far." },
  "world/net.MESH_HOP_S": { value: 2, unit: "s", module: "world/net", source: declared, range: [1, 5], note: "Delay per mesh hop." },
  "world/net.COURIER_MS": { value: 10, unit: "m/s", module: "world/net", source: declared, range: [5, 20], note: "A courier vehicle's average speed." },
  "world/net.COURIER_SETUP_S": { value: 300, unit: "s", module: "world/net", source: declared, range: [120, 900], note: "Getting a courier on the road." },
  "world/net.RETRY_S": { value: 30, unit: "s", module: "world/net", source: declared, range: [10, 120], note: "A message lost to jamming is tried on the next bearer after this long." },
  "world/net.KEEP_S": { value: 900, unit: "s", module: "world/net", source: declared, range: [900, 900], note: "Delivered messages are kept this long for the AAR, then dropped." },

  // ── HQ pictures (world/command/fusion) ──
  "world/command/fusion.SPOTREP_S": { value: 60, unit: "s", module: "world/command/fusion", source: declared, range: [30, 300], note: "Each HQ passes what is new in its picture up to the HQ above this often." },
  "world/command/fusion.QUALITY_DECAY_S": { value: 30, unit: "s", module: "world/command/fusion", source: declared, range: [15, 120], note: "A track in an HQ's picture loses one point of quality (0–15) for every this many seconds since it was seen." },

  // ── Command and control (world/domains/c2) ──
  "world/domains/c2.RHYTHM_COMPANY_S": { value: 1200, unit: "s", module: "world/domains/c2", source: declared, range: [900, 1800], note: "A company or tactical planner re-plans this often (15–30 sim-minutes, §8)." },
  "world/domains/c2.RHYTHM_BATTALION_S": { value: 1800, unit: "s", module: "world/domains/c2", source: declared, range: [1200, 3600], note: "A battalion HQ re-plans this often (default 30 min, W4)." },
  "world/domains/c2.RHYTHM_BRIGADE_S": { value: 3600, unit: "s", module: "world/domains/c2", source: declared, range: [1800, 7200], note: "A brigade (formation) HQ re-plans this often (30–60 min, §8)." },
  "world/domains/c2.RHYTHM_HIGH_S": { value: 14400, unit: "s", module: "world/domains/c2", source: declared, range: [7200, 43200], note: "A division, component or joint HQ re-plans this often (2–12 h, §8)." },
  "world/domains/c2.PLAN_COMPANY_S": { value: 300, unit: "s", module: "world/domains/c2", source: ref("R1, P1"), range: [120, 900], note: "A hasty company plan; the HQ uses at most a third of the time available (P1)." },
  "world/domains/c2.PLAN_BATTALION_S": { value: 900, unit: "s", module: "world/domains/c2", source: ref("R1, P1"), range: [600, 2400], note: "A hasty battalion plan." },
  "world/domains/c2.PLAN_BRIGADE_S": { value: 2400, unit: "s", module: "world/domains/c2", source: ref("R1, P1"), range: [1200, 7200], note: "A hasty brigade plan (the vignette's 40 sim-minutes)." },
  "world/domains/c2.PLAN_HIGH_S": { value: 5400, unit: "s", module: "world/domains/c2", source: ref("P25 (default)"), range: [3600, 21600], note: "A division, component or joint plan, hasty." },
  "world/domains/c2.CCIR_MIN_GAP_S": { value: 600, unit: "s", module: "world/domains/c2", source: declared, range: [300, 1800], note: "An HQ does not re-plan on a CCIR trigger within this long of its last plan." },
  "world/domains/c2.CCIR_NEW_CONTACTS": { value: 3, module: "world/domains/c2", source: declared, range: [2, 6], note: "New enemy contacts in the COP since the last plan that count as a major enemy force found." },
  "world/domains/c2.HEARTBEAT_S": { value: 300, unit: "s", module: "world/domains/c2", source: declared, range: [120, 900], note: "An HQ restates its intent to its subordinates this often; each answers with a situation report." },
  "world/domains/c2.OUT_OF_COMMS_S": { value: 900, unit: "s", module: "world/domains/c2", source: declared, range: [300, 1800], note: "A unit that has heard nothing from its HQ for this long is out of comms (the outOfComms decision)." },
  "world/domains/c2.EMIT_LIMIT_S": { value: 1200, unit: "s", module: "world/domains/c2", source: declared, range: [600, 3600], note: "A command post on the air this long since it last moved is asked whether to displace." },
  "world/domains/c2.FIRES_RANK_S": { value: 120, unit: "s", module: "world/domains/c2", source: declared, range: [60, 300], note: "The fires cell ranks competing calls for fire no more often than this." },
  "world/domains/c2.SEAT_MOVE_S": { value: 900, unit: "s", module: "world/domains/c2", source: declared, range: [300, 3600], note: "After its post is destroyed, an HQ's seat takes this long to move to the alternate." },
  "world/domains/c2.DISPLACE_M": { value: 1000, unit: "m", module: "world/domains/c2", source: declared, range: [500, 3000], note: "How far a command post moves when it displaces." },
  "world/domains/c2.FIRES_CELL_S": { value: 30, unit: "s", module: "world/domains/c2", source: declared, range: [10, 120], note: "The fires cell's own handling of a call for fire, between receiving it and tasking a shooter." },
  "world/domains/c2.COP_S": { value: 10, unit: "s", module: "world/domains/c2", source: declared, range: [10, 10], note: "HQ pictures are fused this often." },
  "world/domains/c2.INBOX_KEEP": { value: 200, module: "world/domains/c2", source: declared, range: [200, 200], note: "Messages kept in an HQ's inbox." },
} as const satisfies Record<string, WorldFigure>;

export type WorldFigureKey = keyof typeof WORLD_FIGURES;

/** Overrides for this game (a scenario's figure overrides, or a sensitivity run). */
let overrides: Partial<Record<string, number>> = {};

/** A world figure's value, or this game's override of it. */
export function wfig(key: WorldFigureKey): number {
  return overrides[key] ?? WORLD_FIGURES[key].value;
}

/**
 * Run `fn` with some figures overridden (a sensitivity sweep, a scenario's
 * overrides in a headless batch). Restored afterwards, even if it throws.
 */
export async function withFigures<T>(values: Partial<Record<string, number>>, fn: () => Promise<T> | T): Promise<T> {
  const before = overrides;
  overrides = { ...before, ...values };
  try {
    return await fn();
  } finally {
    overrides = before;
  }
}

/** Every figure the real-time game runs on, engine and world, for the table and the research backlog. */
export function allFigures(): Record<string, Figure & { range?: [number, number] }> {
  return { ...FIGURES, ...WORLD_FIGURES };
}
