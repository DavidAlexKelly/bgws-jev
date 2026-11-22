# Anderson and Littlefield (1994): long rods against armour-technology targets

**Source:** Charles E. Anderson Jr. and David L. Littlefield, *Pretest
Predictions of Long-Rod Interactions with Armor Technology Targets*.
Technical report, SwRI Report 5117/001 (SwRI Project 07-5117), Southwest
Research Institute, San Antonio TX, April 1994 (work November 1992 to
April 1994). Prepared for the US Army Research Office under contract
DAAL03-92-K-0001. Approved for public release. DTIC ADA281384.

**File:** `docs/sources/raw/ballistics/anderson-1994-pretest-predictions-long-rods-armor-targets.pdf`
(122 PDF pages; the report form says 112)

**Read:** 22 November 2026

**Data extracted:**

- `docs/sources/data/anderson-1994-pretest-predictions-long-rods-armor-targets--baseline-semi-infinite-rha.csv` (7 rows, Table 3)
- `docs/sources/data/anderson-1994-pretest-predictions-long-rods-armor-targets--witness-pack-penetration.csv` (36 rows, Tables 4, 8, 10 and 11)
- `docs/sources/data/anderson-1994-pretest-predictions-long-rods-armor-targets--residual-rod-rha-target.csv` (8 rows, Table 5)
- `docs/sources/data/anderson-1994-pretest-predictions-long-rods-armor-targets--short-rod-regression.csv` (5 rows, Table 6)
- `docs/sources/data/anderson-1994-pretest-predictions-long-rods-armor-targets--short-rod-cth.csv` (12 rows, Table 7)

All page numbers are PDF page numbers. PDF page = report page + 14
(report p. 1 is PDF p. 15).

## What the PDF is

A scan with **no text layer**. We rendered every page at 200 dpi and ran
Tesseract OCR on it (text kept out of the repository). Running text OCRs
well. Every table number in the CSVs was read from the page image, not the
OCR. The report says the original contains colour plates; the scan is grey.

## What it is

SwRI's predictions, made before the tests, of how far L/D 30 tungsten rods
would get through three "armor technology" targets, compared with the test
results afterwards. The tests were fired by the Institute for Advanced
Technology (University of Texas at Austin) at the Arnold Engineering
Development Center (p. 15). Predictions used the CTH hydrocode and two
analytical models (Walker–Anderson and MITI). The yardstick is the depth
the leftover rod makes in an RHA witness pack 7.62 cm behind each target
(p. 17).

### The rods (pp. 15–17, Table 1)

Constant kinetic energy, 2.88 MJ (about 42% of a full-size round; a 75%
replica scale), L/D 30, Teledyne Firth Sterling X27X (91% W, Ni-Co
matrix), 17.45 g/cm³, UTS 1.3 GPa, 10% elongation.

| Velocity (km/s) | Mass (g) | L (cm) | D (cm) |
|---|---|---|---|
| 1.775 ("super-ordnance") | 1830 | 49.4 | 1.65 |
| 2.60 ("hypervelocity") | 850 | 38.4 | 1.28 |

The two velocities were chosen to give about the same semi-infinite depth
for the same energy (p. 16, Fig. 2). 1.775 km/s is "higher than the muzzle
velocity of current field systems"; a laboratory double-travel 120 mm gun
almost reaches it (p. 16).

### The targets

| Target | Make-up | Page |
|---|---|---|
| RHA block | 45.72 cm of RHA (MIL-A-12560G Class 3; two 10.16 cm and two 12.70 cm plates welded), 0° | 27, Fig. 5 |
| Spaced high-hard steel | six 1.90 cm high-hard plates (MIL-A-45100D, about 500 BHN), 2.54 cm apart, at 65°; witness pack also at 65°. Line-of-sight steel 27.0 cm | 51, 83, Fig. 27 |
| Ceramic laminate | 5.71 cm RHA cover (300 BHN), four 99.5% Al2O3 elements totalling 30.48 cm, 11.43 cm RHA back plate, 0°. Total 47.62 cm | 87, Fig. 43 |

Areal densities (ours, with steel 7.85 and alumina 3.89 g/cm³ from
Table 9, p. 89): RHA block 359 g/cm²; ceramic laminate 253 g/cm²
(steel 135, ceramic 119); spaced plates 89 g/cm² normal to the plates,
212 g/cm² along the 65° shot line.

