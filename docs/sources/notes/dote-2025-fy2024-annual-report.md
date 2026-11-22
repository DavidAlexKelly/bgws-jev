# DOT&E FY2024 Annual Report: land combat, air defence and munitions

**Source:** Director, Operational Test and Evaluation (DOT&E), *FY 2024
Annual Report*. Office of the Secretary of Defense, Washington DC, January
2025. 498 pages. Director (Acting) Dr Raymond D. O'Toole, Jr.

**File:** `docs/sources/raw/test-reports/dote-2025-fy2024-annual-report.pdf`

**Data:** `docs/sources/data/dote-2025-fy2024-annual-report--test-results.csv`
(38 rows, one per quantitative result).

Read on 22 November 2026, through `pdftotext`. Every page number below is
the **PDF page**. The printed page number is the PDF page minus 14 (printed
p. 85 = PDF p. 99). One exception: the contents list puts NGSW at printed
p. 149, but it starts on printed p. 148 (PDF p. 162).

## What the report is, and its limits

- It is DOT&E's yearly summary to Congress of operational and live fire
  testing of the systems on its oversight list (265 systems, PDF p. 19).
- Each programme gets 2–4 pages: description, test events, adequacy, and a
  one-line judgement on effectiveness, suitability, survivability and
  lethality.
- **It is unclassified and gives very few raw numbers.** Hit counts,
  probabilities of kill, MTBF and detection ranges sit in classified
  annexes or separate reports. Phrases such as "Additional details are
  found in the FOT&E report and classified annex" recur on almost every
  page.
- Many FY24 entries report no operational test at all (prototypes,
  rapid-acquisition programmes, deployments that pre-empted testing).
- Most figures it does give are counts of events, shots or failures, and
  operational availability. **There is no hit-out-of-shots figure for any
  land weapon.**
- Treat it as a source of qualitative checks and a pointer to the reports
  worth finding (e.g. the March 2024 Javelin FOT&E report, the November
  2024 Stryker 30mm report).

## Chapters relevant to the simulation

| Area | Chapter | PDF pages |
|---|---|---|
| DoD-wide | Executive summary: performance trends | 22–24 |
| DoD-wide | T&E resources (directed energy and C-UAS test gaps) | 35–44 (gaps p. 43) |
| Tanks / IFVs | 120mm AMP cartridge M1147 | 99–100 |
| Tanks / IFVs | Armored Multi-Purpose Vehicle (AMPV) | 104–106 |
| Tanks / IFVs | HERCULES M88A3 recovery vehicle | 134–136 |
| Tanks / IFVs | M10 Booker | 157–158 |
| Tanks / IFVs | Stryker Family of Vehicles (30mm) | 180–181 |
| ATGM | Javelin Antitank Missile System – Medium | 150–153 |
| Infantry | Next Generation Squad Weapons (NGSW) | 162–165 |
| Infantry | Integrated Visual Augmentation System (IVAS) | 147–149 |
| Infantry | Soldier Protection System (SPS) | 176–179 |
| Artillery / precision fires | ER GMLRS / GMLRS AW | 122–125 |
| Artillery / precision fires | Precision Strike Missile (PrSM) | 170–172 |
| Artillery / precision fires | Long Range Hypersonic Weapon (Dark Eagle) | 154–156 |
| Artillery / precision fires | Small Diameter Bomb II (air-delivered) | 356–359 |
| Air defence | Army Integrated Air and Missile Defense (AIAMD / IBCS) | 107–109 |
| Air defence | Integrated Fires Test Campaign (IFTC; LTAMDS, IFPC Inc 2) | 137–139 |
| Air defence | Sentinel A4 radar | 173–175 |
| Air defence | Missile Defense System (Patriot, LTAMDS, THAAD, Aegis BMD) | 385–395 |
| Air defence | 3DELRR radar (Air Force) | 363–365 |
| Counter-UAS | DE M-SHORAD (laser) | 116–118 |
| Counter-UAS | Center for Countermeasures (C-UAS, HEL, HPM testing) | 399–404 |
| Counter-UAS | JTCG/ME (HPM lethality against sUAS swarms) | 429–443 (p. 440) |
| Drones | FUAS Air Launched Effects | 131–133 |
| C2 / networks | CPCE/TSI (command post computing) | 110–112 |
| C2 / networks | Integrated Tactical Network (ITN) | 144–146 |
| C2 / networks | Terrestrial Layer System BCT (SIGINT/EW) | 185–188 |
| C2 / networks | DAPS (dismounted PNT) | 119–121 |
| C2 / networks | MAPS (mounted PNT) | 159–161 |
| C2 / networks | Joint T&E: ATTACKS counterfire kill-web | 444–452 (p. 449) |
| Logistics / other | FMTV A2, Common Tactical Truck, NBCRV SSU, STE-LTS | 126–130, 113–115, 166–169, 182–184 |

