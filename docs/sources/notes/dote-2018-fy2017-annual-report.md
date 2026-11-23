# DOT&E (2018): FY2017 Annual Report

**Source:** Director, Operational Test and Evaluation (DOT&E), *FY 2017
Annual Report*, January 2018 (Director Robert F. Behler). Public.

**File:** `raw/test-reports/dote-2018-fy2017-annual-report.pdf` (394 PDF
pages; text layer). Read on 23 November 2026. Page references use the
report's printed page, with the PDF page in brackets.

## What it is

The yearly summary of every DoD programme under operational or live-fire
test oversight. Each programme gets two to four pages: what was tested,
what happened, and DOT&E's assessment. It reports test outcomes, not
combat ones, and many effectiveness figures are classified and left out.

## What we use

### Javelin Spiral 2 (pp. 105–106 [123–124])

| Figure | Value | Where |
|---|---|---|
| Flight tests (FY16–FY17) | 22 | p. 106 |
| Hits on vehicle targets | 18 of 18 | p. 105, p. 106 |
| Hits on structure targets | 2 | p. 105 |
| Against an IED team in the open | 1 near miss, 1 complete miss | p. 105 |
| All hits | 20 of 22 = **0.91** | derived |
| Failures | the new precursor warhead failed to detonate in 2 static and 2 flight tests (FY16; age-degraded explosive) | p. 106 |
| After the fix | "performed reliably in 14 of 14 flight tests" | p. 105, p. 106 |
| Static warhead tests | 16 | p. 106 |
| Range | defeats armour "out to 2,500 meters" | p. 105 |
| System weight | 49.5 lb | p. 105 |

**Applied:** `fire.GUIDED.hit` (0.9) now cites this. The value was
already 0.9, so it is unchanged. This also settles the lead in
`leads/atgm-and-company-attack.md`: 20/22, and 18/18 against vehicles.
The leads' FY1998 figure (51 of 54) is not in this report.

**Reliability (`missile_fail_p`):** 2 failures in the first 7 FY16
flights, all one fixed fault, then 0 in 14. A per-shot failure rate
from this is 2/22 = 0.09 for a missile with a new component, and 0/14
after the fix. That fits the 0.01–0.08 band in the leads. No value is
changed.

### Patriot PAC-3 MSE flight tests (p. 120 [138])

- **MFT-A1** (June 2017, WSMR): a TBM engaged with a PAC-3 MSE and a
  GEM-T (two interceptors), then a cruise missile with one PAC-3 MSE.
- **MFT-B** (September 2017, Kwajalein): an MRBM engaged by ripple fire
  with **three** PAC-3 MSE.
- **MFT-A2** (November 2017, WSMR): two TBMs at once, with two mixed
  ripples of two interceptors each.
- "Patriot successfully engaged all five targets during the PDB-8
  IOT&E" (p. 120).

**Applied:** this supports `world/domains/air.SALVO_VS_BALLISTIC = 2`
(from the FY2024 report). Its sweep range is now [1, 3] to take in the
three-interceptor MRBM shot.

### Active protection systems (pp. 92–93 [110–111])

Phase 1 of the Army's APS programme (Trophy on Abrams, Iron Fist on
Bradley, Iron Curtain on Stryker). Trophy had 46 live-fire events and
"demonstrated the capability … to counter two of the three class threats
tested" (p. 92). DOT&E adds that "it is reasonable to assume that any
performance reporting from Phase 1 is optimistic" (p. 93). There is no
intercept probability, so `lethality.P_APS_INTERCEPT` (0.5) stays
declared.

## Limits

- These are developmental and operational **test** shots at instrumented
  ranges, against targets chosen for the test. Combat hit rates are lower
  (see the Ukraine and 2006 figures in the leads).
- Small samples: 22 Javelin flights, 5 Patriot targets.
- Classified shortfalls are mentioned but not given.
