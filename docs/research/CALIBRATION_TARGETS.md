# First calibration targets: the land fight

These are the unsourced figures (`[default]`) that move land-fight outcomes
most, from a Morris screening over the land fight's figures. Find sources
for these first.

## How it was run

```
bgws sensitivity <vignette> --prefix land --replicates 3 --trajectories 4 --sobol-top 0
```

- **Figures screened:** 318, the `land` set: `direct.*`, `people.*`,
  `sensing.acquire`, `sensing.observer`, `env.landcover`, `env.optics`,
  `sim.cover`, `sim.sense`, `sim.fire`, `sim.posture`, `sim.doctrine` and
  `decide.heuristic`.
- **Run size:** 4 trajectories, 1,276 design points, Morris only.
- **Replicates:** 3 per point, with common random numbers.
- **Exception:** MBT engagement was run with 8 trajectories and 200
  replicates, because a single hit or miss is too coarse to rank with 3.
- **Run times on 4 cores:**

  | Vignette | Time |
  |---|---|
  | MBT engagement | seconds |
  | Suppression recovery | seconds |
  | ATGM engagement | 7 min |
  | Tank engagement range | 10 min |
  | Attack halt | 38 min |

μ* is the mean absolute effect of moving a figure across its range, in the
metric's units. "Norm." is that effect as a share of the vignette's top
figure.

**Caveats:**

- This is screening, not a variance decomposition: it ranks figures, it
  does not apportion the outcome between them.
- Attack halt plays a whole company battle. With 3 replicates many effects
  fall on the same step (0.25), so read its ranking as a pointer.
- The run came after this branch's two realism changes: sight lines
  through trees that hold until someone moves, and cover from features.
  It also came before the cap on open-ground canopy (below).

## The targets, by area

| # | Area | Figure (value, range) | Moves | Why it matters |
|---|---|---|---|---|
| 1 | Search and detection | `sensing.acquire.cue_factor` (0.25, 0.05–1) | tank range 52%, attack halt 47%, ATGM 26% | How much faster a cued observer finds a target. Sets who sees whom first. |
| 2 | | `sim.sense.search_fov_deg` (60°, 10–180) | tank range 21%, attack halt 55%, ATGM 17% | The sector a unit searches without a cue. |
| 3 | | `sensing.acquire.fov_exponent` (0.5) and `fov_ref_deg` (10°) | tank range 59%, 27% | How search time grows with the sector. |
| 4 | | `sensing.acquire.tau_base_s` (8 s, 1–60) | ATGM 31%, tank range 10% | Search time constant. |
| 5 | | `sensing.observer.moving_factor` (×2, 1–5) | tank range 23%, ATGM 22% | Search slows while moving. |
| 6 | | `sim.sense.max_range_m` (6 km) | ATGM 41%, attack halt 39%, tank range 11% | Longest range a ground observer searches. |
| 7 | Hitting | `direct.guided.ph` (0.8, 0.3–0.99) | ATGM 100% | A guided missile's chance of reaching its target. The ATGM band's own source is thin too. |
| 8 | | `sim.fire.lay_min_s` (6 s, 2–30) | ATGM 37%, tank range 17% | Time to lay on a new target and fire. |
| 9 | | `direct.gun.mbt.v0_mps` (1,700 m/s) and `drag_k_per_m` | MBT 66%, 60%; attack halt 51% (drag) | Time of flight, so error from target motion and range. Published for real rounds, so easy to source. |
| 10 | Suppression | `people.suppression.decay_tau_s` (60 s, 10–600) | suppression 100%, attack halt 55%, ATGM 13% | How fast suppression wears off. |
| 11 | | `people.suppression.gain_per_near_miss` (0.05) | attack halt 59%, suppression 2% | How fast it builds. |
| 12 | Reaction | `people.reaction.median_s.practised` (8 s) | attack halt 44%, ATGM 21% | Drills: how long a practised task takes. |
| 13 | Ground and light | `env.landcover.forest_light.conceal_mean` (0.65) and `canopy_m` (15 m) | attack halt 67%, 59% | How well light woods hide. |
| 14 | | `env.optics.visible.haze_ratio` | attack halt 100% (noisy, σ 0.75) | Contrast loss with range. Check against Koschmieder and visibility data. |

