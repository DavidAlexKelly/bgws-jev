# BGWS real-time: the multi-domain plan

**Status:** plan, October 2026. **Scope:** the real-time mode only. Turn mode stays exactly as it is, as every earlier plan has required.

**How to read this:**

| Section | What it covers |
|---|---|
| §1 | The end goal |
| §2 | The design rules that hold everything together |
| §3 | How modern war actually works, in the terms this simulation needs |
| §4 | The target architecture |
| §5 | Thirteen workstreams, one per capability, each with how it works, its decision points, its numbers, its build steps and its "done when" |
| §6 | The order to build them in, as milestones with a playable scenario at each |
| §7 | Running Jev and Claude at scale |
| §8 | Risks and open questions |

Companion documents:

- [DECISION_FLOW_GRAPHS.md](DECISION_FLOW_GRAPHS.md): the full specification of editable behaviour templates (workstream W1).
- [MODERN_WARFARE_REFERENCE.md](../research/MODERN_WARFARE_REFERENCE.md): the research this plan is built on, with sources. Numbers below marked **[ref]** come from it. Numbers marked **[declared]** are design defaults that have no source yet; they go in the declared-figures registry (W12) until a source replaces them.

---

## 1. The end goal

> A real-time, multi-domain combat simulation, as close to how modern war is actually fought as the data allows. Every unit, on land, at sea, in the air, in orbit and in the spectrum, acts on **its own picture of the battle**. It is linked to the others by **networks that can be jammed, saturated and destroyed**, and it is supplied by **logistics that can be cut**. When something happens to a unit, **Jev decides what it does**, from options its **decision flow graph** allows. Commanders, played by **Claude** or by a person, plan from their headquarters' picture, and their orders reach the units over those same networks.

What that looks like in play, at the end state. This is a single vignette, with every element built by a workstream below:

1. **Planning.** A brigade attack is planned at 0300 by the Blue brigade HQ (Claude). It writes the plan:
   - phases, with phase lines and boundaries
   - a fire support plan
   - a request for two air sorties and a reconnaissance drone tasking
   - a logistics annex
   - a PACE comms plan

   It takes 40 sim-minutes to plan, and the companies get 2/3 of the remaining time to prepare (**W4**).
2. **Reconnaissance.** Overnight, a Blue Group 2 drone finds a Red air-defence radar. The report travels over the battalion data link (**W4**) to the brigade fires cell. The cell's Jev-driven template chooses between three shooters (**W11**):
   - a rocket launcher, 4 min to effect
   - an FPV team, out of range
   - an air sortie, 25 min out

   The rocket fires.
3. **Strike under jamming.** Red's GNSS jammer degrades the guided rocket (**W6**). The battle damage assessment drone sees the radar still standing. The fires cell re-strikes with a fibre-optic FPV, which cannot be jammed but must be within 20 km (**W5**).
4. **The approach.** At 0545 the Blue companies cross the line of departure. Red's FPV operators get a link-quality hit from Blue's forward jammers, which moved up behind the lead companies as Ukraine did at Kursk **[ref]**. Two Red operator teams decide, each at its own decision point, to switch to fibre drones, and one decides to relocate because it has been transmitting for 6 minutes (**W5/W6**).
5. **Contact.** A Blue tank troop running the *stand fast* template (**W1**) comes under ATGM fire. Jev may choose among *return fire*, *hull-down* and *call fire*, but not *pull back*, because the template removed it.
6. **Sustainment.** By 1000 the lead battalion is at 30% of its basic load of 120 mm and 40% fuel. The logistics cell sends a LOGPAC. A Red loitering munition hunts the convoy 12 km behind the line; the driver's template allows *push on*, *scatter* and *take cover*. Ground robots finish the last 3 km (**W10**).
7. **The sea flank.** A Blue frigate on the coastal flank detects a raid of four anti-ship missiles. Its layered defence fires from a magazine that will not be reloaded until it reaches port (**W8**). The salvo arithmetic **emerges from the entities**; it is not rolled from a table.
8. **Space and cyber.** A Red satellite pass at 1112 images the Blue assembly area. The image reaches Red's corps HQ 45 minutes later **[declared]**, so its picture is that old (**W9**). A Red cyber operation, prepared before the game started, takes down Blue's cellular-based logistics app for 3 hours (**W9**).
9. **After-action review.** Every decision, with the graph path walked and Jev's probabilities, is in the event stream for the AAR (**W13**).

---

## 2. Design rules

These are carried forward from the existing real-time documents and extended. They decide every trade-off below.

1. **Claude plans, Jev decides, the rules execute.** Claude, or a human, issues mission orders from an HQ's picture. Jev picks one option at a decision point, only from options the rules and the unit's graph generate, so it can never invent one. The engine moves, senses, shoots, communicates and consumes, every tick, deterministically. Escalation bias in LLMs is well documented: all five models in Rivera et al. 2024 escalated **[ref]**. That is one more reason Claude never chooses individual engagements and ROE is a hard gate.
2. **Truth is not knowledge.** Every unit acts on its own beliefs. These come from its own sensors and from what it has been told over a network, with ages and errors. HQs act on a fused picture that is older and coarser than the truth. The existing `knowledge.ts` belief model generalises to every domain.
3. **Every number has a source or is declared.** A registry lists every parameter: value, unit, source, confidence and how sensitive outcomes are to it (W12). This is how the simulation stays honest as it grows.
4. **Emergent, not scripted.** A salvo exchange, a kill zone, a broken supply line or an air-defence collapse must come from entities acting under rules, not from a table that names the outcome. Tables are allowed only *inside* a mechanism, e.g. a Pk curve for one missile against one target type.
5. **Deterministic and replayable.** All randomness comes from the seeded `config.rng`. Every Jev and Claude answer is recorded, so a replay reproduces a game exactly, as `runner.ts` already guarantees.
6. **Resolution where it matters.** Entity-level for anything that manoeuvres, senses or fires independently: platoon-sized ground elements (as today), each aircraft, ship, drone and missile in flight. Aggregate or abstract for what does not need it: depots, rear-area traffic, distant formations, cyber.
7. **A unit's behaviour is data, not code.** The decision flow graph (W1) is the only place that says what a unit may consider. Code supplies the verbs and the physics.
8. **One clock, many tempos.** A submarine transit, an FPV dive and a satellite pass all live on the same 1-second clock. Fast things are resolved analytically within a tick; slow things are evaluated sparsely (§4.2).

---

## 3. How modern war works: the reference model

The [research reference](../research/MODERN_WARFARE_REFERENCE.md) has the detail and the sources. This section condenses it into the claims that shape the simulation.

### 3.1 Planning and command

- **Planning is a process with a clock.** The US Army MDMP has seven steps. The one-third/two-thirds rule says a commander uses at most a third of the available time and leaves two-thirds to subordinates; units routinely break it **[ref]**. *Simulation:* planning takes sim-time, and orders arrive late if planning runs long.
- **Mission command.** Orders give a task, a purpose and the commander's intent, and subordinates act on intent when the plan breaks or comms fail. *Simulation:* each unit's graph has an "out of comms" trigger and an initiative parameter, and Jev is told the intent, as today.
- **Orders have structure.** An order has five paragraphs: Situation, Mission, Execution, Sustainment, Command and signal. It carries control measures: phase lines, boundaries, objectives, fire support coordination measures, airspace control measures. It has decision points, with branches and sequels. *Simulation:* `UnitOrders` grows into an order with geometry and triggers (W4).
- **Echelons.** Section → platoon → company → battalion → brigade → division → corps, with joint component commands for land, maritime, air, space and cyber above. Each HQ has a staff and runs a battle rhythm.
- **Russian and Chinese contrasts** are useful for asymmetric sides:
  - Russian battalion tactical groups were short of infantry and gave commanders little authority, and were largely dropped by late 2022 **[ref]**. *Simulation:* a side-level "initiative" setting.
  - The PLA's "systems destruction warfare" aims to paralyse an enemy's networks rather than destroy its forces **[ref]**. *Simulation:* a side doctrine that prioritises C2 and network nodes.

### 3.2 Communications

- **Every bearer has a range, a capacity, a latency and a signature.** Figures from the reference:

  | Bearer | Range or capacity | Latency |
  |---|---|---|
  | SINCGARS VHF | 5–10 km at high power, 10–40 km with an amplifier | — |
  | HF NVIS | 35 to several hundred km | — |
  | Link 16 | 31.6–115.2 kbps on a 7.8125 ms TDMA slot structure | — |
  | Starlink | 25–220 Mbps nominal, about 10 Mbps per terminal on a crowded front | 25–100 ms |

