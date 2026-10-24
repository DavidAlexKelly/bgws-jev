// ── realtime/catalogue/schema.ts ─────────────────────────────────────────
// The synthetic data catalogue (docs/plan/MULTI_DOMAIN_PLAN.md W14): the full
// record for every kind of platform the real-time engine can field, each
// component of §4.1 with every field it needs.
//
// BEST GUESSES, AND SAID TO BE. Almost every value here is synthetic: a
// plausible figure that makes the simulation work, set from the research
// where it has one, otherwise from physics and proportion, otherwise a round
// number that plays sensibly. A record says which, field by field
// (`sources`), with a one-line rationale. Realistic data replaces values
// record by record later, with no code change.
//
// Every unit is in the field name or the comment: metres, seconds, km/h,
// litres, US dollars. Signatures are multipliers on the visual hazard (1 is a
// tank in the open), so they compare across domains without pretending to be
// a physical unit.

import type { Planner } from "../scope";

export type Provenance = "synthetic" | "reference" | "sourced";

/** Where one field's value came from, when it is not the record's default. */
export interface FieldSource {
  kind: Provenance;
  /** "R4.5, P13" for the research reference; a dataset or document otherwise. */
  ref?: string;
}

/** Which medium a platform moves in. "node" is a fixed installation; "munition" flies once. */
export type PlatformDomain = "land" | "surface" | "subsurface" | "air" | "space" | "node" | "munition";

export type MoverKind =
  | "tracked"
  | "wheeled"
  | "foot"
  | "rotary"
  | "fixedWing"
  | "multirotor"
  | "surface"
  | "sub"
  | "orbit"
  | "static"
  | "ballistic"
  | "cruise"
  | "pursuit";

export interface MoverSpec {
  kind: MoverKind;
  /** Best speed, km/h (on road, in level flight, on the surface). */
  speedKmh: number;
  /** Usual working speed, km/h. */
  cruiseKmh: number;
  /** Air: how high it can fly, m. Sub: how deep it can dive (as a positive number), m. */
  ceilingM?: number;
  /** How long it can keep moving on one load of fuel or battery, hours. */
  enduranceH?: number;
  /** Orbit: altitude, km; period follows from it. */
  orbitAltKm?: number;
  /** Orbit: inclination, degrees. */
  inclinationDeg?: number;
}

/** How a platform is hurt: what a round must do to it. */
export type Vulnerability = "armoured" | "light" | "soft" | "personnel" | "aircraft" | "ship" | "structure" | "missile" | "satellite";

export interface BodySpec {
  /** Platforms in the element: 4 tanks, 2 aircraft, 1 ship. */
  count: number;
  vulnerability: Vulnerability;
  /** Frontal armour, mm (RHA equivalent). Zero for soft targets. */
  armourMm: number;
  /** Ships, structures: hits of a standard 200 kg warhead it takes before it is out of action. */
  stayingPower: number;
  /** People per platform. */
  crew: number;
}

/** Multipliers on the visual hazard per second (1 = a tank in the open). */
export interface SignatureSpec {
  visual: number;
  /** Thermal and infrared, the same scale. */
  ir: number;
  /** Radar cross-section, m² (front aspect). */
  rcsM2: number;
  /** Noise in water, dB re 1 µPa at 1 m, for sonar; 0 where it does not apply. */
  acousticDb: number;
}

export type SensorKind = "eye" | "optic" | "thermal" | "radar" | "esm" | "rwr" | "sonar" | "eo" | "sar" | "cbr" | "irst" | "missileWarning";

/** How far up the classification ladder a sensor can get: track → class → type → identity. */
export type Classification = "track" | "class" | "type" | "identity";

export interface SensorSpec {
  kind: SensorKind;
  /** Against a standard target (signature 1, RCS 5 m², 130 dB), m. */
  rangeM: number;
  /** Field of regard, degrees. 360 for an all-round radar. */
  arcDeg: number;
  /** How often it looks again, s. Eyes 1–2 s, radars 4–12 s, satellites only on access. */
  scanS: number;
  classifies: Classification;
  /** Can see aircraft and drones as well as the surface. */
  seesAir: boolean;
  /** Radar and sonar: transmits, so it is an emitter while on. */
  active: boolean;
  /** Antenna or mast height, m: the radar horizon follows from it. */
  heightM: number;
}

export type WeaponKind =
  | "directFire"
  | "atgm"
  | "mortar"
  | "howitzer"
  | "rocket"
  | "samLong"
  | "samMedium"
  | "samShort"
  | "gunAd"
  | "aam"
  | "agm"
  | "glideBomb"
  | "arm"
  | "ascm"
  | "cruise"
  | "ballistic"
  | "torpedo"
  | "ciws"
  | "fpv"
  | "loitering"
  | "owa"
  | "interceptorDrone"
  | "shotgun";

