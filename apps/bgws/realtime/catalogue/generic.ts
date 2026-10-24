// ── realtime/catalogue/generic.ts ────────────────────────────────────────
// The generic archetypes (W14): one record per KIND of platform, not per real
// system. Real systems from the curated tables (the 819 armour rows today)
// inherit from their generic archetype and override what the curated data
// holds; until then these are what the engine fields.
//
// How each guess is made, in order: a [ref] figure from
// docs/research/MODERN_WARFARE_REFERENCE.md where there is one (named in
// `sources`), else physics and proportion (a heavier vehicle burns more
// fuel), else a round number that plays sensibly. Each record says so in its
// rationale.

import type {
  BearerId,
  BodySpec,
  CarriedSpec,
  CatalogueRecord,
  EmitterSpec,
  FieldSource,
  MoverSpec,
  PlatformDomain,
  SensorSpec,
  SignatureSpec,
  StoresSpec,
  WeaponSpec,
} from "./schema";
import { WEAPONS } from "./weapons";
import type { Planner } from "../scope";

/** A friendly SIDC; the hostile one swaps the affiliation letter. */
function sidc(friendly: string): { blue: string; red: string } {
  const code = friendly.padEnd(15, "-");
  return { blue: code, red: `${code[0]}H${code.slice(2)}` };
}

const eye = (rangeM: number, heightM = 2): SensorSpec => ({
  kind: "eye", rangeM, arcDeg: 360, scanS: 2, classifies: "type", seesAir: true, active: false, heightM,
});
const thermal = (rangeM: number, heightM = 2.5): SensorSpec => ({
  kind: "thermal", rangeM, arcDeg: 120, scanS: 1, classifies: "type", seesAir: true, active: false, heightM,
});
const radar = (rangeM: number, scanS: number, heightM: number, seesAir = true, arcDeg = 360): SensorSpec => ({
  kind: "radar", rangeM, arcDeg, scanS, classifies: "class", seesAir, active: true, heightM,
});
const esm = (rangeM: number, heightM = 5): SensorSpec => ({
  kind: "esm", rangeM, arcDeg: 360, scanS: 5, classifies: "class", seesAir: true, active: false, heightM,
});
const radio = (powerW = 50): EmitterSpec => ({ kind: "radio", band: "vhf", powerW, dutyCycle: 0.1 });
const radarEmitter = (band: EmitterSpec["band"], powerW: number): EmitterSpec => ({ kind: "radar", band, powerW, dutyCycle: 1 });

const NO_STORES: StoresSpec = { fuelL: 0, burnLph: { moving: 0, idle: 0 }, ammo: {}, batteryWh: 0, medical: 0, spares: 0 };

interface Draft {
  id: string;
  name: string;
  domain: PlatformDomain;
  role: string;
  sidc: string;
  template: string;
  plannedBy?: Planner;
  mover: MoverSpec;
  body: Partial<BodySpec> & Pick<BodySpec, "count" | "vulnerability">;
  signatures?: Partial<SignatureSpec>;
  sensors?: SensorSpec[];
  weapons?: WeaponSpec[];
  emitters?: EmitterSpec[];
  bearers?: BearerId[];
  stores?: Partial<StoresSpec>;
  carried?: CarriedSpec;
  crewQuality?: number;
  costUsd: number;
  rationale: string;
  sources?: Record<string, FieldSource>;
}

function rec(d: Draft): CatalogueRecord {
  return {
    id: d.id,
    name: d.name,
    domain: d.domain,
    role: d.role,
    sidc: sidc(d.sidc),
    template: d.template,
    ...(d.plannedBy ? { plannedBy: d.plannedBy } : {}),
    mover: d.mover,
    body: { armourMm: 0, stayingPower: 1, crew: 3, ...d.body },
    signatures: { visual: 1, ir: 1, rcsM2: 10, acousticDb: 0, ...d.signatures },
    sensors: d.sensors ?? [eye(3000)],
    weapons: d.weapons ?? [],
    emitters: d.emitters ?? [radio()],
    bearers: d.bearers ?? ["vhf"],
    stores: { ...NO_STORES, ...d.stores, burnLph: { ...NO_STORES.burnLph, ...d.stores?.burnLph } },
    carried: d.carried ?? {},
    crewQuality: d.crewQuality ?? 4,
    costUsd: d.costUsd,
    rationale: d.rationale,
    provenance: "synthetic",
    ...(d.sources ? { sources: d.sources } : {}),
  };
}

const tracked = (speedKmh: number, cruiseKmh: number): MoverSpec => ({ kind: "tracked", speedKmh, cruiseKmh });
const wheeled = (speedKmh: number, cruiseKmh: number): MoverSpec => ({ kind: "wheeled", speedKmh, cruiseKmh });
const foot: MoverSpec = { kind: "foot", speedKmh: 6, cruiseKmh: 4 };
const fixed: MoverSpec = { kind: "static", speedKmh: 0, cruiseKmh: 0 };

// ── Land ────────────────────────────────────────────────────────────────