- **Networks decide the speed of the kill chain:**
  - Ukraine's Delta: detect-to-strike went from about 20 minutes to 3–5 minutes, and is claimed at about 2 minutes against up to 72 hours early in the war **[ref]**.
  - Kropyva: about 1 minute for an unplanned target **[ref]**.
  - Project Convergence 2020: about 20 minutes cut to under 20 seconds in a demonstration, with officials warning that contested networks would not achieve this **[ref]**.
- **Comms are contested.** Russian jamming disrupted Russia's own communications in 2022 **[ref]**. Starlink was jammed, and in February 2026 it was restricted to registered terminals, which cut Russian links for about two weeks **[ref]**. *Simulation:* jamming is spatial and indiscriminate, and an outside party's decision can switch a bearer off.

### 3.3 The kill chain

- **Find, fix, track, target, engage, assess (F2T2EA).** Each step is done by a different entity, often in a different domain, linked by a network. The decide-detect-deliver-assess cycle (D3A) and high-payoff target lists govern priorities. *Simulation:* the kill chain is not a mechanic but a path through entities and networks. The simulation measures its time; it does not set it (W11).

### 3.4 Land

- **The transparent battlefield.** The drone kill zone is about 5–15 km deep, with claims of 20 km and more **[ref]**. About 60–70% of Russian equipment losses are attributed to drones (RUSI) **[ref]**.
- **Infantry.** Small groups of five or fewer now infiltrate, timed for thermal crossover and bad weather, because mass assaults die to drones **[ref]**.
- **Breaching.** Russian minefields deepened from about 120 m to about 500 m. Line charges cleared lanes only about 6 m wide. A breach took about 1.5 h, and deliberate attacks advanced about 700–1,200 m every five days in 2023 **[ref]**.
- **Fires.** Russia fired about 27,000 rounds a day in 2025 by Ukrainian figures, at a fire ratio of about 1.8:1 in July 2025 **[ref]**. Counter-battery radars detect to about 40 km (Zoopark-1M claim) **[ref]**. Lancets kept guns from moving forward **[ref]**.
- **Armour.** Most Abrams losses in Ukraine were to drones. Trophy has gaps against dives steeper than about 55°. Tanks are used as indirect-fire guns at 5–8 km **[ref]**.
- **Attrition.** Attacking battalions almost always halted near 20% casualties, and about 40% of those broke (Dupuy). Fixed-percentage breakpoints are a gross oversimplification **[ref]**. The engine's break tests already sit close to this (20% for attackers, 40% for defenders).

### 3.5 Drones and counter-drone

- **Volume and cost:**
  - FPV production was about 200,000 a month at the end of 2024, and about 3 million for 2025 **[ref]**.
  - Shahed/Geran: about 5,300–5,600 a month by late 2025, about 60% real strike drones and the rest decoys, about 85% intercepted in February 2026, at roughly $30–50k each **[ref]**.
  - Interceptor drones, at $1.5–16k, made about 70% of Shahed kills by January 2026 **[ref]**.
- **Effectiveness:**
  - 60–80% of FPV strikes fail to destroy the target (OSW, Oct 2025).
  - Hit rates are 30–50% for skilled pilots and about 10% for novices **[ref]**.
- **Jamming and links.** Fibre-optic FPVs cannot be jammed. Most fly 20 km or less, the longest NATO knew of was 41 km, and they are used against logistics routes **[ref]**.
- **Ground robots.** Ukraine's ground robots ran 25,143 logistics and evacuation missions in August 2026 alone **[ref]**.

### 3.6 Air

- **Contested airspace is the norm.** In Ukraine, neither side has air superiority; SAM coverage keeps aircraft at stand-off range. Russia's answer is glide bombs: about 3,500 a month in early 2025, rising to more than 5,000 by October 2025 **[ref]**.
- **Missile range decides beyond-visual-range fights.** India's Rafale loss in May 2025 is attributed to underestimating the PL-15's range: the shot came from about 200 km while pilots assumed about 150 km **[ref]**. *Simulation:* a pilot's belief about enemy weapon range is part of its picture and can be wrong.
- **Attack helicopters.** They suffered heavy early losses: 23 or more of 90 Ka-52s by October 2022. Survivors were effective with stand-off ATGMs **[ref]**.
- **Deep strike and missile defence.** Israel–Iran, June 2025: about 85% of missiles were intercepted. Interceptor stocks were the binding constraint: Israel came close to running out of Arrow-3, and the US used roughly 92–150+ THAAD rounds **[ref]**.

### 3.7 Naval

- **The Red Sea showed magazine depth and cost exchange.**
  - About 220 missiles (SM-2 at about $2–2.4M, SM-6 at about $4M) and 160 5-inch rounds over about 380 engagements by January 2025.
  - Ships must leave station to reload.
  - The Navy calls reliance on SM-3/SM-6 "unsustainable" **[ref]**.
- **The Black Sea showed a navy defeated from the shore and by drones.** Moskva was sunk. About 25% of the Black Sea Fleet was sunk or damaged by February 2024, and the fleet withdrew from Sevastopol. Sea drones armed with air-to-air missiles have shot down helicopters **[ref]**.
- **Salvo combat (Hughes).** Outcomes depend on offensive firepower, defensive firepower, staying power and, above all, scouting and firing effectively first **[ref]**. *Simulation:* these quantities are not parameters; they emerge from sensors, missiles, interceptors and damage.

### 3.8 Electronic warfare, space and cyber

- **GNSS jamming breaks precision.** Excalibur effectiveness reportedly fell from more than 50% to under 10% within months, on thin data. JDAM-ER bombs lost effectiveness within weeks until fixed **[ref]**. Each Pole-21 jamming module covers at least 25 km **[ref]**.
- **Emitting is dangerous.** Russia uses its EW systems to cue artillery **[ref]**. The time from detecting an emitter to a strike landing is not sourced, so it is **[declared]**, and it is an emergent kill-chain time in the simulation.
- **Space.** In 2025 the main counterspace activity was GNSS jamming and spoofing and satellites manoeuvring, not kinetic ASAT **[ref]**. Imagery access is a policy lever: Ukraine lost free Maxar access for about a week in March 2025, and ICEYE's radar satellites filled part of the gap **[ref]**.
- **Cyber is months of access followed by a short effect.** Sandworm was inside Kyivstar from May 2023. In December it cut service for about 24 million people for 2–3 days **[ref]**. Viasat KA-SAT: tens of thousands of modems wiped on 24 February 2022, with limited military effect **[ref]**.

### 3.9 Logistics

- **Fuel.** An M1 Abrams burns about 60 gal/h cross-country and about 10 gal/h idling, about 300 gal per 8 hours **[ref]**. A brigade's fuel need rose from about 40,000 L/day to over 300,000 L/day when it re-equipped with heavy armour **[ref]**.
- **Reach.** Supply hubs were pulled back to about 120 km, beyond GMLRS range. The 40-mile convoy north of Kyiv stalled for lack of fuel and food **[ref]**.
- **The last mile is robotic.** Drones and ground robots make 50–80% of front-line deliveries in some Ukrainian brigades **[ref]**.
- **Casualty evacuation.** Delays of 60–120 minutes are now common under drones, against the "golden hour", and are sometimes far longer **[ref]**.

### 3.10 What the professional simulations teach

| Simulation | Approach |
|---|---|
| **AFSIM** | Builds platforms from components (movers, sensors, weapons, comms, processors) |
| **JTLS-GO** | Aggregates ground combat, but models aircraft, ships and network nodes individually |
| **OneSAF** | Composes behaviours from primitives |
| **VBS4** | Edits behaviour trees visually |
| **Command: Modern Operations** | Exposes weapon release authority, EMCON and doctrine as player settings |
| **SWORD/TARAN** | Doctrine-driven AI from brigade to corps |

Orders and scenarios have standards: C-BML for orders, requests and reports; MSDL for scenarios; C2SIM, which is replacing both **[ref]**. Detection models for EO sensors descend from ACQUIRE, P = (N/N50)^E / (1 + (N/N50)^E) with E = 2.7 + 0.7·(N/N50) **[ref]**.

**The lesson this plan takes:**

- components for platforms (§4.1)
- a doctrine-editable decision layer (W1)
- a hard gate for weapon release and EMCON (W1, W6)
- mixed resolution (rule 6)
- C2SIM-shaped scenarios and orders (W0, W4)

---

## 4. Target architecture

### 4.1 Platforms built from components

Today a unit is a `ForceElement` plus an `RtUnit`. It is shaped for ground vehicles: three target classes, armour by aspect and a ground speed. The target is a **platform made of components**, as AFSIM does. Every domain is then the same kind of object with different parts.

