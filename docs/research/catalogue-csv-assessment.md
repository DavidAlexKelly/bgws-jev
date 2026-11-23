# The asset catalogue CSVs: an assessment

> **Status, 23 November 2026.** Done since this was written:
>
> - **Importer.** `npm run catalogue:import` (`src/scripts/catalogue-import.ts`,
>   logic in `src/apps/bgws/realtime/catalogue/csvImport.ts`) writes
>   `data/catalogue/imported.json`: land platforms as overlay records in the shape of
>   `sourced.json` and munitions in the shape of `munitions.json`. A figure goes in only with a
>   row-level source, and `figures` on each record says which. Everything left out is in
>   `data/catalogue/imported-rejections.csv` with the reason. First run: 114 platforms and
>   119 munitions imported, 673 figures or rows rejected (93 munitions on the blanket README
>   reference; most mass, engine and speed figures for want of a row-level source). The test
>   (`csvImport.test.ts`) fails if a figure is imported without a source, or if the committed
>   files are out of date. No engine reads `imported.json` yet: it is staged for the Rust loader.
> - **Quality columns**, from what the tables already say, no new source:
>   - Platforms gain `is_towed` (8), `speed_road_kmh`, `speed_xc_kmh` (left empty: never
>     invented), `speed_note`, and `mass_inherited` / `engine_inherited` (14 / 13 variants copy the
>     base vehicle's figure).
>   - Munitions gain `pen_basis` (`rha`, `behind_era` or `unstated`) and `era_effect`
>     (`precursor_defeats_era`, `era_degrades`, `defeats_era`).
>   - The 93 munitions that cite only the README are regraded to D. Their old grade is kept in
>     `source_confidence_original`.
>
>   The import adds these columns and is idempotent.
>
> Still open from the list below: per-row sources for the 93 munitions, a newer source for the
> T-72B3, T-80BVM and T-90M, `has_smoke`, cross-country speeds, the `bgws_cs_index` formula, and the
> support-asset timings.

`data/catalogue/csv/`, added 22 November 2026. Six files:

| File | Rows |
|---|---|
| Platforms | 579 (land 174, sea 236, air 169) |
| Capabilities | 954 |
| Munitions | 213 |
| Support assets | 350 |
| Sections | 20 |
| Image index | 733 |

`tools/check_catalogue_csv.py` reruns every check below.

## Summary

The files are **internally sound but thinly sourced**.

**What holds:**
- The links between files all resolve: capabilities to platforms,
  support payloads to munitions, platform images to the image index (2
  missing).
- The stated calculations are applied correctly (power-to-weight).
- The physical rules hold: penetration never rises with range; a shaped
  charge's penetration does not change with range.

**What is weak is where the numbers come from:**
- Much of the data is a class default or a blanket formula, not a value
  for the asset.
- Most rows have no source a reader could check.
- The variants copy their base model's figures.

Nothing in the sim reads these files yet. The game's catalogue is
`data/catalogue/*.json`: 105 platforms, in the sim's own schema. The CSVs
are a larger, separate source in another schema (turns, protection bands,
capability tags), to be mapped in rather than a correction to what is
loaded.

## What the checks found

**Provenance (the main weakness):**
- **Munitions:** 93 of 213 cite only "published reference tables /
  manufacturer data (see README sources)". There is no README with the
  CSVs, so these can't be traced.
- **Platform mass:** `mass_source` is "curated_equipment_table" for all 579,
  with no row-level source.
- **Support assets:** 312 of 350 have class-typical or class-default
  loadouts and timings ("MF identity; class-default loadout and timings";
  "magazine/salvo sizes class-typical").
- **Confidence grades are generous:**
  - 185 of 213 munitions are rated B, including the 93 with no traceable
    source;
  - all 20 sections are B or C, though "published TO&E" is the only
    reference given for 17;
  - platform protection has no grade on 397 of 579 rows, and 56 are
    rated D.

**Blanket calculations:**
- **APFSDS penetration against range:** every round, from a 30 mm cannon
  to a 125 mm tank gun, keeps 97.2% at 1 km, 91.7% at 2 km and 86.2% at
  3 km (19 rounds). That is one curve applied to all. In reality the loss
  per km depends on the round:
  - its launch velocity;
  - its drag (sectional density, L/D);
  - how penetration scales with velocity.

  Small-calibre sub-calibre rounds lose much more per km than a 120 mm
  rod. The curve also loses more in the second km (5.6%) than the first
  (2.8%). Velocity falls roughly linearly with range, so penetration
  should too.
- **Variants:** all 33 land rows that share a base with another row have
  the same mass and engine as a sibling. Up-armoured and re-engined
  variants get their base model's figures, and so its power-to-weight.
  Examples:
  - T-72 (Ural) and T-72B3 obr. 2016: both 46 t and 839 hp. The B3 obr.
    2016 has the 1,130 hp V-92S2F (recalled).
  - T-80 and T-80BVM: both 46 t and 1,250 hp.
  - Ajax and Ares; Boxer and Boxer (GMG fit); Warrior and Warrior TES(H).
- **Support timings:** 125 of 183 fire-support rows share response 2,
  loiter 12, 3 sorties a day. That covers every destroyer and frigate's
  naval gunfire alike.

**Values that look wrong.** These are checked against my own recall of
published figures, not a source in hand, so check them before changing:

| Asset | CSV | Published (recalled) |
|---|---|---|
| General Dynamics Ajax | 30 t | about 38 t, up to 42 t |
| ARTEC Boxer | 25.6 t | about 33–38.5 t |
| M3 Bradley | 22.66 t | about 30–33 t, as the M2 (36.66 t here) |
| BMP-2 | 399.6 hp | UTD-20, 300 hp (the "Russian 2020s" row has 300) |
| T-72 (Ural) | 46 t, 839 hp | about 41 t, V-46 780 hp |
| T-80 (MBT) | 46 t | T-80B about 42.5 t |
| BTR-82A | two rows, 13.6 t and 15.4 t | one vehicle, about 15.4 t |

**Checked against sources (22 November 2026).** The sources in
`docs/sources/` now settle several of these:

| Asset | CSV | Source | Where |
|---|---|---|---|
| BMP-2 engine | 399.6 hp | 300 hp | WEG 2001, PDF p. 97 |
| T-72B | 46 t, 839 hp (as "T-72 (Ural)" and "T-72B3") | 44.5 t, 840 hp | WEG p. 152 |
| T-72M1 (export T-72A) | – | 41.5 t, 780 hp | WEG p. 154 |
| T-80 | 46 t, 1,250 hp | T-80B/BV 44.5 t, 1,000–1,100 hp; 1,250 hp is the T-80U's | WEG pp. 156, 158 |
| T-90 | 46.5 t, 1,250 hp (the T-80U's figure again) | 840–1,000 hp | WEG pp. 160–161 |
| T-55 engine | 80.5 hp (looks like 60 kW mis-converted) | 620–690 hp (T-55AMV) | WEG p. 146 |
| BTR-60 engine | 89.8 hp (one of its two engines) | 2 × 90 hp | WEG p. 73 |
| SA-6 (2K12) | 25 t, 65 km/h | 14 t, 45 km/h | WEG p. 282 |
| BTR-D | 8.2 t | 6.7 t | WEG p. 77 |
| 30 mm "PMC308 APFSDS-T" (`m_mk30_ap`) | named after the airburst round | APFSDS-T is PMC287 / PMC359: 1,405 m/s, 53 mm at 60° at 1,000 m | Rheinmetall D129, B305 |
| 35 mm "PMC308 APFSDS-T" (`m_35_apfsds`) | 164 mm at the muzzle | PMD343: 1,400 m/s; no penetration published | Rheinmetall D251 |

The full comparison of 53 rows is in
`docs/sources/notes/tradoc-2001-worldwide-equipment-guide.md`, with the
uncertain matches listed separately. The 2001 guide predates the T-72B3,
T-90M and T-80BVM, so it confirms that their figures are copied, not what
they should be.

**Corrected (22 November 2026).** In the platform CSV:

- **Source column:** a new `figure_source` column records where each
  corrected figure came from.
- **Corrected rows:** BMP-2 (and the BMP-2M's engine), T-72 (Ural),
  T-80, T-90, T-55 engine, BTR-60, BTR-D and SA-6 now carry the WEG
  values, and `hp_per_tonne` is recomputed.
- **SA-6 engine:** blanked. Its 839 hp was copied from another row.
- **Still unsourced:** the T-72B3, T-80BVM and T-90M keep their copied
  figures, marked `UNSOURCED` in `figure_source` until a newer source is
  found.

In the munition CSV, `m_mk30_ap` is renamed PMC287/PMC359 and
`m_35_apfsds` PMD343, with their sources stated. The 35 mm round's
penetration is graded D because Rheinmetall publishes none.

**Penetration against range, measured.** The 25 mm KBA cannon sheet gives
flight times to 2 km. Fitted, a sub-calibre round loses about 125 m/s per
km. With the long-rod fits in Toksoy (2025) and the SwRI database
(Anderson et al. 1992; 208 measured tungsten-rod shots), a 25–35 mm rod
keeps about 80–85% of its muzzle penetration at 1 km and 62–73% at 2 km
(the range covers the two fits and 25 against 35 mm); the CSV gave 97%
and 92%. For a tank rod (faster, slowing less) the same
fits give a fall close to the CSV's curve. Details:
`docs/sources/notes/rheinmetall-datasheets-medium-calibre.md`,
`toksoy-2025-tungsten-long-rod-penetration-models.md`,
`anderson-1992-penetration-mechanics-database.md`.

**Gaps:**
- **Smoke:** `has_smoke` is false on every platform, and
  `smoke_max_range_m` and `l6_asset_id` are empty. Most armoured vehicles
  carry smoke dischargers. The sim uses them.
- **Engines:** `engine_hp` is given for 141 rows (137 land, 4 air).
- **Speeds:**
  - 0 km/h for towed systems (L118, Rapier). Better marked as towed, with
    the prime mover's speed.
  - 120–145 km/h for some light vehicles (Jackal, LMV, ISV). These are
    road maxima; cross-country speed is far lower and matters more.
- **Tandem warheads** give penetration "behind ERA" (Kornet 1,200 mm,
  TOW-2A 900 mm), while other rounds give it against plain armour (RHA).
  If the sim models ERA separately, a "behind ERA" figure counts the ERA
  twice. The two need separating, with a field for each.
- **Combat strength index:** `bgws_cs_index` has no stated formula. It
  presumably lives in the missing README.

## How to improve the sources

**Per row, not per file.** Add `source_url` (or document) and
`source_locator` (page, table, section) to every row and every
calculated column. Grade confidence by what is cited:

| Grade | Meaning |
|---|---|
| A | Read in an official or manufacturer document |
| B | Two independent open sources agree |
| C | One open source, or a derivation from sourced values |
| D | Estimate or class default |

A blanket reference is grade D, whatever it says.

**Better public sources, by family:**
- **Land vehicles (mass, power, speed, armour, armament, crew):** the US
  Army TRADOC *Worldwide Equipment Guide* (WEG). It is public, on ODIN,
  covers most Russian, Chinese and other threat systems with sourced
  tables, and its versions are dated.
  - For NATO vehicles: national fact files (army.mod.uk, army.mil fact
    files, Bundeswehr), manufacturer datasheets, and parliamentary or GAO
    reports.
  - For inventories: IISS *The Military Balance*.
  - Jane's is the standard paid source.
- **Ammunition (velocity, penetration):** manufacturer datasheets
  (Rheinmetall, Nammo, General Dynamics OTS, Bazalt, KBP); NATO STANAG
  4385 (test conditions); DOT&E annual reports (for US munitions:
  reliability and test hit rates).
- **Armour estimates:** keep them as ranges with their spread (low,
  central, high), not single numbers. Cite the estimate's origin. The
  open figures for composite armour are estimates; say so.
- **Naval gunfire, air sorties and loiter:** doctrine publications
  (sortie rates, response times) and the sim's own vignettes
  (`sortie-surge`, `sortie-sustained`, `detect-to-strike`), which already
  hold sourced bands to check against.

## How to improve the calculations

1. **Long-rod penetration against range, per round. Done:
   `tools/penetration_model.py`, inputs in `data/catalogue/ke_rounds.csv`.**
   - **Velocity at range:** v(x) = v₀·exp(−x/λ). λ is 11.2 km for 25 mm,
     fitted to the KBA's flight times, and about 24.5 km for 120 mm, from
     published losses of 62 m/s at 1 km and 123 m/s at 2 km (M829A1).
     λ = 11.2 km·(calibre/25)^0.5 reproduces both, and is used between and
     beyond them.
   - **Penetration at that velocity:** the SwRI fit (Anderson et al. 1992,
     eq. 7). It is within 1% on average of 194 measured shots with L/D
     10–23; it over-predicts L/D 24–33 rods by 12%, which the anchoring
     removes.
   - **Anchoring:** each curve passes through the round's published
     penetration at its published range, so only the shape comes from the
     model.
   - **Applied to:** the 19 APFSDS rows of the munition CSV and the 7
     kinetic rounds in the sim's `munitions.json`. APDS rounds (short,
     spin-stabilised cores) are left out; the fit is for long rods.
   - **Result:** 2 km/muzzle ratios now run from about 0.70 (25 mm) to
     0.87 (older tank rounds), not 0.92 for all. For the 30 mm round:
     104, 89, 75 and 63 mm at 0, 1, 2 and 3 km. Its anchor is the
     thick-block depth for "53 mm at 60°": 89 mm, not the 106 mm line of
     flight (Mullin et al. 1995 put it at 1.6–2.0 times the plate figure).
   - **Thin plates:** a kinetic rod perforates up to 1.05 times its
     thick-block depth (`direct.ke_limit_ratio`, Mullin et al. 1995).
   - **Still to source:** most muzzle velocities are recalled reference
     figures, marked as such in `ke_rounds.csv`. So are the rods' L/D and
     the carbide-cored rounds' density. `--check` prints the model against
     its sources.
2. **Shaped charges.** Keep penetration constant with range (correct now).
   Record against plain armour and the effect of ERA separately (a tandem
   precursor defeats ERA; a single charge loses roughly 50–80% to it).
3. **Variants.** Inherit only the fields a variant does not change, and
   flag inherited values (`mass_inherited: true`) so they are visible and
   can be sourced in turn.
4. **Power-to-weight.** Compute it from the variant's own mass and engine,
   as now, once those are its own.
5. **Speeds.** Separate road, cross-country and swimming speeds. The sim's
   movement uses cross-country going.
6. **Support timings.** Replace the class defaults with per-type values
   where doctrine or exercise data gives them, and state the turn length
   that converts turns to seconds.
7. **Combat strength index.** Write down its formula and inputs, or drop
   it if the sim does not need it. The sim computes effects from the
   physical figures.

## Mapping into the sim

When the CSVs are imported into the JSON catalogue:
- **Map fields to fields:**
  - mass → `body.massT`;
  - power and speeds → `mover`;
  - armour by facing → `body.armour.ke` / `ce`;
  - munitions → `munitions.json`, with penetration as a function of range.
- **Keep the confidence and source with each value,** as the figures
  registry already does: sources, and `[default]` for unsourced.
- **Check against the existing hand-built entries first** (`land.json`,
  `sourced.json`), then let the calibration vignettes judge the result.

An importer with a test that rejects rows missing a source would keep the
catalogue honest as it grows.