**Listed by name only** (naval, space, aviation or IT, nothing usable for
land or ground-based air defence): AVCAD, JBTDS, DMS IT, DHMSM, GCCS-J,
JCWA, JOMIS, JPES, KMI, NBIS, PKI, IPPS-A, AN/APR-39E(V)2 MRWR, F-35, all
Navy ship, submarine, torpedo and naval aviation programmes (AARGM-ER to
VH-92A), LRUSV, all Air Force aircraft and IT programmes other than SDB II
and 3DELRR, GPS Enterprise, Space C2, Dry Combat Submersible, CAP, ITEP,
JASP, TETRA.

## Programme by programme

### Javelin (PDF pp. 150–153)

- **Tests:** Limited User Test at Fort Greely, Alaska (March 2023) and
  FOT&E at Yuma (August 2023). Both compared the Lightweight CLU with the
  Block 1 CLU, using current inventory missiles. The G-model missile was not
  mature enough to take part (p. 151).
- **Numbers:** a new software fault "resulted in three system aborts,
  causing the LW CLU to fail in demonstrating its operational reliability
  and availability requirements" (p. 153). No firing count, hit count or
  MTBF is given.
- **Qualitative:** soldiers with the LW CLU "performed as well or better
  than soldiers equipped with the Block 1 CLU at engaging targets day or
  night, and across the arctic, temperate, and hot desert climates"
  (p. 152). Battery performance of both CLUs "was significantly degraded in
  the cold temperatures" (p. 153). Lethality last evaluated in 2019 for the
  F-model: "met or exceeded its lethality requirement" (pp. 152–153).
- **Range:** design range "out to 2,500 meters" (p. 151). DOT&E still wants
  the maximum effective range demonstrated (p. 152).
- **Assessment:** effective; **not suitable** (software fault, since fixed
  in v4.1 but not yet verified); cyber survivability pending.

### ER GMLRS / GMLRS AW (PDF pp. 122–125)

- **Tests:** three system qualification test shots (October 2023). One
  operational test mission at White Sands (February 2024), firing two
  Unitary rockets in point detonate mode at a threat representative target
  (pp. 122–124).
- **Numbers:** reliability failures of the new side-mounted proximity
  sensor (SMPS), "predominantly on the ER GMLRS Alternative Warhead (AW)
  variant" (p. 122). Failures per shot not given. Range increased "from
  70 kilometers out to 150 kilometers" (design, p. 123). The test programme
  has "14 test rockets (with spares)" (p. 123).
- **Qualitative:** "accurate and capable of exceeding the legacy objective
  range requirement of 70 kilometers" (p. 124). "Increased mission
  processing times ... could negatively impact the timely delivery of
  fires" (p. 124). Point-detonate rockets "had good effects against the
  threat representative target" (p. 124). Lethality "is dependent on HOB,
  angle of fall, and target location error" (p. 124). Reliability KPP "not
  demonstrated ... with statistical confidence" (p. 124).
- **Assessment:** insufficient data for effectiveness, lethality,
  suitability; survivability report pending.

### PrSM (PDF pp. 170–172)

- **Tests:** two early operational capability missiles shot at a maritime
  target (June 2024); three production qualification test events (November
  2023 – August 2024). Outcomes not stated (pp. 170–171).