Items 1–6 are the strongest common thread. The search model (ACQUIRE/TTP
with a cue and a sector) decides who sees whom first in every sim-played
vignette. Almost all of it is `[default]`. Sources to try:

- the NVESD/ACQUIRE search-time papers (τ against field of view, cueing);
- the US Army TTP model reports;
- field trials of search with and without cueing.

The rest:

- **Hitting (7–9):** sourced from trials and manufacturer data.
- **Suppression and reaction (10–12):** sourced from the suppression
  studies (UK DOAC/DSTL work on suppressive fire, US Army ARI) and drill
  timings in field manuals.

## Fixed as a result

The tank engagement range run put `env.landcover.open.canopy_m` and
`los_block_per_100m` at the top (100%, 84%). Their ranges, 0–60 m and 0–1,
let the sweep grow a wood on open ground. Open ground, bare ground and
water now range 0–2 m and 0–0.1, so later runs rank the figures that are
really in doubt.

## Not moved

- **`decide.heuristic.*`:** these drive the HQ planner. No vignette here
  exercises them; a planner-level suite would.
- **`sim.cover.*` and `sim.sense.sight_line_cell_m`:** none moved
  attack halt at all (μ* exactly 0). Every other vignette is on flat open
  ground, where no features stand. Attack halt's ground is generated
  (`skirmish-advance-to-contact-v1`): it has no 5 m surface or railways, and
  its smooth relief leaves few points below their surroundings. To weigh
  walls, tree lines and folds, add a vignette on a theatre pack.

## On real ground: attack-halt-real-ground

A new vignette plays the company attack on the Kaliningrad theatre pack
(`gen-kgd-company-attack`) with the pack's buildings, trees, railways and
roads. It runs only where that pack is built (packs are not in git), so CI
skips it.

**How a halt is counted:** when no blue manoeuvre unit has moved for a
minute. A game in which the attack reaches its objective (within its 600 m
radius) gives no value. P14 is about attacks stopped short; this rule also
applies to `attack-halt` wherever its scenario has an objective.

**At the generator's 3:1, the attack never halts.** In 40 games out of 40
the attack takes its objective. The fight is in town: first sightings come
at 36–156 m, between buildings. At 1.5:1 it still takes the objective in 38
games out of 40. The vignette therefore plays the attack at 1:1.

**At 1:1 (100 games):**

- In 25 games the attack halts, at a median of 37.5% losses (range 25–44%).
  The P14 band is 10–30%.
- In 75 games it takes the objective, though the attackers are not
  stronger than the defence in town.
- On generated ground (`attack-halt`), the attack halts at a median of 33%.
  This branch's sight-line and cover changes moved that up from 25%.

So on both grounds attacks press on past the losses at which P14 says real
attacks stop, and on real ground they succeed too often at even odds. The
HQ is set to halt the attack at 20% losses (`decide.heuristic.culmination_loss`,
P14). Losses come in lumps, though: one volley can knock out a whole
platoon, about 19% of the company, before the halt takes effect. These
settings are worth a look next:

- the HQ's culmination check and how quickly it acts;
- the defence's strength in town;
- the morale settings that hold attacking units in the fight.

**Feature cover changes nothing here.** With every `sim.cover` setting set
to 0, the attack halts in 26 games out of 100 instead of 25, at the same
median of 37.5% losses (Kolmogorov–Smirnov p 0.69, Mann–Whitney p 0.39).
The town fight is decided at close range between buildings, where the
town blocks' own cover (0.6–0.9) already outweighs anything the features
add. Feature cover should matter in open country with hedgerows and tree
lines. A test scenario there, for example east of Kaliningrad, would
measure it.

## Fixed after the real-ground runs

Watching single games of the 1:1 attack showed two faults behind those
results.

**1. Generated objectives missed the defence.**
- A company defence is three elements on a 1,200 m front: platoons at
  ±600 m, the anti-tank team in the middle.
- The objective was a 600 m circle, so the flank platoons stood 608 m
  from its centre, just outside it.
- Blue "held the objective with no enemy on it" after beating only the
  middle element. In half the games, blue walked in after 13 minutes
  without a fight.
- A battalion attack was worse: a 2,800 m front under a 900 m circle.
- The objective now takes in the whole front: radius = half the front
  plus 200 m (800 m for a company, 1,600 m for a battalion). The
  generated scenarios in `data/scenarios/gen-*.json` were regenerated.