```ts
// realtime/model/platform.ts (W2)
interface Platform {
  id; side: SideId; label; sidc; archetype: ArchetypeId;   // archetype → default decision graph
  domain: "land" | "surface" | "subsurface" | "air" | "space" | "cyber" | "node";
  echelon?: Echelon; parent?: PlatformId;                  // command tree (W4)
  kin: Kinematics;          // position {lat, lng, altM | depthM}, velocity, heading
  mover: MoverSpec;         // ground | wheeled | foot | rotary | fixedWing | surface | sub | orbit | static
  body: BodySpec;           // count of platforms (vehicles, aircraft), damage states, vulnerability class
  signatures: Signature[];  // visual, IR, RCS by aspect, acoustic, RF (from emitters)
  sensors: SensorSpec[];    // eye/optic/thermal, radar, ESM/RWR, sonar, acoustic, SAR/EO
  weapons: WeaponMount[];   // with magazines; munitions may fly as entities
  comms: Radio[];           // bearers and the nets it is on
  emitters: Emitter[];      // radars, radios and jammers; on/off is EMCON
  stores: Stores;           // fuel, ammunition by type, batteries, spares, medical, carried drones/troops
  carried?: PlatformId[];   // dismounts, drones, embarked aircraft, cargo
  crew: CrewState;          // quality, suppression, cohesion, fatigue, casualties
  orders: Orders | null; activity: RtOrder;                // as today, generalised
  picture: Beliefs;         // its own knowledge (generalised knowledge.ts)
}
```

A **ground platoon today** maps onto this without loss:

| Today | Component |
|---|---|
| `vehicles {total, fit}` | `body` |
| `armour` | `body` vulnerability |
| `capabilities` | `weapons` |
| `speedKmh`, `moveType` | `mover` |
| detection multipliers | `signatures` |

W2 does this mapping first, behind an adapter, and keeps the golden tests green.

### 4.2 Space and time

- **Three dimensions.** `LatLng` gains `altM` (above sea level; negative for depth). Line of sight is between two heights. `observerHeightM` is finally used: a mast, a drone at 300 m, an aircraft at 8,000 m. The radar horizon is d ≈ 4.12·(√h₁ + √h₂) km, with h in metres **[declared, standard 4/3-Earth approximation]**.
- **Scales.** The board is a 10 km square today. The engine needs:

  | Scale | Size |
  |---|---|
  | Tactical | 10–30 km |
  | Operational | 100–300 km, for a brigade and division with drones and artillery |
  | Theatre | 1,000+ km, for air, naval, missiles and space |

  The map (decho-basemap) already handles any extent. The terrain raster becomes a tiled, lazily loaded pyramid; `shared/routing/tiledRouter.ts` already tiles. Sea areas need bathymetry: GEBCO, as a Foundry dataset.
- **One base tick of 1 s, with three ways to handle other tempos:**
  1. **Fast movers within a tick.** Missiles at about 1–3 km/s and aircraft at about 0.25 km/s are moved per tick, but intercepts and hits use the closest point of approach within the tick, so nothing tunnels through a target.
  2. **Slow processes run sparsely**, through an event scheduler (a priority queue keyed on sim-time) next to the tick:
     - Consumption accrues every tick, but resupply nodes run every 60 s.
     - Satellites are propagated analytically and only evaluated when a pass or an access check is due.
     - Cyber effects are scheduled windows.
  3. **Speed multiplier up to ×60** for quiet periods. Long games of 24–72 h, needed for logistics, are run partly headless at ×600.
- **Weather and light.** A time of day, sun angle and weather state (visibility, cloud base, precipitation, wind, sea state) affect sensors, movement, flying and drones. Thermal crossover and bad weather are when assaults happen **[ref]**.

### 4.3 Knowledge: truth, unit beliefs and the HQ picture

Today's `beliefOf` states (*unaware → suspected → located → identified → lost*) generalise to every domain. What changes:

- **Every belief carries a source, an age, a position error and a classification.** Classification has levels: *track → class (vehicle/aircraft/ship) → type (MBT) → identity (T-90M, hostile)*. Sensors differ in how far up that ladder they get.
- **Track quality**, as on Link 16 (0–15), is the common currency between sensors, data links and the COP.
- **HQ pictures (a COP per HQ)** are fused from reports. Fusion can merge two tracks into one or split one into two, and holds false and decoy tracks. Deception (decoys, Shahed-style mixed raids **[ref]**) works because pictures can be wrong.
- **Each side's picture is visible to the player separately from the truth** (W13). This is the most important teaching tool.

### 4.4 Command tree

- `Side → Component (Land / Maritime / Air / Space / Cyber / Logistics) → Formation HQ → … → Unit`.
- **Every HQ is an entity.** A command post can be found, jammed and destroyed. It has a staff with **cells**: manoeuvre, fires, ISR, EW, sustainment and air. Each cell is driven by its own decision graph (W1) and its own commander type:

| Commander type | Decides | When |
|---|---|---|
| **Claude** (or human) | Plans and orders: tasks, phases, control measures, task organisation, priorities | On the battle rhythm, on CCIR triggers, or when the player asks (W4) |
| **Jev** | A cell's choices: which shooter answers a fire request, whether to commit the reserve, which convoy goes first | At the cell's decision points |
| **Rules** | Routing reports, fusing tracks, scheduling | Always |

### 4.5 Networks

Messages travel over **bearers** (radio types, data links, SATCOM, mesh, fibre, courier) on **nets** (command, fires, logistics, air–ground, data link). Each bearer has:

- a range model
- capacity
- latency
- a loss model
- a jamming susceptibility
- an RF signature

Today's single side net, 5 km range with HQ relay, becomes one configuration of this model (W4).

### 4.6 Sensors and signatures

Every sensor turns target signatures into a detection hazard per second, through a model of its type:

| Sensor type | Model |
|---|---|
| Visual, optic, thermal | ACQUIRE |
| Radar | Range equation with horizon and clutter |
| ESM / RWR / SIGINT | Emitter on, in band, in range |
| Sonar | Passive: SL − TL − NL + DI ≥ DT; active: a range equation |
| SAR / EO satellite | Imaging access windows |

Today's per-second hazard with multipliers (`detection.ts`) is the visual case, and the generalisation keeps it tick-rate independent (W2).

### 4.7 Weapons and munitions

- **A weapon mount** has a magazine, reload time, salvo size, minimum and maximum range, flight profile, guidance and Pk model.
- **Munitions with flight time over about 2 s are entities.** This covers missiles, rockets, artillery rounds of interest, loitering munitions, FPVs, torpedoes and glide bombs. They can be detected, intercepted, decoyed, jammed or spoofed in flight. This is what makes air defence, naval salvo combat and EW work without special cases.
- **Cost per round** is data, so the simulation can report cost exchange (a $4M interceptor against a $40k drone).

### 4.8 Effects

- **Damage states per platform**, not just "destroyed":

  | State | Effect |
  |---|---|
  | Mobility kill | The platform cannot move |
  | Firepower kill | Its weapons cannot fire |
  | Sensor kill | It cannot sense |
  | Comms kill | It cannot communicate |
  | Catastrophic | It is destroyed |

  Ships take hits against staying power, with compartments as an option. Aircraft take damage states that force a return to base.
- **People.** Infantry and crews take casualties, split into killed and wounded, and the wounded need evacuation (W10).
- **Suppression and cohesion** stay as today. They extend to fatigue over long games and to the strain of being under drone threat (a per-unit "exposure" stress).

### 4.9 The decision layer

| Layer | Who | Where |
|---|---|---|
| Plans and orders | Claude / human | HQs (W4) |
| Choices at decision points | Jev | Every unit and every HQ cell, through its **decision flow graph** (W1) |
| Drills, ROE, weapon release, EMCON | Rules (hard gates) | Autopilot |

The runner's batching, cooldowns, reaction time and deterministic wait are kept. They scale with batching per side and per tick (§7).

### 4.10 Performance budget

**Target:** 2,000 platforms and 500 munitions in flight at ×10 on a mid-range laptop.

**Budget:** ×10 means 10 ticks per wall-second. The worker gets 100 ms per wall-second, so about 10 ms per tick, or about 4 µs per platform per tick.

| How | What |
|---|---|
| **Engine in a Web Worker** | The UI thread only renders interpolated positions, as today's screen already does |
| **Spatial hash** | A grid keyed by sensor range class, so detection checks are not N² |
| **Staggered sensors** | A radar evaluates on its scan period (4–12 s **[declared]**); a person's eyes every 1–2 s; a satellite only on access |
| **Line-of-sight cache** | Static terrain line of sight between grid cells is cached, then refined for close pairs |
| **Structure-of-arrays** | For the hot loops (kinematics and sensing) once profiling shows the need |
| **Headless Node runner** | The same engine in Node for calibration batches and CI (W0, W12) |

### 4.11 Code layout (new and changed)