## Model inputs

**CTH** (pp. 19–20, Table 2): Johnson–Cook strength for the rod and the
steels. RHA was modelled as 4340 steel scaled to 270 BHN; the high-hard
plates as 500 BHN; the ceramic target's cover as 300 BHN.

| Material | Y0 (GPa) | B (GPa) | n | C | m | Tm (K) | G (GPa) | ν | σfail (GPa) |
|---|---|---|---|---|---|---|---|---|---|
| Tungsten alloy | 1.35 | 0 | 0 | 0.006 | 1.00 | ∞ | 118.3 | 0.30 | 1.75 |
| 4340 steel (270 BHN) | 0.735 | 0.473 | 0.26 | 0.014 | 1.03 | 1793 | 77.7 | 0.29 | 1.42 |
| High-hard steel (500 BHN) | 1.36 | 0.517 | 0.22 | 0.003 | 1.17 | 1778 | 77.7 | 0.29 | 2.00 |
| 4340 steel (300 BHN) | 0.844 | 0.543 | 0.26 | 0.014 | 1.03 | 1793 | 77.7 | 0.29 | 1.42 |

The alumina used the Johnson–Holmquist model with AD-85 constants adjusted
to 99.5% alumina (density 3.89 g/cm³; Table 9, p. 89). The failed-ceramic
strength cap was tuned to 2.6 GPa against Woolsey's depth-of-penetration
tests (L/D 10, 65 g rods at 1.5 km/s; pp. 90–93).

**Walker–Anderson** (pp. 21–23, 36, 79–80, 94–95, 107–113):

- Rigid-plastic flow stresses: RHA 1.2 GPa, tungsten rod 1.5 GPa (p. 36);
  high-hard steel 1.8 GPa (p. 79); failed alumina 2.4 GPa (fitted to CTH)
  or 1.2 GPa (fitted to Woolsey's DOP tests and LANL PHERMEX X-ray
  data) (pp. 94–95).
- Target resistance in steady penetration: Rt = (7/3) ln(α) Yt (eq. 7,
  p. 22), where α is the ratio of plastic-zone radius to crater radius from
  compressible cavity expansion (eq. 6, p. 22).
- Breakout: once the plastic zone reaches the back face, α is run down to
  1 by α = α0 − (α0 − 1)[(x − (T − Rc(α0 − 1)))/(Rc(α0 − 1))]^n, with
  n = 2 fitted to Hohler and Stilp's residual length and velocity data
  (eq. 8, p. 22; footnote p. 23).
- Residual velocity after perforation from a momentum balance with a
  linear velocity profile over μD, μ ≈ 1.5–2.0 (eq. 4, p. 21).
- Oblique plates: line-of-sight thickness, each plate taken alone, and
  1.5 D or 1.8 D of rod nose thrown away after each plate to stand for the
  bent and broken nose seen by Hohler and Stilp (pp. 79–80).
- Post-test ceramic version: flow stress proportional to pressure,
  Y = βp, with β = 0.75 (fits 1.775 km/s) or 0.50 (fits 2.6 km/s)
  (pp. 108–113).

## Results

### Baseline: semi-infinite RHA (p. 24, Table 3)

| V (km/s) | P (mm) | P/L | Note |
|---|---|---|---|
| 1.85 | 546 | 1.10 | measured |
| 2.62 | 541 | 1.41 | measured |
| 1.775 | 510 | 1.03 | adjusted by eq. 10 (P/L slope 0.984 per km/s for L/D 30, 1.4–1.75 km/s) |
| 2.600 | 540 | 1.41 | adjusted by eq. 12 |

**Against our model:** the SwRI database fit (Anderson et al. 1992, eq. 7)
with ρp = 17.45 g/cm³, L/D 30, BHN 270 gives P/L 0.95 at 1.85 km/s and
1.40 at 2.62 km/s. So for this large (16.5 mm) L/D 30 rod **the fit is
14% low at 1.85 km/s** and right at 2.62 km/s. This is the opposite sign to
the 12–15% over-prediction for L/D 30+ seen against the model-scale data.

### RHA block (pp. 36–40, Tables 4–5)