const LAND: CatalogueRecord[] = [
  rec({
    id: "generic/mbt", name: "Main battle tank troop", domain: "land", role: "Direct-fire manoeuvre: kill armour, take ground.",
    sidc: "SFGPUCA", template: "core/armour", mover: tracked(65, 25),
    body: { count: 4, vulnerability: "armoured", armourMm: 650, crew: 4 }, signatures: { visual: 1, ir: 1.2, rcsM2: 20 },
    sensors: [eye(3000, 2.5), thermal(4000)], weapons: [WEAPONS.tankGun],
    stores: { fuelL: 1900, burnLph: { moving: 227, idle: 38 }, ammo: { tank120: 160, smallArms: 4000 }, batteryWh: 2000, spares: 4 },
    costUsd: 9_000_000,
    rationale: "Fuel burn from the Abrams figures (about 60 gal/h across country, 10 gal/h idle); armour and gun as the land engine's tank.",
    sources: { "stores.burnLph": { kind: "reference", ref: "R7, P17" } },
  }),
  rec({
    id: "generic/ifv", name: "Infantry fighting vehicle platoon", domain: "land", role: "Carries and supports a rifle platoon; 30 mm cannon and ATGM.",
    sidc: "SFGPUCIZ", template: "core/ifv", mover: tracked(65, 25),
    body: { count: 3, vulnerability: "light", armourMm: 200, crew: 3 }, signatures: { visual: 0.9, ir: 1, rcsM2: 15 },
    sensors: [eye(3000, 2.5), thermal(3500)], weapons: [WEAPONS.atgm],
    stores: { fuelL: 860, burnLph: { moving: 110, idle: 20 }, ammo: { cannon30: 1200, atgm: 6, smallArms: 6000 }, batteryWh: 2000, spares: 3 },
    carried: { dismounts: { archetype: "generic/infantrySection", sections: 3 } },
    costUsd: 4_000_000, rationale: "Burn scaled from the tank by mass (about half); one rifle section per vehicle.",
  }),
  rec({
    id: "generic/infantrySection", name: "Rifle section", domain: "land", role: "Holds and clears ground; carries an anti-armour weapon.",
    sidc: "SFGPUCI", template: "core/infantry", mover: foot,
    body: { count: 1, vulnerability: "personnel", crew: 8 }, signatures: { visual: 0.4, ir: 0.5, rcsM2: 1 },
    sensors: [eye(2000, 1.7)], weapons: [], stores: { ammo: { smallArms: 2400, atgm: 2 }, batteryWh: 300, medical: 4 },
    costUsd: 200_000, rationale: "Dismounted movement 4 km/h on a road, slower across country.",
    sources: { "mover.speedKmh": { kind: "reference", ref: "P26 (default)" } },
  }),
  rec({
    id: "generic/atgmTeam", name: "Anti-tank guided missile team", domain: "land", role: "Ambushes armour from cover at long range.",
    sidc: "SFGPUCAA", template: "core/atgmTeam", mover: foot,
    body: { count: 2, vulnerability: "personnel", crew: 3 }, signatures: { visual: 0.3, ir: 0.4, rcsM2: 0.5 },
    sensors: [eye(2500, 1.7), thermal(4000, 1.5)], weapons: [WEAPONS.atgm], stores: { ammo: { atgm: 8 }, batteryWh: 400, medical: 2 },
    costUsd: 1_500_000, rationale: "Two firing posts; eight missiles between them.",
  }),
  rec({
    id: "generic/recce", name: "Reconnaissance patrol", domain: "land", role: "Finds the enemy and reports without being found.",
    sidc: "SFGPUCR", template: "core/recce", mover: wheeled(90, 40),
    body: { count: 2, vulnerability: "light", armourMm: 40, crew: 4 }, signatures: { visual: 0.5, ir: 0.6, rcsM2: 6 },
    sensors: [eye(4000, 3), thermal(6000, 3)], weapons: [], stores: { fuelL: 400, burnLph: { moving: 40, idle: 8 }, ammo: { smallArms: 2000 }, batteryWh: 1500 },
    costUsd: 2_000_000, rationale: "Long-range optics on a mast; light, quiet vehicles.",
  }),
  rec({
    id: "generic/engineerSection", name: "Combat engineer section", domain: "land", role: "Breaches and lays obstacles; digs in.",
    sidc: "SFGPUCE", template: "core/engineer", mover: tracked(60, 25),
    body: { count: 2, vulnerability: "light", armourMm: 150, crew: 8 }, signatures: { visual: 0.9, ir: 1, rcsM2: 15 },
    stores: { fuelL: 800, burnLph: { moving: 100, idle: 18 }, ammo: { smallArms: 3000 }, spares: 4 },
    costUsd: 3_000_000, rationale: "A lane 6 m wide in about 1.5 h, from the research on breaching.",
    sources: { "role": { kind: "reference", ref: "R4.5, P13" } },
  }),
  rec({
    id: "generic/mortar120", name: "120 mm mortar section", domain: "land", role: "Indirect fire on call for the company.",
    sidc: "SFGPUCFM", template: "core/mortar", mover: wheeled(80, 35),
    body: { count: 2, vulnerability: "light", armourMm: 20, crew: 5 }, signatures: { visual: 0.7, ir: 0.8, rcsM2: 8 },
    weapons: [WEAPONS.mortar120], stores: { fuelL: 300, burnLph: { moving: 30, idle: 6 }, ammo: { mortar120: 120 }, batteryWh: 800 },
    costUsd: 1_000_000, rationale: "Range 7 km; a basic load of 60 bombs a tube.",
  }),
  rec({
    id: "generic/sph155", name: "155 mm self-propelled battery (platoon)", domain: "land", role: "General-support artillery; shoots and scoots.",
    sidc: "SFGPUCFHE", template: "core/artillery", plannedBy: "land", mover: tracked(60, 30),
    body: { count: 3, vulnerability: "light", armourMm: 40, crew: 5 }, signatures: { visual: 1, ir: 1.3, rcsM2: 20 },
    weapons: [WEAPONS.howitzer155, WEAPONS.precision155],
    stores: { fuelL: 900, burnLph: { moving: 120, idle: 20 }, ammo: { artillery155: 120 }, batteryWh: 1500 },
    costUsd: 12_000_000, rationale: "Burst of four rounds a gun; precision rounds a small share of the load.",
  }),
  rec({
    id: "generic/mlrs", name: "Rocket launcher section", domain: "land", role: "Long-range precision rockets at high-payoff targets.",
    sidc: "SFGPUCFRMR", template: "core/artillery", plannedBy: "land", mover: wheeled(85, 50),
    body: { count: 2, vulnerability: "light", armourMm: 20, crew: 3 }, signatures: { visual: 1, ir: 1.2, rcsM2: 18 },
    weapons: [WEAPONS.rocketGmlrs], stores: { fuelL: 500, burnLph: { moving: 60, idle: 10 }, ammo: { rocket: 12 }, batteryWh: 1500 },
    costUsd: 5_000_000, rationale: "Six rockets a pod; GNSS-guided, so jamming matters.",
  }),
  rec({
    id: "generic/cbRadar", name: "Counter-battery radar", domain: "land", role: "Tracks incoming rounds back to the guns that fired them.",
    sidc: "SFGPUCFTR", template: "core/sensor", mover: wheeled(80, 40),
    body: { count: 1, vulnerability: "soft", crew: 4 }, signatures: { visual: 0.8, ir: 0.8, rcsM2: 10 },
    sensors: [{ kind: "cbr", rangeM: 40_000, arcDeg: 90, scanS: 1, classifies: "class", seesAir: true, active: true, heightM: 6 }],
    emitters: [radio(), radarEmitter("x", 5000)], stores: { fuelL: 200, burnLph: { moving: 20, idle: 6 } },
    costUsd: 10_000_000, rationale: "About 40 km detection, from the Zoopark-1M claim.",
    sources: { "sensors.0.rangeM": { kind: "reference", ref: "R4.3" } },
  }),
  rec({
    id: "generic/shorad", name: "Short-range air defence section", domain: "land", role: "Protects the force against drones, helicopters and low aircraft.",
    sidc: "SFGPUCDS", template: "core/gbad", plannedBy: "air", mover: tracked(60, 30),
    body: { count: 2, vulnerability: "light", armourMm: 30, crew: 3 }, signatures: { visual: 0.9, ir: 1, rcsM2: 12 },
    sensors: [radar(25_000, 2, 6), eye(3000)], weapons: [WEAPONS.samShort, WEAPONS.gunAd],
    emitters: [radio(), radarEmitter("x", 2000)], stores: { fuelL: 700, burnLph: { moving: 90, idle: 15 }, ammo: { samShort: 16, cannon30: 800 } },
    costUsd: 15_000_000, rationale: "Radar 25 km; cheap guns before missiles.",
  }),
  rec({
    id: "generic/samMedium", name: "Medium-range SAM battery", domain: "land", role: "Area air defence against aircraft and cruise missiles.",
    sidc: "SFGPUCDM", template: "core/gbad", plannedBy: "air", mover: wheeled(70, 35),
    body: { count: 2, vulnerability: "soft", crew: 4 }, signatures: { visual: 1, ir: 1, rcsM2: 15 },
    sensors: [radar(80_000, 5, 10)], weapons: [WEAPONS.samMedium], emitters: [radio(), radarEmitter("c", 20_000)],
    stores: { fuelL: 600, burnLph: { moving: 70, idle: 15 }, ammo: { samMedium: 16 } },
    costUsd: 80_000_000, rationale: "Radar 80 km against a fighter; launchers reloaded from stores.",
  }),
  rec({
    id: "generic/samLong", name: "Long-range SAM battery", domain: "land", role: "Theatre air and missile defence.",
    sidc: "SFGPUCDH", template: "core/gbad", plannedBy: "air", mover: wheeled(65, 30),
    body: { count: 2, vulnerability: "soft", crew: 5 }, signatures: { visual: 1.2, ir: 1.2, rcsM2: 25 },
    sensors: [radar(180_000, 10, 15)], weapons: [WEAPONS.samLong], emitters: [radio(), radarEmitter("c", 100_000)],
    stores: { fuelL: 800, burnLph: { moving: 90, idle: 20 }, ammo: { samLong: 16 } },
    costUsd: 1_000_000_000, rationale: "Interceptor cost from the PAC-3 figure.",
    sources: { "weapons.samLong.costUsd": { kind: "reference", ref: "R4.4, P11" } },
  }),
  rec({
    id: "generic/ewJammer", name: "Electronic attack team", domain: "land", role: "Jams radios, drone links and GNSS over an area.",
    sidc: "SFGPUUMSE", template: "core/ewJammer", mover: wheeled(80, 40),
    body: { count: 1, vulnerability: "soft", crew: 4 }, signatures: { visual: 0.8, ir: 0.8, rcsM2: 8 },
    sensors: [esm(30_000, 8)],
    emitters: [radio(), { kind: "jammer", band: "uhf", powerW: 1000, dutyCycle: 1, jams: ["vhf", "uhf", "l", "gnss"], jamRadiusM: 25_000 }],
    stores: { fuelL: 300, burnLph: { moving: 30, idle: 10 } }, costUsd: 5_000_000,
    rationale: "A Pole-21-like module denies GNSS out to about 25 km.",
    sources: { "emitters.1.jamRadiusM": { kind: "reference", ref: "R5.3, P16" } },
  }),
  rec({
    id: "generic/esmTeam", name: "Electronic support (SIGINT) team", domain: "land", role: "Finds emitters and fixes them by bearings.",
    sidc: "SFGPUUMSEI", template: "core/ewEsm", mover: wheeled(80, 40),
    body: { count: 1, vulnerability: "soft", crew: 3 }, signatures: { visual: 0.6, ir: 0.6, rcsM2: 5 },
    sensors: [esm(40_000, 10)], stores: { fuelL: 250, burnLph: { moving: 25, idle: 6 } }, costUsd: 3_000_000,
    rationale: "Direction finding to about 40 km on a mast.",
  }),
  rec({
    id: "generic/droneIsrTeam", name: "ISR quadcopter team", domain: "land", role: "Finds targets and adjusts fire from the air.",
    sidc: "SFGPUCVU", template: "core/droneTeam", mover: foot,
    body: { count: 1, vulnerability: "personnel", crew: 3 }, signatures: { visual: 0.3, ir: 0.4, rcsM2: 0.5 },
    emitters: [radio(), { kind: "droneLink", band: "l", powerW: 5, dutyCycle: 1 }], stores: { batteryWh: 3000 },
    carried: { drones: { archetype: "generic/quadIsr", count: 4 } }, costUsd: 60_000,
    rationale: "Operators in a dugout with four quadcopters and spare batteries.",
  }),
  rec({
    id: "generic/fpvTeamRf", name: "FPV strike team (radio)", domain: "land", role: "Strikes vehicles and people with cheap FPV drones.",
    sidc: "SFGPUCVUF", template: "core/droneTeam", mover: foot,
    body: { count: 1, vulnerability: "personnel", crew: 3 }, signatures: { visual: 0.3, ir: 0.4, rcsM2: 0.5 },
    emitters: [radio(), { kind: "droneLink", band: "l", powerW: 5, dutyCycle: 1 }], stores: { batteryWh: 3000, ammo: { drone: 20 } },
    carried: { drones: { archetype: "generic/fpvRf", count: 20 } }, costUsd: 40_000,
    rationale: "Hit rate 30–50% for skilled pilots; the radio link can be jammed and gives the operator away.",
    sources: { "weapons.fpv.pk": { kind: "reference", ref: "R5.1, P9" } },
  }),
  rec({
    id: "generic/fpvTeamFibre", name: "FPV strike team (fibre)", domain: "land", role: "FPV strikes that jamming cannot stop, within the spool's reach.",
    sidc: "SFGPUCVUF", template: "core/droneTeam", mover: foot,
    body: { count: 1, vulnerability: "personnel", crew: 3 }, signatures: { visual: 0.3, ir: 0.4, rcsM2: 0.5 },
    emitters: [radio()], stores: { batteryWh: 3000, ammo: { drone: 12 } },
    carried: { drones: { archetype: "generic/fpvFibre", count: 12 } }, costUsd: 60_000,
    rationale: "Fibre spools are commonly 20 km at most (41 km is the record).",
    sources: { "carried.drones": { kind: "reference", ref: "R4, P8" } },
  }),
  rec({
    id: "generic/loiterTeam", name: "Loitering munition team", domain: "land", role: "Hunts high-value targets well behind the line.",
    sidc: "SFGPUCVUL", template: "core/droneTeam", mover: wheeled(80, 40),
    body: { count: 1, vulnerability: "soft", crew: 3 }, signatures: { visual: 0.6, ir: 0.6, rcsM2: 4 },
    emitters: [radio(), { kind: "droneLink", band: "l", powerW: 20, dutyCycle: 1 }], stores: { fuelL: 200, burnLph: { moving: 20, idle: 4 }, ammo: { drone: 4 } },
    carried: { drones: { archetype: "generic/loiter", count: 4 } }, costUsd: 400_000,
    rationale: "Lancet-like: 40 km, an hour aloft, a shaped charge.",
  }),
  rec({
    id: "generic/interceptorTeam", name: "Interceptor drone team", domain: "land", role: "Hunts enemy drones with drones.",
    sidc: "SFGPUCDSI", template: "core/gbad", mover: foot,
    body: { count: 1, vulnerability: "personnel", crew: 3 }, signatures: { visual: 0.3, ir: 0.4, rcsM2: 0.5 },
    sensors: [radar(8000, 2, 3), eye(2000)], weapons: [WEAPONS.interceptorDrone], emitters: [radio(), { kind: "droneLink", band: "l", powerW: 5, dutyCycle: 1 }],
    stores: { ammo: { drone: 12 }, batteryWh: 2000 }, costUsd: 80_000,
    rationale: "Interceptor drones cost $1.4–16k each.",
    sources: { "weapons.interceptorDrone.costUsd": { kind: "reference", ref: "R4.4, P11" } },
  }),
  rec({
    id: "generic/ugv", name: "Ground robot", domain: "land", role: "Carries supplies and wounded the last few kilometres.",
    sidc: "SFGPEVUL", template: "core/convoy", mover: { kind: "tracked", speedKmh: 15, cruiseKmh: 8 },
    body: { count: 1, vulnerability: "light", crew: 0 }, signatures: { visual: 0.3, ir: 0.4, rcsM2: 1 },
    sensors: [eye(500, 1)], emitters: [{ kind: "droneLink", band: "l", powerW: 5, dutyCycle: 1 }], stores: { batteryWh: 20_000 },
    carried: { cargoT: 0.3 }, costUsd: 50_000,
    rationale: "Some brigades move 50–80% of kill-zone supply by drone and robot.",
    sources: { "role": { kind: "reference", ref: "R7, P19" } },
  }),
  rec({
    id: "generic/convoyTruck", name: "Supply convoy (4 trucks)", domain: "land", role: "Moves fuel and ammunition forward.",
    sidc: "SFGPEVUL", template: "core/convoy", plannedBy: "sustainment", mover: wheeled(80, 40),
    body: { count: 4, vulnerability: "soft", crew: 2 }, signatures: { visual: 1.1, ir: 1, rcsM2: 15 },
    stores: { fuelL: 1600, burnLph: { moving: 35, idle: 5 } }, carried: { cargoT: 40 }, costUsd: 600_000,
    rationale: "Ten tonnes a truck.",
  }),
  rec({
    id: "generic/medevac", name: "Ambulance section", domain: "land", role: "Evacuates casualties from the collection point.",
    sidc: "SFGPUSM", template: "core/convoy", plannedBy: "sustainment", mover: tracked(60, 30),
    body: { count: 2, vulnerability: "light", armourMm: 30, crew: 3 }, signatures: { visual: 0.9, ir: 0.9, rcsM2: 12 },
    stores: { fuelL: 600, burnLph: { moving: 60, idle: 10 }, medical: 20 }, carried: { cargoT: 1 }, costUsd: 3_000_000,
    rationale: "Four stretchers a vehicle.",
  }),
  rec({
    id: "generic/coastalAsm", name: "Coastal anti-ship battery", domain: "land", role: "Denies the sea from the shore.",
    sidc: "SFGPUCMS", template: "core/coastal", plannedBy: "maritime", mover: wheeled(70, 35),
    body: { count: 2, vulnerability: "soft", crew: 4 }, signatures: { visual: 1, ir: 1, rcsM2: 15 },
    sensors: [radar(150_000, 6, 20, false)], weapons: [WEAPONS.ascm], emitters: [radio(), radarEmitter("x", 20_000)],
    stores: { fuelL: 500, burnLph: { moving: 60, idle: 10 }, ammo: { antiShip: 8 } }, costUsd: 200_000_000,
    rationale: "The battery that sank Moskva fired two missiles.",
  }),
];

