# TRADOC Worldwide Equipment Guide (September 2001)

**Source:** US Army Training and Doctrine Command, Deputy Chief of Staff for
Intelligence (TRADOC DCSINT), Threat Support Directorate, Fort Leavenworth.
*Worldwide Equipment Guide* (WEG), update of September 2001 (cover
memorandum dated 24 September 2001). Approved for public release;
distribution unlimited (PDF p. 5).

**File:** `docs/sources/raw/equipment/tradoc-2001-worldwide-equipment-guide.pdf`
(414 PDF pages; the PDF itself was built in November 2003).

Read on 22 November 2026. Page numbers below are **PDF page numbers**
(1-414). The guide's own sheet numbers (e.g. "4-15") are given in
brackets where useful. Figures are quoted as printed.

**Tables:**
`data/tradoc-2001-worldwide-equipment-guide--land-vehicles.csv` (99 rows)
and `data/tradoc-2001-worldwide-equipment-guide--munitions.csv` (122 rows).

## What the guide is

- A loose-leaf reference for training, simulation and modelling. It lists
  the "basic characteristics" of equipment available to the opposing force
  (OPFOR) in the FM 100-60 and FM 100-63 series (PDF p. 6).
- Each system has a one- or two-page data sheet in a fixed format: SYSTEM,
  Automotive Performance, Protection, ARMAMENT, FIRE CONTROL, VARIANTS,
  MAIN ARMAMENT AMMUNITION, NOTES.
- It is a mix of editions. Sheets carry either "7 Nov 2000" or "Sep 2001"
  in the header. The September 2001 supplement replaced or added the
  pages listed at PDF p. 12 (e.g. Kliver turret, Type 63A, Ch. 14).
- It covers Soviet/Russian, Chinese, North Korean and other threat systems,
  and some Western systems that an OPFOR might own (Leopard 1/2,
  Challenger 2, M60, M113, Warrior, Marder).

## Limits

- **2001 data.** No T-72B3, T-72B3M, T-80BVM, T-90M/T-90A, BMP-2M, BMP-3
  upgrades beyond the "BMP-3M" variant note (PDF p. 10), BTR-82A, BMD-4,
  2S19M2, 2S35, Pantsir, Tor-M2 or S-400. The T-90 sheet is the early T-90
  and T-90S (PDF p. 160).
- **Not test data.** The guide says protection figures on the sheets are
  "published data provided for general information use" and may not match
  vulnerability data from certifying agencies (PDF p. 67, sheet 2-3).
- **Armour penetration basis is often unstated.** "If an OPFOR armor
  penetration figure does not say RHA ... do not assume that is the
  standard for the figure" (PDF p. 6). Most tank-round figures say only
  "590-630 at 2,000 meters" with no plate angle.
- Many fields are "INA" (information not available). Several sheets are
  base models only, with upgrades listed as text variants and no figures.
- Some printed figures look wrong (see "Suspect figures in the WEG").

## Units

- Mass is in **metric tonnes**: "mt = metric ton(s) (mt = 1,000 kg)" (units
  table, PDF p. 8). Sheets print "Combat Weight (mt)". No short tons are
  used. Trucks are named by payload in mt (e.g. "Ural-375D 4.5 mt").
- Power in hp, defined as kW x 1.341 (PDF p. 8), so metric and mechanical
  horsepower are not distinguished.
- Speeds in km/h, ranges in km (vehicle) and m (weapons), armour and
  penetration in mm. "RHAe" is RHA equivalent (PDF p. 8).
- The CSV columns use the same units, so no conversion was needed.
  `hp_per_t` is calculated (engine hp / combat weight) and is not printed in
  the WEG.

## Errata that change figures (PDF pp. 9-11)

- 30-mm APDS (BTR-80A, BMD-3, BMP-2, BMP-3, BRM-3K): penetration is
  **25 mm at 60 deg and 1,500 m**. The same data applies to the 2S6M,
  Ka-50, Mi-24, Mi-28 and Su-25 (PDF p. 9).
- Guns of 100-125 mm: maximum aimed range for HEAT rounds is no more than
  2,500 m unless stated (PDF p. 9).
