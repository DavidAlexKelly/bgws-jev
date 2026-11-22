# O'Kane et al. (2004): N50 for camouflaged targets (ACQUIRE-LC)

**Source:** Barbara L. O'Kane (US Army RDECOM CERDEC NVESD), Gary L. Page
(Booz Allen Hamilton), David L. Wilson and David J. Bohan (NVESD), "Cycle
Criteria for Detection of Camouflaged Targets". Paper 25 in
*Sensors and Sensor Denial by Camouflage, Concealment and Deception*,
NATO RTO-MP-SCI-145, symposium held in Brussels, 19–20 April 2004
(slides dated 22 April 2004). DTIC ADA456742 (DTIC form dated December
2005). Approved for public release.

**File:** `raw/sensing/okane-2004-cycle-criteria-camouflaged-targets.pdf`
(24 PDF pages). Read on 22 November 2026.

The PDF holds a **9-page paper** (printed pp. 25-1 to 25-9, plus a blank
25-10), a DTIC form, and **13 presentation slides**. Page references
below use the printed number, with the PDF page in brackets: PDF page 1
is 25-1, PDF page 2 is the DTIC form, then PDF page n is 25-(n−1) up to
PDF 11. The slides are PDF pp. 12–24 and repeat the paper.

**Table:** `data/okane-2004-cycle-criteria-camouflaged-targets--n50-by-contrast.csv`
(computed from the paper's two equations; the paper prints no table of
values).

## What it is

NVESD's fix for a known fault in ACQUIRE: it hardly responds to target
contrast. Lowering a target's thermal contrast barely shortens the
predicted range, because the range is "almost entirely dominated by
target size" (p. 25-4 [5]). Field studies show camouflage does shorten
detection range (p. 25-1 [1]).

Analysts had been raising N50 by hand from field data for each vehicle.
ACQUIRE-LC ("ACQUIRE for Low Contrast Targets", 2003) replaces that with a
formula: **N50 rises as the inverse square of the target's inherent
thermal contrast** (pp. 25-4 to 25-5 [5–6]). Ordinary ACQUIRE use is not
changed: for a high-contrast target the formula returns the usual N50.

## The contrast measure (p. 25-2 [3], eq. 1)

**ΔT_RSS = √[ (μ_tgt − μ_bkgd)² + σ_tgt² ]**

μ_tgt and μ_bkgd are the mean target and background temperatures; σ_tgt
is the standard deviation of temperature across the target. The
background is the ring inside a box √2 times the target's height and
width (p. 25-2 [3]). So a target with no mean difference from its
background but a patchy surface still has contrast. Units are kelvin (°C
differences); the example uses 2 °C (p. 25-6 [7]).

For ACQUIRE-LC the **inherent** ΔT_RSS is used: the value at about
1,000 m, with atmospheric loss handled by Beer's law (p. 25-5 [6]).

## The cycle criteria (p. 25-5 [6], eq. 4 and 5; slide PDF p. 18)

| Background | Uncamouflaged N50 | ACQUIRE-LC N50 |
|---|---|---|
| Moderate to high clutter ("woodland") | 1.5 | **N50 = 6 / ΔT_RSS² + 1.50** |
| Low clutter ("bland") | 0.75 | **N50 = 0.75 / ΔT_RSS² + 0.75** |

The baseline values are those of the ACQUIRE User's Guide (p. 25-4 [5]).
"The traditional ACQUIRE N50 is the pedestal upon which the ACQUIRE-LC
N50 rests" (slide, PDF p. 18).

**Ratio to the uncamouflaged N50:**

- moderate/high clutter: **1 + 4 / ΔT_RSS²**;
- low clutter: **1 + 1 / ΔT_RSS²**.

So clutter magnifies the effect of camouflage fourfold. Values:

| ΔT_RSS (K) | N50, moderate/high | ratio | N50, low | ratio |
|---|---|---|---|---|
| 0.5 | 25.5 | 17 | 3.75 | 5.0 |
| 1.0 | 7.5 | 5.0 | 1.50 | 2.0 |
| 1.5 | 4.17 | 2.8 | 1.08 | 1.44 |
| 2.0 | 3.0 | 2.0 | 0.94 | 1.25 |
| 3.0 | 2.17 | 1.44 | 0.83 | 1.11 |
| 5.0 | 1.74 | 1.16 | 0.78 | 1.04 |