- The vignettes now count an attack as successful when blue is on the
  objective and no defender is, as the victory rule has it.

**2. The HQ halted late, or not at all.**
- Losses were counted over all of the HQ's units, mortars and command
  post included. Those rarely lose anything, so 23% losses among the
  fighting platoons could read as under 20%.
- Even when the check fired, the halt came only with the hasty plan,
  10–30 minutes later (`c2.hasty.company_h`). That new plan then
  re-attacked: it judged the odds 5:1, because the HQ had seen only part
  of the defence.
- Now both the HQ's loss check and its planner's culmination test count
  losses among manoeuvre units only.
- When the check fires, the HQ orders its advancing units to go firm
  (stop and watch) at once, by radio (`c2.go_firm` in the log), and plans
  while they hold.

**Results:**

| Vignette | Before | After | Halted |
|---|---|---|---|
| attack-halt-real-ground (town, 1:1) | 37.5% | 31% | 26 of 60 (was 22) |
| attack-halt-open-country (new, 1:1) | — | 19% (in band) | 27 of 60 |
| attack-halt (generated, no HQ planner) | 25% | 25% | unchanged |

- **Town:** five games now halt at 19%. The rest overshoot, because losses
  come in lumps and reports reach the HQ late.
- **Open country:** farmland broken by tree lines south-west of the city,
  at 54.606 N, 20.398 E.

## Feature cover in open country

Packs built before tree cover went into the simulation had only
OpenStreetMap's woods on their 5 m sight surface. Hedgerows, tree lines
and garden trees were missing. `bgws basemap <pack> --tree-cover-only`
now adds WorldCover's trees to an existing pack's surface as light
woodland, where a cell has no building and no mapped wood. It was run on
`kaliningrad-big`.

Even with those trees in, switching feature cover off does not change
the open-country attack. Over 100 games each:

| Variant | Median losses at halt | Halted |
|---|---|---|
| Feature cover on | 15.4% | 43 |
| Feature cover off | 17.1% | 42 |

Kolmogorov–Smirnov p 0.89, Mann–Whitney p 0.98. So neither town nor
country shows the features in this outcome. Either units seldom stand on
them, or a dug-in defence's cover outweighs them. Telling the two apart
needs a trace of where units stand and what cover they get.

## The search model: sourced and in (Howe 1993, TETAM)

Volume 4 of the *IR/EO Systems Handbook* (Howe, pp. 110–114) and the TETAM
paper (Bryson) were read in the original. The notes are in
`docs/papers/notes/`.

**What changed:**
- Detection levels off at P∞, drawn once per pair and position
  (eq. 2.33). The rate for an observer that will find the target is 1/τ.
- τ for one field of view is 3.4/P∞, or 6.8·N50/N when P∞ ≥ 0.9
  (eq. 2.38). Howe's worked example reproduces: 2.65 s and 4.52 s.
- `fov_exponent` goes from 0.5 to 1.0: n fields of view take n times as
  long (eq. 2.40).
- The search detection N50 is scaled by clutter at the target: ×0.5 low,
  ×2 high (Table 2.5).
- `sensing.acquire.tau_base_s` (8 s, unsourced) is gone.

**Vignettes, 60 games each, before → after:**

| Vignette | Before | After |
|---|---|---|
| Attack halt (generated) | 25% | 33%, out of band (200 games, interval 25–33%) |
| Attack halt (real ground) | 25% | 19% |
| Attack halt (open country) | 19% | 25% |
| ATGM engagement | 0.73 | 0.70 |
| Spot report latency | 2.04 | 1.79 min |
| Detect-to-strike (networked) | 3.42 | 3.20 min |

Every other vignette is unchanged.

**The generated attack-halt result needs explaining.** Its scenario has no
HQ planner, so only units' own morale halts the attack. With slower,
capped search, the defence finds the attackers later and the attack gets
further before morale breaks. Giving that scenario a company HQ (as the
generated `gen-*` attacks have) would test the HQ's halt there too.

**TETAM check:** first sighting to first shot is a median 7 s (quartiles
4.5–15 s) over 60 shots in `gen-company-attack`,
`gen-meeting-engagement` and `gen-company-defence`. TETAM's anti-tank
missile crews took 8–11 s to engage after detecting. The lay time
(`sim.fire.lay_min_s`, 6 s minimum) stands.