- HE rounds with no penetration figure: guns 100 mm and over "can defeat
  IFVs on impact"; 76-99 mm "can defeat most IFVs on impact" (PDF pp. 9-10).
- T-72B variants added: T-72B1 (1986, no ATGM) and T-72BV (with ERA)
  (PDF p. 10).

## How it is organised

| Chapter | Content | Sheet pages | PDF pages |
|---|---|---|---|
| Front matter | Memo, contents, introduction, units, errata, page changes, glossary | i-xiv, G | 1-22 (glossary 14-19) |
| 1 Infantry weapons | Small arms, MGs, AGLs, recoilless guns, RPGs, disposable AT weapons, flame weapons; tables of night sights and selected infantry weapons | 1-1 to 1-24 | 23-64 (tables 38-46) |
| 2 Infantry vehicles | APCs, IFVs, Kliver turret | 2-1 to 2-30 | 65-106 |
| 3 Reconnaissance | Recon vehicles, Horizon radar | 3-1 to 3-10 | 107-121 |
| 4 Tanks/assault vehicles | Light tanks, MBTs | 4-1 to 4-26 | 122-165 |
| 5 Antitank | AT guns, ATGM vehicles, ATGM launchers, gun-launched ATGM report | 5-1 to 5-26 | 166-196 |
| 6 Artillery | Artillery recon/C2 vehicles, radars, towed and SP cannon, MRLs, mortars, special munitions, TOS-1 | 6-1 to 6-34 | 197-260 |
| 7 Air defence | Towed and SP AA guns, MANPADS, SAM systems, radars | 7-1 to 7-18 | 261-298 |
| 8 Engineer and logistics | Mine primer and tables, minelayers, mine clearing, bridges, trucks, recovery vehicles | 8-1 to 8-16 | 299-341 |
| 9 Rotary-wing aircraft | Light, attack, utility, transport helicopters | 9-1 to 9-18 | 342-361 |
| 10 Fixed-wing aircraft | Strike, ground attack, multi-role, transport | 10-1 to 10-14 | 362-375 |
| 11 Command and communications | BMP-1KSh, radio tables | 11-1 to 11-8 | 376-383 |
| 12 UAVs | Fox AT1/AT2, D-4, Shmel-1, Tu-143, Hermes 450S | 12-1 to 12-8 | 384-393 |
| 13 Heavy rocket/theatre missile | Luna-M/SCUD-B table | 13-1 to 13-6 | 394-400 |
| 14 Equipment upgrades | Upgrade tables (qualitative), countermeasures | 14-1 to 14-14 | 401-414 |

## Where each data sheet starts (PDF page)

Land vehicles and weapons that matter to the catalogue. Two-page sheets
continue on the next PDF page, which usually holds the ammunition data.

**APCs (Ch. 2):** BOV-M 69; BTR-152 70; BTR-60PA 71; BTR-60PB 73; BTR-80
74 (title only; data on 75); BTR-80A 76; BTR-D 77; M113A1 78; MT-LB 79;
Pandur 80; V-150 82; VTT-323 83; WZ 551 84; YW 531A/531C 85; YW 531H /
Type 85 86.

**IFVs (Ch. 2):** AMX-10P 87; BMD-1 89; BMD-3 91; BMP-1 93; BMP-1P 95;
BMP-2 97; BMP-3 99; Marder 1 101; Warrior 103; Kliver turret 105.

**Recon (Ch. 3):** AMX-10RC 111 (title at foot of 110); BRDM-2 113; BRM-1K
114; BRM-3K 116; EE-9 117; Fox 118; Shorland S55 119; Horizon radar 120.

**Tanks (Ch. 4):** AMX-13 124; M41A3 125; M1985 126; Type 63A 128; PT-76B
130; Scorpion 131; AMX-30 132; Challenger 2 134; Chieftain Mk 5 136;
Leopard 1A1 138; Leopard 2 140; M60A1/A3 142; T-34/85 144; T-55AMV 146;
T-62M 148; T-64B 150; T-72B 152; T-72M1 154; T-80B 156; T-80U 158; T-90
160; Type 59-II 162; Type 85-IIM 164.