| V (km/s) | Witness pack (cm) | Block + witness (cm) | Semi-infinite at this V (cm, from Table 3) |
|---|---|---|---|
| 1.80 | 6.7 | 52.4 | about 52.2 |
| 2.59 | 7.4 | 53.1 | about 54 |

- CTH, Walker–Anderson and MITI all predicted the witness depth within a
  millimetre or so (6.7–6.9 and 7.4–7.5 cm; p. 38).
- **The block plus the witness pack add up to the semi-infinite depth**
  within about 2%. A 45.7 cm block, about 0.9 of the semi-infinite depth,
  costs the rod no more than a semi-infinite target would (our
  comparison).
- Rods leaving the block (Table 5, p. 40): at 1.73–1.77 km/s, Vr/V 0.75–0.77
  and Lr = 7.7–8.2 cm (4.7–5.0 D, 16% of L); at 2.53–2.57 km/s, Vr/V 0.88
  and Lr = 4.4–4.9 cm (3.4–3.8 D). Repeat shots differ by 2–5% in Vr and
  6–11% in Lr. The models over-predict Vr and under-predict Lr at
  1.775 km/s; the errors cancel in the witness pack (p. 39).
- The rod "senses" the back face about 4.2–4.5 D (1.775 km/s) and 4.7 D
  (2.6 km/s) before it reaches it; penetration speeds up from there (CTH,
  p. 29).
- The authors note the analytical models do not fully capture the L/D
  effect (MITI has none; Walker–Anderson about half), so breakout and L/D
  errors must have cancelled (p. 36).

### Spaced, oblique high-hard plates (pp. 79–85, Table 8)

| V (km/s) | Witness pack (cm) | Pitch, yaw (°) |
|---|---|---|
| 1.72 | 14.4 | not known (X-ray fault) |
| 1.78 | 10.0 | 0.45, 2.34 |
| 2.60 | 10.8 | 0.04, 0.44 |
| 2.62 | not reported; rod tail broken off before the witness | −1.70, 2.62 |

Predictions (1.775 / 2.600 km/s): CTH corrected 18.0 / 14.1 cm;
Walker–Anderson 16.3 / 15.8 (1.5 D lost per plate) and 12.4 / 13.2 (1.8 D);
MITI 28.5 / 24.9 (about 100% too high) (p. 84).

- Total steel penetrated (27.0 cm line-of-sight plates plus witness):
  41.4 cm at 1.72 km/s and 37.8 cm at 2.60 km/s, against 52.4 and 53.1 cm
  for the RHA block. "Repeated impacts and breakouts, combined with target
  obliquity, have a detrimental effect on projectile performance" (p. 83).
- **The slower rod went about 40% deeper into the witness pack** than the
  faster one, although the faster one carries the same energy (p. 83).
  CTH shows the faster rod erodes more per plate (pp. 79–80): leaving the
  sixth plate, the 1.775 km/s rod is 19 cm long (11.5 D) at 1.60 km/s; the
  2.6 km/s rod is 9.6 cm (7.5 D) at 2.45 km/s.
- Each plate is only about 3 D thick in line of sight (2.7 D and 3.5 D for
  the two rods; pp. 52, 79).
- Yaw matters a great deal: critical yaw about 1.0° (1.775 km/s) and 1.7°
  (2.6 km/s); critical pitch 0.16–0.32° (pp. 84–85). The 1.78 km/s shot at
  2.3° yaw lost 4.4 cm of witness depth against the 1.72 km/s shot (p. 85).

### Ceramic laminate (pp. 98–105, 113, Tables 10–11)

| V (km/s) | Witness pack (cm) |
|---|---|
| 1.73 | 8.58 |
| 1.77 | 8.20 |
| 2.59 | 7.78 |
| 2.60 | 7.10 |

- The target was defeated at both velocities. Before the test, CTH and
  Walker–Anderson at 2.4 GPa said the 1.775 km/s rod would stop at the
  ceramic–RHA interface; they had the ceramic too strong (p. 103).
- **The slower rod again went deeper** into the witness pack. Only the
  post-test pressure-dependent model reproduced that, and it needed a
  different β at each velocity (pp. 113, 116).
- In CTH the rod penetrates alumina about 0.25 km/s slower than RHA at
  1.775 km/s, but at the same speed (about 1.50 km/s) at 2.6 km/s (p. 98).
- Model calibration from DOP tests at one velocity (1.5 km/s) with thin
  tiles did not carry over to a thick target at 1.775 and 2.6 km/s
  (pp. 104–105).

