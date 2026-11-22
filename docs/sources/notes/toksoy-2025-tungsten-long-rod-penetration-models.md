# Toksoy (2025): tungsten long rods into RHA, by penetration model

**Source:** Ahmet Kaan Toksoy (Roketsan, Ballistic Protection Systems
Directorate), "Tungsten Uzun Çubuk Tipi Penetratörlerin Zırh Çelikleri
Üzerindeki Delme Etkinliğinin Penetrasyon Modeller ile İncelenmesi /
Depth of Penetration Behaviour Analysis of Tungsten Long-Rod Penetrator
Impacting on Armour Steel by Penetration Models". *Dokuz Eylül
Üniversitesi Mühendislik Fakültesi Fen ve Mühendislik Dergisi* (DEU FMD)
27(79), 2025, pp. 22–30. doi:10.21205/deufmd.2025277904. Received
23 January 2024, accepted 20 March 2024.

**File:** `raw/ballistics/toksoy-2025-tungsten-long-rod-penetration-models.pdf`
(9 pages; Turkish, with an English abstract and extended abstract).
Read on 22 November 2026. Page numbers are the journal's (22–30).

## What it is

A comparison of four empirical equations and two analytical models for
the depth a tungsten long rod penetrates into semi-infinite RHA, against
150 test results collected from the literature (rods of 25–140 mm calibre
and gas-gun rods; L/D 10–30; impact velocity 665–3,500 m/s). The author
wrote Python codes for the analytical models.

**Headline result:** the Walker–Anderson model was closest to the tests
at every L/D, with at most 3.3% error, and its error did not grow with
L/D. The Alekseevskii–Tate model's error was 5–13.7% and grew with L/D.
The empirical equations were within 5% inside the L/D they were fitted
for, but the wide-range Silsby equation reached 9% (p. 28, Table 3).

## The test data (p. 25, Table 1)

| L/D | Impact velocity (m/s) | Sources | Tests |
|---|---|---|---|
| 10 | 863–1,756 | [23–28] | 54 |
| 12 | 665–1,535 | [23] | 18 |
| 14 | 984–1,358 | [18], [29] | 11 |
| 15 | 1,240–2,990 | [30–32] | 21 |
| 20 | 1,278–3,020 | [31], [33–36] | 34 |
| 30 | 1,094–3,500 | [27], [31], [37] | 12 |

There are no L/D = 15 points between 1,900 and 2,800 m/s (p. 22). The
paper gives the source list but not the 150 points themselves: they have
to be taken from its references (several of which are in the SwRI
database, `anderson-1992-penetration-mechanics-database`).

## The equations (pp. 25–26)

P is depth of penetration, L rod length, V impact velocity. For
equations 1–4, V is in **km/s** (checked: equation 4 gives P/L ≈ 0.78 at
1.5 km/s, a sensible value; m/s would not). Equations 12 and 13 use
**m/s**.

| Eq. | For | Form | Valid | Origin |
|---|---|---|---|---|
| 1 | L/D = 10 | P/L = 1.06 V − 0.712 | 0.8–1.75 km/s | Woolsey et al. [34–35] |
| 2 | L/D = 15 | P/L = 0.997 V − 0.682 | 0.8–1.75 km/s | Anderson et al. [13] |
| 3 | L/D = 20 | P/L = 1.606 V^4.273 / (1.542^4.273 + V^4.273) | 0.8–1.75 km/s | [14] |
| 4 | L/D = 10–30 | P/L = (1.239 + 0.00875 V) / (1 + 117 e^(−3.5 V)) | 0.6–4.5 km/s | Silsby et al. [37], 100 points |
| 12 | L/D = 15 | P/L = 6.332e−11 V³ − 7.22e−6 V² + 0.00275 V − 1.96 | the data's range | this paper |
| 13 | L/D = 20 | P/L = 2.65e−10 V³ − 2.17e−6 V² + 0.006 V − 4.30 | the data's range | this paper |

**Lanz–Odermatt** (eq. 5–7, p. 25):

