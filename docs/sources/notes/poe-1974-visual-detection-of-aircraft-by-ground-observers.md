# Poe (1974): a glimpse model for naked-eye detection of aircraft

**Source:** Arthur C. Poe III, *A Model for Visual Detection of Aircraft
by Ground Observers*. Technical report RD-75-30, US Army Missile Command
(Missile R&D Laboratory), Redstone Arsenal, Alabama, October 1974.
DTIC ADA017599. Approved for public release.

**File:** `raw/sensing/poe-1974-visual-detection-of-aircraft-by-ground-observers.pdf`
(38 pages; a scan with a DD 1473 form in front). Read on 22 November 2026.
Page numbers below are the report's printed numbers; the PDF page is
**printed + 2** (printed p. 13 is PDF p. 15). The OCR mangles Table 3 and
several numbers; the values here were checked against the page images.

**Tables:** `data/poe-1974-visual-detection-of-aircraft-by-ground-observers--threshold-contrast.csv`,
`--sky-search-scans.csv`, `--amtoc-registration-runs.csv`.

## What it is

A physical, glimpse-by-glimpse model of trained soldiers searching an
assigned sector of sky with the **unaided eye** for a single aircraft
(pp. 3, 10). Every 1/3 s it works out each observer's chance of seeing
the target on that glimpse, from the target's apparent contrast and
angular size and how far it sits from where the observer is looking. It
then cumulates over time and averages over observers. It was fitted to
the 1970 AMTOC-1 (Redeye) field trial at Yuma (p. 22).

This is the kind of model the NVESD search model leaves out: Howe (1993)
says that model does not apply to the unaided eye
(`irhb-1993-v4-eo-systems-design-analysis-testing.md`).

## The model, step by step

**1. Glimpses (pp. 6–10).**

- **"Average glimpse rate is in the neighborhood of 3/sec"** (p. 10). The
  model's time step is 1/3 s (p. 10).
- About 85% of search time is spent fixating, the rest moving between
  fixations; mean glimpse about 0.28 s (p. 10, from Ford et al.).
- Lab fixation means quoted: 0.15 s (Cobb and Moss), 0.28 s (Ford et al.,
  empty 30° screen), 0.37 s (White and Ford, radar screen), 0.31–0.58 s
  (Enoch, map reading; longer for smaller maps) (pp. 6–7, Table 1).
- One observer on a blank screen: 171 gaze shifts in 57 s, so 0.33 s per
  glimpse (pp. 7–8, Sugarman et al.).
- The scan pattern is a back-and-forth sweep along two lines, 2.5° and
  7.5° above the horizon, in a 10° tall sector (p. 10). The step between
  glimpses grows with the sector (Fig. 2, p. 9; Table 2 below).
- Observers start at random points: the model averages one observer per
  5° square of the sector (36 for a 90° × 10° sector) (p. 10).

**Table 2 (p. 8): soldiers scanning desert sky (AMTOC videotapes,
3 glimpses a second assumed)**

| Sector (azimuth) | Mean full scan | Glimpses per scan | Step between glimpses |
|---|---|---|---|
| 30° | 2.72 s (1.75–4.50) | 8.2 | 3.7° |
| 60° | 2.99 s (2.00–4.75) | 9.0 | 6.7° |
| 120° | 3.94 s (2.50–6.00) | 11.8 | 10.2° |

So a sector four times as wide took only about 1.4 times as long to
sweep: observers stretch their steps rather than slow down. Poe warns
against using Fig. 2 below about 0.07 sr or above about 0.4 sr (p. 10).

**2. Apparent contrast (pp. 11–12).** Intrinsic contrast
C0 = |Lt − Lb| / Lb. Through haze, **C = C0 · exp(−k·R / V)**, with V the
meteorological visibility and k = 3.912 (or 3.44, depending on how V was
defined) (p. 12). Typical aircraft C0 is 0.05 to 0.75 (p. 11). Poe calls
the choice of C0 "the weakest point in the model": it is an input, and
results swing widely with it (p. 11).

**3. Angular size (p. 18).** α = 3.88 · √Ap / R, α in minutes of arc,
Ap the presented area in m², R in km. (3.88 is the diameter of a circle
of area Ap, in arcmin at 1 km.)

**4. Threshold contrast Ct (p. 13, Table 3).** Ct is the contrast seen
50% of the time, for daylight adaptation of 75–100 ft-L and a 1/3 s
exposure (p. 12). It depends on α and on the **peripheral angle λ**
(degrees between the gaze and the target):