/** Command posts, one per echelon: every HQ is an entity that can be found, jammed and destroyed (§4.4.1). */
const COMMAND_POSTS: CatalogueRecord[] = (
  [
    ["company", "Company command post", 2, 10_000, 20],
    ["battalion", "Battalion command post", 3, 15_000, 40],
    ["brigade", "Brigade command post", 5, 30_000, 120],
    ["division", "Division main command post", 8, 50_000, 300],
  ] as const
).map(([echelon, name, vehicles, esmRange, crew]) =>
  rec({
    id: `generic/cp-${echelon}`, name, domain: "land", role: `Commands a ${echelon}: its staff cells plan, fuse reports and pass orders.`,
    sidc: "SFGPUH", template: "core/hq", plannedBy: "land", mover: wheeled(70, 30),
    body: { count: vehicles, vulnerability: "soft", crew }, signatures: { visual: 1 + vehicles * 0.1, ir: 1.2, rcsM2: 20 },
    sensors: [eye(2000)], emitters: [radio(100), { kind: "datalink", band: "uhf", powerW: 50, dutyCycle: 0.3 }, { kind: "satcom", band: "ku", powerW: 50, dutyCycle: 0.2 }],
    bearers: ["vhf", "hf", "datalink", "satcomMil", "satcomLeo", "courier"],
    stores: { fuelL: 300 * vehicles, burnLph: { moving: 40, idle: 12 }, batteryWh: 5000 * vehicles },
    costUsd: 2_000_000 * vehicles, rationale: `Size grows with echelon; its emissions are what ESM finds (${esmRange / 1000} km is a typical intercept range).`,
  }),
);

