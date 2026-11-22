# Rheinmetall datasheets: medium-calibre cannons and ammunition

**Covers** (all one- or few-page manufacturer datasheets; the date is in
each document code, e.g. `D129e07.21` = July 2021):

| File (`raw/`) | Code | Subject |
|---|---|---|
| `ammunition/rheinmetall-2021-apfsds-t-30x173-pmc287.pdf` | D129e0721 | APFSDS-T 30×173 PMC287 (for the Mk44 Bushmaster II) |
| `ammunition/rheinmetall-2021-apfsds-t-35x228-pmd343.pdf` | D251e0721 | APFSDS-T 35×228 PMD343 (for the Bushmaster III) |
| `weapons/rheinmetall-2026-mk30-2-abm-automatic-cannon.pdf` | B305e0526 | MK30-2/ABM cannon and its 30×173 family: APFSDS-T PMC359, ABM/KETF PMC308, HEI-T PMC416, TP-T PMC355, TPFDS-T PMC307 |
| `weapons/rheinmetall-2026-mk35e-automatic-cannon.pdf` | B304e1126 | MK35-E cannon and its 35×228 family: APFSDS-T PMD343, ABM/KETF PMD330, HEI-T PMD040, TPFDS-T PMD346 |
| `weapons/rheinmetall-2025-oerlikon-kba-25x137-cannon.pdf` | D867e0125 | Oerlikon KBA 25×137 cannon, with flight times to range |
| `weapons/rheinmetall-2021-sea-snake-20-naval-gun.pdf` | D956e0721 | Sea Snake 20 naval mount (KAE 20×128) |
| `weapons/rheinmetall-2021-sea-snake-27-ciws.pdf` | D795e0721 | Sea Snake 27 naval mount (BK27 27×145) |
| `weapons/rheinmetall-2025-sea-snake-30-naval-rcws.pdf` | B432e1225 | Sea Snake 30 naval mount (KCE-30 30×173), with EO detection ranges |

Read on 22 November 2026. Each sheet is one to six pages; figures are
quoted as printed. These are sales documents: they state performance
without test conditions except where noted, and the sheets' disclaimers
say the data "have only an information value".

**Tables:** `data/rheinmetall-datasheets--medium-calibre-rounds.csv`,
`data/rheinmetall-datasheets--cannons.csv`,
`data/rheinmetall-datasheets--kba-flight-times.csv`,
`data/rheinmetall-datasheets--sea-snake-eo-ranges.csv`.

## Rounds

| Round | Type | Projectile (g) | Muzzle velocity (m/s) | Penetration | Other |
|---|---|---|---|---|---|
| 30×173 PMC287 | APFSDS-T | 235 | 1,405 | **53 mm at 60° (NATO) at 1,000 m** | tungsten rod; dispersion ≤0.5 mil |
| 30×173 PMC359 | APFSDS-T | 235 | 1,405 | 53 mm at 60° (NATO) at 1,000 m | same data as PMC287, for the MK30-2 |
| 30×173 PMC308 | ABM/KETF airburst | 360 (201 g payload: 162 × 1.24 g tungsten) | 1,100 | – | fuze set at the muzzle; 2 ms resolution; self-destructs 8.2 s (≈4 km) |
| 30×173 PMC416 | HEI-T | 360 | 1,100 | – | STANAG 4187 fuze, self-destruct |
| 35×228 PMD343 | APFSDS-T | 297 | 1,400 | **not given** ("effective against RHA and modern high-hardness spaced targets"; operational distance "up to 2,000 m" / "more than 2,000 m") | cobalt-free tungsten |
| 35×228 PMD330 | ABM/KETF airburst | 750 (500 g payload: 407 × 1.24 g) | 1,050 | – | self-destructs 8.2 s (≈5 km) |
| 35×228 PMD040 | HEI-T | 535 | 1,175 | – | |

"53 mm at 60° (NATO)" means a 53 mm RHA plate set at 60° from the
vertical, which is 106 mm of steel along the line of flight. It is a
perforation figure for a sloped plate, not a depth into a thick block.

## Cannons