**Antitank (Ch. 5):** ZIS-3 168; D-44 169; M36 170; MT-12 172; 2A45M 173;
9P148 174; 9P149 176; AMX-10 HOT 178; M901 180; AT-3 182; AT-4/AT-5 183;
AT-7/AT-13 184; Eryx 185; Kornet-E 186; MILAN 2/3 188; Red Arrow-8 189;
gun-launched ATGM report 190-196.

**Artillery (Ch. 6):** 1V13 199; 1V14/15 200; 1V16 201; 1V18/19 202; 1V110
203; 1V111 204; 1V119 205; PRP-3/PRP-4M 207; D-30A 217; M-46 218; 2A36
219; D-20 220; G5 222; GH N-45 223; 2S1 225; 2S3M 226; 2S5 227; 2S19 228;
Type 83 229; G6 231; AU-F1 233; BM-21 235; 9P132 237; M77 239; 9P140 241;
Fadjr-3 243; ASTROS II 245; WM-80 247; 9A52-2 248; MO-120-RT 249; 2S12
251; 2S23 252; TOS-1 259.

**Air defence (Ch. 7):** M-1939 263; KS-19M2 265; S-60 266; ZU-23 269;
Gepard 270; ZSU-23-4 271; 2S6M 272; ZSU-57-2 273; SA-7b 275; SA-14 276;
SA-16 277; SA-18 278; SA-2 279; SA-3 280; SA-5 281; SA-6 282; SA-8b 283;
SA-10b 285; SA-10c 286; SA-11 287; SA-12a/b 288; SA-13b 289; SA-15b 290;
Aspide 291; Crotale 4000 292; Crotale-NG 293.

**Engineer/logistics (Ch. 8), not put in the vehicle CSV:** GMZ-3 316;
MTK-2 326; BAT-M 327; BAT-2 328; IMR-2M 329; MT-55A 330; MTU-72 331;
UAZ-469 336; GAZ-66 337; Ural-375D 338; KrAZ-255B 339; BREM-1 340;
T-54-T 341. **Command:** BMP-1KSh 378.

## Key figures

### Main battle tanks

| Tank | Mass (t) | Engine | Road / off-road (km/h) | Crew | Main gun | Turret front armour (mm) | PDF p. |
|---|---|---|---|---|---|---|---|
| T-55AMV | 40.5 | 620-690 hp diesel | 50 / 35 | 4 | 100-mm D-10T2S | 200 (base T-55) | 146 |
| T-62M | 41.5 | 620 hp diesel | 45 / INA | 4 | 115-mm 2A20 | 230 | 148 |
| T-64B | 40.3 | 720 hp diesel | 60 / INA | 3 | 125-mm 2A46-2 | 450 KE at 2,000 m / 900 CE | 150 |
| T-72B | 44.5 | 840 hp diesel | 60 / 45 | 3 | 125-mm 2A46M | 520 / 950 against HEAT; Kontakt or Kontakt-5 | 152 |
| T-72M1 | 41.5 (without ERA) | 780 hp diesel | 60 / 45 | 3 | 125-mm 2A46M | 500 / 560 against HEAT | 154 |
| T-80B | 44.5 | 1,000 or 1,100 hp gas turbine | 70 / 48 | 3 | 125-mm 2A46-2 | "defeat 120-mm rounds" | 156 |
| T-80U | 46.0 | 1,250 hp gas turbine | 70 / 48 | 3 | 125-mm 2A46M-1 | "against 120-mm ammunition"; Kontakt-5 | 158 |
| T-90 | 46.5 | 1,000 hp diesel (original 840 hp; options 950/1,000/1,100, p. 161) | 60 / INA | 3 | 125-mm 2A46M-1 or -4 | 780-810 KE, 1,020-1,220 CE; Kontakt-5; Shtora-1 | 160 |
| Type 85-IIM | 41.0 | 730 hp diesel | 57 / 45 | 3 | 125-mm | INA | 164 |
| Leopard 2 | 55.15 | 1,500 hp diesel | 72 / 45 | 4 | 120-mm smoothbore | 700 KE / 1,000 against HEAT | 140 |
| Challenger 2 | 62.5 | 1,200 hp diesel | 59 / 45 est | 4 | 120-mm L30 | 500 KE at 2,000 m / 800 CE | 134 |
| M60A1/A3 | 52.5 | 750 hp diesel | 48 / 45 | 4 | 105-mm M68 | 325 KE at 2,000 m / 325 CE | 142 |