- **Numbers:** Increment 1 "threshold lethal range of 400 kilometers"
  (specification, p. 171). Nine PQT shots remain.
- **Assessment:** insufficient data on all counts (p. 172). DOT&E wants
  shots under GPS jamming.

### SDB II, air-delivered (PDF pp. 356–359)

- **Tests:** two F/A-18E/F operational tests in FY24; none on F-35B/C.
- **Numbers:** "zero successful F-35B/C operational tests and two
  successful F/A-18E/F operational tests in FY24" (p. 356). In the two
  tests, SDB II "performed as expected in normal mode against one static
  and one moving land-based target" (p. 358). Seven attempts at the last two
  events all failed to happen or failed: a hung weapon, weather (three),
  wrong cryptographic keys (two), and one weapon malfunction plus Link 16
  incompatibility (p. 357).
- **Lethality (earlier Phase I):** verified "against a variety of static
  and moving targets including main battle tanks, infantry fighting
  vehicles, anti-aircraft guns, surface-to-air missile target
  erector-launchers, and small patrol boats" (p. 358).
- **Assessment:** effective on F-15E (Phase I); not yet shown on other
  aircraft; suitability not assessable; cyber survivability not addressed.

### LRHW Dark Eagle (PDF pp. 154–156)

- No Army flight test in FY24; the planned JFC-IGNITE test "did not occur"
  (p. 156). Insufficient data on all counts.

### Air defence: AIAMD, IFTC, LTAMDS, Patriot, Sentinel A4 (PDF pp. 107–109, 137–139, 173–175, 385–395)

- **IFTC 23 (November 2023):** one LTAMDS radar, one IBCS Engagement
  Operations Center and four simulated Patriot launchers. "Three days of
  software- and hardware-in-the-loop (S/HWIL) and four days of uncontested
  live air testing" (p. 138). DOT&E judged it "inadequate to support an
  assessment of operational effectiveness for the LTAMDS system, due to
  immature and unaccredited LTAMDS M&S tools" (pp. 108, 138).
- **LTAMDS developmental flight tests** (Table 2, p. 389), each a single
  target and each a kill:
  - subscale aircraft target, "with one PAC-3 missile" (November 2023);
  - close-range ballistic missile target, "with two PAC-3 missiles"
    (December 2023);
  - long-range cruise missile target, "with one PAC-3 missile" (March 2024).
- **Patriot PDB-8.1 LUT:** "training and human system integration improved
  over PDB-8, but shortfalls remain in reliability and survivability"
  (p. 394). Numbers are in a classified report.
- **Patriot (general):** "demonstrated the capability to provide point
  defense against missile and aircraft attacks" (p. 393).
- **Aegis FTM-48 (October 2023, naval but relevant to raid engagement):**
  two SM-3 Block IA against two SRBMs while "concurrently engaging a raid of
  two subsonic anti-ship cruise missile drones with four SM-2 Block IIIA
  guided missiles" (p. 388). Intercept results not stated.
- **Sentinel A4:** IOT&E not funded as planned; no assessment (pp. 173–175).
- **Assessment:** AIAMD unchanged from the classified March 2023 IOT&E
  report; LTAMDS, IFPC Inc 2 and Sentinel A4 not yet assessable.

### Counter-UAS: DE M-SHORAD, CCM, JTCG/ME (PDF pp. 116–118, 399–404, 440)

- **DE M-SHORAD:** four Stryker prototypes with a "50-kilowatt spectral
  beam combined laser" against Group 1–3 UAS, aircraft and RAM (p. 117).
  They deployed overseas in February 2024, which stopped the planned tests.
  ATEC ran a "three-day controlled assessment" in June 2024 that DOT&E did
  not see (pp. 116–117). DOT&E "has insufficient data to assess" anything
  (p. 118).
- **CCM:** 32 test events in FY24, including High Energy Laser defeat of
  small UAS, an IFPC High Power Microwave event, a "Probability of Weapon
  Effectiveness" HEL series compared with M&S, and an OUSD(R&E) C-UAS
  experiment (pp. 399–403). **No results are given.**