**Worked example (p. 25-6 [7], eq. 6):** a camouflaged target, inherent
ΔT_RSS 2 °C, moderate clutter: N50 = 6/4 + 1.5 = **3.0**, twice the
baseline. Front view, 3 m × 3 m. ACQUIRE is then run as usual with
N50 = 3.0.

Fig. 5 (p. 25-5 [6]) plots both curves for ΔT_RSS from 0 to 3. Figure 4
(p. 25-4 [5]) shows why N50 matters: at one notional range, Pd might be
0.9 with N50 = 0.75 and 0.25 with N50 = 1.5.

## The reasoning (pp. 25-6 to 25-8 [7–9])

In a bland background a low-contrast target one pixel in size still
stands out as a peak above the noise (Fig. 6). In moderate clutter the
same target is lost among the clutter (Fig. 7). To find it as reliably it
must be bigger in the image, that is, closer (Fig. 8). Raising N50 is
how ACQUIRE says "closer".

## Bands and test conditions

- **Thermal only.** "ACQUIRE-LC is to be used only for detection of
  camouflaged targets in the thermal spectrum" (p. 25-8 [9]; slide PDF
  p. 23). **Nothing on visible-band or naked-eye camouflage.**
- **Detection only.** Not for recognition or identification (p. 25-8 [9]).
- Fitted for **woodland and littoral** environments (p. 25-8 [9]). Not
  validated for desert, day or night; that work was "continuing"
  (p. 25-8 [9]).
- Treat each target aspect separately: the N50 of the average ΔT_RSS
  does not give the same result as each aspect with its own N50
  (p. 25-8 [9]).
- **No test data are given.** The paper states the equations as a
  "power law with offset" fitted to field data (p. 25-5 [6]) but gives no
  numbers of targets, observers, sensors, ranges or the scatter of the
  fit. They are presumably in NVESD's internal analyses.

## Limits

- Thermal sensors only, detection only, woodland and littoral only.
- The fit's data and error are not published here.
- The formula blows up as ΔT_RSS → 0 (N50 = 25 at 0.5 K, 98 at 0.25 K in
  clutter). In practice that means "not detected", which is plausible, but
  the curve below about 0.5 K is extrapolation with no visible support.
- Its baselines (1.5 moderate, 0.75 low) are the ACQUIRE User's Guide's.
  Howe's Table 2.5 gives 2-D values of 0.75 (medium) and 0.375 (low)
  (`irhb-1993-v4-eo-systems-design-analysis-testing.md`). The two NVESD
  sources differ by a factor of two in the absolute N50, so use the
  **ratio**, not the absolute values.
- "Moderate to high clutter" is the woodland curve. It is not Howe's
  "high" (a vehicle among similar vehicles).

## How we could use it

**For thermal sights: a camouflage multiplier on N50.**

`acquire.rs` sets detection N50 from the catalogue (0.75) and scales it
by clutter (×0.5 low, ×1 medium, ×2 high). A camouflaged target could
take a further factor:

- medium clutter (woods, towns): **N50 × (1 + 4 / ΔT_RSS²)**;
- low clutter (fields, roads): **N50 × (1 + 1 / ΔT_RSS²)**;

with ΔT_RSS the target's inherent thermal contrast in kelvin. That needs
a ΔT_RSS per vehicle and state (engine hot or cold; thermal net or not).
The sim already holds a ΔT per target for its contrast factor.

**Watch for double counting.** `acquire.rs` already scales the resolved
cycles by a contrast factor that falls to 0 at threshold contrast. If
that factor already cuts range enough for low-ΔT targets, adding the
ACQUIRE-LC multiplier on top counts camouflage twice. O'Kane's premise is
that the MRTD route alone is too weak. Check first: run a 3 m × 3 m
target at ΔT 2 K and 0.5 K through our thermal sight and see how much
range falls. If the fall is small, add the multiplier; if it is already
large, calibrate one against the other.

**For eyes and day sights: no direct use.** The source is explicit that
it is thermal only. It does support, by analogy, a rule that low contrast
hurts more in clutter than on bland ground (fourfold here). A visible-band
camouflage factor would still need its own source.
