# Anderson, Morris and Littlefield (1992): A Penetration Mechanics Database

**Source:** Charles E. Anderson Jr., Bruce L. Morris and David L. Littlefield,
*A Penetration Mechanics Database*. SwRI Report 3593/001, Southwest Research
Institute, San Antonio TX, January 1992 (report date 31 January 1992).
Prepared for DARPA under contracts DE-AC04-90AL58770 and DAAL03-91-C-0021.
Approved for public release. DTIC ADA246351.

**File:** `docs/sources/raw/ballistics/anderson-1992-penetration-mechanics-database.pdf`
(468 PDF pages)

**Read:** 22 November 2026

**Data extracted:**

- `docs/sources/data/anderson-1992-penetration-mechanics-database--tungsten-rods-semi-infinite-rha.csv` (208 rows)
- `docs/sources/data/anderson-1992-penetration-mechanics-database--du-rods-semi-infinite-rha.csv` (9 rows)

All page numbers below are PDF page numbers, unless marked "report p.".
In the main text, PDF page = report page + 6. In Appendix A, PDF page =
A-page + 93 (A-1 is PDF p. 94).

## What the PDF is

The whole PDF is a 300 dpi black-and-white scan with an OCR text layer
(checked with `pdfimages`, p. 200). There is no born-digital text. The OCR
layer is good for numbers in most tables. Row labels and material names are
often garbled ("P (ram)", "HzB,A" read as "l-IzBA", "D18" read as "D!8").
The figure pages carry only axis labels and legends as text; the plotted
points can only be read from the images.

## How the database is organised

| Part | Content | PDF pages |
|---|---|---|
| Front matter | Cover, DD 1473 with abstract, contents, list of tables | 1–6 |
| §1 Introduction | Scope, units, hardness conversions, yaw definition, data counts | 7–10 |
| §2.1 Semi-infinite data, graphs | Tables 2.1.1–2.1.2 (index of figures by L/D and material), Figs 2.1.1–2.1.53 (one or more per source) | 11–40 |
| §2.2 Finite-thickness data, graphs | Residual velocity (Figs 2.2.1–2.2.12), limit velocity (Figs 2.2.13–2.2.30), Tables 2.2.1–2.2.4 | 41–59 |
| §2.3 Segmented-rod data, graphs | Figs 2.3.1–2.3.8 | 61–65 |
| §3 Cross-plots | Semi-infinite (Figs 3.1.1–3.1.10, Eq. 7), finite thickness (Figs 3.2.1–3.2.10), segmented | 66–85 |
| §4–6 | Acknowledgements (p. 86), nomenclature (p. 87), references (pp. 88–91) | 86–91 |
| Appendix A | Semi-infinite data tables, 31 sources, A-1 to A-224 | 92–317 |
| Appendix B | Finite-thickness tables: B-1 residual velocity (pp. 320–377), B-2 limit velocity (pp. 378–440) | 318–440 |
| Appendix C | Segmented (multiple-impact) tables | 441–455 |
| Errata, April 1992 | Letter (p. 457) and replacement sheets: Table 2.1.2, Figs 2.1.1–2.1.2 and 2.1.31–2.1.34, tables A-135, A-137, A-138 | 456–464 |
| Errata, July 1992 | Letter (p. 466) and replacement tables A-199, A-200 (Wilson et al., steel rods) | 465–468 |

The authors count more than 2300 data points: 1460 semi-infinite, 386
residual-velocity, 410 limit-velocity and 71 segmented-rod (p. 10). Hohler
and Stilp of EMI supplied 759 points in tabular form (p. 86).

### Table layout (Appendix A)

Each page is one source and holds up to seven shots. **Shots are columns;
variables are rows** (see p. 94 for a clean example). The rows are:

- Header: source (author, year), `Test No.`
- PROJECTILE: `L (mm)`, `D (mm)`, `L/D`, `M (g)`, `Mat'l`, `Supplier`,
  `ρ (g/cm³)`, `Nose`, `BHN`, `Elong (%)`