- foveal: Ct0(α) = a0 / α^b0;
- slope in the near periphery: Ct1(α) = a1 / α^b1;
- the coefficients change in steps with α (in the CSV);
- λ ≤ 0.6°: Ct = Ct0(α);
- 0.6° < λ ≤ 15°: **Ct = Ct0(α) + (λ − 0.6) · Ct1(α)**, linear in λ;
- λ > 15°, α ≤ 9.1′: Ct = [Ct0 + 14.4·Ct1] · exp[0.000643 (λ² − 225)];
- λ > 15°, α > 9.1′: Ct = [Ct0 + 14.4·Ct1] · (690/α)^[0.0001486 (λ² − 225)].

(The two far-periphery forms agree at α = 9.1′, a check on the reading.)
Valid for 0 < α ≤ 120′ and 0 ≤ λ ≤ 90°, horizontal meridian only (p. 13).
Sources: Blackwell and McCready (fovea), Taylor (5–12.5°), Sloan's trends
(15–90°) (pp. 12–13). The originators claim accuracy only to a factor of
about 3 (p. 13); Poe thinks it is better, and notes a 20% change in Ct
over part of the space can move predicted detection by 5 s (p. 16).

**5. Glimpse probability (pp. 16–18).** Contrast-at-detection over Ct is
taken as normal with mean 1 and standard deviation σ:

**Pg = Φ( (C/Ct − 1) / σ ), with σ = 0.32** (p. 18).

Blackwell's eleven studies gave σ from 0.28 to 0.67, grand means 0.30 to
0.52 (p. 16, Table 4). Poe found 0.32 fitted AMTOC best (p. 17).
Blackwell preferred a larger σ with a skewed lower tail (p. 17). The
choice matters: at C/Ct = 0.2, Pg is 0.0045 with σ = 0.3 but 0.09 with
σ = 0.6; after 30 glimpses (10 s) that is 0.13 against 0.94 (p. 17). Most
AMTOC detections happened where 0.005 < Pg < 0.10 (p. 17).

**6. Cumulating (p. 10).** P_cum(t) = P_cum(t−1) + (1 − P_cum(t−1)) · Pg,
per observer, then averaged over observers. There is no P∞ term: every
observer can in principle find the target; the curve flattens only
because Pg falls as the target leaves, shrinks or loses contrast.

## Validation (pp. 22–33)

- AMTOC-1, June 1970, Yuma: about 250 runs, 20–30 observers each, F-8
  aircraft (p. 22). Sectors 30°, 60° and 120° by about 10°; unmask ranges
  5, 10 and 15 km; offsets 0, 1.5 and 3 km (p. 22).
- Poe kept 42 straight-and-level runs and dropped runs where too many
  observers said they were cued by sound, glint or smoke (p. 22). Ten are
  shown (Table 5, p. 23; Figs. 6–15, pp. 24–33).
- **Contrast was a free parameter** fitted run by run, because it was not
  measured in flight (pp. 22–23). Mean fitted C0 over the 42 runs:
  **0.22 against sky, 0.15 against terrain** (p. 23). The aircraft were
  mostly white (p. 23).
- Agreement is "quite good" (p. 23), which with a free contrast is not a
  strong test. In some runs the model levels off well below the data
  (run 109: model about 0.42, observed 0.65 at 11 s; Fig. 6, p. 24).
- Typical times in the shown runs: cumulative detection of 0.4–0.7 within
  10–25 s of unmask at 5–10 km (Figs. 6–15).

## Other findings worth keeping (pp. 19–22)

- **Structured backgrounds are out of scope.** Against unstructured
  terrain (distant mountains, flat sand, hazy ground) use the same model
  with lower contrast; "if the terrain is structured … the model is no
  longer applicable" (p. 19; also p. 6).
- Peripheral cues: smoke, glint or the target itself can catch attention
  up to **80° off the line of gaze** in a featureless field (p. 6).
- Smoke trails can be treated as detection of the aircraft (p. 19).
- Angular motion did not help detection against an unstructured
  background below several degrees a second (residual regression on
  AMTOC, p. 20). In structured fields it probably does, but there were no
  data (p. 21).
- Two observers sharing one 120° sector or splitting it into two 60°
  sectors gave almost the same chance that at least one detects (p. 22;
  a model result, not a trial).