```
apps/bgws/realtime/
  engine/            today's engine; becomes the core loop + land rules
  model/             platform, components, archetypes, sides          (W2)
  behaviour/         decision flow graphs: types, walk, conditions,   (W1)
                     actions/{land,drone,air,sea,ew,log}.ts, library/
  sensing/           sensor models, signatures, fusion, COP           (W2, W4)
  comms/             bearers, nets, messages, PACE                    (W4)
  fires/             indirect fire, kill web, airspace deconfliction  (W3, W11)
  domains/
    drones/ air/ sea/ space/ ew/ cyber/ logistics/                    (W5–W10)
  scenario/          C2SIM-shaped scenario format, loaders            (W0)
  worker/            engine host for Web Worker and Node              (W0)
  ui/                setup, map layers, inspectors, editors           (W13)
apps/bgws/behaviour/ the template editor screen at /bgws/behaviour   (W1)
```

---

## 5. Workstreams

Each workstream has the same parts:

- **Goal**
- **How it works**
- **Decision points and default templates**: what Jev decides for these units
- **Numbers**: [ref] or [declared]
- **Build steps**
- **Done when**

### W0 — Foundations

**Goal.** Make the repository buildable and testable here and in CI, and remove the blockers every other workstream hits.

**Build steps.**

1. **Restore the project shell.** The last two commits (`7f7f4d9`, `76685db`) moved the source into `apps/bgws/` and `shared/` but committed no `package.json`, `tsconfig.json`, `vite.config.ts` or `index.html`. Restore these, with the private `@acc/*` and `@dk-real-time-ai-wargaming/sdk` packages resolved from the Foundry npm registry. Add CI that runs `vitest` and `tsc --noEmit`. Remove the committed `.DS_Store` files and ignore them.
2. **Run the engine in a host.** `realtime/worker/`: one engine host API usable from a Web Worker (browser) and from Node (headless batch runs).
3. **N sides.** Replace the hard-coded `"blue" | "red"` loops (`engine.ts:1074,1922`, `comms.ts:172`) with a `SideId` and a relationship table: hostile / neutral / friendly. Neutrals (civil shipping, civilians) matter for the Red Sea and for ROE.
4. **Scenario file.** A versioned JSON scenario, shaped after C2SIM/MSDL **[ref]**, holding:
   - area, time and weather
   - sides and relationships
   - forces as a command tree
   - initial orders
   - template assignments (W1)
   - victory conditions
   - declared-figure overrides

   Saved to Foundry and to local file.
5. **Declared-figures registry.** `realtime/figures.ts`: every tunable number, with value, unit, source, confidence and sensitivity, exported as a table in the UI. Move today's constants into it (`timing.ts`, `detection.ts`, `comms.ts:31-50`, `fire.ts`, `lethality.ts`).
6. **Event scheduler** next to the tick (§4.2), with tests that replay is unchanged.

**Done when:** the repo builds and the test suite runs in CI; the same seeded scenario gives byte-identical event streams in browser and Node; existing tests pass.

### W1 — Decision flow graphs and the template editor

**The full specification is in [DECISION_FLOW_GRAPHS.md](DECISION_FLOW_GRAPHS.md).** In brief:

- Every unit gets a **decision flow graph** from its archetype's core template, masked at run time by what it can actually do. "By default a tank may fire, hold or retreat when it spots an enemy."
- **Templates** are derived by patches: remove an option, re-weight it, add a gate, change the fallback, add doctrine text. They are assigned per unit, per archetype per side, or per scenario. "My tank template never retreats."
- **Jev picks among what the graph leaves open**, as today. Gates, ROE and capability masks are hard. Bias and doctrine text are soft.
- **The editor** has a simple table view for most needs, a node-graph view for full control, validation, a test bench on the calibration scenes, and batch comparison of two templates.
- The first build reproduces today's `decisionPointOf` / `optionsAt` / `ruleFallback` behaviour exactly (golden tests) before anything changes.

W1 comes early (§6) because every later domain adds its decisions as templates rather than as new `switch` cases.

### W2 — Platform model, sensors and weapons

**Goal.** One component model (§4.1) for every domain. Generalised sensing (§4.6) and weapons (§4.7). Three dimensions.

**How it works.**

- **Adapter first.** `toPlatform(ForceElement)` builds the component model for today's units. The engine reads components. The golden full-game tests from W1 must be unchanged.
- **Archetypes** (`model/archetypes.ts`): a table from data fields to an archetype id. The fields are `CuratedPlatform.domain`, `subclass`, `moveType`, `targetClass` and weapons. The archetype id picks the default decision graph and the default signatures and sensors when data is missing.
- **Data.** The L6/L7 curated tables already hold a few fields nothing reads yet:
  - air and sea platforms (`domain: air | sea`, "browsable but not playable")
  - `opticsClass` and `signatureClass`
  - `CuratedSupport` (350 support missions over 311 platforms: response, loiter, sorties per day, range, sensor class)

  W2 plays them for the first time. Each archetype's missing fields come from the declared-figures registry until data replaces them.
- **Sensors.** `sensing/models/*.ts`, one file per sensor type, each returning a hazard per second for an observer–target pair, plus a classification level reached. The visual model is today's `detectChance` with its multipliers. Optics and thermal use ACQUIRE with `opticsClass`. Radar, ESM, sonar and space come with their workstreams.
- **Line of sight between heights**, with the 3,000 m cap (`LOS_CAP_M`) replaced by a per-sensor maximum and the radar horizon.
- **Weapons.** `WeaponMount` with magazines, minimum ranges, salvo, guidance and an `inFlight` flag. Munitions in flight become entities with a simple kinematic model (ballistic, cruise, pursuit, proportional navigation), resolved by closest point of approach.

**Done when:**

- golden tests pass through the adapter
- a mortar fires indirect rounds that are entities in flight
- a ground radar unit detects a helicopter at altitude beyond 3 km
- every new constant is in the registry

### W3 — Land: combined arms and unit sustainment

**Goal.** A realistic combined-arms company-to-battalion fight: infantry that dismounts, indirect fire with a call-for-fire chain, engineers and obstacles, ammunition and fuel at unit level.

**How it works.**

- **Infantry.**
  - Dismounts carried by IFVs (`dismountsCarried` is in L7 data) become their own platforms when they dismount.
  - Small-team infiltration, buildings and trenches as cover and posture.
  - Movement and detection timed by light and weather; thermal crossover matters **[ref]**.
- **Indirect fire.**
  - Guns, mortars and rockets fire missions on a target location. The location comes from the shooter's own picture or from a call for fire received over the fires net.
  - The fire-mission chain is: request → fires cell (W4) → battery assignment → lay → fire → time of flight → impact → adjust or fire for effect → battle damage assessment.
  - Dispersion depends on round type and whether PNT is good; with GNSS jammed, precision rounds degrade (W6).
  - Shoot-and-scoot, and counter-battery radar tracking incoming rounds back to the gun position. Detection is about 40 km for the Zoopark-1M claim **[ref]**; 10–15 km for AN/TPQ-50 **[ref]**.