| Cannon | Calibre | Rate of fire | Mass | Effective range | Sheet |
|---|---|---|---|---|---|
| MK30-2/ABM | 30×173 | 200 rds/min controlled; 600–650 max | 210 kg (350 kg with cradle) | "up to 3,000 m" | B305 |
| MK35-E | 35×228 | single, up to 200 rds/min | – | "up to 3,000 m" | B304 |
| Oerlikon KBA | 25×137 | up to 600 rds/min; rapid single 175 | 112 kg | "up to 2,500 m" | D867 |
| KAE (Sea Snake 20) | 20×128 | 1,000 rds/min | mount 460 kg | – | D956 |
| BK27 (Sea Snake 27) | 27×145 | 1,700 rds/min | mount 449 kg | – | D795 |
| KCE-30 (Sea Snake 30) | 30×173 | – | – | – | B432 |

The MK30-2/ABM is in the Puma IFV, the Boxer with the Lance turret
(Australia) and the Lynx KF41 with Lance (Hungary) (B305 p. 2). The
MK35-E is in BAE's CV9035 (B304).

## Flight times: the one measured drag figure (D867, p. 2)

The KBA sheet gives shell flight times "depending on ammunition type
(sub- or full-calibre)":

| Range | Flight time |
|---|---|
| 1,000 m | 0.75–1.17 s |
| 1,500 m | 1.15–2.07 s |
| 2,000 m | 1.57–3.30 s |

with muzzle velocity 1,100–1,400 m/s. Taking the short times as the
fastest sub-calibre round and the long ones as full-calibre HEI, and
fitting v(x) = v0·exp(−x/λ):

- **Sub-calibre:** v0 = 1,395 m/s, λ = 11.2 km fits all three times
  exactly. Speed 1,276 m/s at 1 km and 1,167 m/s at 2 km: **about
  120–125 m/s lost per km.**
- **Full-calibre HEI:** λ ≈ 1.8–2.1 km, 425–465 m/s lost in the first km.

The sheet does not say which sub-calibre round (APFSDS-T or FAPDS-T) the
short times are for. This is our fitted reading of the sheet, not a figure
Rheinmetall prints.

## Sea Snake 30 sensors (B432, PDF p. 4)

Daylight camera and cooled MWIR imager, in clear air (visibility 23 km;
IR per STANAG 4347):

| | Detection | Recognition | Identification |
|---|---|---|---|
| Day camera (narrowest FoV 1.35°) | >18,000 m | >9,000 m | >5,000 m |
| MWIR 3.6–4.15 µm (NFoV 1.40°) | >18,000 m | >9,000 m | >5,500 m |

Laser rangefinder 1,550 nm: up to 6,000 m on a 2.3 × 2.3 m target. The
target for the DRI figures is not stated; the STANAG 4347 reference
implies a standard NATO target, and the Sea Snake 20/27 sheets show the
same bars graphically without numbers.

## Checks against the catalogue CSV (`data/catalogue/csv/`)

- **`m_mk30_ap` is labelled "PMC308 APFSDS-T (30×173)"**, but PMC308 is
  the ABM/KETF airburst round. The APFSDS-T rounds are PMC287 (Mk44)
  and PMC359 (MK30-2). Its penetration, 100 mm at 1,000 m (source "Rheinmetall:
  >100mm @1000m"), is close to the line-of-flight thickness of the
  datasheet's 53 mm at 60° (106 mm), so the number is defensible; the
  name and source note are wrong.
- **`m_35_apfsds` is labelled "35mm PMC308 APFSDS-T"**; the 35 mm
  APFSDS-T is PMD343. Rheinmetall gives no penetration for it. The
  catalogue's 164/159/150 mm ("~140–160 mm RHA @1000 m") has no source in
  these sheets.
- **Range fall-off for both is too flat.** With the measured 25 mm
  decay and the Silsby long-rod equation, a 1,400 m/s rod keeps about 80%
  of its muzzle penetration at 1 km and 62% at 2 km
  (`toksoy-2025-tungsten-long-rod-penetration-models.md`). The catalogue
  keeps 97% and 91–92%.
- Muzzle velocities, projectile masses and airburst payloads above can go
  straight into the catalogue: it has no velocity or mass column for
  munitions at present.

## Limits

Sales datasheets. Penetration conditions are given only for PMC287/359.
Effective ranges are the maker's claims. The flight-time fit rests on one
sheet's three points and on our reading of which round they are for.