P/L = a · (cos θ)^m · √(ρp/ρt) · exp(−c·Rm / (ρp·V²))

with θ the obliquity, ρp and ρt the rod's and target's densities, Rm the
target's tensile strength. Originally m = 0.745 and c = 25.9 (about 41
tests, L/D 11–31, 1.1–1.9 km/s); Jeanquartier and Odermatt later gave
m = 0.775 and c = 22.1 + 1.274e−8·Rm − 9.47e−18·Rm² (Rm in Pa). The text
extraction of the L/D term `a` (eq. 6) is garbled: it contains
`3.94 (1 − tanh((L/D − 10)/11.2))`. **Read eq. 6 on the printed page
before coding it.** The paper finds Lanz–Odermatt off by more than 14%
on the newer data, though it was within 5% on its own (p. 22).

**Alekseevskii–Tate** (eq. 8–9): ½ρp(v − u)² + Yp = ½ρt u² + Rt, with
Yp the rod's strength (taken as its Hugoniot elastic limit,
Y(1 − ν)/(1 − 2ν)) and Rt the target's resistance (4–5 times its dynamic
yield).

**Walker–Anderson** (eq. 10–11): a time-resolved momentum model with an
assumed velocity profile in rod and target; the extent of the plastic zone
in the target is R = Rp(1 + 0.287 v0 + 0.148 v0²), Rp the rod radius,
v0 the impact velocity (km/s).

**Material values used (p. 26, Table 2):** tungsten Yp 0.8, 1, 1.14,
1.66 or 2 GPa depending on the alloy and source; RHA Rt 4.37, 4.94 or
5.1–5.5 GPa.

## Errors (p. 28, Table 3; % mean error against the tests)

| Model | L/D 10 | 15 | 20 | 30 |
|---|---|---|---|---|
| Eq. 1 | 2.9 | | | |
| Eq. 2 | | 14 | | |
| Eq. 3 | | | 4.6 | |
| Eq. 4 (Silsby, all L/D) | | | | 9 overall |
| Eq. 12 | | 5.1 | | |
| Eq. 13 | | | 2 | |
| Alekseevskii–Tate | 5 | 7 | 8.8 | 13.7 |
| Walker–Anderson | 2.9 | 2.4 | 3 | 3.3 |

(The extracted table places the Silsby value in the L/D = 20–30 columns;
the text, p. 28, says 9% over all L/D.)

## Limits

- Normal impact into semi-infinite RHA only. No oblique plates, no
  finite plates, no spaced or composite armour, no ERA.
- The model inputs (Yp, Rt) are spread widely between sources; the
  Walker–Anderson fit depends on choosing them.
- Small calibres (25–35 mm) are inside the data, but most points are
  larger rods or gas-gun rods.

## How we will use it

For the per-round penetration model that replaces the catalogue's blanket
curve (`research/catalogue-csv-assessment.md`):

1. Velocity at range from each round's drag
   (`rheinmetall-datasheets-medium-calibre.md` gives a measured decay for
   a 25 mm sub-calibre round).
2. P/L at that velocity. **First choice: equation 4 (Silsby)**: one
   closed form for all L/D, 0.6–4.5 km/s, 9% error, no material
   constants. **Better, later:** Walker–Anderson, if we code its time
   integration; it is the most accurate here.
3. Scale by rod length, and fit each round so the model gives its
   published penetration at its published range.

**What it shows straight away.** With equation 4, a rod that leaves the
muzzle at 1.40 km/s and slows by 125 m/s per km (the 25 mm figure) keeps
about **80% of its muzzle penetration at 1 km, 62% at 2 km and 48% at
3 km**. The catalogue keeps 97%, 92% and 86% for every round. For a
large-calibre rod leaving at 1.75 km/s and slowing by 50 m/s per km (an
assumed figure, to be sourced), the same equation gives 96%, 92% and
88%: close to the catalogue's curve. So the blanket curve is roughly right
for tank rounds and badly wrong for 25–35 mm rounds, which it credits
with about half as much again as they keep at 2 km.
