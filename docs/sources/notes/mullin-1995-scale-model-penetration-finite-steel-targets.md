# Mullin, Anderson et al. (1995): scale-model tests on finite steel plates

**Source:** Scott A. Mullin, Charles E. Anderson Jr., Andrew J. Piekutowski,
Kevin L. Poormon, Neil W. Blaylock and Bruce L. Morris, *Scale Model
Penetration Experiments: Finite-Thickness Steel Targets*. SwRI Report
3593/003, Southwest Research Institute, San Antonio TX, May 1995. Prepared
for the US Army Research Office and ARPA under contracts DE-AC04-90AL58770
and DAAL03-91-C-0021. Approved for public release. DTIC ADA296435.

**File:** `docs/sources/raw/ballistics/mullin-1995-scale-model-penetration-finite-steel-targets.pdf`
(74 PDF pages)

**Read:** 22 November 2026

**Data extracted:**

- `docs/sources/data/mullin-1995-scale-model-penetration-finite-steel-targets--normal-obliquity-tests.csv` (33 rows, Table 13)
- `docs/sources/data/mullin-1995-scale-model-penetration-finite-steel-targets--60-degree-obliquity-tests.csv` (31 rows, Table 14)
- `docs/sources/data/mullin-1995-scale-model-penetration-finite-steel-targets--zero-yaw-summary.csv` (36 rows, Table 16)

All page numbers are PDF page numbers. PDF page = report page + 10
(report p. 1 is PDF p. 11).

## What the PDF is

A scan with an OCR text layer. Running text extracts well. Tables 13, 14
and 16 are landscape and the OCR scrambles them, so every number in the
CSVs was read from the page image (rendered at 300 dpi). Table 8 was also
checked on the image: the OCR reads the plate hardness as "R 37"; the page
says Rc 27 ± 2.

## What it is

A replica scale-model study. Tungsten-alloy long rods, L/D 20, were fired
at single steel plates at three scales, two velocities and two obliquities,
to see whether small models behave like the full-size round (pp. 11, 27).

- **Prototype rod:** tungsten alloy, hemispherical nose, D = 25.4 mm,
  L/D 20 (L = 508 mm, 4.37 kg) (pp. 27–28, Table 6).
- **Model rods:** WNi08F (90% W, 8% Ni, 2% Fe), 17.13 g/cm³, from GTE
  (p. 27). Scales 1/3.15 (D 8.06 mm, L 161.2 mm, 140 g), 1/6.30 (4.03 mm,
  80.6 mm, 17.5 g) and 1/12.60 (2.02 mm, 40.3 mm, 2.19 g) (p. 28, Table 6).
  The rods were threaded for a puller sabot, which adds about 12% mass
  (p. 31, Table 7).
- **Plates:** 4340 steel, Rc 27 ± 2, "a surrogate material for RHA"
  (p. 33). Rc 27 is about BHN 269 by the SwRI conversion
  (BHN = 7.595 Rc + 63.86), so this is RHA-hard.
