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