- TARGET: `Mat'l`, `Thick (mm)` (`---` for semi-infinite), `ρ (g/cm³)`,
  `BHN`, `Elong (%)`, `Obliq (deg)`
- IMPACT: `V (m/s)`, `Yaw (deg)`
- RESULTS: `P (mm)` (depth of penetration), `d (mm)` (crater diameter),
  `Vol (cm³)` (crater volume)
- SCALING: `P/αL`, `P/L`, `d/D`, `P/d`, where α = √(ρp/ρt)
  (nomenclature, p. 87)
- `Figure No.`: the §2 figure where the column is plotted (p. 8).

Units and conventions (pp. 8–9):

- Metric throughout. Data reported in English units were converted without
  rounding, so some values carry false precision (e.g. 4680 ft/s shown as
  1427 m/s; L = 78.74 mm is 3.100 in).
- All hardness is given as BHN. Rockwell C, Vickers and tensile strength
  were converted with Eqs (1)–(3) (pp. 8–9): BHN = 1.979·UTS(ksi) + 11.24;
  BHN = 7.595·Rc + 63.86; BHN = −14.29 + 1.038·Vk − 0.00014·Vk².
- Missing projectile mass, density or size was back-calculated by the
  authors and inserted (p. 9).
- Yaw is the total yaw, combined from two orthogonal angles where needed
  (Eq. 4, p. 9). Blank means not reported. `<1` means the source only said
  "less than 1°".

### References

The reference list (pp. 88–91) is **not numbered**. It is alphabetical by
author and year. Tables cite sources by author and year only. Two "sources"
are letters: Hohler and Stilp (1991a), letter of 21 June 1991, covering
their 1977–1987 Ballistics Symposium and IJIE papers; Hohler and Stilp
(1991b), letter of 6 August 1991 (p. 89). Woolsey (1990) and (1991) are
also letters (p. 91).

The CSV `source_ref` column uses the same author-year form.

## The subset we need

**Tungsten-alloy and DU long rods, L/D about 10–30, into semi-infinite RHA
or armour steel, 1.0–2.0 km/s, 0° obliquity.**

Selection rules used for the CSVs:

- Appendix A only (semi-infinite targets).
- Penetrator density ≥ 15 g/cm³. This drops the low-density Hohler alloys
  H00T, H60T, H70T (13.5–14.8 g/cm³, L/D 10, into HzB,A; pp. 208–210).
- Target is armour steel: RHA, German armour steel (EMI "HzB,A", BHN 255,
  and "Ger Arm St", BHN 295), STA 61 (Tate et al.), or 4340 steel at BHN
  ≥ 250 (the authors group 4340 with armour steel in Fig. 3.1.1, p. 70).
  The CSV `target_material` column keeps the distinction.
- Tabulated L/D between 9.5 and 32.5.
- Impact velocity 1000–2000 m/s inclusive.
- Obliquity 0°.
- Yaw below 3° or not reported. This matches the authors' own limit for
  Eq. (7) (p. 67). 27 shots with yaw ≥ 3° were left out: pp. 227, 235–237,
  279, 305–308, 311, 313, 314 (mostly Woolsey 1990 deliberate-yaw shots and
  Morris and Anderson 1991).

Result: **208 tungsten-alloy rows and 9 DU rows.** Without the velocity
limit there would be 79 more points above 2 km/s and 30 below 1 km/s.