- **Engineers.** Minefields as areas with density and depth: about 120 m to 500 m **[ref]**. Breach operations: lanes about 6 m wide, about 1.5 h per breach **[ref]**. Obstacles (ditches, dragon's teeth) slow or block routing. Digging in improves cover over time.
- **Unit sustainment.**
  - Ammunition by type per mount (basic load).
  - Fuel by burn rate per hour by activity: about 60 gal/h cross-country, about 10 gal/h idle for an Abrams **[ref]**; others **[declared]** by mass and engine.
  - Batteries for drones and radios.
  - Units at threshold raise a `stocksLow` trigger. Units at zero lose the capability, so the option disappears by masking.
- **Morale.** Keep the break tests; they match Dupuy's 20% attacker halt closely **[ref]**. Add fatigue for games over 6 h **[declared]**.

**Decision points added** (default templates `core.land.*`):

| Trigger | Who | Options (masked by capability) |
|---|---|---|
| `fireRequestReceived` | mortar/artillery section | fire now, fire when laid, refuse (out of range/ammo), shoot and scoot after |
| `counterBatteryThreat` (incoming rounds tracked near own position, or own position likely located) | artillery | displace now, finish mission then displace, stay |
| `obstacleEncountered` | manoeuvre unit | breach, bypass, call engineers, wait |
| `minefieldStruck` | any ground | halt and call engineers, push through lane, reverse out |
| `dismountPoint` (orders or Jev) | IFV with dismounts | dismount here, dismount in cover nearby, stay mounted |
| `stocksLow` | any | request resupply, conserve (fire only in self-defence), withdraw to resupply |

**Done when:**

- a mechanised company attack scenario runs with dismounts, mortar support called by the company, a minefield breach, and ammunition and fuel drawn down
- the breach takes about as long as the reference figure, within ±50% over 50 seeded runs (W12)

### W4 — Command, control and communications

**Goal.** Echelons, HQs and staff cells; orders with real structure; networks with physics; HQ pictures; Claude planning on a battle rhythm.

**How it works.**

- **Command tree** in the scenario: component → formation → unit, each with an HQ platform and staff cells (§4.4). Span of control and relays follow the tree.
- **Orders** grow from `UnitOrders` into a structured order shaped after the five-paragraph order and C2SIM:

  | Part | Contents |
  |---|---|
  | Mission | Task and purpose |
  | Commander's intent | Plain text, given to Jev |
  | Phases | With **triggers**: time, event or position |
  | Branches and sequels | Each with a trigger |
  | Control measures | Real geometry: phase lines (polylines), boundaries (polygons), objectives, named and target areas of interest, fire support coordination measures (FSCL, CFL, NFA, RFA), airspace control measures (ROZ, corridors, kill boxes) |
  | ROE and weapon control status | Free / tight / hold |
  | PACE plan | Which bearers, in which order |
  | Sustainment notes | Resupply points and priorities |

  Today's axis-aligned `Boundary` becomes a polygon.
- **Claude's role** stays planning, but it can now happen during play:
  1. **Battle rhythm.** Each Claude-commanded HQ re-plans every *T* sim-minutes (default 30) **[declared]**, or when a CCIR trigger fires: objective taken or lost, a subordinate combat-ineffective, a major enemy force found. The player can still press *Generate orders*.
  2. **Planning takes sim-time.** The new order applies at `t + planningTime`. Planning time scales with echelon and with the time available (the one-third/two-thirds rule) **[ref]**, **[declared]** values. The game does not pause for it. Claude's real latency is hidden inside that sim-time; if Claude is slower than the sim-time allows, the runner waits deterministically, as it does for Jev today.
  3. **Orders travel.** A new order is a message on the command net. A unit out of comms keeps its last order, and acts on intent through its graph's `outOfComms` trigger.
  4. **Claude plans from the HQ's COP**, as today's `hqPicture` does, never the truth.
- **Networks** (`comms/`):
  - **Bearer catalogue.** Each bearer has a range model, capacity, latency, loss, jam susceptibility and signature:

    | Bearer | Range model | Capacity and latency |
    |---|---|---|
    | VHF combat net radio | Line of sight with terrain and antenna height; 5–10 km high power, 10–40 km with amplifier **[ref]** | — |
    | HF | NVIS 35–450 km **[ref]** | Low data rate |
    | UHF air–ground | Line of sight | — |
    | Tactical data link (Link-16-like) | TDMA slots, line of sight, relay via air | 31.6–115.2 kbps **[ref]** |
    | Military SATCOM | GEO footprint | — |
    | LEO commercial SATCOM | Per-terminal capacity under congestion | 25–100 ms; ~10 Mbps congested **[ref]** |
    | Mesh MANET | Hop count | — |
    | Fibre / landline | Fixed routes, cut by damage | — |
    | Courier | Physical movement | — |

  - **Nets.** Membership, priority and discipline. The existing one-talker-per-net rule stays for voice nets. Data links carry track updates automatically.
  - **Messages.** Typed after real formats: SALUTE/SPOTREP contact reports, call for fire, 9-line CAS, 9-line MEDEVAC, SITREP, LOGSTAT, FRAGO, request and reply (existing), and track data (data link). Voice reports take compose time and carry errors; data link tracks are fast and precise but need equipment and bandwidth.
  - **PACE.** Each unit's order lists primary, alternate, contingency and emergency bearers. The unit falls back down the list when a bearer fails, as a rule, not a decision.
  - **Every transmission is an emission** that ESM can detect (W6).
- **Fusion and the COP** (`sensing/fusion.ts`): an HQ fuses incoming reports into tracks with quality and age. Contradictory reports, duplicates and decoys are kept, not resolved by the truth.

**Decision points added:**

| Trigger | Who | Options |
|---|---|---|
| `outOfComms` (no contact with HQ for *n* s) | any unit | carry on with the current phase, act on intent (next phase), hold and try to re-establish, move to regain comms |
| `fireRequestsCompeting` | fires cell | which request first (Jev ranks by the HPTL and the intent) |
| `commitReserve?` | battalion/brigade HQ cell | commit to X, commit to Y, hold |
| `subordinateIneffective` | HQ cell | reinforce, swap with reserve, withdraw it, carry on |
| `emitting too long` (from W6) | HQ | displace the command post, go to alternate bearer, accept risk |

**Done when:**

- a battalion scenario runs with three company HQs and a battalion HQ, each on its own nets
- Claude re-plans on CCIR without pausing
- a destroyed relay or HQ visibly slows reports, measured in the AAR
- a unit cut off acts on intent

### W5 — Drones and counter-drone

**Goal.** The transparent battlefield, with the kill zone emerging from drone teams, links, batteries and counter-drone measures.

**How it works.**

- **Drone teams are ground platforms** (operators, often in a dugout) that **carry** drones in stores. A launched drone is an air platform with:
  - a battery or fuel endurance
  - a sensor (EO/thermal)
  - a warhead, or none for ISR
  - a **control link**

  Link types:

  | Link | Properties |
  |---|---|
  | **RF** | Line of sight to the operator or a relay; can be jammed (W6); its emissions reveal the operator |
  | **Fibre** | Cannot be jammed; spool length caps the range (commonly ≤20 km, record ~41 km **[ref]**); a cut fibre ends the flight |
  | **Autonomous** | Terminal guidance after link loss, if the drone has it |

- **Drone classes.** Each is an archetype with a core template:

  | Class | Role |
  |---|---|
  | ISR quadcopter | Finds and adjusts fire |
  | FPV strike, RF or fibre | Strike |
  | Loitering munition | Lancet- or Switchblade-like |
  | Long-range one-way attack | Shahed-like; flies a pre-planned route, no live link; mixed with decoys |
  | MALE ISR/strike | TB2/Reaper-like; vulnerable to air defence |
  | Interceptor drone | Hunts other drones |
  | Ground robot (UGV) | Logistics, evacuation, strike |
  | Uncrewed surface vessel | With W8 |

- **Strike resolution** is a chain of probabilities: reach the target (link, jamming, battery, interceptors), acquire it (sensor and pilot skill), hit (pilot skill: about 30–50% skilled, about 10% novice **[ref]**), then damage, using the existing penetration and lethality model with top attack. The aggregate "60–80% of strikes fail to destroy" **[ref]** is a calibration target, not an input (W12).
- **Drone-to-fires loop.** An ISR drone's detections go into the team's picture and onto the fires net. The time from sighting to rounds landing emerges from W4 and W11.
- **Counter-drone.** Each is a platform component:
  - EW (W6)
  - guns and shotguns, at short range
  - interceptor drones
  - air-defence missiles (W7), at high cost
  - passive measures: cages and netted roads that reduce the chance of a hit; the Trophy dive gap above 55° **[ref]**
  - posture: hiding and dispersal, and movement timed for bad weather

**Decision points added:**

| Trigger | Who | Options |
|---|---|---|
| `targetFound` | ISR drone team | report and keep watching, call fire, hand off to a strike team, strike (if armed) |
| `linkDegraded` | RF drone team | push on (risk losing it), climb for link, return, switch to autonomous terminal (if able) |
| `batteryLow` | team | return, one more pass, ditch on target |
| `jammedHeavily` (repeated losses) | team | switch to fibre drones, relocate, call fire on the jammer (if located), wait |
| `operatorLocated?` (own emissions long, or cues) | team | relocate now, finish flight then relocate, stay |
| `droneOverhead` | any ground unit | freeze and hide, shoot it, scatter, move to cover, call counter-drone |
| `interceptTasked` | interceptor team | engage target A/B (Jev ranks by threat and cost), hold for a better target |

**Done when:**

- a "drone line" scenario has two sides with drone teams 5–15 km apart and ground units moving between them
- the measured depth of the kill zone (where ground losses happen) comes out within the reference band
- switching one side to fibre visibly changes the effect of the other side's jamming

### W6 — Electronic warfare and PNT

**Goal.** The spectrum as terrain. Emitting is useful and dangerous; jamming is spatial and indiscriminate; GNSS loss degrades precision.

**How it works.**

- **Emitters.** Every radio, radar, data link, drone link and jammer is an emitter. It has a band, power, duty cycle and antenna pattern, and is on or off. **EMCON** is a weapon-release-style hard gate set in orders (*silent / limited / free*), plus template decisions.
- **Electronic support.** ESM and SIGINT platforms detect emitters in band and range, and produce bearings. Two or more bearings, or one bearing with time and motion, give a fix with an error ellipse, which goes onto the fires net (W11). Detection hazard rises with duty cycle and time on air **[declared]**. The time from emission to strike emerges from the kill web.
- **Electronic attack.** A jammer has power, band and pattern. For each victim link, a jam-to-signal check (J/S, with burn-through range against radar) decides whether the link is degraded or lost **[declared, standard formulas in the reference]**. Effects by victim:

  | Victim | Effect |
  |---|---|
  | Radios | Messages lost or delayed |
  | Data links | Track quality decays |
  | Drone RF links | Loss of control |
  | Radar | Range cut, false tracks |
  | **GNSS** | PNT quality falls in an area (Pole-21-like module ≥25 km **[ref]**); GNSS-only guided munitions lose accuracy (Excalibur >50% → <10% **[ref]**, a calibration target); units' own position error grows (affects call-for-fire accuracy) |

  Jamming is indiscriminate: it hits **own** links in the area too. Russia's 2022 jamming disrupted its own comms **[ref]**.
- **Electronic protection.** Frequency hopping and spread spectrum as bearer properties (resistance to J/S), directional antennas, fibre, and changing frequencies as a decision that costs time.
- **Spoofing** (later): false PNT that pulls munitions and drones off course; false tracks on radar.

**Decision points added:**

| Trigger | Who | Options |
|---|---|---|
| `emitterDetected` | EW team | locate (needs time), jam, report for strike, ignore |
| `ownEmissionsExposed` (estimated exposure from time on air and known enemy ESM) | any emitter | go silent, relocate after this transmission, carry on, switch bearer |
| `jammingOrder` | jammer | jam sector A/B, protect the advance (move up behind it, as at Kursk **[ref]**), go silent to survive |
| `pntDegraded` | artillery, strike | fire anyway, switch to non-GNSS munitions, wait, request drone guidance |

**Done when:**

- a GNSS jammer measurably cuts precision-round effectiveness toward the reference band
- an emitter that stays on air too long is found and struck by an emergent chain
- a jammer degrades its own side's radios in its footprint

### W7 — Air and air defence

**Goal.** Aircraft, helicopters, airbases and sorties, air tasking, beyond-visual-range combat, SEAD/DEAD, CAS, glide bombs and ground-based air defence. Airspace control lets fires, drones and aircraft share the sky.

**How it works.**

- **Aircraft** are air platforms with altitude, speed, fuel (**bingo**), weapons (**Winchester**), an RWR, radar and IFF. Each **flight** of 2–4 aircraft is one entity, like a ground platoon.
- **Airbases** are node platforms. Sorties are generated by turnaround time, with damage to runways and dispersal. Airbase attack cut sortie output by 35–40% in an illustrative study **[ref, illustrative]**. Dispersal reduces sorties, because there are fewer spares and repairs are slower **[ref]**. The CuratedSupport data (sorties per day, response, loiter) seeds this.
- **Air tasking**, a light version of the 72-hour ATO cycle **[ref, doctrine]**. The Air component HQ (Claude) writes a tasking:
  - mission packages: CAP stations, strike, SEAD, CAS on call, ISR orbits, tanker tracks
  - times and routes
  - airspace control measures

  Dynamic retasking happens through the Air Ops cell's Jev template when a time-sensitive target appears (W11).
- **Engagements.**
  - Air-to-air missiles are entities with range and kill probability by aspect and range; a pilot's belief about the threat's missile range comes from its picture (the PL-15 lesson **[ref]**).
  - Defensive manoeuvres (crank, notch, beam), chaff and flares, and missile energy modelled as Pk falling off toward maximum range **[declared]**.
- **Ground-based air defence (GBAD).** SHORAD, medium-range and long-range air defence as platforms with:
  - search and fire-control radars (emitters for W6)
  - launchers with magazines
  - a weapon control status (free/tight/hold)
  - an engagement doctrine (shoot-look-shoot, salvo size)
  - **cost-aware weapon-to-target assignment**: interceptor drone $1.5–16k, PAC-3 $4M+, Shahed $30–50k **[ref]**
- **SEAD/DEAD.** Anti-radiation missiles home on emitters, decoys load the defence, and jammers support. Radars that go silent survive but stop defending (W6 decision). Opening-strike vignette from Rising Lion: drones staged in advance hit air-defence radars at H-hour **[ref]**.
- **CAS.** A JTAC on the ground passes a 9-line over the air–ground net. Glide bombs are released from stand-off range outside SAM coverage. Helicopters fire stand-off ATGMs and are vulnerable to MANPADS.
- **Airspace control.** Restricted operating zones, corridors, coordinating altitudes, kill boxes and the FSCL. Artillery trajectories, drones and aircraft are deconflicted by rule (a hold-fire gate) or flagged when they are not. Fratricide is possible when deconfliction fails.

**Decision points added** (core templates `core.air.fighter`, `core.air.attackHelo`, `core.air.cas`, `core.gbad.*`):

| Trigger | Who | Options |
|---|---|---|
| `rwrSpike` / `missileWarning` | aircraft | defend (crank/notch/beam + countermeasures), press the attack, abort |
| `targetInWez` | fighter | shoot now, close for higher Pk, hold (ID not confirmed, ROE) |
| `bingoFuel` / `winchester` | aircraft | RTB, tanker, stay one more pass (risk) |
| `threatRingAhead` | strike/CAS | go around, go low, request SEAD, abort |
| `trackAppears` | GBAD | engage (choose interceptor by cost/threat), hold (WCS tight, not identified), go silent and let pass |
| `radarTargetedByArm` | GBAD radar | shut down, stay up and engage, relocate |
| `nineLineReceived` | CAS flight | accept, accept with delay, decline (threat or fuel) |

**Done when:**

- a SEAD package against an air-defence site produces plausible outcomes over 50 runs (radars silenced or destroyed, aircraft losses)
- a Shahed raid with decoys against a layered defence produces about the reference interception rate as an output, with interceptor expenditure and cost exchange reported
- artillery and aircraft are deconflicted

### W8 — Naval

**Goal.** Surface, subsurface and littoral warfare on the same clock as land and air, with salvo combat, magazine depth and the shore as a weapon.

**How it works.**

- **Ships** are surface platforms with:
  - radar on a mast height (the horizon matters)
  - ESM
  - vertical launch cells as a finite magazine that is **only reloaded in port**: Red Sea **[ref]**
  - point defence (CIWS, short-range missiles), decoys and EW
  - a sonar and a helicopter on some
  - staying power in hits **[declared per class]**
- **Submarines** are subsurface platforms with depth, speed (noise rises with speed) and passive sonar. Detection uses the passive sonar equation **[declared]**; convergence zones are optional, later.
- **Anti-surface warfare.** Anti-ship missiles are entities with sea-skimming profiles and seekers. Targeting beyond the horizon needs a cue from another sensor (aircraft, drone, satellite or shore radar): the kill chain (W11). **Salvo outcomes emerge** from scouting, firing effectively first, layered defence and staying power **[ref, Hughes]**.
- **Littoral and asymmetric.**
  - Coastal anti-ship batteries (Moskva).
  - Uncrewed surface vessels with explosives or air-to-air missiles (the Black Sea **[ref]**).
  - Mines.
  - Ports as nodes that can be struck, forcing a fleet to relocate (Sevastopol → Novorossiysk **[ref]**).
- **Naval C2.** The composite warfare commander concept as staff cells: air defence, surface and ASW commanders. Cooperative engagement (sharing fire-control tracks across ships) as a data-link property. EMCON as a hard gate.
- **Water terrain.** Bathymetry, coastline, shipping lanes, sea state (affects small craft and USVs). Neutral shipping as a third side, for ROE and for the economic effect (Suez transits fell from more than 26,000 in 2023 to 13,213 in 2024 **[ref]**).
- **Naval gunfire and Tomahawk-like strike** link to land via W11.

**Decision points added** (`core.sea.*`):

| Trigger | Who | Options |
|---|---|---|
| `vampireInbound` | warship | engage with long/medium/short layer (cost-aware), decoy + manoeuvre, turn to unmask more launchers |
| `magazineLow` | warship | leave station to reload, stay and conserve (point defence only), request escort |
| `surfaceContact` | warship | engage beyond horizon (needs cue), close to identify, shadow, avoid |
| `sonarContact` | sub / ASW unit | attack, track, evade (go deep, sprint and drift) |
| `counterDetected` | sub | evade, attack first, go silent |
| `usvSwarm` | ship / harbour | engage with guns/helicopter, manoeuvre, withdraw |

**Done when:**

- a Red Sea-style scenario (USVs, drones, anti-ship missiles against a destroyer task group) reports engagements, interceptors used and cost exchange in the reference's order of magnitude
- a Black Sea-style scenario shows ships driven out of a port by long-range strike

### W9 — Space and cyber

**Goal.** Space and cyber as enabling layers that the other domains feel: imagery, PNT, SATCOM and missile warning; and effects on networks and logistics.

**How it works (space).**

- **Satellites** are orbit platforms with simple two-body orbits (period, inclination, altitude), propagated analytically and only evaluated at access checks.
- **ISR satellites** (EO, SAR):
  - Tasking queue at the Space component cell.
  - Access windows over named areas.
  - Weather (cloud) blocks EO but not SAR.
  - The image is a batch of detections delivered to an HQ's picture after a **processing latency** (**[declared]**, e.g. 30–90 min).
  - Commercial imagery is a **provider** that can be cut off by a scenario event, as Maxar was in March 2025 **[ref]**.
- **PNT.** A side-wide GNSS service whose quality is reduced by W6 jamming per area, and by counterspace events.
- **SATCOM.** GEO military and LEO commercial bearers (W4), with terminals that can be jammed, and a scenario lever for the provider cutting off terminals (February 2026 **[ref]**).
- **Missile warning.** Detects ballistic launches and cues GBAD and naval defence with a warning time.
- **Counterspace** as scenario-level actions: jamming uplinks/downlinks, dazzling, cyber against ground stations, and kinetic anti-satellite weapons as a rare escalation, gated by ROE. The 2025 trend was jamming and spoofing, not kinetic **[ref]**.

**How it works (cyber).**

- A **cyber cell** per side holds **accesses** prepared before the game: target network, readiness, probability of effect, effect duration, detection risk. Sandworm's Kyivstar access took months; the effect lasted 2–3 days **[ref]**.
- **Effects** act on modelled systems:

  | Target | Effect |
  |---|---|
  | A bearer or net | Outage or degradation (W4) |
  | A logistics app or system | Delays (W10) |
  | An air-defence network | Track-sharing loss |
  | A public network | Alerts and morale (later) |

- Executing an access is a **Claude- or player-level decision**, gated by ROE, because it is strategic and escalatory. Jev decides only tactical timing within an authorised window.
- Information operations are out of scope until after M8.

**Decision points added:** Space cell `tasking` (which area this pass, Jev ranks by CCIR); cyber cell `windowOpen` (execute now, hold for a better moment, abandon if detected).

**Done when:**

- Red's picture of Blue visibly updates on satellite passes, with latency
- cutting Blue's commercial imagery or LEO SATCOM in a scenario event measurably slows Blue's kill chain
- a cyber effect on a logistics system delays resupply

### W10 — Logistics and medical

**Goal.** Sustainment as a network that can be cut, so that long fights are decided by supply as much as by fire.

**How it works.**

- **Supply classes modelled:**

  | Class | What it is |
  |---|---|
  | I | Food and water; simplified, matters for long games |
  | III | Fuel |
  | V | Ammunition, by family: small arms, 120 mm, 155 mm, missiles, drones |
  | VIII | Medical |
  | IX | Spares; maintenance |

  Others are folded into these or ignored **[ref, doctrine]**.
- **Consumption** per platform from W3 (fuel by activity, ammunition by firing).
- **Nodes** (node platforms):
  - railheads, ports, airfields
  - depots and supply hubs, about 120 km back where the threat dictates **[ref]**
  - brigade support areas
  - forward logistic points
  - company trains
- **Movement** by convoy (entities on roads, visible and targetable), rail (fixed lines, can be cut), air and the last mile (drones and ground robots, as Ukraine does **[ref]**). Throughput per route per hour, with a rule-of-thumb reach from the railhead **[ref, unsourced rule of thumb]**.
- **The LOGPAC cycle.** Requests from units (`LOGSTAT`) go to the logistics cell. Jev chooses priorities and routes inside the plan's sustainment priorities. Convoys move to logistic release points, units come back to draw, or deliveries go forward.
- **Maintenance and recovery.** Mobility-killed vehicles can be recovered and repaired (time **[declared]**) or cannibalised.
- **Medical.** Wounded from W2 effects need evacuation: point of injury → casualty collection point → treatment. Evacuation time affects survival **[declared curve]**. Typical evacuation is now 60–120 min **[ref]**. Drones deliver blood **[ref]**. Losses affect morale.

**Decision points added** (`core.log.*`):

| Trigger | Who | Options |
|---|---|---|
| `resupplyRequests` | logistics cell | which unit first, by which route and means (convoy/UGV/drone), when (now/dark/bad weather) |
| `routeThreatened` | convoy | push on, take the alternate route, wait for dark, scatter |
| `droneAttackOnConvoy` | convoy vehicle | scatter, cover, push through |
| `casualtiesWaiting` | unit / medical | evacuate now under threat, wait for dark, request UGV/drone |
| `nodeStruck` | logistics cell | relocate the node back, disperse stocks, repair and stay |

**Done when:**

- a 24-hour brigade scenario shows fighting power tracking fuel and ammunition availability
- cutting the main supply route forces a culmination the AAR can show
- casualty evacuation times are reported and fall in the reference band

### W11 — The joint kill web and cross-domain integration

**Goal.** Any sensor, any shooter, through real networks and real staff decisions, in every combination the domains allow. Its speed is **measured, not set**.

**How it works.**

- **Target nomination.** Any track in an HQ's COP that matches the **high-payoff target list** and the **target selection standards** (location error, age) becomes a nomination. These come from Claude's plan, as a fires annex.
- **Shooter options** are generated by rule. For every shooter the fires or air cell can task, in range, with ammunition, with a valid track, and deconflicted from airspace, the rules compute four figures:

  | Figure | Feeds |
  |---|---|
  | Estimated time to effect | Jev's options |
  | Expected effect | Jev's options |
  | Cost | Jev's options and the cost-exchange report |
  | Risk | Jev's options |

  The shooters range across domains: artillery, rockets, drones, aircraft, ships and EW. The cell's **template** chooses through Jev, with doctrine biases ("prefer cheapest effective").
- **Tracking.** The track must be maintained, by keeping a sensor on it, or the strike flies to a stale position. Moving targets punish slow chains.
- **Battle damage assessment.** After impact, a sensor must look. Until then the HQ does not know the result; re-striking a dead target or leaving a live one are both possible errors.
- **Measurement.** Each chain's step times (find → fix → track → target → engage → assess) are recorded per strike in the AAR. Comparing Delta-like settings (data link, flat kill web) with hierarchical voice nets should reproduce the reference contrast: minutes against hours **[ref]**. This is a key validation target.
- **Cascades.** These come free from the model, because everything is coupled through components:
  - GNSS jamming → precision fails → shift to laser, drone or unguided fire.
  - An HQ destroyed → slower nominations.
  - SATCOM cut → the drone feed drops → the kill chain lengthens.
  - A port struck → no reloads → magazine-limited fleet withdraws.

**Decision points added:** fires/air/maritime cell `nomination` (choose shooter, or defer), `bdaUnclear` (re-strike, look again, move on), `airspaceConflict` (hold the guns, shift the aircraft, accept risk).

**Done when:**

- a "joint strike" scenario runs the opening of a Rising Lion-like operation **[ref]**: staged drones hit radars, SEAD opens a corridor, then strikes follow
- the AAR shows every kill chain step by step
- swapping the network model from voice to data link changes kill-chain times by the reference factor

### W12 — Calibration, validation and analysis

**Goal.** Know how far to trust the simulation, and make it better with evidence.

**How it works.**

- **Declared-figures registry** (W0) with a source, confidence and sensitivity per figure. The UI lists the least-sourced, most-sensitive figures: the research backlog.
- **Headless batch runner** (Node, W0): N seeded runs of a scenario at maximum speed. Jev can be replaced by the recorded-answer decider (replay) or by the rule decider when a run must be free and fast. A sample of runs uses live Jev to check that conclusions hold.
- **Vignette tests:** historical or reference outcomes as **distributions**, checked in CI on the rule decider and nightly on Jev.

  | Vignette | Target **[ref]** |
  |---|---|
  | Minefield breach | ~1.5 h per lane |
  | Attacking battalion | Halt near 20% casualties |
  | FPV effectiveness | 60–80% of strikes fail to destroy |
  | Shahed raid | ~85% intercepted |
  | Precision rounds under GNSS jamming | Effectiveness from >50% down toward <10% |
  | Red Sea exchange | Interceptors used per engagement |
  | Kill-chain times by network type | Minutes against hours |

- **Sensitivity analysis:** vary each high-sensitivity figure across its plausible range and report which conclusions change. Results are reported as ranges, never as point predictions. This is analytic wargaming practice.
- **Template A/B testing** (W1 batch compare) is the same machinery.
- **LLM behaviour checks:**
  - Claude's plans are tested for escalation and ROE breaches by a fixed suite of scenarios (the escalation literature **[ref]**).
  - Jev's choices are checked for degenerate patterns, such as always choosing one option, against the calibration scenes.

**Done when:** every figure is in the registry; each vignette has a passing distribution test; a sensitivity report can be generated from the UI.

### W13 — User interface at scale

**Goal.** A screen that lets a person understand a multi-domain battle at a glance, and drill into why anything happened.

**Build steps.**

1. **Map layers by domain** (land, air, sea, space ground tracks, spectrum), each toggleable. **Truth / Blue picture / Red picture** switch (§4.3). Military symbols from `milsymbol` with proper SIDCs per archetype. Today the SIDC is a fixed default per side (`forceBuilder.ts:75`).
2. **Scale.** Zoom from section to theatre, with clustering by echelon, so a zoomed-out view shows battalions, not 40 platoons.
3. **Altitude.** A side profile panel for the selected air track or missile; altitude labels; range rings for weapons and sensors; footprints for jammers and radar coverage.
4. **Inspectors:**
   - unit: stores, damage, picture, template, last decisions with graph path
   - HQ: COP, nets, pending requests, plan
   - network: who can talk to whom now, and what is jammed
   - logistics: stocks, convoys, requests
5. **Kill-chain timeline**: each strike as a bar from find to assess.
6. **AAR and replay** from the event stream (`EVENT_STREAM.md` already streams to Foundry). Scrub, filter by unit, domain or decision point. Export to Foundry dashboards.
7. **Setup.** A scenario editor for command tree, orders, templates, figures overrides and weather, saved per W0.
8. **The behaviour editor** at `/bgws/behaviour` (W1).

---

## 6. Roadmap: milestones

Each milestone ends with a **playable scenario** that exercises everything built so far, plus its vignette tests (W12). W12 (calibration) and W13 (UI) run alongside every milestone.

```
M0 Foundations ──► M1 Editable behaviour ──► M2 Combined-arms company ──► M3 Battalion C2
                                                                            │
          ┌─────────────────────────────────────────────────────────────────┘
          ▼
   M4 Transparent battlefield (drones + EW) ──► M5 Air over the battle ──► M6 Sustained fight
                                                                            │
          ┌─────────────────────────────────────────────────────────────────┘
          ▼
   M7 Littoral (naval) ──► M8 Joint all-domain (space, cyber, kill web, theatre scale)
```

| Milestone | Workstreams | Playable scenario | Key acceptance |
|---|---|---|---|
| **M0 Foundations** | W0 | Today's real-time game, built in CI, running in a worker | Byte-identical replays in browser and Node; all tests green |
| **M1 Editable behaviour** | W1 (steps 1–8), W13.8 | Today's tank battle, with "Tank: stand fast" built in the editor and assigned to Red | Golden test identical to old behaviour; a template made and assigned in under 2 min; test bench works |
| **M2 Combined-arms company** | W2, W3, W1 (archetype templates) | Mech company hasty attack: dismounts, mortars on call, a minefield breach, ammunition and fuel | Breach time and attacker halt in reference bands |
| **M3 Battalion C2** | W4, W13.1–4 | Battalion attack: 3 company HQs, a battalion HQ, nets with terrain-limited VHF, Claude re-planning on CCIR | Out-of-comms units act on intent; HQ loss measurably slows reporting |
| **M4 Transparent battlefield** | W5, W6 | "Drone line": drone teams on both sides, jammers, fibre FPVs, a counter-drone screen | Kill-zone depth and FPV effectiveness in reference bands; GNSS jamming degrades precision rounds |
| **M5 Air over the battle** | W7 | SEAD package and CAS on a contested front; a Shahed-style raid on a layered defence | Interception and cost exchange as outputs in reference order; fires/air deconfliction |
| **M6 Sustained fight** | W10, W13.5–6 | 24-hour brigade fight with a supply network, convoys and medical evacuation | Culmination when supply is cut; casualty evacuation in reference band |
| **M7 Littoral** | W8 | Red Sea task group defence; Black Sea port strike | Magazine-limited defence; ships leave to reload; salvo outcomes emergent |
| **M8 Joint all-domain** | W9, W11, theatre scale | Rising Lion-style opening plus a land campaign: satellites, cyber accesses, joint kill web | Full kill-chain AAR; network swap changes kill-chain times by reference factor |

**Why this order:**

- **W1 first after foundations**, because every later domain is expressed as templates. Building domains first would mean writing more `switch` cases and migrating them later.
- **Land depth before C2.** Indirect fire and logistics at unit level give C2 something to coordinate.
- **Drones and EW before air.** They dominate the modern land fight **[ref]**, they reuse the land map at tactical scale, and they force the 3D, sensor and emitter work that air and naval then build on.
- **Logistics network after air.** Sortie generation, interceptor stocks and convoys all need it, but it needs long games to matter, which in turn need the headless runner and the speed work.
- **Naval and space/cyber last.** They need theatre scale, and they matter mostly through the kill web, which ties everything together.

---

## 7. Jev and Claude at scale

**Jev call volume.**

- Today one decision is in flight per unit, with a 20 s cooldown and 3 s coalescing, batched per side.
- With 2,000 platforms of which perhaps 20% are in contact at a time, expect on the order of 10–40 decisions per sim-second at the peak of a fight **[estimate]**.
- At ×10, that is 100–400 per wall-second, batched into one request per side per tick: 20 requests per wall-second.

**Jev cost.**

- The reference figure is about $0.042 per million input tokens, with output free **[ref, JEV_INTEGRATION_PLAN.md]**.
- At about 3,000 tokens per unit picture, 100,000 decisions in a long game is about 300M tokens, about $13. Cost is not the constraint.
- **Latency is.** It is 70–500 ms per request. The runner's deterministic wait means a slow answer pauses sim time rather than breaking it. At high speeds, the player sees the wait as a lower effective speed.

**Mitigations, in order:**

1. **Fewer questions.** Templates resolve by gates whenever the answer is obvious. A single remaining option is taken without asking (DFG §3.3).
2. **Smaller states.** Each domain gets a compact picture: only relevant beliefs, odds in words, as today.
3. **Answer caching for identical states.** This is off today (`cache: false`); it is safe to turn on for calibration runs.
4. **The rule decider** for units far from the player's focus, as a per-scenario level-of-detail setting.
5. **The recorded-answer decider** for replays and batches.

**Claude** is called per HQ on the battle rhythm or on CCIR, which is rare by design (a few calls per sim-hour per HQ). Its prompt is the HQ picture plus the order schema. Its output is validated as strictly as `parseOrders` does today, with the same heuristic fallback. The Foundry functions (`bgwsCommanderRealtimeOrders` and new ones per component) live in the separate functions repository that `REALTIME_COMMAND.md` refers to as `llmfunctions.ts`.

---

## 8. Risks and open questions

### Risks

| Risk | Mitigation |
|---|---|
| **Scope.** Thirteen workstreams is years of work for one person. | Milestones are each playable. Domains can be cut (e.g. naval after M6) without breaking the core. W0–W1 are valuable on their own. |
| **False precision.** A detailed simulation looks authoritative even where numbers are guesses. | Registry, confidence labels in the UI, results as ranges, vignette tests, and the research backlog (W12) |
| **Open-source data limits.** Many figures, such as Pk values, sonar performance and EW parameters, are classified or contested. | Declared figures with honest labels; sensitivity analysis to show which matter; curated data in Foundry can be replaced by better data without code changes |
| **Performance at theatre scale** | Worker, spatial hash, staggering and level of detail (§4.10); profile from M2 on |
| **LLM escalation and unrealistic choices** | Claude plans only; ROE and weapon release are hard gates; Jev chooses only from rule-generated options; escalation suite (W12) |
| **Determinism under async AI** | Kept by the runner's deterministic wait and recorded answers; tested in M0 |
| **Sensitive content** | The simulation uses open-source information only. Scenarios use real systems' public figures. Nothing here targets real people or places beyond public historical events. |

### Open questions for you

1. **Target scale for the first version of each domain.** Is theatre scale (1,000 km+) needed before M7, or can air stay operational-scale (≤300 km) until then?
2. **Human players.** Should a person be able to command a side or an HQ live? That needs a different orders UI and possibly multiplayer over Foundry.
3. **Where Claude sits.** Brigade and above only, or also battalion and company HQs?
4. **Template sharing.** Team-wide in Foundry from the start, or local first?
5. **Data.** Is there a better source than the L6/L7 curated tables for air, naval and drone figures, and can the curated pipeline be extended to fill the new component fields?
6. **Build shell.** Is the missing `package.json`/Vite setup kept in another repo (the original Mapplications), or should W0 recreate it here?
