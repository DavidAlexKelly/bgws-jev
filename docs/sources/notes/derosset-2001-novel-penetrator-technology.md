# de Rosset (2001): An Overview of Novel Penetrator Technology

**Source:** William S. de Rosset, *An Overview of Novel Penetrator
Technology*. ARL-TR-2395, US Army Research Laboratory, Weapons and
Materials Research Directorate, Aberdeen Proving Ground MD, February 2001
(work October–December 1999). Approved for public release. DTIC ADA387329.

**File:** `docs/sources/raw/ballistics/derosset-2001-novel-penetrator-technology.pdf`
(43 PDF pages)

**Read:** 22 November 2026

**Data extracted:**

- `docs/sources/data/derosset-2001-novel-penetrator-technology--rod-tube-penetration.csv` (6 rows, Table 1)

All page numbers are PDF page numbers. PDF page = report page + 10
(report p. 1 is PDF p. 11). Born-digital text; the equations are images
and were read from the page.

## What it is

A short, unclassified review of penetrator shapes that depart from a plain
long rod: extending rods (rod-tube, split rod), non-circular cross-sections,
segmented rods, tandem rods, sheathed (jacketed) rods, and penetrator
materials (DU against tungsten). It explains each concept with simple
one-dimensional penetration formulas and gives a few numbers from the
open literature. It is not a test report; most figures are quoted from
other authors. Classified details of extension mechanisms are left out
(p. 18).

## The penetration formulas (pp. 13–17)

With μ = ρp/ρt, the density ratio:

- **Density law** (eq. 1): P = L √μ. The high-velocity limit of a long
  rod.
- **Odermatt velocity function** (eqs. 2–3): P = L √μ exp(−b/v²), taken
  from the Lanz–Odermatt limit-thickness equation (1992). Rapacki et al.
  (1995) fit b = 2S/ρp with S = q (BHN)^m, so target hardness enters
  through S (eqs. 4–5). The values of q and m are not given.
- **Short rods** (eq. 7): P/D = c ρ^(1/3) v^(2/3), from crater volume
  proportional to kinetic energy for L/D 1.
- **Heuristic L/D formula** (eq. 8, after Frank and Zook 1991):
  P = L (1 − D/L) √μ exp(−b/v²) + D c ρ^(1/3) v^(2/3). The steady phase
  takes all but one diameter of the rod; the last diameter penetrates like
  a short rod. The author adds that Anderson et al. (1997) found this
  **does not represent the L/D effect accurately for tungsten into RHA at
  1–2 km/s** (p. 15).
- **Length against density** (eqs. 9–11): two rods of the same mass and
  diameter, at high velocity: P1/P2 = √(ρ2/ρ1) > 1 for the longer,
  less dense rod (p. 17).

The author's point (p. 16): penetrator strength, ductility and toughness
matter little for normal impact on monolithic targets, but "for attack of
oblique, spaced, and reactive armors, mechanical properties and material
processing become very important".

## Quantitative results quoted

| Concept | Result | Velocity | Page |
|---|---|---|---|
| Rod-tube, extended (Lynch et al. 1995): 10.6 mm OD tube, 46.5 mm long, 5 mm bore, 40.55 mm rod extension | +31% depth against a unitary rod 46.5 × 10.6 mm | 1.83 km/s | 18, 20 |
| Same | +52% | 2.63 km/s | 20 |
| Same | +57% | 2.90 km/s | 20 |
| Equal-mass solid rod of higher L/D (Lynch et al.) | beat the rod-tube at every velocity tested | – | 19 |
| H-rod: L/D 5 tungsten rod with axial holes against an equal-mass, equal-diameter solid L/D 4 rod (Silsby 1996) | "only slightly higher" in tests | 1.6–1.7 km/s | 22 |
| Same, CTH | little difference at 1.6 km/s; +10% at 2.5 km/s; density law gives √(17.71/14.13) = 1.12 | 2.5 km/s | 22 |
| Triform and cruciform cross-sections, equal mass and length (Bless et al. 1995) | very little difference into solid RHA | – | 22 |
| Segmented telescopic rod, three pieces (Anderson et al. 1997) | +33% penetration efficiency over an equivalent long rod, calculated | 2.5 km/s | 24 |
| Same | 23% less efficient than one seg-tel segment alone | 2.5 km/s | 24 |
| Segmented rods at ordnance velocity (de Rosset and Sherrick 1996) | n segments penetrate less than n times one segment: debris interferes | ordnance | 25 |
| Sheathed rod, steel sheath on DU core, constant energy (Sorensen et al. 1994, computed) | P/L rises slightly with sheath thickness to a peak near sheath T/D = 0.15, then falls fast | ordnance | 28 |
| Same, constant velocity | never better than the equivalent monolithic DU rod | ordnance | 28 |
| Sheathed rod, system view (Sorensen et al. 1998) | a sheath can raise muzzle velocity against a monolithic rod | – | 28 |

Table 1 (p. 20) is in the CSV. Baseline penetration hardly rises from 2.62
to 2.92 km/s (77 to 78 mm), while the rod-tube's rises from 117.5 to
122.5 mm.

## Materials (pp. 29–30)

- Large changes in the mechanical properties of tungsten alloys did **not**
  change their penetration of RHA much (Magness and Farrand 1990).
- Harder DU **did** penetrate better. DU forms a self-sharpening nose
  through adiabatic shear; tungsten alloy forms a mushroom nose that needs
  more energy (Fig. 9, p. 30).
- No DU replacement with the same behaviour existed in 2001, but "the
  performance gap between depleted uranium and other high-density alloys
  has been narrowed" (p. 30). No number is given for the gap.
- Rigid-body (non-eroding) penetration happens only against soft, light
  targets; tank armour at high velocity rules it out (p. 29).

## Conclusions (p. 31)

- Most novel concepts (rod-tube, segmented, sheathed, H-rod) work best
  against monolithic targets **at high velocity**; none was easy to build
  at full scale.
- The 1-D models describe monolithic armour at 0° obliquity. Modern tank
  armour is multi-material, spaced and oblique; the models help mainly for
  the residual rod against the RHA back plate.
- Tandem rods target specific armours: the lead element sets off ERA,
  disturbs ceramic interface defeat, or upsets the timing of
  momentum-transfer armour (pp. 26–27). Spaced oblique plates and armour
  that strikes the rod from the side are the threat to segmented rods
  (p. 25).

## Limits

- A qualitative review. Almost every number comes from another paper, and
  several of those (seg-tel, sheathed rods) are hydrocode results, not
  tests.
- No obliquity, finite-plate or armour-equivalence data.
- No tables of DU against tungsten penetration.

## How we could use it

1. **No change to the per-round model for fielded rounds.** Fielded APFSDS
   are near-solid long rods; the novel concepts here are not in service
   rounds, and their gains appear mainly above 2.5 km/s.
2. **L/D correction.** Do not adopt the heuristic eq. 8 to fix the L/D
   trend of the SwRI fit: the author himself cites Anderson et al. (1997)
   against it for tungsten into RHA at 1–2 km/s (p. 15).
3. **Hardness.** If we add a target-hardness term, the Rapacki et al.
   form b = 2S/ρp, S = q·BHN^m, is the one to look up (p. 14; the
   coefficients are in Rapacki et al. 1995, not here).
4. **DU rounds.** Supports scaling DU above tungsten at ordnance
   velocities (self-sharpening), but gives no factor; keep the factor from
   `anderson-1992-penetration-mechanics-database` (pp. 29–30).
5. **Armour.** Supports treating spaced, oblique and reactive armour as a
   separate case from RHA, where rod strength and integrity matter (p. 16).