- **Velocities:** nominal 1.5 km/s ("typical of present-day long-rod tank
  rounds") and 2.2 km/s (p. 29). Two-stage light-gas guns, 50/20 mm and
  75/30 mm (p. 41).
- **Obliquity:** 0° and 60°. At 60° the plates were cut to the **same
  line-of-sight thickness** as the 0° plates, so the normal thickness is
  half (p. 34).
- **Thickness classes** (p. 33–34): NLT, near the limit thickness
  (semi-infinite depth of penetration plus 1–2 rod diameters, from the Tate
  model); IB, in between (planned residual velocity about 66% of impact);
  WB, well below the limit (planned residual 85–90%).
- 54 planned shots (36 + 18 repeats); 64 reported, because shots with a
  launch fault were repeated (pp. 37–41). Many 1/12.6 and some 1/6.3 shots
  had large yaw (p. 41).

## The test matrix (p. 33, Table 8; planned plate thicknesses, cm)

| Velocity | Obliquity | NLT (1/3.15, 1/6.30, 1/12.60) | IB | WB |
|---|---|---|---|---|
| 1.50 | 0° | 14.52, 7.37, 3.62 | 13.4, 6.8, 3.34 | 10.16, 5.16, 2.53 |
| 1.50 | 60° | 7.26, 3.68, 1.81 | 6.70, 3.40, 1.67 | 5.08, 2.58, 1.26 |
| 2.25 | 0° | 24.32, 12.35, 6.07 | 23.4, 11.88, 5.84 | 22.25, 11.30, 5.55 |
| 2.25 | 60° | 12.16, 6.18, 3.04 | 11.70, 5.94, 2.92 | 11.12, 5.65, 2.78 |

The as-tested plates (T/L in Tables 13–14, L = 161.2 mm at 1/3.15) are
not all the planned ones. At 1.5 km/s they are close (0° IB and WB tested
at T/L 0.80 and 0.65, planned 0.83 and 0.63). At 2.2 km/s they are much
thinner: 0° NLT, IB and WB tested at 1.43, 1.28 and 0.96, planned 1.51,
1.45 and 1.38. The report does not say why. The CSVs use the T/L printed
in Tables 13–14. The IB entry
for 2.25 km/s, 0°, 1/3.15 reads "2.34" in the table, a misprint for 23.4.

## Main results

### The plates were tougher than planned (p. 47)

The plates were sized from L/D 10 data, but "penetration performance, as
measured by P/L, in the ordnance velocity range is degraded approximately
14% as the projectile L/D increases from 10 to 20". So the NLT plates acted
as semi-infinite and only five IB plates were perforated.

### Semi-infinite depth at zero yaw (p. 53, Table 16)

| | 1.5 km/s, 0° | 1.5 km/s, 60° (LOS) | 2.2 km/s, 0° | 2.2 km/s, 60° (LOS) |
|---|---|---|---|---|
| 1/12.6 | – | 0.56 | 1.21 | – |
| 1/6.30 | 0.63 | 0.64 | 1.12 | 1.26 |
| 1/3.15 | 0.71 | 0.69 | 1.26 | 1.21 |

P/L of the NLT (non-perforated) plates. For the 60° plates the depth is
along the shot line: P/L 1.20 in a plate whose normal thickness is
0.71 L cannot be normal to the face without perforation (pp. 42–43). The
report does not define P for oblique plates (p. 45, Fig. 12).

Our check: the SwRI database fit (Anderson et al. 1992, eq. 7) with
ρp = 17.13 g/cm³ and BHN 269 gives P/L 0.70 at 1.5 km/s and 1.25 at
2.2 km/s, matching the 1/3.15-scale results.

### Line-of-sight depth at 60° equals normal depth

At both velocities the line-of-sight P/L into the 60° plates is the same
as the normal P/L into the 0° plates, within scatter: 0.56–0.69 against
0.63–0.71 at 1.5 km/s, 1.21–1.26 against 1.12–1.26 at 2.2 km/s (p. 53).
For these L/D 20 rods into RHA-hard steel the simple rule holds: **an
oblique plate is worth its line-of-sight thickness** for depth of
penetration.

### Where the limit thickness falls (pp. 42–43, Tables 13–14)

Perforation results at low yaw (T and P in rod lengths L; at 60° the
line-of-sight thickness is twice T):

| V | Obliquity | LOS T/L | Result |
|---|---|---|---|
| 1.5 | 0° | 0.87 (NLT) | none perforated; P/L 0.60–0.72 |
| 1.5 | 0° | 0.80 (IB) | none perforated; P/L 0.57–0.66 |
| 1.5 | 0° | 0.65 (WB) | 1/12.6 stopped (P/L 0.53–0.59, yaw 2.7–4.3°); 1/6.3 through, Vr/V 0.13; 1/3.15 through, Vr/V 0.58 |
| 1.5 | 60° | 0.88 (NLT) | none perforated; P/L 0.38–0.74 |
| 1.5 | 60° | 0.80 (IB) | 1/6.3 through (yaw 2.1°); others stopped, P/L 0.58–0.69 |
| 1.5 | 60° | 0.64 (WB) | all through; Vr/V 0.47, 0.71, 0.82 |
| 2.2 | 0° | 1.43 (NLT) | none perforated; P/L 1.10–1.27 at yaw < 4° |
| 2.2 | 0° | 1.28 (IB) | 1/3.15 through; 1/6.3 stopped (P/L 1.17, yaw 2.5°) |
| 2.2 | 0° | 0.96 (WB) | 1/12.6 at 0° yaw through, Vr/V 0.90, Lr/L 0.20; 1/3.15 through |
| 2.2 | 60° | 1.42 (NLT) | none perforated; P/L 1.20–1.26 at low yaw |
| 2.2 | 60° | 1.28 (IB) | 1/12.6 (0° yaw) and both 1/3.15 through |
| 2.2 | 60° | 0.98 (WB) | 1/6.3 and 1/3.15 through; 1/12.6 (yaw 5–7°) stopped |

Our reading (the report does not state a limit thickness):

- At 0°, the limit thickness lies **between the semi-infinite depth and
  that depth plus about 2 rod diameters** (0.1 L). At 2.2 km/s the
  1/3.15 rod went through 1.28 L with a DOP of 1.26 L. At 1.5 km/s it
  went through 0.65 L with 58% of its speed left, but stopped in 0.80 L,
  against a DOP of 0.71 L.
- At 60°, the line-of-sight limit is about the same as at 0°, perhaps a
  little more; the 60° WB plates leave more residual velocity than the
  0° WB plates of the same line-of-sight thickness (0.47–0.82 against
  0–0.58 at 1.5 km/s; p. 53).

### Hohler and Stilp's curve (p. 34, Fig. 9)

The test design used a residual-velocity curve from Hohler and Stilp
(1990, ref. 9): Vr/V0 against T/(L cos θ) at 1,520 m/s for 0° and 60°.
Read by eye: Vr/V0 is about 0.9 at T/(L cos θ) = 0.5 and falls steeply
after 0.8; the ballistic limit is at about **1.1 L line-of-sight at 0° and
about 1.2 L at 60°**. So at 60° about 10% more line-of-sight steel is
needed to stop the rod. The rod's L/D is not given on the figure.

### Scale effect (pp. 63–70)

- Larger models penetrate a little deeper and perforate more easily. At
  1.5 km/s the NLT P/L rises from 0.63 (1/6.3) to 0.71 (1/3.15), and the
  WB plate stops the 1/12.6 rod but lets the 1/3.15 rod through at 58% of
  its speed (pp. 53, 64).
- The clear, statistically significant effects are in crater height and
  maximum hole size at 60° (pp. 57, 63, 67).
- The effect is larger at 1.5 km/s than at 2.2 km/s, and larger at 60°
  than at 0° (p. 67). It is "most pronounced near a threshold condition,
  such as a ballistic limit condition" (p. 70).
- The authors' explanation is time-dependent damage: smaller models have
  less time for damage to grow, so they "appear" stronger (pp. 67–69).
  Strain-rate hardening is only about 5% over a scale factor of 10 (p. 26).
- They cite Magness and Leonard (1993) for a scale effect in P/L and in
  ballistic limit velocity (p. 70).

## Limits

- One rod L/D (20), one plate steel (4340 at Rc 27), two velocities, two
  obliquities. No spaced, ceramic or composite targets.
- Model scale only. The largest rod is 8 mm across, about a third of a
  120 mm rod and similar to a 25–30 mm APFSDS rod.
- Few perforations and heavy yaw at the smaller scales, so the limit
  thickness is bracketed, not measured. No ballistic limit velocities.
- The 60° depths are line-of-sight by our inference, not by statement.

## How we could use it

1. **Oblique plates in the per-round model.** Treat a plate at angle θ as
   its line-of-sight thickness T/cos θ for depth of penetration; this
   report supports that at 60° for L/D 20 tungsten into RHA-hard steel.
   For perforation, allow about 10% more line-of-sight steel at 60°
   (Hohler and Stilp, Fig. 9, p. 34), which matches the Lanz–Odermatt
   (cos θ)^0.75 term (`toksoy-2025-tungsten-long-rod-penetration-models`,
   p. 25): at 60°, 0.5^-0.255 = 1.19.
2. **Converting "X mm at 60°" plate figures to semi-infinite depth.**
   Line-of-sight thickness is 2X. In the 60° tests the line-of-sight
   limit lies between 1.0 and about 1.25 times the semi-infinite depth
   (2.2 km/s: perforated at 1.28 L, stopped at 1.42 L, DOP 1.21–1.26 L;
   1.5 km/s: perforated at 0.64 L, mixed at 0.80 L, stopped at 0.88 L,
   DOP 0.64–0.69 L). So the semi-infinite normal depth is roughly
   **1.6X to 2.0X**, nearer 2X at high velocity. Example: Rheinmetall's
   53 mm at 60° at 1,000 m for the 30 mm round gives about 85–106 mm of
   semi-infinite RHA at that range. The Lanz–Odermatt term gives
   2X/1.19 ≈ 1.7X, inside the same band.
3. **Finite plates.** A plate of thickness close to the semi-infinite DOP
   is roughly the limit; plates thinner by 10–20% let the rod through with
   much of its speed. A plate below about 0.5 DOP barely slows it
   (Fig. 9, p. 34).
4. **Check of the P/L fit.** Eq. 7 of the 1992 database matches these
   L/D 20 results at both velocities (1/3.15 scale). Smaller models fall
   up to 12% short, so data from very small rods may understate full-size
   penetration.