// ── Drones (the aircraft themselves) ────────────────────────────────────

const DRONES: CatalogueRecord[] = [
  rec({
    id: "generic/quadIsr", name: "ISR quadcopter", domain: "air", role: "Watches and reports; adjusts fire.",
    sidc: "SFAPMFQ", template: "core/drone", mover: { kind: "multirotor", speedKmh: 60, cruiseKmh: 40, ceilingM: 1000, enduranceH: 0.6 },
    body: { count: 1, vulnerability: "aircraft", crew: 0 }, signatures: { visual: 0.15, ir: 0.1, rcsM2: 0.01 },
    sensors: [{ kind: "eo", rangeM: 3000, arcDeg: 360, scanS: 2, classifies: "type", seesAir: false, active: false, heightM: 0 }],
    emitters: [{ kind: "droneLink", band: "l", powerW: 1, dutyCycle: 1 }], stores: { batteryWh: 100 }, costUsd: 3000,
    rationale: "Mavic-class: 40 minutes aloft.",
  }),
  rec({
    id: "generic/fpvRf", name: "FPV drone (radio)", domain: "air", role: "One-way strike, flown by an operator over a radio link.",
    sidc: "SFAPWMU", template: "core/drone", mover: { kind: "multirotor", speedKmh: 120, cruiseKmh: 100, ceilingM: 500, enduranceH: 0.2 },
    body: { count: 1, vulnerability: "aircraft", crew: 0 }, signatures: { visual: 0.1, ir: 0.1, rcsM2: 0.01 },
    sensors: [{ kind: "eo", rangeM: 1500, arcDeg: 90, scanS: 1, classifies: "class", seesAir: false, active: false, heightM: 0 }],
    weapons: [WEAPONS.fpv], emitters: [{ kind: "droneLink", band: "l", powerW: 1, dutyCycle: 1 }], stores: { batteryWh: 60 }, costUsd: 500,
    rationale: "About 15 km reach on a relay; 60–80% of strikes fail to destroy.",
    sources: { "weapons.fpv.pk": { kind: "reference", ref: "R5.1, P9" } },
  }),
  rec({
    id: "generic/fpvFibre", name: "FPV drone (fibre)", domain: "air", role: "One-way strike on a fibre-optic spool: no radio to jam.",
    sidc: "SFAPWMU", template: "core/drone", mover: { kind: "multirotor", speedKmh: 90, cruiseKmh: 70, ceilingM: 300, enduranceH: 0.3 },
    body: { count: 1, vulnerability: "aircraft", crew: 0 }, signatures: { visual: 0.12, ir: 0.1, rcsM2: 0.01 },
    sensors: [{ kind: "eo", rangeM: 1500, arcDeg: 90, scanS: 1, classifies: "class", seesAir: false, active: false, heightM: 0 }],
    weapons: [{ ...WEAPONS.fpv, id: "fpvFibre", maxRangeM: 20_000 }], emitters: [], stores: { batteryWh: 80 }, costUsd: 1500,
    rationale: "Spool length caps it at 20 km.",
    sources: { "weapons.fpvFibre.maxRangeM": { kind: "reference", ref: "R4, P8" } },
  }),
  rec({
    id: "generic/loiter", name: "Loitering munition", domain: "air", role: "Searches an area and dives on what it finds.",
    sidc: "SFAPWMU", template: "core/drone", mover: { kind: "fixedWing", speedKmh: 110, cruiseKmh: 90, ceilingM: 3000, enduranceH: 1 },
    body: { count: 1, vulnerability: "aircraft", crew: 0 }, signatures: { visual: 0.2, ir: 0.2, rcsM2: 0.05 },
    sensors: [{ kind: "eo", rangeM: 4000, arcDeg: 120, scanS: 1, classifies: "type", seesAir: false, active: false, heightM: 0 }],
    weapons: [WEAPONS.loitering], emitters: [{ kind: "droneLink", band: "l", powerW: 5, dutyCycle: 1 }], stores: { batteryWh: 500 }, costUsd: 35_000,
    rationale: "Lancet-like.",
  }),
  rec({
    id: "generic/owaDrone", name: "One-way attack drone", domain: "air", role: "Flies a planned route hundreds of kilometres to a fixed target.",
    sidc: "SFAPWMU", template: "core/drone", plannedBy: "air", mover: { kind: "cruise", speedKmh: 185, cruiseKmh: 180, ceilingM: 4000, enduranceH: 10 },
    body: { count: 1, vulnerability: "missile", crew: 0 }, signatures: { visual: 0.4, ir: 0.4, rcsM2: 0.1 },
    sensors: [], weapons: [WEAPONS.owa], emitters: [], stores: { fuelL: 30, burnLph: { moving: 3, idle: 0 } }, costUsd: 40_000,
    rationale: "Shahed-like: $30–50k; raids mix about 60% armed with 40% decoys, and about 85% are intercepted.",
    sources: { "costUsd": { kind: "reference", ref: "R4.4, P11" }, "role": { kind: "reference", ref: "R5.2, P10" } },
  }),
  rec({
    id: "generic/owaDecoy", name: "Decoy drone", domain: "air", role: "Looks like a one-way attack drone; draws interceptors.",
    sidc: "SFAPWMU", template: "core/drone", mover: { kind: "cruise", speedKmh: 185, cruiseKmh: 180, ceilingM: 4000, enduranceH: 10 },
    body: { count: 1, vulnerability: "missile", crew: 0 }, signatures: { visual: 0.4, ir: 0.4, rcsM2: 0.1 },
    sensors: [], weapons: [], emitters: [], stores: { fuelL: 30, burnLph: { moving: 3, idle: 0 } }, costUsd: 10_000,
    rationale: "Gerbera-like: cheap foam, no warhead.",
  }),
  rec({
    id: "generic/male", name: "MALE ISR/strike drone", domain: "air", role: "Long-endurance surveillance and precision strike.",
    sidc: "SFAPMFQ", template: "core/aircraft", plannedBy: "air", mover: { kind: "fixedWing", speedKmh: 300, cruiseKmh: 200, ceilingM: 8000, enduranceH: 24 },
    body: { count: 1, vulnerability: "aircraft", crew: 0 }, signatures: { visual: 0.5, ir: 0.6, rcsM2: 1 },
    sensors: [{ kind: "eo", rangeM: 15_000, arcDeg: 360, scanS: 3, classifies: "type", seesAir: false, active: false, heightM: 0 }],
    weapons: [WEAPONS.agm], emitters: [{ kind: "satcom", band: "ku", powerW: 50, dutyCycle: 1 }], bearers: ["satcomMil"],
    stores: { fuelL: 300, burnLph: { moving: 12, idle: 0 }, ammo: { agm: 4 } }, costUsd: 5_000_000, rationale: "TB2/Reaper-like; slow, and easy prey for air defence.",
  }),
];