- Threshold contrast changes little across daylight levels (10 to
  10,000 ft-L); the model uses 75–100 ft-L throughout (p. 21).

## Does it apply to ground targets?

**Not as written.** It is built and fitted for aircraft against sky.
Three parts transfer; one does not.

- **Transfers:** the glimpse rate (3/s) and the lab threshold-contrast
  surface. The surface is about the eye, not the target: it gives the
  contrast needed for a small blob of size α at λ off-axis.
- **Transfers:** Koschmieder haze (C = C0·e^(−3.912R/V)).
- **Transfers with care:** the sky-scan data. A 10° band above the
  horizon is close to the band a ground observer searches below it.
- **Does not transfer:** the assumption of an unstructured background.
  Poe says so twice (pp. 6, 19). Ground targets sit in clutter, which
  slows scanning and adds confusers. Clutter is exactly what the NVESD
  model's N50 and P∞ carry, and Poe has nothing for it.

## Limits

- One aircraft, daylight, unstructured background, trained observers
  told where and when to search (p. 3).
- Sectors 10° tall, 20°–180° wide, centred within 25° of the horizon
  (p. 5).
- Contrast is an input with no good way to set it (p. 11); in the
  validation it was fitted, so the validation mainly tests the shape of
  the curves.
- The lab thresholds are for a 1/3 s exposure and good daylight; no
  night or twilight.
- σ = 0.32 is a fit, and Pg is very sensitive to it at low C/Ct (p. 17).
- The normal form gives Pg ≈ Φ(−1/0.32) ≈ 0.001 per glimpse even at
  zero contrast. Clamp it to zero below some contrast if used.
- Binoculars are not covered.

## How we could use it

**1. Confirm the glimpse time.** Poe's 3 glimpses a second (p. 10) is
independent support for Howe's t_f ≈ 0.3 s. Cite it beside Howe for
`sensing.acquire.tfov_k_s`; no change to the value.

**2. A naked-eye detection lobe.** The real gap is the eye's P∞ and τ.
Poe's Ct(α, λ) and Pg give, for a target of known size and contrast, how
far off the line of gaze it can be and still be seen on one glimpse.
Our own worked example (not in the source): tank front ~8 m² and a
standing man ~0.75 m² (both assumed), C0 = 0.15 (Poe's mean against
terrain), visibility 20 km. Pg per glimpse:

| Target | Range | α (′) | λ = 0° | 2° | 5° | 10° |
|---|---|---|---|---|---|---|
| Tank | 1 km | 11.0 | 1.00 | 1.00 | 1.00 | 0.93 |
| Tank | 2 km | 5.5 | 1.00 | 1.00 | 0.43 | 0.05 |
| Tank | 3 km | 3.7 | 1.00 | 0.57 | 0.03 | 0.01 |
| Tank | 5 km | 2.2 | 0.64 | 0.01 | 0 | 0 |
| Man | 0.5 km | 6.7 | 1.00 | 1.00 | 0.98 | 0.33 |
| Man | 1 km | 3.4 | 1.00 | 0.86 | 0.07 | 0.01 |
| Man | 2 km | 1.7 | 0.72 | 0.02 | 0 | 0 |

The radius λ at which Pg is about 0.5 is the eye's useful search lobe.
A sector of area A searched with a lobe of area a at 3 glimpses a second
gives roughly **τ ≈ (A / a) × 0.33 s**. For the tank at 2 km
(lobe radius ~5°, about 80 deg²) in a 60° × 10° sector that is about
2.5 s. That is the same order as Grau's Soviet figures (a tank against
forest seen in 3–7 s at 0.5–2.5 km; `leads/naked-eye-search.md`).

This gives τ for the eye. It does **not** give P∞ in clutter: on an
unstructured background Poe's model predicts near-certain detection of a
tank at 3 km, which no one expects among trees. So: use Poe for τ, keep
the NVESD P∞ with its clutter N50, and treat the result as an estimate
until field data (Maxey and Caviness; Grau) arrive.

**3. Peripheral cueing.** The steep fall of Pg with λ is the mechanism
behind cueing: a cue that puts the gaze within a few degrees of the
target turns a Pg of 0.05 into 1.0. That is a physical basis for
`sensing.acquire.cue_factor` (see `yeh-2001-head-up-head-down-cue-effectiveness.md`
for measured times).

**4. Air targets.** If the sim models helicopters or drones seen against
sky by infantry, this model applies almost as written, with C0 about
0.2 against sky.