- **JTCG/ME:** live fire tests generated "lethality data to verify and
  validate dynamic engineering-level modeling for effectiveness of HPM
  weapons against small unmanned aircraft system swarm targets" (p. 440).
  No figures.
- **Test gaps:** future HEL tests "will need accredited threat surrogates
  for anti-ship cruise missiles and swarming unmanned airborne and surface
  vehicles" (p. 43).

### Vehicles: AMPV, M10 Booker, Stryker 30mm, 120mm AMP, M88A3 (PDF pp. 99–106, 134–136, 157–158, 180–181)

- **AMPV:** effective, suitable and survivable "against specified kinetic
  threats" (January 2023 report). Fire extinguishing system (AFES) still
  being fixed after live fire (p. 105).
- **M10 Booker:** live fire began June 2024; IOT&E (gunnery, then
  force-on-force) in FY25. No assessment yet (p. 158).
- **Stryker 30mm:** FOT&E and LFT&E done in FY24; the assessment is only in
  the November 2024 report with classified annex (p. 181).
- **120mm AMP M1147:** "operationally effective, lethal, suitable, and
  survivable" (December 2022 report, restated p. 100). Intended to defeat
  "dismounted Anti-Tank Guided Missile teams at extended ranges" (p. 99).
- **M88A3:** no assessment yet (p. 135).

### Infantry: NGSW, IVAS, SPS (PDF pp. 147–149, 162–165, 176–179)

- **NGSW (October 2023 Ops Demo):** 6.8mm GP ammunition "provides increased
  lethality over the M855A1"; soldiers rated the XM157 fire control "below
  average/failing"; "The XM7 with mounted XM157 demonstrated a low
  probability of completing one 72-hour wartime mission without incurring a
  critical failure" (p. 164). No probability is given.
- **IVAS:** no operational test in FY24 (p. 149).
- **SPS:** two of three VTP ESAPI plate designs and one of one MSV Gen II
  design met first article requirements; "over 280 LATs" with one failed
  VTP lot (pp. 178–179). The chapter summary says "over 250" (p. 176).

### C2, networks, EW and PNT (PDF pp. 110–112, 119–121, 144–146, 159–161, 185–188, 449)

- **DAPS GEN II (November 2023 IOT&E):** "no essential function failures";
  "Operational availability was 99 percent" (p. 121). Effective, but with
  "decreased position and notification accuracy under very challenging
  threat environments" (p. 121). Effective, suitable, survivable.
- **MAPS GEN II (February 2024 IOT&E):** "only experienced one essential
  function failure during IOT&E, meeting its reliability requirement"; the
  linked JBC-P display "experienced three essential function failures";
  "Operational availability was 99 percent" (p. 161). Effective, suitable,
  survivable.
- **CPCE:** Increment 1 was "operationally effective and not suitable due
  to reliability issues" (December 2021); Increment 2 cyber survivable when
  defended by trained soldiers (p. 112).
- **ITN:** no operational test; no assessment (p. 146).
- **TLS BCT Manpack:** "able to detect enemy emitter signals with some
  error"; SIGINT "did not provide soldiers with information on the source of
  the emission"; electronic attack "disrupted enemy voice communications";
  networks "may not provide sufficient data rates" (p. 187). Potentially
  effective and suitable.
- **ATTACKS (Joint T&E quick reaction test, Korea counterfire):** automation
  could "reduce the total time required to neutralize the long-range
  artillery threat from minutes to seconds" (p. 449). This is a claim of
  intent, not a measured result.

### Drones: FUAS Air Launched Effects (PDF pp. 131–133)

- Ops Demo cancelled; a vendor flight test showed launch from an MH-60,
  dynamic retasking and auto-land (p. 133). No assessment.

### DoD-wide trends (PDF pp. 22–24)

- FY24: 54 percent (7 of 13) assessed effective, 58 percent (7 of 12)
  suitable, 27 percent (3 of 11) survivable.