// ── Air ─────────────────────────────────────────────────────────────────

const AIR: CatalogueRecord[] = [
  rec({
    id: "generic/fighter4", name: "4th-generation fighter flight", domain: "air", role: "Air superiority and strike.",
    sidc: "SFAPMFF", template: "core/aircraft", plannedBy: "air", mover: { kind: "fixedWing", speedKmh: 2100, cruiseKmh: 900, ceilingM: 15_000, enduranceH: 2.5 },
    body: { count: 2, vulnerability: "aircraft", crew: 1 }, signatures: { visual: 1, ir: 2, rcsM2: 5 },
    sensors: [radar(150_000, 2, 0, true, 120), { kind: "rwr", rangeM: 300_000, arcDeg: 360, scanS: 1, classifies: "class", seesAir: true, active: false, heightM: 0 }],
    weapons: [WEAPONS.aamBvr, WEAPONS.aamWvr, WEAPONS.glideBomb, WEAPONS.arm], emitters: [radarEmitter("x", 5000), { kind: "datalink", band: "l", powerW: 200, dutyCycle: 0.3 }],
    bearers: ["uhf", "datalink"], stores: { fuelL: 7000, burnLph: { moving: 3000, idle: 0 }, ammo: { aam: 6, bomb: 2, missile: 2 } },
    costUsd: 70_000_000, rationale: "Surge 2–3 sorties a day, 1–1.5 sustained.",
    sources: { "role": { kind: "reference", ref: "P30 (default)" } },
  }),
  rec({
    id: "generic/fighter5", name: "5th-generation fighter flight", domain: "air", role: "Stealthy air superiority, SEAD and strike.",
    sidc: "SFAPMFF", template: "core/aircraft", plannedBy: "air", mover: { kind: "fixedWing", speedKmh: 1900, cruiseKmh: 950, ceilingM: 15_000, enduranceH: 3 },
    body: { count: 2, vulnerability: "aircraft", crew: 1 }, signatures: { visual: 1, ir: 1.5, rcsM2: 0.005 },
    sensors: [radar(170_000, 2, 0, true, 120), { kind: "rwr", rangeM: 300_000, arcDeg: 360, scanS: 1, classifies: "type", seesAir: true, active: false, heightM: 0 }],
    weapons: [WEAPONS.aamBvr, WEAPONS.glideBomb, WEAPONS.arm], emitters: [radarEmitter("x", 3000), { kind: "datalink", band: "l", powerW: 100, dutyCycle: 0.2 }],
    bearers: ["uhf", "datalink"], stores: { fuelL: 8000, burnLph: { moving: 3200, idle: 0 }, ammo: { aam: 4, bomb: 2, missile: 2 } },
    costUsd: 100_000_000, rationale: "A radar cross-section a thousand times smaller than a 4th-generation fighter's.",
  }),
  rec({
    id: "generic/attackHelo", name: "Attack helicopter pair", domain: "air", role: "Stand-off anti-armour fire; vulnerable to MANPADS.",
    sidc: "SFAPMHA", template: "core/aircraft", plannedBy: "air", mover: { kind: "rotary", speedKmh: 290, cruiseKmh: 230, ceilingM: 4000, enduranceH: 2.5 },
    body: { count: 2, vulnerability: "aircraft", crew: 2 }, signatures: { visual: 1, ir: 1.5, rcsM2: 3 },
    sensors: [radar(8000, 2, 0), thermal(8000, 0)], weapons: [WEAPONS.agm], emitters: [radio(), radarEmitter("ku", 500)],
    bearers: ["uhf", "vhf"], stores: { fuelL: 1400, burnLph: { moving: 500, idle: 0 }, ammo: { agm: 16 } }, costUsd: 35_000_000,
    rationale: "Eight missiles each.",
  }),
  rec({
    id: "generic/transportHelo", name: "Transport helicopter pair", domain: "air", role: "Moves troops and casualties.",
    sidc: "SFAPMHU", template: "core/aircraft", plannedBy: "air", mover: { kind: "rotary", speedKmh: 280, cruiseKmh: 240, ceilingM: 4500, enduranceH: 2.5 },
    body: { count: 2, vulnerability: "aircraft", crew: 4 }, signatures: { visual: 1.3, ir: 1.6, rcsM2: 10 },
    sensors: [eye(3000)], bearers: ["uhf", "vhf"], stores: { fuelL: 1400, burnLph: { moving: 600, idle: 0 } },
    carried: { cargoT: 4, dismounts: { archetype: "generic/infantrySection", sections: 2 } }, costUsd: 20_000_000, rationale: "A section each.",
  }),
  rec({
    id: "generic/tanker", name: "Air-to-air tanker", domain: "air", role: "Refuels fighters on a track.",
    sidc: "SFAPMFKB", template: "core/aircraft", plannedBy: "air", mover: { kind: "fixedWing", speedKmh: 900, cruiseKmh: 800, ceilingM: 12_000, enduranceH: 10 },
    body: { count: 1, vulnerability: "aircraft", crew: 4 }, signatures: { visual: 2, ir: 2, rcsM2: 100 }, sensors: [],
    bearers: ["uhf", "datalink"], stores: { fuelL: 120_000, burnLph: { moving: 6000, idle: 0 } }, costUsd: 200_000_000, rationale: "Offloads most of its fuel.",
  }),
  rec({
    id: "generic/aew", name: "Airborne early warning aircraft", domain: "air", role: "Sees low and far; relays the air picture.",
    sidc: "SFAPMFRW", template: "core/aircraft", plannedBy: "air", mover: { kind: "fixedWing", speedKmh: 850, cruiseKmh: 650, ceilingM: 11_000, enduranceH: 9 },
    body: { count: 1, vulnerability: "aircraft", crew: 12 }, signatures: { visual: 2, ir: 2, rcsM2: 80 },
    sensors: [radar(400_000, 10, 0)], emitters: [radarEmitter("s", 1_000_000), { kind: "datalink", band: "l", powerW: 500, dutyCycle: 0.5 }],
    bearers: ["uhf", "datalink", "satcomMil"], stores: { fuelL: 60_000, burnLph: { moving: 5000, idle: 0 } }, costUsd: 500_000_000,
    rationale: "Radar horizon from 10 km up reaches 400 km.",
  }),
  rec({
    id: "generic/mpa", name: "Maritime patrol aircraft", domain: "air", role: "Hunts submarines and ships.",
    sidc: "SFAPMFP", template: "core/aircraft", plannedBy: "maritime", mover: { kind: "fixedWing", speedKmh: 900, cruiseKmh: 750, ceilingM: 12_000, enduranceH: 10 },
    body: { count: 1, vulnerability: "aircraft", crew: 9 }, signatures: { visual: 2, ir: 2, rcsM2: 60 },
    sensors: [radar(300_000, 6, 0, false), { kind: "sonar", rangeM: 20_000, arcDeg: 360, scanS: 30, classifies: "class", seesAir: false, active: false, heightM: 0 }],
    weapons: [WEAPONS.torpedo, WEAPONS.ascm], emitters: [radarEmitter("x", 50_000)], bearers: ["uhf", "datalink", "satcomMil"],
    stores: { fuelL: 30_000, burnLph: { moving: 2500, idle: 0 }, ammo: { torpedo: 5, antiShip: 4 } }, costUsd: 150_000_000,
    rationale: "Sonobuoys modelled as one passive sonar.",
  }),
];