| Source | Penetrator | Target | L/D | Rows | PDF pages |
|---|---|---|---|---|---|
| Hohler and Stilp (1991a) | D17, D17K, D17.6, D18, D18.5, W75, W90, Elmet, pure W (19.3) | HzB,A (BHN 255); Ger Arm St (BHN 295) | 10, 16, 18, 20, 22, 32 | 106 | 195–207, 212, 214–222 |
| Woolsey (1990) | X27, X27C, X21C, W-2 | RHA, BHN 264 | 10 | 46 | 305–315 |
| Woolsey (1991) | X27 | RHA, BHN 264 | 20 | 9 | 316–317 |
| Tate, Green, Chamberlain and Baker (1978) | "W alloy", 17.0 | STA 61, 150 mm block | 12 | 14 | 271–273, 278–279 |
| Morris and Anderson (1991) | X27C | 4340, BHN 264 | 10, 15 | 12 | 235–237 |
| Magness (1990), errata sheets | X9C | RHA | 10, 30 | 9 | 463–464 |
| Magness and Farrand (1990) | 93W | RHA | 20 | 6 | 233–234 |
| Silsby (1984) | W10 | RHA, BHN 232 and 270 | 23 | 3 | 250 |
| Charters, Menna and Piekutowski (1990) | W10 | 4340, BHN 344 | 13 | 1 | 124 |
| Sorensen et al. (1991) | X27 | RHA, BHN 269 | 20 | 1 | 255 |
| Wilkins et al. (1991) | W-10 | German RHA, BHN 390 | 10 | 1 | 284 |
| **DU:** Magness and Farrand (1990) | U-3/4Ti | RHA | 20 | 7 | 232–233 |
| **DU:** Keele, Rapacki and Bruchey (1990) | U-3/4Ti, 266.7 × 13.33 mm | RHA, BHN 269 | 20 | 2 | 227 |

L/D spread of the tungsten rows: L/D 10 dominates (138 rows). L/D 12: 14;
L/D 13–16: 8; L/D 18–23: 34; L/D 30–32: 14.

Rod sizes are mostly model scale: D = 2.8–8.7 mm, L = 28–164 mm. The only
full-scale rods are Sorensen et al. (380 × 19 mm, p. 255) and Keele et al.
(266.7 × 13.3 mm, p. 227).

### How the CSV columns were filled

- `l_over_d` and `p_over_l` are **computed** from L, D and P, not copied.
  Every row was then checked against the tabulated L/D and P/L. Rows where
  computed and tabulated P/L disagree by more than about 1.5% were dropped.
  Mass was also checked against ρ·πD²L/4 (12% tolerance) to catch misread
  diameters.
- `impact_velocity_km_s` is the tabulated m/s value ÷ 1000.
- `nose_shape` is "flat" or "hemispherical" as tabulated.
- `target_hardness_bhn` is blank where the source gave none (36 rows:
  Tate et al., Magness, Magness and Farrand).
- `mass_g` is blank where the OCR row was unreadable or where the table is
  wrong (12 rows; see p. 231 note below).
- Yaw is not a CSV column. Kept rows have yaw < 3° or none reported.

### Spot checks

These rows were checked by eye against the page text or the page image
(page, shot): 198/5, 199/2, 199/6, 203/3, 204/3 (image), 206/3, 221/3,
229 (image), 233/3, 250/3, 272/3, 255/1, 227/5–6, 124/1, 314/7, 316/1–7,
463/1–7, 464/1–2. All match.

### Corrections applied

- p. 204, shot 3: OCR gives D = 5.3 mm. The image shows 5.8 mm. Material
  read as blank; image shows D18. Corrected.
- p. 204, shot 6: target BHN OCR "2!5"; set to 255 like the rest of the
  page.
- p. 231, shot 4 (superseded anyway): mass 19.00 g is a table error for a
  76.8 × 7.68 mm rod.

### Oblique and finite-thickness data

- **Oblique, semi-infinite:** Tate et al. (1978), "W alloy" L/D 12 into STA 61
  (130–150 mm blocks) at 30°, 45°, 60° and 70°; 25 shots on pp. 279–283.
  One Magness and Farrand (1990) DU shot at 9° (p. 232). These are not in
  the CSVs. Check how P was measured (along the shot line or normal to the
  face) before using them.
- **Finite thickness, residual velocity** (Appendix B-1, pp. 320–377): Hohler
  and Stilp (1991a) on pp. 328–354; Lambert (1978) 65 g rods into armour
  plate on pp. 355–375; DU (U-0.75Ti) obliquity and yaw tests by Fugelso
  (pp. 322–327). Graphs on pp. 43–48, cross-plots on pp. 77–79.
