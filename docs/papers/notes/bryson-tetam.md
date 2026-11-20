# Bryson: Tactical Effectiveness Testing of Antitank Missiles (TETAM)

**Source:** Dr. Marion R. Bryson (Scientific Advisor, US Army Combat
Developments Experimentation Command, Fort Ord), "Tactical Effectiveness
Testing of Antitank Missiles". Symposium paper, pp. 13–20, DTIC ADP000598.

**File:** `docs/papers/pdfs/TACTICAL EFFECTIVENESS TESTING OF ANTITANK MISSILES.pdf`

Read in the original on 20 November 2026. Page numbers are the paper's own.

## The trials

- **Field experiment, 1972–73:**
  - 12 sites in West Germany (around Fulda, Hohenfels and Bergen-Hohne-Soltau);
  - Hunter Liggett, California;
  - Fort Lewis, Washington.
- **Weapons:** TOW, Shillelagh, Dragon, Swingfire and Milan.
- **Purpose:** to supply data for the US Army's high-resolution combat
  models (DYNTACS, CARMONETTE, IUA).

## Numbers usable as checks

| Phase | Finding | Page |
|---|---|---|
| IE | Intervisibility: 10 tank trails of 3–5 km per site, sampled every 25 m against 30–36 ATM positions, at 3 observer heights (4, 6, 10 ft) and 2 target heights (4, 7 ft) | 14–15 |
| IA | A tank fired on manoeuvred to break line of sight: **median 70 m** of travel | 16 |
| IB (part 1) | Detection of advancing vehicles by players scanning with the **unaided eye or binoculars**; times to detect, given an opportunity, are in a companion paper (not in this file) | 16 |
| IB (part 2) | After detection: **median 2 s to identify, 8–11 s to engage**, depending on weapon | 17 |
| IC | Hand-off of a target from one man to another by telephone (platoon leader → squad leader, rifleman → squad leader, squad leader → ATM gunner): **51–68% succeeded** (depending on range); when successful, **mean 18–31 s** | 17 |
| IIA | Attacking armour crews detecting ATM launches (simulated signature): **TOW 32–47%**, **Shillelagh 22–35%**, **Dragon 18–29%** of launches detected | 19 |
| IIB | The missile's flight was reported as the cue in **Dragon 0%, TOW 5%, Milan 7%, Shillelagh 9%, Swingfire 13%** of firings; hit data classified | 19 |

## How BGWS uses it

- **8–11 s from detection to engagement** is a check on our lay time
  (`sim.fire.lay_min_s`, 6 s minimum) plus reaction. See
  `docs/research/CALIBRATION_TARGETS.md`.
- **Hand-off, 51–68% success in 18–31 s,** is a check for cued detection
  (`sensing.acquire.cue_factor`). It isn't a direct source: a hand-off
  is a description passed by telephone, not a precise cue.
- **Launch detection, 18–47%,** is a check on how often a firing ATM
  team gives itself away (its signature after firing).
- **The 70 m median to break line of sight** is a check on how far a
  vehicle under fire has to move to reach cover.
