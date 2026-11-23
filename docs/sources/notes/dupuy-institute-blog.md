# The Dupuy Institute blog, *Mystics & Statistics* (2016–2026)

**Source:** The Dupuy Institute (TDI), *Mystics & Statistics* blog,
dupuyinstitute.org, posts dated 15 February 2016 to 25 August 2026. Most
are by Christopher A. Lawrence (author of *War by Numbers*, 2017), some
by Shawn Woodford. Each file is a post's text with its title, date and
URL.

**File:** `raw/simulation/dupuy-institute-blog/` (211 posts as `.txt`,
plus `_manifest.json` mapping URL to file). Read on 23 November 2026: the
posts on breakpoints, force ratios, attrition and validation, by title
and by searching all of them for percentages and rates. The posts cited
below were read in full.

**Tables:**

- `data/dupuy-institute-blog--force-ratio-attacker-wins.csv`
- `data/dupuy-institute-blog--reasons-for-abandoning-attack-or-defence.csv`

## What it is

A secondary source. The posts summarise TDI's databases (752
division-level engagements 1904–1991, battalion and company databases,
Kursk, Ardennes) and older studies, often quoting them. Cite the
underlying study where one is named, and treat the posts as the pointer
to it.

## Breakpoints

From 2018-04-16 *C-WAM 4 (Breakpoints)*, 2018-04-18 *Breakpoints in U.S.
Army Doctrine*, 2018-04-27 *What Is A Breakpoint*, and 2024-03-26 *The
40% Rule*:

- **FM 105-5, *Maneuver Control* (1964; 1967 ed. pp. 128–133),
  battalion level:** a defender's chance of breaking "approached 100
  percent" at about **40%** casualties. An attacker at about **20%**
  nearly always halts (type I) and breaks (type II) 40% of the time. TDI
  has never found the data behind these curves.
- **Clark (1954), 43 WWII battalions:** losses at the breakpoint
  averaged about 40%, ranging from about 1% to nearly 100%. In 15 cases
  (35%) the battalion had lost more than 40% before it broke. Clark: "a
  gross oversimplification not supported by combat data."
- **Clark's categories:** attack → rapid reorganisation → attack (9
  cases); attack → defence (21); defence → withdrawal by order (13);
  defence → collapse (5, discarded).
- **McQuie (1987), 52 of 80 engagements, 1941–1982:** forces gave up an
  attack or defence because of enemy manoeuvre in 64% of cases. Only
  **10%** were for casualty or equipment losses. "In most cases, a force
  has quit when its casualties reached less than ten percent per battle."
  Many simulations end battles at 15–30% casualties a day.
- **ADRP 1-02:** an enemy unit is suppressed at 3% losses, neutralised at
  10%, and destroyed at 30%. The basis is unknown.
- **C-WAM** (the Army's wargame): a brigade below 50% may only defend,
  and is removed below 30%.
- **WWII:** only 2 of 91 US divisions ever lost more than 30% in a week.
- **Movchin (1928):** a unit's combat strength starts to fall at about
  40% losses.

## Force ratios (2019-10-31, division level, 1904–1991)

The share of attacks won, against force ratio (672 cases; 570 after
removing limited probing attacks). Attackers win **55–62%** at 1.0–1.5:1
and 80–83% at 2.0–3.0:1. This is against the US Army's reading of the
3:1 rule as an even chance at 3:1. The table is in the CSV.

## Attrition (2019-12-26, *How Attrition is Calculated in the QJM vs the TNDM*)

The QJM's standard daily loss rates for division-sized forces after
1900: **attacker 2.8%, defender 1.5%**. They are scaled by force size,
mission, force ratio and so on, up to 13.4% and 5.8% a day. In the TNDM,
leaving out surprise, amphibious assault and CEV, the most is 18% a day
(attacker) and 21.6% (defender).

## Applied

- **`timing.DEFENDER_BREAK_AT` (0.4)** now cites FM 105-5 by way of these
  posts. The value is unchanged. `timing.ATTACKER_BREAK_AT` (0.2) already
  cited R4.6/P14 of the reference document. FM 105-5's 20% attacker
  figure agrees with it.

## Not applied: calibration targets instead

These are added to `research/calibration-targets.md`:

1. **Breaks are mostly not about casualties** (McQuie: 10%). The model's
   break tests run on losses alone. A loss-only break will over-predict
   fighting on when a unit is outflanked, and under-predict quitting
   with few casualties.
2. **Attacker win rate against force ratio** (table above), for the
   batch runs at company and battalion level. That use comes with a scale
   warning: these are division-level engagements.
3. **Daily loss rates** of 2.8% (attacker) and 1.5% (defender) for
   division-level runs.

## Limits

Blog posts, not papers. The figures are TDI's own databases or quoted
studies, and the underlying reports (Clark 1954, McQuie 1987, FM 105-5,
DMSi 1988) are not in the collection. The posts were scraped as text, so
their tables lost formatting. The CSVs were rebuilt by hand from the text.