- **Finite thickness, limit velocity** (Appendix B-2, pp. 378–440): Grabarek
  (1971), many obliquities (pp. 379–408); Hohler and Stilp (1991a)
  (pp. 409–426); Lambert; Magness; Magness and Farrand; Tate et al.
  (pp. 427–440). Graphs on pp. 49–59; cross-plots on pp. 80–83.

## Trends and fits given by the authors

- **Hydrodynamic limit.** At high velocity, P/L for long rods tends to
  √(ρp/ρt), about 1.5 for tungsten alloy into steel (pp. 11, 66; Fig. 3.1.1
  on p. 70). Below that, P/L for long rods never exceeds it (p. 12).
- **L/D effect.** At a given velocity, P/L falls as L/D rises (Fig. 3.1.1,
  p. 70; regression lines for L/D 1, 3, 6, 10, 20, 30). Short rods (L/D 1)
  exceed the hydrodynamic limit and keep rising with velocity.
- **Alloy composition.** For a fixed L/D, tungsten alloys of different
  composition give nearly the same P/L (Fig. 3.1.2, p. 70; text p. 66).
- **Energy.** For a fixed mass, longer rods penetrate deeper at almost every
  velocity, and the velocity that maximises depth for a given kinetic energy
  is lower for long rods (Fig. 3.1.3, p. 71; text p. 66).
- **DU against tungsten.** At the same L/D, DU and tungsten data agree, but
  below about 1.5 km/s DU gives about 10% more P/L (Fig. 3.1.4, p. 71;
  text p. 67).
- **Target hardness** matters more than target density at ordnance
  velocities (Fig. 3.1.5, p. 72; text p. 67).
- **Yaw** reduces P/L; effect grows with L/D and as velocity falls (Figs
  3.1.6–3.1.7 for W into RHA at L/D 10 and 20, pp. 72–73; text pp. 68–69).
- **Crater diameter** d/D rises with velocity, roughly independent of L
  (Fig. 3.1.10, p. 74).
- **Regression fit, Eq. (7), p. 67** (read from the page image; the OCR text
  is garbled):

  P/L = 1.121 · (ρp/ρt)^0.3704 · tanh[ 0.01750 · (L/D)^−0.3020 · (ρp·V²/St)^1.137 ]

  Valid for L/D ≥ 10, total yaw δ < 3°, θ = 0°, ρp·V²/Sp > 20 (excludes
  rigid-body penetration), ρt = 7.85 g/cm³. St and Sp are the static
  tensile strengths of target and projectile; the group ρp·V²/St is
  dimensionless (use consistent units, e.g. kg/m³, m/s, Pa). Projectile
  strength does not appear. r = 0.9838. The asymptote is not forced to the
  hydrodynamic limit, because that gave a worse fit (p. 67). The same
  equation is overlaid on limit-velocity data for L/D 10 (Fig. 3.2.7, p. 82;
  text p. 80).
- **Residual velocity fit, Eq. (5), p. 49:** Vr = a·(Vs^p − VL^p)^(1/p),
  fitted for a, p and VL. Limit velocities from Fig. 3.2.1 are 721, 1145 and
  1277 m/s (p. 75).
- **Finite against semi-infinite:** t/L at the limit velocity follows the
  same shape as P/L against V, but P/L is slightly smaller than t/L because
  of reduced resistance near the rear face (p. 80).

### Our check of Eq. (7) against the extracted rows

We converted target BHN to tensile strength with the inverse of Eq. (1)
(p. 8) and used BHN 270 where none was given. Mean measured/predicted P/L:

| Group | Rows | Mean ratio | SD |
|---|---|---|---|
| W, L/D 10–12 | 152 | 0.98 | 0.06 |
| W, L/D 13–16 | 8 | 1.02 | 0.07 |
| W, L/D 18–23 | 34 | 0.97 | 0.11 |
| W, L/D 30–32 | 14 | 0.85 | 0.16 |
| DU, L/D 20 | 9 | 1.14 | 0.07 |