Variants with no separate figures: T-72 (original), T-72M, T-72A and
T-72AV/T-72M1V (text only, PDF p. 154); T-72BM with Kontakt-5 and T-72S
(PDF p. 152); T-80BV (PDF p. 156); T-80UD with a 1,000 hp diesel (PDF
p. 158); T-64BV (PDF p. 150); T-72M4CZ "48t weight" (PDF p. 155).

### Tank ammunition

| Gun | Round | Penetration (mm) | At range | Notes | PDF p. |
|---|---|---|---|---|---|
| 125-mm 2A46 series | BM-42M APFSDS-T | 590-630 | 2,000 m | plate angle not stated; "may have a version ... with a DU penetrator" | 153 (same on 151, 155, 156, 159, 161, 164, 173) |
| 125-mm | BK-29M HEAT-MP | 650-750 | any | | 153 |
| 125-mm | BK-27 HEAT | 700-800 | any | triple charge | 153 |
| 125-mm | AT-11/SVIR ATGM | 800 / 700 behind ERA | to 4,000 m (T-72B), 5,000 m (T-80U, T-90) | | 153, 159 |
| 125-mm | AT-11B/INVAR ATGM | 870 / 800 behind ERA | as above | tandem | 153, 159 |
| 125-mm | AT-8/SONGSTER ATGM | 700 (RHA) | to 4,000 m | T-64B, T-80B | 151, 157 |
| 115-mm 2A20 | BD/36-2 APFSDS-T | 520 (RHA, 71 deg as printed) | 1,000 m | | 148 |
| 115-mm 2A20 | BM-6 APFSDS-T | 237 (RHA) | 1,000 m | | 148 |
| 115-mm 2A20 | BK-4 HEAT | 495 (RHA) | any | | 149 |
| 100-mm D-10T2S | BM-412M APFSDS-T (Romanian) | 418 / 380 | 2,000 / 3,000 m | | 147 |
| 100-mm D-10T2S | BM-8 APDS-T | 200 | 1,000 m | | 147 |
| 100-mm D-10T2S | BK-17 HEAT | 380 | any | | 147 |
| 100-mm | AT-10 / AT-10 Improved | 650 (RHA) / 700 behind ERA | to 4,000 m | also BMP-3 (p. 100), MT-12 (p. 172, 5,000 m) | 147 |
| 120-mm (Leopard 2) | DM43 / Olin GD120 APFSDS-T | 450 / 520 | 2,000 m | | 141 |
| 105-mm | New Chinese APFSDS | 460 | 2,000 m | Type 63A, Type 59-II, M60 | 128, 142, 163 |
| 105-mm | M456 HEAT | 432 | any | | 129, 143, 163 |

Western 105-mm and 120-mm rounds are often given only as "NATO single" or
"triple heavy target" at a range, with no mm figure (e.g. PDF pp. 133, 139,
143). Those rows are in the munitions CSV with a blank `penetration_mm`.
BM-42M has no muzzle velocity anywhere in the guide. Muzzle velocities are
given only for infantry weapons, artillery and AA guns.

### IFVs and APCs