**Still unsourced:**
- `cue_factor`;
- `fov_ref_deg`;
- the observer multipliers;
- the clutter mapping from land cover.

ADA447823, the cueing report, could not be found. The file added as
"cueing" is a 2024 laboratory study of contextual cueing (Su, Zhao & Ma,
*Frontiers in Psychology*), not a field source. It is not used.

## Why feature cover does not move the attack (21 November)

**1. The generated attack-halt test now has an HQ.** It plays
`gen-company-attack` at 1:1 on generated ground, with its company HQ, as
the two Kaliningrad tests do. It replaces the skirmish preset, which had
none. Losses at the halt are a median of 38.5% over 100 games, still out
of band (10–30%). The HQ's halt works; the losses come before it can act.
In most seeds a single dug-in ATGM team destroys blue's four-tank platoon
at 1.7–1.9 km, within one to two minutes of first contact, in about three
two-missile volleys. Four tanks are 31% of the company, so it is past 20%
before the HQ's 10-second check and its radio order to go firm.

**2. A trace of where units stand and what cover they get** (6 games each,
sampled every 30 s for units within 2.5 km of an enemy):

| Ground | Attacker cover: class draw → final | Halted attackers masked | Defender cover: class → final |
|---|---|---|---|
| Open country (Kaliningrad) | 0.20 → 0.29 (folds 45%, trees 31%, rail 14%) | 16% | 0.35 → 0.36 |
| Town (Kaliningrad) | 0.44 → 0.63 (walls 35%, trees 88%) | 43% | 0.48 → 0.55 |
| Generated | 0.42 → 0.42 (folds only, shallow) | 0% | 0.20 → 0.21 |

The features do reach the attackers. Defenders gain little: their land
cover and digging in already give more.

**3. Two reasons this did not move outcomes:**
- **Vehicles ignored ground cover.** Direct fire at a vehicle used only
  its posture: hull-down after settling, or in a firing position chosen
  for it. A tank halted behind a railway bank or in a fold was fully
  exposed. Now a halted vehicle counts as hull-down at once behind
  features that mask its hull (`sim.cover.hull_mask`, 0.4 of cover from
  walls, railway banks and folds). Trees and hedges don't count: they
  hide a vehicle but do not stop rounds.
- **Most losses happen on the move,** where no cover helps. Over 12
  games each:

  | Ground | Lost moving | Lost halted | Halted and masked or hull-down |
  |---|---|---|---|
  | Open country | 31 | 10 | 4 |
  | Town | 20 | 13 | 13 |
  | Generated | 65 | 4 | 0 |

  Even with masking, switching feature cover off makes no detectable
  difference to losses at the halt (open country p 0.28, town p 0.45,
  generated p 0.97; 100 games each).

**What decides these fights, and so what to look at next:**
- how attackers cross ground the defence can see: bounding with
  overwatch, smoke, and routes through dead ground;
- how quickly a dug-in missile team kills a platoon that keeps coming:
  its volley rate, and the tanks going to ground when fired on from
  beyond their reach.

## Crossing observed ground, and the missile team's kill rate (21 November)

**One game, second by second** (`gen-company-attack` at 1:1, seed 1):
- 418 s: the red anti-tank team launches two missiles at the tank
  platoon, 1.8 km off. The tanks fire their smoke dischargers.
- 429 s: both missiles hit. The platoon goes Steady → Shaken → Pinned →
  Broken → Routed in four ticks (losing half its tanks at once is a burst
  of stress that makes each next step near-certain). Its leader is hit, so
  it can take no new orders for 120 s.
- It then sits in the open, or runs in a straight line away. The team
  finishes it 3½ minutes later, still at 1.9 km.

**The missile team's rate is not the problem.** It is two firing posts
(`generic/atgmTeam`, count 2), each firing one missile a volley, about 30 s
apart: a launcher's reload plus about 10 s of flight. That is close to real
practice. TETAM's crews took 8–11 s from detection to engagement.

**What changed: a break-contact drill.**
- **A vehicle that notices a missile launched at it** fires its smoke, as
  before, and now also drives to the nearest ground the launcher cannot
  see, within `sim.react.break_contact_m` (400 m). It searches 25 m rings,
  straight back before the sides (`Ground::dead_ground`).