export type Guidance = "none" | "gnss" | "laser" | "ir" | "radar" | "activeRadar" | "antiRadiation" | "operator" | "acoustic" | "inertial";
export type FlightProfile = "none" | "ballistic" | "cruise" | "pursuit" | "seaSkim" | "glide" | "underwater";

/** What a weapon can be aimed at. */
export type TargetDomain = "land" | "air" | "surface" | "subsurface" | "munition";

export interface WeaponSpec {
  id: string;
  kind: WeaponKind;
  minRangeM: number;
  maxRangeM: number;
  /** Rounds per engagement (a salvo, a volley). */
  salvo: number;
  /** Rounds carried before it needs reloading from stores. */
  magazine: number;
  /** Time between salvoes, s. */
  reloadS: number;
  flight: FlightProfile;
  /** Average speed in flight, m/s. Zero for direct fire resolved in the tick. */
  speedMs: number;
  guidance: Guidance;
  /** Chance one round kills a target it reaches and hits, against its usual target, 0–1. */
  pk: number;
  /** Circular error probable, m: unguided by range, guided fixed. */
  cepM: number;
  /** Lethal radius of the warhead against people and soft targets, m. */
  lethalRadiusM: number;
  /** Penetration against armour, mm; 0 for blast only. */
  penetrationMm: number;
  /** Attacks the roof. */
  topAttack: boolean;
  targets: TargetDomain[];
  /** Ammunition family it draws on (stores.ammo). */
  family: AmmoFamily;
  costUsd: number;
}

export type AmmoFamily =
  | "smallArms"
  | "tank120"
  | "cannon30"
  | "atgm"
  | "mortar120"
  | "artillery155"
  | "rocket"
  | "samLong"
  | "samMedium"
  | "samShort"
  | "aam"
  | "agm"
  | "bomb"
  | "antiShip"
  | "torpedo"
  | "ciws"
  | "drone"
  | "missile";

export type EmitterKind = "radio" | "radar" | "datalink" | "jammer" | "droneLink" | "satcom";
export type Band = "hf" | "vhf" | "uhf" | "l" | "s" | "c" | "x" | "ku" | "gnss";

export interface EmitterSpec {
  kind: EmitterKind;
  band: Band;
  /** Effective radiated power, W. */
  powerW: number;
  /** Fraction of time it is on the air when on. */
  dutyCycle: number;
  /** Jammers: what it jams. */
  jams?: Band[];
  /** Jammers: radius it denies against a typical victim link, m. */
  jamRadiusM?: number;
  /** Jammers: the radius by band, where it differs (GNSS is denied much further than a radio link). */
  jamRadiiM?: Partial<Record<Band, number>>;
}

/** A way of passing messages (W4 bearers). */
export type BearerId = "vhf" | "hf" | "uhf" | "datalink" | "satcomMil" | "satcomLeo" | "mesh" | "fibre" | "courier";

export interface StoresSpec {
  /** Fuel carried, litres. Zero for foot, battery or static platforms. */
  fuelL: number;
  /** Fuel burnt, litres per hour per platform, moving across country and idling. */
  burnLph: { moving: number; idle: number };
  /** Basic load by ammunition family, rounds per element. */
  ammo: Partial<Record<AmmoFamily, number>>;
  /** Batteries for radios and drones, watt-hours. */
  batteryWh: number;
  /** Medical supplies, casualty treatments. */
  medical: number;
  /** Spares, repairs. */
  spares: number;
}

export interface CarriedSpec {
  /** Dismounted troops carried, as sections, and what they are. */
  dismounts?: { archetype: string; sections: number };
  /** Drones carried, and how many. */
  drones?: { archetype: string; count: number };
  /** Cargo, tonnes. */
  cargoT?: number;
  /** Aircraft embarked or based (ships, airbases). */
  aircraft?: { archetype: string; count: number };
}

export interface CatalogueRecord {
  /** "generic/mbt", "generic/fpvTeamRf". Real systems inherit with `parent`. */
  id: string;
  name: string;
  domain: PlatformDomain;
  /** One line on what it is for. */
  role: string;
  /** MIL-STD-2525 symbol, one per side (milsymbol). */
  sidc: { blue: string; red: string };
  /** The behaviour template it uses by default (realtime/behaviour/templates). */
  template: string;
  /** The planner that tasks it, when there is one (§4.4.2). */
  plannedBy?: Planner;
  mover: MoverSpec;
  body: BodySpec;
  signatures: SignatureSpec;
  sensors: SensorSpec[];
  weapons: WeaponSpec[];
  emitters: EmitterSpec[];
  bearers: BearerId[];
  stores: StoresSpec;
  carried: CarriedSpec;
  /** Default troop quality, 1–8. */
  crewQuality: number;
  /** Cost of one platform, US dollars: for the cost-exchange report. */
  costUsd: number;
  /** One line on how the guesses were made. */
  rationale: string;
  /** Default provenance of every field. */
  provenance: Provenance;
  /** Fields whose value came from somewhere better than the default, by path ("mover.speedKmh"). */
  sources?: Record<string, FieldSource>;
}