| Vehicle | Mass (t) | Engine | Road (km/h) | Swim (km/h) | Crew + troops | Main armament | PDF p. |
|---|---|---|---|---|---|---|---|
| BMP-1 | 13.3 | "Diesel" (no power) | 65 | 7 | 3 | 73-mm 2A28, AT-3 | 93 |
| BMP-1P | 13.3 | 300 hp | 65 | 7 | 3 | 73-mm 2A28, AT-4/AT-5 | 95 |
| BMP-2 | 14.3 | 300 hp | 65 | 7 | 3 | 30-mm 2A42, AT-4/AT-5 | 97 |
| BMP-3 | 18.70 | 500 hp | 70 | 10 | 3 | 100-mm 2A70 + 30-mm 2A72 | 99 |
| BMD-1 | 13.3 (suspect) | 240 hp | 65 | 7 | 2 + 5 | 73-mm 2A28, AT-3 | 89 |
| BMD-3 | 12.9 | 450 hp | 70 | 10 | 3 | 30-mm 2A42 | 91 |
| BTR-60PB | 10.3 | 2 x 90 hp | 80 | 10 | 2 + 8 | 14.5-mm KPVT | 73 |
| BTR-80 | 13.6 | 260 hp | 85 | 10 | 2 + 8 | 14.5-mm KPVT | 75 |
| BTR-80A | 14.6 | 260 hp | 90 | 10 | 2 | 30-mm 2A72 | 76 |
| BTR-D | 6.7 | 240 hp | 61 | 10 | 1 + 12 | 2 x 7.62-mm PKT | 77 |
| MT-LB | 11.9 | 290 hp | 61.5 (70 modernised) | 5-6 | 2 + 11 | 7.62-mm PKT | 79 |
| Warrior | 24.00 | 550 hp | 75 | N/A | 3 | 30-mm RARDEN | 103 |
| Marder 1 / 1A3 | 29.2 / 35.0 | 590-600 hp | 75 | N/A | 3 + 1 | 20-mm Rh202 | 101 |

IFV ammunition: 30-mm APDS 25 mm (RHA) at 1,500 m; 30-mm AP-T 18 mm (RHA,
60 deg) at 1,500 m (PDF pp. 97-98). 73-mm PG-9 HEAT-FS 335 mm (RHA), newer
HEAT-FS ">400" (PDF p. 94). 14.5-mm API-T 30 mm at 500 m, 20 mm at 1,000 m
(PDF p. 73). AT-5 650, AT-5B 925, AT-4 480, AT-4B 550 mm (PDF p. 98).

### ATGMs (launcher sheets)

AT-3 400 (BMP-1 sheet says 410 RHA), AT-3c 520, Malyutka-2 800 (p. 182);
AT-4 480, AT-5 650, AT-5B 925 (p. 183); AT-6 750 / 600 behind ERA, AT-9
950 / 800 behind ERA (p. 176); AT-7 460, AT-13 1,000 / 900 behind ERA
(p. 184); Kornet-LR 1,200 to 5,500 m (p. 186); MILAN 600, MILAN 2 800,
MILAN 3 880 (RHA) (p. 188); HOT 800, HOT 2 900, HOT 2T 1,250 (p. 178); TOW
600, ITOW 800 (pp. 180-181). Missile speeds are "average velocity", not
muzzle velocity.

### Self-propelled artillery

| System | Mass (t) | Engine | Road (km/h) | Crew | Gun | Max range (m) | PDF p. |
|---|---|---|---|---|---|---|---|
| 2S1 | 15.7 | 300 hp | 60 | 4 | 122-mm 2A31 | 15,300 (OF-81); 21,900 RAP | 225 |
| 2S3M | 27.5 | 520 hp | 60 | 4 | 152-mm 2A33 | 17,400; 24,400 (OF-96) | 226 |
| 2S5 | 28.2 | 520 hp | 63 | 5 | 152-mm 2A37 | 28,400; 30,500 (OF-86) | 227 |
| 2S19 | 42 | 840 hp | 60 | 5 | 152-mm 2A64 | 24,700; 29,000 base-bleed | 228 |
| 2S23 | 14.5 | 260 hp | 80 | 4 | 120-mm 2A60 | 8,850; 13,000 RAP | 252 |
| BM-21 | 13.7 | 180 hp | 75 | 5 | 40 x 122-mm | 20,380 (9M22U) | 235 |
| 9P140 | 20.0 | 2 x 177 hp | 65 | 4 | 16 x 220-mm | 35,000 | 241 |
| 9A52-2 | 43.7 | 518 hp | 60 | 4 | 12 x 300-mm | 20,000-90,000 (9M55F/K) | 248 |
| TOS-1 | 46.0 | 780 hp | 60 | 3 | 220-mm rockets | 3,500 / 6,000 | 259 |

Direct-fire HEAT: 122-mm HEAT-FS 460 mm at 0 deg, any range, 740 m/s
(PDF pp. 217, 225). 152-mm BP-540 HEAT 655 m/s, penetration INA (PDF
pp. 226-228).

## Checks against the catalogue CSV