- **A vehicle hit by a missile** from a launcher it cannot see does the
  same; the hit shows where it came from.
- **A broken or routed unit** runs to that dead ground instead of in a
  straight line.
- **A noticed launch counts as fire from the launcher,** so a unit that is
  still steady calls smoke on it.
- **It's a drill:** it needs no orders. Until the unit reaches the dead
  ground, decisions and late answers don't send it back.
- **The chance of noticing a launch** (`smoke.vehicle.warning_p`) is now
  0.35, from TETAM Phase IIA: armour crews detected 18–47% of simulated
  launches. It was 0.5, unsourced.

**What it can and can't do:**
- Retreating tanks make about 3–4 m/s.
- On the generated farmland the nearest dead ground from 1.8 km is about
  400 m away, too far to reach within a 10 s missile flight.
- TETAM's 70 m median to break line of sight was on rolling hills
  (Hunter Liggett).
- So the drill saves tanks where there is ground to use, not on open
  plains.

**Effect, 100 games each, losses at the halt:**

| Test | Drill off | Drill on | Halted (off → on) |
|---|---|---|---|
| Open country | 31% (out of band) | 23% (in band) | 40 → 56 |
| Generated | 38.5% | 34% | 84 → 84 |
| Kaliningrad town | 19% | 25% (p 0.02) | 55 → 66 |

It helps in open country and costs a little in town, where vehicles
pulling back are exposed in the streets.

**Tried and switched off: approach routes that avoid the objective's
view.** A route request can now carry the position its destination
overlooks (`overlook`): ground the objective sees costs as exposure, its
dead ground does not. At weight 0.6, attacks lost more before halting:

| Test | Without | With |
|---|---|---|
| Real ground | 25% | 37.5% |
| Open country | 23% | 31% |

Both differences are significant (p 0.04 and 0.02). Routing round what the
objective's centre sees takes longer detours, and the flank platoons still
overlook them. It stays in the code at weight 0
(`sim.route.overlook_weight`). A version that takes in what the whole
defended front sees, or the HQ's own terrain appreciation, may do better.

**Still open:**
- **The morale cascade:** four states in four ticks after a lumped loss.
  A minimum time in each state, or stress that builds over seconds, would
  stop it.
- **Bounding with overwatch at small odds:** at 1:1 the plan gives no unit
  the support-by-fire task, so every unit moves at once.

## Morale in steps, and covering fire at even odds (21 November)

**Morale takes time to collapse.** A unit now stays in a morale state at
least `people.morale.min_dwell_s` (20 s, unsourced) before it can get
worse. Recovery is not held back.

- **Before:** a tank platoon that lost two of four tanks to one volley went
  Steady → Shaken → Pinned → Broken → Routed in four ticks. The loss is a
  burst of stress that makes every next step near-certain.
- **Now:** the same collapse takes 60–75 s, and in one seed of four the
  platoon steadies once before breaking.
- **Losses at the halt don't change** (100 games each, p 0.46–0.92). The
  missiles destroy the tanks whatever their morale. This is a realism fix
  for what units do in the meantime: firing back, taking orders, the HQ
  pulling them out.

**Covering fire with three platoons.** The planner used to hold one of a
company's three platoons back as the reserve. That left two to lead,
fewer than the three needed for one to give fire, so both went in with no
one covering them. Now a reserve is held only if enough are left to lead
with one giving fire. With three platoons, one supports by fire and two
assault; with four or more, there is still a reserve. This reverses an
earlier choice that three were "too few to split", in favour of the
doctrinal company attack.

Losses at the halt, 100 games each, against the run before (break-contact
drill on, neither change):

| Test | Before | After | Attacks halted |
|---|---|---|---|
| Open country | 23% | 15% | 56 → 67 |
| Generated | 34% | 38.5% | 84 → 94 |
| Kaliningrad town | 25% | 31% (out of band) | 66 → 45 |

The effect is mixed, as the doctrine would expect. In town, more attacks
now take the objective (55 of 100, from 34). In open country they halt
sooner and at lower losses. On generated ground the covering platoon,
standing off at 1.8 km, still meets the anti-tank team it can't
out-range.

**A mine strike can break a tank platoon out of contact.** In the breach
test (seed 6), one tank in four lost to a mine, with no enemy near, takes
the platoon from Steady to Routed (now over a minute). It stays Broken or
Routed for 40 minutes and never uses the lane the engineers cleared. The
test now excuses a platoon that broke, as it already excused one left
immobilised.