### Short rods and residual rods (pp. 41–49, Tables 6–7)

P/(αL), with α = √(ρp/ρt), for L/D 1, 3 and 5–6 rods collapses onto one
curve in x = (ρpV²/σt)^(1/3) (ρp/ρt)^(−0.15), with σt (GPa) =
3.48 × 10⁻³ (BHN − 11.24) (eq. 16, p. 44 and 47). Linear fits are in the
regression CSV. The residual rods from the RHA block, shot into the
witness pack, fall on these curves, so **a leftover rod penetrates like a
fresh short rod of the same length and speed** (p. 49).

## Mass and thickness efficiency (our derivation)

The report gives no efficiency figures. We derived them as: RHA-equivalent
thickness = semi-infinite RHA depth at the shot's velocity (Table 3, with
the eq. 10 slope below 2 km/s) minus the witness-pack depth. This assumes
depths add, which the RHA block supports (above). The witness depth behind
the 65° plates is taken as line-of-sight.

| Target | V (km/s) | RHA equivalent (cm) | Thickness efficiency | Mass efficiency |
|---|---|---|---|---|
| Ceramic laminate, whole (47.6 cm, 253 g/cm²) | 1.73 / 1.77 | 40.2 / 42.6 | 0.84 / 0.89 | 1.25 / 1.32 |
| Same | 2.59 / 2.60 | 46.2 / 46.9 | 0.97 / 0.98 | 1.43 / 1.45 |
| Alumina alone (30.5 cm, 119 g/cm²; RHA plates taken as 1:1) | 1.73 / 1.77 | 23.1 / 25.4 | 0.76 / 0.83 | 1.53 / 1.68 |
| Same | 2.59 / 2.60 | 29.1 / 29.8 | 0.95 / 0.98 | 1.93 / 1.97 |
| Spaced 500 BHN plates at 65°, per line-of-sight steel (27.0 cm) | 1.72 | 33.9 | 1.26 | 1.26 |
| Same | 2.60 | 43.2 | 1.60 | 1.60 |

The spaced-plate figures are against line-of-sight steel; against the
plates' normal thickness (11.4 cm) the RHA equivalent is 3–3.8 times. The
1.78 km/s shot with 2.3° yaw would give 1.53, the yaw helping the target.

## Limits

- One rod type (L/D 30 tungsten, 2.88 MJ), two velocities, one or two shots
  per target and velocity. No DU.
- The targets are generic, deliberately defeated, and not any real
  vehicle's armour. The spaced plates are high-hard steel at 65°, not
  composite; the ceramic laminate has no confinement design to speak of.
- Efficiency figures above are ours and rest on the semi-infinite baseline
  and on adding depths.
- 75% replica scale: by Mullin et al. (1995) larger rods penetrate a little
  more, so full-size results may be slightly higher.
- Yaw was not always measured, and the spaced target is very sensitive
  to it.

## How we could use it

1. **Finite RHA plates:** treat a thick RHA plate as consuming the same
   rod as semi-infinite RHA of that thickness; the residual rod then
   penetrates like a fresh short rod (pp. 38, 49). A plate of 0.9 × DOP
   leaves about 16% of the rod at about 75% speed at 1.8 km/s (p. 40).
2. **L/D 30 rods and the SwRI fit:** this full-size-ish L/D 30 data point
   says the fit under-predicts by 14% at 1.85 km/s, not over-predicts.
   Keep anchoring each round at its published figure, and do not apply a
   blanket L/D 30 penalty from model-scale data alone (p. 24).
3. **Spaced oblique plates:** credit about 1.3–1.6 times their
   line-of-sight thickness in RHA, with the lower figure at ordnance
   velocity; faster rods do worse per plate (pp. 79–83). Yaw of 1–2°
   makes a large difference.
4. **Ceramic composite:** about 1.3–1.45 mass efficiency for the whole
   laminate, or 1.5–2.0 for the alumina alone, rising with velocity
   (Tables 10–11). Thickness efficiency is below 1 at ordnance velocity,
   so ceramic armour costs space even as it saves weight.
5. **Witness-pack trend:** against both non-RHA targets the slower,
   longer rod did better at equal energy. For rounds of similar energy,
   L matters more than V against spaced and ceramic armour.