// ── Sea ─────────────────────────────────────────────────────────────────

const ship = (speedKmh: number): MoverSpec => ({ kind: "surface", speedKmh, cruiseKmh: speedKmh * 0.55 });

const SEA: CatalogueRecord[] = [
  rec({
    id: "generic/destroyer", name: "Air-defence destroyer", domain: "surface", role: "Area air and missile defence for a task group.",
    sidc: "SFSPCLDD", template: "core/warship", plannedBy: "maritime", mover: ship(56),
    body: { count: 1, vulnerability: "ship", crew: 300, stayingPower: 4 }, signatures: { visual: 3, ir: 3, rcsM2: 5000, acousticDb: 160 },
    sensors: [radar(300_000, 3, 25), esm(400_000, 30), { kind: "sonar", rangeM: 20_000, arcDeg: 360, scanS: 10, classifies: "class", seesAir: false, active: true, heightM: 0 }],
    weapons: [WEAPONS.samLong, WEAPONS.samMedium, WEAPONS.ciws, WEAPONS.ascm, WEAPONS.navalGun],
    emitters: [radio(), radarEmitter("s", 4_000_000), { kind: "datalink", band: "l", powerW: 500, dutyCycle: 0.5 }],
    bearers: ["hf", "uhf", "datalink", "satcomMil"],
    stores: { fuelL: 1_500_000, burnLph: { moving: 15_000, idle: 2000 }, ammo: { samLong: 40, samMedium: 32, ciws: 1500, antiShip: 8 } },
    carried: { aircraft: { archetype: "generic/transportHelo", count: 1 } }, costUsd: 2_000_000_000,
    rationale: "Ninety-odd cells, reloaded only in port; defence layers at 100–240, about 50, 9 and 2 km.",
    sources: { "weapons": { kind: "reference", ref: "R6.2, P28 (default)" } },
  }),
  rec({
    id: "generic/frigate", name: "Frigate", domain: "surface", role: "Escort and anti-submarine warfare.",
    sidc: "SFSPCLFF", template: "core/warship", plannedBy: "maritime", mover: ship(52),
    body: { count: 1, vulnerability: "ship", crew: 180, stayingPower: 3 }, signatures: { visual: 2.5, ir: 2.5, rcsM2: 3000, acousticDb: 150 },
    sensors: [radar(200_000, 3, 22), esm(300_000, 25), { kind: "sonar", rangeM: 30_000, arcDeg: 360, scanS: 10, classifies: "class", seesAir: false, active: true, heightM: 0 }],
    weapons: [WEAPONS.samMedium, WEAPONS.ciws, WEAPONS.ascm, WEAPONS.torpedo],
    emitters: [radio(), radarEmitter("s", 1_000_000), { kind: "datalink", band: "l", powerW: 300, dutyCycle: 0.5 }],
    bearers: ["hf", "uhf", "datalink", "satcomMil"],
    stores: { fuelL: 800_000, burnLph: { moving: 9000, idle: 1500 }, ammo: { samMedium: 32, ciws: 1500, antiShip: 8, torpedo: 12 } },
    costUsd: 900_000_000, rationale: "A towed array makes it the better submarine hunter.",
  }),
  rec({
    id: "generic/corvette", name: "Missile corvette", domain: "surface", role: "Littoral strike and patrol.",
    sidc: "SFSPCLFF", template: "core/warship", plannedBy: "maritime", mover: ship(56),
    body: { count: 1, vulnerability: "ship", crew: 60, stayingPower: 1.5 }, signatures: { visual: 1.5, ir: 1.5, rcsM2: 800, acousticDb: 145 },
    sensors: [radar(80_000, 3, 15), esm(150_000, 15)], weapons: [WEAPONS.samShort, WEAPONS.ciws, WEAPONS.ascm],
    emitters: [radio(), radarEmitter("x", 100_000)], bearers: ["hf", "uhf", "datalink"],
    stores: { fuelL: 150_000, burnLph: { moving: 2500, idle: 400 }, ammo: { samShort: 16, ciws: 1000, antiShip: 8 } },
    costUsd: 200_000_000, rationale: "Small, fast, short on defence.",
  }),
  rec({
    id: "generic/ssk", name: "Diesel-electric submarine", domain: "subsurface", role: "Quiet ambush in coastal waters.",
    sidc: "SFUPSC", template: "core/submarine", plannedBy: "maritime", mover: { kind: "sub", speedKmh: 37, cruiseKmh: 9, ceilingM: 300 },
    body: { count: 1, vulnerability: "ship", crew: 50, stayingPower: 1 }, signatures: { visual: 0.1, ir: 0.1, rcsM2: 1, acousticDb: 110 },
    sensors: [{ kind: "sonar", rangeM: 40_000, arcDeg: 300, scanS: 10, classifies: "type", seesAir: false, active: false, heightM: 0 }],
    weapons: [WEAPONS.torpedo, WEAPONS.ascm], emitters: [], bearers: ["hf", "satcomMil"],
    stores: { fuelL: 100_000, burnLph: { moving: 200, idle: 50 }, ammo: { torpedo: 18, antiShip: 4 } }, costUsd: 500_000_000,
    rationale: "Quietest on batteries; noise rises with speed.",
  }),
  rec({
    id: "generic/ssn", name: "Nuclear attack submarine", domain: "subsurface", role: "Hunts ships and submarines in open ocean.",
    sidc: "SFUPSN", template: "core/submarine", plannedBy: "maritime", mover: { kind: "sub", speedKmh: 55, cruiseKmh: 20, ceilingM: 450 },
    body: { count: 1, vulnerability: "ship", crew: 130, stayingPower: 1.5 }, signatures: { visual: 0.1, ir: 0.1, rcsM2: 1, acousticDb: 115 },
    sensors: [{ kind: "sonar", rangeM: 60_000, arcDeg: 360, scanS: 10, classifies: "type", seesAir: false, active: false, heightM: 0 }],
    weapons: [WEAPONS.torpedo, WEAPONS.cruise], emitters: [], bearers: ["hf", "satcomMil"],
    stores: { ammo: { torpedo: 25, missile: 12 } }, costUsd: 3_000_000_000, rationale: "Unlimited endurance; a towed array.",
  }),
  rec({
    id: "generic/usv", name: "Explosive uncrewed surface vessel", domain: "surface", role: "One-way attack on ships and ports.",
    sidc: "SFSPCU", template: "core/drone", mover: ship(80),
    body: { count: 1, vulnerability: "ship", crew: 0, stayingPower: 0.3 }, signatures: { visual: 0.2, ir: 0.3, rcsM2: 2, acousticDb: 140 },
    sensors: [{ kind: "eo", rangeM: 5000, arcDeg: 180, scanS: 1, classifies: "class", seesAir: false, active: false, heightM: 1 }],
    weapons: [], emitters: [{ kind: "satcom", band: "ku", powerW: 20, dutyCycle: 1 }], bearers: ["satcomLeo"],
    stores: { fuelL: 600, burnLph: { moving: 60, idle: 5 } }, costUsd: 250_000, rationale: "Magura-like: 300 kg of explosive, steered over LEO SATCOM.",
  }),
  rec({
    id: "generic/merchant", name: "Merchant ship", domain: "surface", role: "Neutral traffic: what ROE protects.",
    sidc: "SNSPXM", template: "core/neutral", mover: ship(30),
    body: { count: 1, vulnerability: "ship", crew: 20, stayingPower: 2 }, signatures: { visual: 4, ir: 3, rcsM2: 10_000, acousticDb: 170 },
    sensors: [radar(40_000, 3, 30, false)], emitters: [{ kind: "radar", band: "x", powerW: 25_000, dutyCycle: 1 }], bearers: ["satcomLeo"],
    costUsd: 50_000_000, rationale: "A bulk carrier at 15 knots.",
  }),
];