Method: read `data/catalogue/csv/[SIM] L7 bgws_platform_profile.csv` with
pandas (`encoding="utf-8-sig"`). 53 catalogue rows were matched by hand to a
WEG sheet. Fields compared: `mass_combat_t`, `engine_hp`,
`statcard_speed_kmh` (against WEG max road speed) and `crew_size`. The
catalogue has no main-armament text column. `best_calibre_mm` often holds
the ATGM calibre, so calibre was only checked by eye. Differences are
(CSV - WEG) / WEG. Where the WEG gives a range (e.g. 620-690 hp), a CSV
value inside it counts as 0%.

### Disagreements over 5%, good matches

| Catalogue row | WEG sheet | Field | CSV | WEG | Diff | PDF p. |
|---|---|---|---|---|---|---|
| T-55 | T-55AMV | engine_hp | 80.46 | 620-690 | -87% | 146 |
| SA-6 / 2K12 | SA-6 TEL | mass_t | 25.0 | 14 | +79% | 282 |
| SA-6 / 2K12 | SA-6 TEL | road speed | 65 | 45 | +44% | 282 |
| BMP-2 | BMP-2 | engine_hp | 399.6 | 300 | +33% | 97 |
| T-90 | T-90 | engine_hp | 1,249.8 | 1,000 (840 original) | +25% | 160-161 |
| BTR-D | BTR-D | mass_t | 8.2 | 6.7 | +22% | 77 |
| 9P149 Shturm-S | 9P149 | engine_hp | 236.0 | 290 | -19% | 176 |
| MT-LB (M1970) | MT-LB | engine_hp | 240.0 | 290 | -17% | 79 |
| FV510 Warrior | Warrior | mass_t | 28.0 | 24.00 | +17% | 103 |
| BM-21 (Grad) | BM-21 | mass_t | 15.11 | 13.7 | +10% | 235 |
| BM-27 / 9P140 | 9P140 | mass_t | 22.04 | 20.0 | +10% | 241 |
| TOS-1 | TOS-1 | mass_t | 41.1 | 46.0 | -11% | 259 |
| TOS-1 | TOS-1 | engine_hp | 839.5 | 780 | +8% | 259 |
| ZSU-23-4 | ZSU-23-4 | mass_t | 18.6 | 20.5 | -9% | 271 |
| BM-30 / 9A52-2 | 9A52-2 | mass_t | 40.0 | 43.7 | -8% | 248 |
| T-90 | T-90 | road speed | 65 | 60 | +8% | 160 |
| BMP-1 | BMP-1 | mass_t | 12.5 | 13.3 | -6% | 93 |
| BTR-80 | BTR-80 | road speed | 80 | 85 | -6% | 75 |
| SA-11 | SA-11 TELAR | mass_t | 34.0 | 32.34 | +5% | 287 |
| MT-12 (towed) | MT-12 | mass_t | 2.75 | 3.1 | -11% | 172 |
| KS-19 (towed) | KS-19M2 | mass_t | 9.55 | 11.0 (11,000 kg) | -13% | 265 |

Crew disagreements (good matches): BTR-D 3 vs 1 + 12 passengers (p. 77);
BTR-80 3 vs 2 (p. 75); 9P148 3 vs 2 (p. 174); BRM-1K 6 vs 4 (p. 114); 2S3
6 vs 4 (p. 226); BM-21 4 vs 5 (p. 235); 9P140 6 vs 4 (p. 241); 9A52-2 3 vs
4 (p. 248); SA-8 5 vs 3 (p. 283); SA-11 6 vs 4 (p. 287); D-30 6 vs 5 (p.
217). Some catalogue crews may count the full gun section; the WEG often
says "section of 6" or "section of 7" beside the vehicle crew.

Blank catalogue engine power where the WEG has one: 9P140 (2 x 177 hp,
p. 241), SA-11 TELAR (700 hp, p. 287), SA-12 (525 hp, p. 288).

### Disagreements over 5%, uncertain matches

