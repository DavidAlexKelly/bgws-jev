# Caviness, Maxey and McPherson (1972): target detection and range estimation

**Source:** James A. Caviness, Jeffery L. Maxey and James H. McPherson,
*Target Detection and Range Estimation*, HumRRO Technical Report 72-34,
Human Resources Research Organization, Division No. 4, Fort Benning,
November 1972. For the Army Small Arms Requirements Study (ASARS).
DTIC/NTIS AD-753 600. Approved for public release.

**File:** `raw/sensing/caviness-1972-target-detection-and-range-estimation.pdf`
(50 PDF pages, a scan with an OCR text layer of uneven quality). Read on
23 November 2026. Figures were checked against the page images where the
OCR was garbled.

**Table:** `data/caviness-1972-target-detection-and-range-estimation--detection-time-by-condition.csv`
(Table 3, p. 14 [PDF 24]).

## What it is

The full report of the field experiment that Maxey and Caviness (1972)
summarise. There were 90 enlisted men in three groups of 30, one group
per test site. Each man watched a 30° sector with a hand switch. A
**camouflaged soldier** started moving across his line of sight at 100,
200 or 300 m, at a walk (1.5 m/s), slow run (2.7 m/s) or fast run
(7.5 m/s). That gave 9 trials per man and 810 in all. Time was counted
from when the target **began moving**, so it includes the observer's
reaction. The observers also estimated range.

Terrain complexity was rated by 36 other men on a 1–7 scale: low 2.72,
medium 4.31, high 4.94 (Table 1, p. 10 [20]).

## Figures (Table 3, p. 14 [24])

| Condition | Detected | Mean time of those detected (s) | SD (s) |
|---|---|---|---|
| All | 79.1% | 3.8 | 4.4 |
| Low complexity (grass, few small pines) | 95.6% | 2.1 | 1.7 |
| Medium complexity | 87.8% | 5.4 | 6.0 |
| High complexity (heavy brush, pine and hardwood) | 54.1% | 4.1 | 3.5 |
| 100 m | 90.4% | 2.3 | 1.8 |
| 200 m | 93.3% | 3.5 | 3.8 |
| 300 m | 53.7% | 6.6 | 6.5 |
| Walk | 80.0% | 5.4 | 6.4 |
| Slow run | 81.1% | 3.5 | 2.9 |
| Fast run | 76.3% | 2.4 | 1.9 |

The OCR shows the walk row's detections as 216 and 89.0%. 216 of 270 is
80.0%, which the text on p. 13 also gives. The CSV uses 80.0%.

- **Nobody** detected a target at 300 m in high-complexity terrain
  (Maxey & Caviness 1972, p. 2).
- Exposure times ran from 2 s (fast run, short path) to 82 s (walk, long
  path at 300 m) (Table 4, p. 15 [25]). A man who had not found the target
  when it left the area counted as a miss.
- The negative-exponential (constant-rate) model was rejected in 22 of
  24 conditions at α = 0.10, and in 13 of 24 at α = 0.01 (Table 6,
  p. 20 [30]).
- Range estimation: mean absolute error 59.6 m (SD 77.4 m) at 100–300 m
  (p. 10 [20]).

## How it compares with the model (not applied)

`engine/detection.ts` finds a moving target on foot at a rate of
0.1/s × (1 km / r)² × 3 (moving) × 0.3 (on foot), × 0.3 more in cover.
Inside 150 m (`detection.CERTAIN_M`) detection is certain. With those
figures:

| Range | Model, open: mean (P within 10 s) | Field, low complexity | Model, in cover | Field, high complexity |
|---|---|---|---|---|
| 200 m | 0.4 s (1.00) | 2.1 s, 96% (all ranges) | 1.5 s (1.00) | 4.1 s, 54% (all ranges) |
| 300 m | 1.0 s (1.00) | — | 3.3 s (0.95) | **0%** |

Two differences:

1. **The model is about 3–5 times too fast.** That is partly the field
   test's reaction time and partly the model's rate.
2. **The model has no ceiling on the chance of detection.** In the field
   about half the targets in heavy cover were never found, and none at
   300 m. The model finds 95% within 10 s.

Both would change many scenarios, so no figure is changed. This is now a
calibration target (`research/calibration-targets.md`). The ceiling is the
same open question as P∞ in `leads/naked-eye-search.md`.

## Limits

- The targets were single men, camouflaged, at 100–300 m, against Fort
  Benning woodland. These are not vehicles, and not long range.
- The observers were alerted, faced the right 30° sector, and knew a
  target would come.
- Detection time counts from the start of movement, so it includes the
  time to turn round and the reaction.
