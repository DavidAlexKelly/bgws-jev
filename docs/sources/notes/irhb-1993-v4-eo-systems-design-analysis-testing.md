# Howe (1993): the NVESD search model

**Source:** James D. Howe, "Electro-Optical Imaging System Performance
Prediction", chapter 2 of *The Infrared & Electro-Optical Systems
Handbook*, vol. 4 (Electro-Optical Systems Design, Analysis, and Testing),
ed. M. C. Dudzik. ERIM / SPIE, 1993. DTIC ADA364024.

**File:** `docs/sources/raw/sensing/irhb-1993-v4-eo-systems-design-analysis-testing.pdf`

| Topic | Book page | PDF page |
|---|---|---|
| §2.6.3 "Search Modeling" begins | 109 | |
| The NVESD search model | 110–114 | 123–127 |
| Table 2.5 (clutter) | 112 | 125 |
| Physical (field-of-regard) search | 113–114 | 126–127 |

Read in the original on 20 November 2026. The equations below are retyped.
Text extraction garbles them, so check against the scan.

## What the model is

- **Status (p. 110):** "developed at the Night Vision and Electronic Sensors
  Directorate (NVESD) and is presently the most commonly used search model
  for military applications … used widely within the U.S. Army community
  as an important component of wargames."
- **Validation (p. 110):** "Most of the field tests that serve as validation
  for the model employed thermal imagers searching for ground targets."
- **Limits (p. 113):**
  - "In its present form the model is not applicable to search with the
    unaided eye, or even with optically aided eyes (using binoculars, for
    example)."
  - The field of view is not an explicit parameter of display search.
  - Moving targets and target groups are open questions.
  - Cued detection is only referenced (ref. 119), with no numbers.

## Display search (one field of view)

The observer makes random, independent glimpses, about 0.3 s per fixation
(t_f). Each glimpse finds the target with probability P_o.

- **Eq. 2.32 (one observer):** P(t) = 1 − exp(−t/τ_FOV), where
  τ_FOV = t_f / P_o.
- **Eq. 2.33 (an ensemble of observers):**

  **P(t) = P∞ · [1 − exp(−t/τ_FOV)]**

  P∞ is "the portion of observers in the ensemble that can find the target
  given unlimited time". So P(t) **levels off at P∞**, not at 1.
- **Eq. 2.34 / 2.37:** P∞ = (N/(N50)_D)^E / [1 + (N/(N50)_D)^E], with
  E = 2.7 + 0.7·N/(N50)_D. N is the resolvable cycles across the target.
- **Eq. 2.35 / 2.38:** "After analysis of field test results, the single
  glimpse probability of search detection, P_o, was proportional to P∞."
  - P_o / t_f = 1/τ_FOV = **P∞ / 3.4** for P∞ < 0.9 (low to medium clutter);
  - 1/τ_FOV = **N / (6.8 · (N50)_D)** for P∞ > 0.9.
- **Worked example (p. 114),** medium clutter, (N50)_D = 1:
  - N = 2.56 cycles gives P∞ = 0.99 and τ_FOV = 2.65 s;
  - N = 1.34 gives P∞ = 0.74 and τ_FOV = 4.52 s.

  The code reproduces both (`acquire.rs`, test `howe_worked_example`).

## Physical search (a field of regard wider than the sensor)

- **Eq. 2.40:** **P(t) = P∞ · [1 − exp(−t / (n · τ_FOV))]**, where
  n = FOR/FOV is "the number of device FOVs in the search FOR". It
  applies "for the case where the target is not found quickly".
- **Worked example:** a 12 deg² search field is 5 FOVs. The ratio is by
  area, so for a band of fixed height, time grows linearly with the
  sector's width.

## Table 2.5: cycles for 50% acquisition in search, (N50)_D (1-D model)

| Clutter | Example | (N50)_D |
|---|---|---|
| Highly conspicuous | bright source, movement, zero clutter | < 0.5 |
| Low | target in field, on road | 0.5 |
| Medium | tank in a desert of tank-size bushes | 1.0 |
| High | vehicle in array of similar vehicles | 2.0 |

"These values of (N50)_D are appropriate for use with the 1-D static
performance model … the N50 values appropriate to the 2-D model were 0.75
times those for the 1-D case. This factor also should be applied to the
(N50)_D values" (p. 112).

## How BGWS uses it

| Figure | Value | From |
|---|---|---|
| `sensing.acquire.tfov_k_s` | 3.4 s | eq. 2.35/2.38 |
| `sensing.acquire.tfov_k_high_s` | 6.8 s | eq. 2.36/2.38 |
| `sensing.acquire.clutter_low` | ×0.5 | Table 2.5 (0.5 vs 1.0) |
| `sensing.acquire.clutter_high` | ×2.0 | Table 2.5 (2.0 vs 1.0) |
| `sensing.acquire.fov_exponent` | 1.0 | eq. 2.40 (n = FOR/FOV) |

- The catalogue's detection N50 (0.75, the 2-D medium value) is scaled by
  the clutter at the target's land cover:
  - fields, open, bare, water and wetland count as low;
  - everything else counts as medium.

  That mapping is ours, not Howe's. Woods and towns are not high: Howe's
  high is a vehicle among similar vehicles. What woods and buildings hide
  is already the target's concealment and the canopy on the sight line,
  and rating them high doubled the threshold. Infantry with eyes then
  could not see a tank in a wood at 520 m at all.
- Whether an observer ever finds a target (P∞) is drawn once for the pair
  where they stand, like a sight line. One that will find it does so at
  the rate 1/τ, with τ = τ_FOV · n · cue · observer.
- **Not sourced:**
  - `sensing.acquire.fov_ref_deg` (10°, a sight's wide field);
  - `sensing.acquire.cue_factor`;
  - the observer multipliers;
  - use with the naked eye, which Howe excludes.