Whether a quarter of a platoon lost to one mine should break it is a
calibration question. It turns on `people.stress.k_casualty` (3.0, tied
to the 20% attack halt), and on slow recovery out of contact
(`people.stress.recover_tau_s`, an hour).

**What still decides the generated attack:** a dug-in missile team on
open farmland with no dead ground near. The losses there come before
morale, covering fire or the HQ's halt can act. Indirect fire on the
launcher once it has fired (counter-fire from the company's mortars),
not yet in the plan, is the next thing to try.

## Mortar counter-fire on the launcher (21 November)

**Why blue never fired on the missile team.** In every traced game, all
the calls for fire were red's. There were three reasons:
- **Only fully steady units called for fire.** After the first missile
  hit, the tanks were Shaken. Now shaken and pinned units call too; only
  broken and routed ones don't.
- **A launcher had to be "seen" to be called on.** Now a noticed launch is
  a target report in itself: the unit calls fire on where the missile came
  from, with twice the usual aiming error, at priority 4 ("a missile
  launched at us from there"). The sim records each launcher's last launch
  position for this (`missiles_launched`).
- **The company's mortars were driving.** They start 6.6 km from the
  objective, within their 7 km range. The planner still sent them forward
  ("move up into range", since they were beyond 70% of their range): 13
  minutes on the road, out of action through the first contact at 7
  minutes. Fire units now move up only if they cannot reach the objective
  at all (`decide.heuristic.fires_move_up` 0.7 → 1.0).

**What happens now.** In seed 1 the tanks call fire on the launcher one
second after it launches (430 s). The battery's first round lands 95 s
later, and it is smoke for an earlier screen request (missions go in
turn). High explosive lands at 670 s, after the tank platoon was
destroyed at 532 s, and knocks nothing out. An unplanned call takes
minutes; a dug-in missile team kills a platoon in one or two. Counter-fire
cannot save the platoon that draws the fire. At most it suppresses the
team for the next one.

**Losses at the halt, 100 games each:**

| Test | Before | After |
|---|---|---|
| Kaliningrad town | 31% (out of band) | 25% (in band) |
| Open country | 15% (in band) | 15% (in band) |
| Generated | 38.5% (out of band) | 38.5% (out of band) |

Keeping the mortars in action accounts for most of the town change
(31% → 25%, with 65 attacks halted against 44), though that comparison is
not significant on its own (Mann–Whitney p 0.33).

**What would help next: planned fire on the objective.** Doctrine fires on
the objective and its likely missile positions while the assault closes,
not only on call. The heuristic plan gives its mortars only "answer calls
for fire". A preparatory and covering fire plan would suppress the
missile team before it can fire: rounds on the objective timed to the
lead platoons crossing into missile range.

## Naked-eye search

Leads are in `docs/papers/notes/naked-eye-leads.md`. The best is Grau
(2022), on Soviet trials: an unaided eye finds a tank against forest in
3–7 s at 0.5–2.5 km. That is consistent with what the model does now.
Each was seen only in search snippets, so none is used.

## Superseded: the search model before the sources were read



The primary sources are NVESD and ACQUIRE reports on DTIC, plus SPIE
papers. Search snippets and recollection point to the following, none of
it read in the original yet:

- **Detection over time levels off at P∞:**
  P(t) = P∞·(1 − e^(−t/τ)), with τ = 3.4/P∞ s per field of view (Howe,
  *IR & EO Systems Handbook* vol. 4, 1993, DTIC ADA364024). Our model's
  detection rate is λ = −ln(1 − P∞)/τ. Its chance keeps rising towards 1
  however low P∞ is, so long searches overstate detection. Mid-range P∞
  gives similar early rates.
- **Search time over a sector:** t ≈ n_FOV × τ_FOV, that is, linear in
  area. That would make `fov_exponent` about 1 with `fov_ref_deg` the
  instrument's field of view, not the current 0.5.
- **Field check (TETAM, 1972, DTIC ADP000598):** target handoffs succeeded
  51–68% of the time, taking 18–31 s when they did.
- **Cueing and observer movement:** no ratio found.

DTIC turns away this environment's downloads, so none of this is
confirmed. Each needs reading in the original before any value changes.