| Catalogue row | WEG sheet used | Field | CSV | WEG | Diff | PDF p. | Why uncertain |
|---|---|---|---|---|---|---|---|
| T-72 (Ural) | T-72M1 | mass_t | 46.0 | 41.5 | +11% | 154 | no T-72 Ural sheet; T-72M1 is the export T-72A |
| T-72 (Ural) | T-72M1 | engine_hp | 839.5 | 780 | +8% | 154 | as above |
| T-72 (Ural) | T-72M1 | road speed | 67 | 60 | +12% | 154 | as above |
| T-72B3 obr.2016 | T-72B | road speed | 67 | 60 | +12% | 152 | B3 not in WEG |
| T-80 (MBT) | T-80B | engine_hp | 1,249.8 | 1,000-1,100 | +14% | 156 | CSV row looks like a T-80U (46 t, 1,250 hp, p. 158) |
| T-80BVM | T-80B/BV | engine_hp | 1,249.8 | 1,000-1,100 | +14% | 156 | BVM not in WEG |
| T-90M Proryv | T-90 | engine_hp | 1,249.8 | 1,000 | +25% | 160 | T-90M not in WEG |
| T-64 | T-64B | road speed | 70 | 60 | +17% | 150 | CSV generic T-64 |
| T-62 | T-62M | engine_hp | 580.7 | 620 | -6% | 148 | CSV generic T-62 |
| BMP-2M Berezhok | BMP-2 | engine_hp | 399.6 | 300 | +33% | 97 | 2M not in WEG |
| BTR-60 | BTR-60PB | mass_t | 11.5 | 10.3 | +12% | 73 | could be PA (10.1 t) or PB |
| BTR-60 | BTR-60PB | engine_hp | 89.8 | 180 (2 x 90) | -50% | 73 | CSV looks like one engine of two |
| BTR-82A (var_395_1) | BTR-80A | mass, speed, crew | 13.6 / 80 / 3 | 14.6 / 90 / 2 | -7% / -11% / +50% | 76 | not the same vehicle |
| SA-12 / S-300V | SA-12a/b | mass_t | 37.0 | 48 | -23% | 288 | sheet does not say which vehicle weighs 48 t |
| SA-12 / S-300V | SA-12a/b | road speed | 65 | 50 | +30% | 288 | as above |
| SA-10 / S-300 | SA-10b | crew | 8 | 6 | +33% | 285 | no mass on sheet |
| Marder 1A5 | Marder 1A3 | mass, speed, crew | 37.4 / 65 / 3 | 35.0 / 75 / 3 + 1 | +7% / -13% | 101 | 1A5 not in WEG |
| Leopard 2A6 | Leopard 2 | mass_t | 62.5 | 55.15 | +13% | 140 | 2A6 not in WEG |

### The suspect rows named in the brief

- **T-72 and T-72B3 both 46 t / 839 hp.** The WEG has no T-72 (Ural)
  sheet. Its T-72M1 (export T-72A) is 41.5 t without ERA and 780 hp
  (PDF p. 154). Its T-72B is 44.5 t and 840 hp (PDF p. 152). So 46 t is too
  heavy for the original T-72, and 839 hp is the T-72B engine. The WEG
  cannot confirm any T-72B3 figure.
- **T-80 and T-80BVM both 46 t / 1,250 hp.** These are the WEG's T-80U
  figures (46.0 t, 1,250 hp; PDF p. 158). The T-80B/BV sheet gives 44.5 t
  and 1,000 or 1,100 hp (PDF p. 156). The WEG notes that "T-80B and -BV
  variants are often misidentified as T-80" (PDF p. 157). The T-80BVM is
  not covered.
- **BMP-2 at 399.6 hp.** The WEG gives 300 hp (PDF p. 97), the same as
  BMP-1P (PDF p. 95). The catalogue's own "BMP-2M / BMP-2 (Russian 2020s)"
  row already uses 300.4 hp.
- **T-72 (Ural) at 46 t.** See above. Nearest WEG mass is 41.5 t.

### Matches within 5%

2S1, 2S5, 2S19, 2S3 (mass, power, speed), 2A36, D-30 (mass), BMP-3 (all
four fields), BRDM-2, 9P148, BRM-1K (mass, power, speed), 9P149 (mass,
speed, crew), BTR-80 (mass, power), MT-LB (mass, speed, crew), Challenger 2,
M113, Gepard, SA-13, SA-15, SA-19/2S6M (mass, speed, crew), SA-8 (mass,
speed), Ural-375 (gross 13.2 t, 180 hp, 75 km/h; PDF p. 338), T-55 and
T-62 (mass, speed, crew), T-64 (mass, power), T-90 (mass).