// ── Space ───────────────────────────────────────────────────────────────

const orbit = (orbitAltKm: number, inclinationDeg: number): MoverSpec => ({ kind: "orbit", speedKmh: 27_000, cruiseKmh: 27_000, orbitAltKm, inclinationDeg });

const SPACE: CatalogueRecord[] = [
  rec({
    id: "generic/satEo", name: "LEO electro-optical imaging satellite", domain: "space", role: "Imagery of named areas, blocked by cloud.",
    sidc: "SFPPS", template: "core/satellite", plannedBy: "spaceCyber", mover: orbit(500, 97),
    body: { count: 1, vulnerability: "satellite", crew: 0 }, signatures: { visual: 0, ir: 0, rcsM2: 2 },
    sensors: [{ kind: "eo", rangeM: 300_000, arcDeg: 30, scanS: 0, classifies: "class", seesAir: false, active: false, heightM: 500_000 }],
    emitters: [], bearers: [], costUsd: 300_000_000, rationale: "Sun-synchronous at 500 km; a swath a few hundred km wide.",
  }),
  rec({
    id: "generic/satSar", name: "LEO radar imaging satellite", domain: "space", role: "Imagery through cloud and at night.",
    sidc: "SFPPS", template: "core/satellite", plannedBy: "spaceCyber", mover: orbit(550, 97),
    body: { count: 1, vulnerability: "satellite", crew: 0 }, signatures: { visual: 0, ir: 0, rcsM2: 2 },
    sensors: [{ kind: "sar", rangeM: 300_000, arcDeg: 30, scanS: 0, classifies: "class", seesAir: false, active: true, heightM: 550_000 }],
    emitters: [], bearers: [], costUsd: 200_000_000, rationale: "Sees through weather; coarser identity.",
  }),
  rec({
    id: "generic/satcomGeo", name: "GEO military SATCOM", domain: "space", role: "Beyond-line-of-sight bearer for HQs and drones.",
    sidc: "SFPPS", template: "core/satellite", plannedBy: "spaceCyber", mover: orbit(35_786, 0),
    body: { count: 1, vulnerability: "satellite", crew: 0 }, signatures: { visual: 0, ir: 0, rcsM2: 20 },
    sensors: [], emitters: [{ kind: "satcom", band: "ku", powerW: 2000, dutyCycle: 1 }], bearers: ["satcomMil"],
    costUsd: 1_000_000_000, rationale: "One footprint covers the theatre.",
  }),
  rec({
    id: "generic/gnss", name: "GNSS constellation", domain: "space", role: "Position, navigation and timing for the whole side.",
    sidc: "SFPPS", template: "core/satellite", plannedBy: "spaceCyber", mover: orbit(20_200, 55),
    body: { count: 24, vulnerability: "satellite", crew: 0 }, signatures: { visual: 0, ir: 0, rcsM2: 10 },
    sensors: [], emitters: [{ kind: "satcom", band: "gnss", powerW: 50, dutyCycle: 1 }], bearers: [], costUsd: 10_000_000_000,
    rationale: "Modelled as one service; jamming lowers it by area.",
  }),
  rec({
    id: "generic/missileWarning", name: "Missile-warning satellite", domain: "space", role: "Sees ballistic launches and cues defences.",
    sidc: "SFPPS", template: "core/satellite", plannedBy: "spaceCyber", mover: orbit(35_786, 0),
    body: { count: 1, vulnerability: "satellite", crew: 0 }, signatures: { visual: 0, ir: 0, rcsM2: 20 },
    sensors: [{ kind: "missileWarning", rangeM: 10_000_000, arcDeg: 360, scanS: 5, classifies: "class", seesAir: true, active: false, heightM: 35_786_000 }],
    emitters: [], bearers: ["satcomMil"], costUsd: 1_500_000_000, rationale: "Warning in the first minute of a boost.",
  }),
];