Eq. (7) fits tungsten rods up to L/D about 23 to within a few per cent on
average. It over-predicts the L/D 30–32 rods by about 15%. It under-predicts
DU by about 14%, which agrees with the authors' DU remark (p. 67).

Binned means for tungsten L/D 10 (138 rows): P/L ≈ 0.41 at 1.0–1.2 km/s,
0.70 at 1.2–1.4, 0.89 at 1.4–1.6, 1.08 at 1.6–1.8 and 1.19 at 1.8–2.0 km/s.
For L/D 18–23: 0.32, 0.52, 0.77, 0.94 and 1.15 in the same bins.

## Limits

- **No full-scale modern APFSDS.** Almost all rods are 3–9 mm diameter model
  rods (1977–1991 tests). Only two sources use rods of 13–19 mm diameter.
  Scale effects are not addressed.
- **L/D gap.** Very few points between L/D 23 and 30 (only Magness X9C at
  L/D 30 and Hohler D17.6 at L/D 32). No data for jacketed, finned or
  sabot-carrying rods; all are bare cylinders with flat or hemispherical
  noses.
- **Few DU points** in our band (9), at only L/D 20.
- **Target variety.** "Armour steel" covers BHN 232–390 across sources, and
  hardness is missing for 36 rows. STA 61 hardness is not given.
- **Only normal impact** in the CSVs. Oblique semi-infinite data are limited
  to one source (Tate et al.).
- **Yaw** is often reported only as "<1" or not at all.
- **Errata.** The April 1992 errata (p. 457) say some Magness (1990) points
  were entered twice, once as tungsten and once as DU, and some table data
  were not copied across. The replacement sheets are pp. 459–464. We used
  replacement pages 463 (A-137) and 464 (A-138) and **ignored the original
  Magness pages 229–231 and the original Luk and Piekutowski page 228**. The
  replacement for A-136 is not in this PDF. On the original p. 229, shots
  5–7 have P/L values that do not match L = 106 mm; they look like L/D 10
  rods. The July 1992 errata (p. 466) replace A-199/A-200 (Wilson et al.,
  steel rods, pp. 467–468); not used here.
- **OCR quirks** handled by checks, not trusted blindly: target ρ/BHN rows
  sometimes come out as vertical stacks (e.g. pp. 162, 199, 314); labels are
  often misspelt; commas appear for decimal points ("118,2"); "I", "l" and
  "!" for "1"; "O" for "0".
- **Pages without usable text:** the cover (p. 1) and divider pages (pp. 92,
  318, 320, 378, 441, 456, 465) carry little or no text. Figure pages
  (pp. 14–40, 43–59, 62–65, 70–74, 77–79, 82–85, 459–461) only give
  labels; plotted points are image-only. In the subset we need, every table
  page had a usable text layer; no table was skipped for unreadable text.
  Tables outside our subset were not checked line by line (e.g. Perez 1982,
  pp. 238–248, has a different layout).

## How we will use it

1. Load the tungsten CSV. Fit P/L against impact velocity separately for L/D
   bands: 10 (dense, 138 points), 12–16, 18–23 and 30–32.
2. Use a long-rod form for the fit. Options: the Lanz–Odermatt form
   P/L = a·exp(−(s²)/V²) with the L/D and density terms; or Walker–Anderson
   with target resistance fitted per band. Eq. (7) of this report is a ready
   baseline and already fits L/D 10–23 to about ±6–11% (1 SD).
3. Treat L/D 30–32 with care: Eq. (7) over-predicts them by about 15%; fit
   them on their own or add an L/D correction.
4. For DU rounds, either fit the 9 DU rows directly or scale tungsten P/L by
   about +10–14% below 1.5–1.7 km/s, as the data and the authors suggest.
5. Per round in the catalogue: compute impact velocity at range from drag,
   then P = L·(P/L)(V, L/D, ρp). This replaces the single blanket
   range-retention curve with one that depends on each round's L/D and
   velocity loss.
6. Check the fit against target hardness: the data span BHN 232–390. Keep a
   hardness term or restrict to BHN 255–270 for RHA.
