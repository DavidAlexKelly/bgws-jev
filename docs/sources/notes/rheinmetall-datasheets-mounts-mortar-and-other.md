# Rheinmetall datasheets: weapon mounts, mortar, ship sensors and other

**Covers:**

| File (`raw/`) | Code | Subject |
|---|---|---|
| `weapons/rheinmetall-2026-rn200-weapon-mount-mag58-minimi.pdf` | D515e0426 | RN-200 mount for MAG58 / Minimi |
| `weapons/rheinmetall-2026-rn300-weapon-mount-mg4-mg5.pdf` | D1137e0426 | RN-300 mount for MG4 / MG5 |
| `weapons/rheinmetall-2026-rn500-weapon-mount-m2-hmg.pdf` | D1219e0326 | RN-500 mount for the M2 .50 HMG |
| `weapons/rheinmetall-2026-rn600-weapon-mount-hmg-agl.pdf` | B278e0326 | RN-600 mount for HMG and 40 mm AGL |
| `weapons/rheinmetall-2026-rn-tripods.pdf` | D433e0426 | RN-10/20/30 tripods |
| `weapons/rheinmetall-2026-ragnarok-120mm-mortar-system.pdf` | B403e0526 | MWS120 RAGNAROK 120 mm vehicle mortar |
| `sensing/rheinmetall-2021-seavision-eo-sensor-platform.pdf` | D862e0721 | SeaVision naval EO sensor head |
| `sensing/rheinmetall-2021-seastorm-sensor-data-systems.pdf` | D864e0721 | SeaStorm ship weather and navigation sensors |
| `other/rheinmetall-2023-emma-ammunition-disposal-module.pdf` | B338e0323 | EMMA: disposal of dumped sea munitions |

Read on 22 November 2026. Sales datasheets; little in them bears on the
model. What does:

## RAGNAROK 120 mm mortar (B403)

- **Rate of fire 18–20 rounds/min**, manually loaded ("almost double"
  that of autoloaded or turreted systems, the sheet claims).
- Aiming accuracy <2 mils; laying is automatic, with roll and pitch
  compensated up to ±20°.
- Crew of 2 (loader and assistant).
- On the trailer version: **in and out of action in about 45 s**; trailer
  about 3 t, 80 km/h. The container module stows about 48 rounds and
  installs in at most 20 minutes.
- Fitted to Eyder 4×4, Boxer 8×8, BvS10 trailer, Caracal, ACSV G5, HX 8×8.

The in/out-of-action time and rate of fire are inputs the fires model
uses for "shoot and scoot" mortars. They are the maker's figures.

## Weapon mounts (D515, D1137, D1219, B278, D433)

The mounts all claim better accuracy and first-hit probability from
recoil damping, without numbers, except:

- RN-500: recoil reduced "up to 80%" for .50 cal; mount <12 kg; elevation
  −25° to +55°.
- RN-600: recoil reduced >80% (.50 cal) and >50% (AGL); <14 kg.
- RN-200: <3.0 kg. RN-300: elevation −30° to +60°.
- Tripods: RN-10 3.7 kg, RN-20 6.7 kg, RN-30 12.0 kg.

There is no hit-probability or dispersion figure we can use.

## SeaVision (D862) and SeaStorm (D864)

SeaVision is the sensor head on the Sea Snake mounts: three day cameras
(narrowest 1.40°, widest 32°), a cooled MWIR imager (1,024 × 768,
1.40°/6.67°), and two laser rangefinders (≥10 km and ≥40 km). Its
detection, recognition and identification ranges appear only as an
unlabelled bar chart; the Sea Snake 30 sheet gives them as numbers (see
`rheinmetall-datasheets-medium-calibre.md`). SeaStorm is a ship's weather
and position sensor suite (wind to 75 m/s, GNSS heading to 0.05°); nothing
for the model.

## EMMA (B338)

A system for recovering and destroying munitions dumped at sea. Off-topic.