### Catalogue rows with no WEG sheet

BTR-70 (variant line only, PDF p. 73), BTR-90, BTR-82A, BMD-2 (variant line
only, PDF p. 89), BMD-4M, 2S9, 2S4, 2S7, 2S34, 2S35, 2A65, 2S25, 9P157-2,
BMPT, T-14, Pantsir-S1, S-400, Iskander, Tochka, Tornado-G/-S, all Western
wheeled vehicles and MRAPs, M1, M2/M3, M109 and later NATO systems.

## Suspect figures in the WEG

- **BMD-1 combat weight 13.3 mt** (PDF p. 89). This equals the BMP-1 figure
  and is far above the BMD-3's 12.9 mt for a smaller vehicle. Probably a
  copy error. Do not use.
- **MT-LB 290 hp** (PDF p. 79) and **9P149 (MT-LB based) 290 hp** (PDF
  p. 176), but the MT-LBu-based 1V13 family is given as "YaMZ-238, 240 hp"
  (PDF pp. 199-201) and the MT-LBu-based 2S1 as 300 hp (PDF p. 225). The
  WEG is not consistent. The catalogue's 240 hp may be right.
- **BTR-60PA "2 x 180-hp Gasoline"** (PDF p. 71), against "2x 90-hp" for the
  BTR-60PB (PDF p. 73) and "2 GAZ-49B 90 hp (180 hp total)" for the
  BTR-60-based 1V18/1V19 (PDF p. 202). Probably 180 hp total. The CSV row
  leaves `engine_hp` blank.
- **Type 63A light tank "1,000-hp Diesel"** with combat weight INA (PDF
  p. 128). Doubtful for an amphibious light tank: the similar M1985 is
  given 320 hp (PDF p. 126). Left as printed.
- **Leopard 2 gun printed as "120-mm smoothbore gun M256"** (PDF p. 140).
  M256 is the US licence-built gun. Kept as a note in the CSV.
- **T-55AMV ERA line** reads "1st Gen raises to KE/700-900" (PDF p. 146).
  Part of the line seems lost. Not used as a figure.
- **2S5 "Max Swim: 4.5"** (PDF p. 227). Probably copied from the 2S1 sheet.
- **YW 531A/531C** gives pairs such as "Max Road: 42/66" with no key (PDF
  p. 85). Copied as printed.
- **AT-3 penetration** is 400 mm on the launcher sheet (PDF p. 182) and 410
  mm (RHA) on the BMP-1 and BMD-1 sheets (PDF pp. 90, 94). Both are in the
  munitions CSV.
- **M1000 100-mm APFSDS**: "NATO triple heavy target, 4,500 m" on the
  T-55AMV sheet (PDF p. 147), "at 4,000 meters" on the MT-12 sheet (PDF
  p. 172).

## Pages not transcribed

- Infantry weapon tables (PDF pp. 38-46): wide landscape tables. The text
  layer runs columns together. Not used.
- Mine tables (PDF pp. 307-310) and line-charge tables (PDF pp. 321-324):
  outside scope and with merged columns.
- OPFOR special munitions chart (PDF p. 253) and upgrade tables (PDF
  pp. 403-408): qualitative, no figures for the CSVs.
- Engineer and logistics vehicles (Ch. 8): figures exist but are outside
  the requested vehicle types. Pages are listed above.
- Small arms, aircraft, radars and UAVs: outside scope.

## Spot-checks

Twelve vehicle rows were checked against the PDF text by script (mass,
engine, road speed and crew strings on the cited page): BTR-D, MT-LB,
BMP-2, BMP-3, T-64B, T-72B, T-80U, T-90, 2S19, ZSU-23-4, SA-6, BTR-80. All
matched once the BTR-80 page was corrected from 74 (title only) to 75. The
T-80U and SA-6 engine text was reworded in the CSV and matched by eye.
Twelve munition rows were checked the same way (BM-42M, BD/36-2, BM-412M
x2, BK-4, DM43, PG-7VR, 30-mm APDS, AT-5B, AT-9 x2, Kornet-LR). All matched.