// ── Installations ───────────────────────────────────────────────────────

const NODES: CatalogueRecord[] = [
  rec({
    id: "generic/depot", name: "Supply depot", domain: "node", role: "Holds a formation's stocks, well back.",
    sidc: "SFGPIB", template: "core/node", plannedBy: "sustainment", mover: fixed,
    body: { count: 1, vulnerability: "structure", crew: 50, stayingPower: 6 }, signatures: { visual: 3, ir: 2, rcsM2: 1000 }, sensors: [eye(1000)],
    stores: { fuelL: 500_000, ammo: { tank120: 4000, cannon30: 30_000, atgm: 300, mortar120: 4000, artillery155: 8000, rocket: 120, samShort: 100, samMedium: 40, drone: 1000, smallArms: 500_000 }, medical: 500, spares: 300, batteryWh: 1_000_000 },
    costUsd: 20_000_000, rationale: "Placed about 120 km back where the threat dictates.",
    sources: { "role": { kind: "reference", ref: "R7, P18" } },
  }),
  rec({
    id: "generic/railhead", name: "Railhead", domain: "node", role: "Where the strategic supply line ends.",
    sidc: "SFGPIRR", template: "core/node", plannedBy: "sustainment", mover: fixed,
    body: { count: 1, vulnerability: "structure", crew: 40, stayingPower: 8 }, signatures: { visual: 4, ir: 2, rcsM2: 3000 }, sensors: [eye(1000)],
    stores: { fuelL: 2_000_000, ammo: { artillery155: 30_000, tank120: 10_000, smallArms: 2_000_000, drone: 5000 }, medical: 2000, spares: 1000 },
    costUsd: 50_000_000, rationale: "Trucks reach about 90–120 miles from it.",
    sources: { "role": { kind: "reference", ref: "R7, P18" } },
  }),
  rec({
    id: "generic/port", name: "Naval port", domain: "node", role: "Where warships reload their cells.",
    sidc: "SFGPIBN", template: "core/node", plannedBy: "maritime", mover: fixed,
    body: { count: 1, vulnerability: "structure", crew: 500, stayingPower: 15 }, signatures: { visual: 5, ir: 3, rcsM2: 10_000 }, sensors: [radar(60_000, 4, 30, true)],
    stores: { fuelL: 20_000_000, ammo: { samLong: 200, samMedium: 200, antiShip: 100, torpedo: 100, ciws: 20_000 } },
    costUsd: 2_000_000_000, rationale: "Sevastopol's lesson: a port within reach is a port lost.",
  }),
  rec({
    id: "generic/airbase", name: "Airbase", domain: "node", role: "Generates sorties; its runway can be cut.",
    sidc: "SFGPIBA", template: "core/node", plannedBy: "air", mover: fixed,
    body: { count: 1, vulnerability: "structure", crew: 2000, stayingPower: 12 }, signatures: { visual: 5, ir: 3, rcsM2: 10_000 }, sensors: [radar(200_000, 6, 30)],
    stores: { fuelL: 5_000_000, ammo: { aam: 200, bomb: 300, missile: 60, agm: 200 }, spares: 500 },
    carried: { aircraft: { archetype: "generic/fighter4", count: 12 } }, costUsd: 1_000_000_000,
    rationale: "Attacking an airbase cut sortie output by 35–40% in an illustrative study.",
  }),
  rec({
    id: "generic/cyberCell", name: "Cyber operations cell", domain: "node", role: "Holds prepared accesses to the enemy's networks.",
    sidc: "SFGPUUMSC", template: "core/cyber", plannedBy: "spaceCyber", mover: fixed,
    body: { count: 1, vulnerability: "structure", crew: 30, stayingPower: 2 }, signatures: { visual: 0.5, ir: 0.5, rcsM2: 100 }, sensors: [],
    emitters: [], bearers: ["fibre"], costUsd: 10_000_000, rationale: "An access takes 6–8 months to build; an effect lasts 2–3 days.",
    sources: { "role": { kind: "reference", ref: "R5.5, P21" } },
  }),
];

export const GENERIC_RECORDS: readonly CatalogueRecord[] = [...LAND, ...COMMAND_POSTS, ...DRONES, ...AIR, ...SEA, ...SPACE, ...NODES];