- FY16–23: 53 percent (82 of 154) effective, 48 percent (70 of 147)
  suitable, 31 percent (37 of 121) survivable.
- The not-suitable programmes "experienced shortfalls in hardware and
  software reliability and availability" (p. 24).

## Figures the simulation could use

| Model need | What this report gives | Use |
|---|---|---|
| Guided-missile hit probability (`direct.guided.ph`) | **Nothing numeric.** Javelin: LW CLU gunners did "as well or better" than Block 1 CLU gunners (p. 152). | No change. The FY2017 and FY1998 DOT&E Javelin figures in the leads file stay the best test numbers. |
| Missile / weapon reliability (`sim.fire.missile_fail_p`) | Javelin LW CLU: **3 system aborts** from one software fault (p. 153), launcher not missile, no denominator. ER GMLRS: SMPS fuze "reliability failures", count not given (p. 122). | Qualitative only. Supports keeping a non-zero failure rate for new or upgraded systems; gives no rate. |
| Small-arms reliability | XM7 + XM157: "low probability of completing one 72-hour wartime mission without incurring a critical failure" (p. 164). | Qualitative. If NGSW is modelled, treat a 72-hour mission as the unit for critical failures. |
| Interceptors per target (salvo size) | PAC-3: **1** per aircraft and per cruise missile target, **2** per close-range ballistic missile (DT, p. 389). SM-2: **4** for **2** cruise-missile drones (p. 388). | Usable as a doctrine check on shots per target in air defence. These are single developmental shots, not kill probabilities. |
| Counter-drone defeat rate | **Nothing numeric.** DE M-SHORAD untested; CCM and JTCG/ME HEL and HPM results not published (pp. 118, 401–403, 440). | No figure. Do not cite this report for a laser or HPM defeat rate. |
| System availability (Ao) | DAPS GEN II **99 percent**, MAPS GEN II **99 percent** (pp. 121, 161). | Usable for navigation aids only: assume PNT kit is almost always up. Not for vehicles or weapons. |
| Essential function failures | DAPS **0**, MAPS **1**, JBC-P **3** in IOT&E (pp. 121, 161). Test hours not given, so no MTBF. | Qualitative: the display is the weak link, not the receiver. |
| Precision fires range | ER GMLRS beyond **70 km**, design **150 km**; PrSM **400 km** threshold (pp. 123–124, 171). | Specifications for indirect fire reach. |
| Precision fires accuracy / timing | ER GMLRS "accurate"; mission processing times increased (p. 124). | Qualitative. No CEP and no time. |
| Moving-target precision attack | SDB II hit as expected against one static and one moving land target (p. 358). | Two events; not a rate. |
| Time to detect and engage | Nothing numeric. ATTACKS "minutes to seconds" for counterfire is an aim (p. 449). | No figure. |
| Sensor detection range | Nothing numeric. 3DELRR flew **0 of 6** instrumented flights (p. 365). | No figure. |
| Effect of GPS jamming | DAPS and MAPS "perform better than the legacy DAGR in GPS-contested environments"; DAPS accuracy drops "under very challenging threat environments" (pp. 121, 160). | Qualitative: newer PNT degrades under heavy jamming but less than DAGR. |
| EW / SIGINT | TLS BCT MP detects emitters "with some error" but not their source; jamming disrupted voice nets (p. 187). | Qualitative support for direction-finding error in any EW model. |
| Prior that new kit works | 54 percent effective, 58 percent suitable (FY24); 53 and 48 percent over FY16–23 (pp. 23–24). | Context only: about half of new systems are judged unsuitable, mostly for reliability. |

**In short:** the FY2024 report changes no figure in the model. Its value is
(a) a small set of interceptor-per-target counts, (b) 99 percent
availability for PNT kit, and (c) pointers to classified or separate
reports that would hold the hit and reliability numbers: the March 2024
Javelin FOT&E report, the November 2024 Stryker 30mm FOT&E/LFT&E report,
the May 2024 NGSW Ops Demo report, the classified Patriot PDB-8.1 LUT
report, and the FY25 DE M-SHORAD early fielding report.
