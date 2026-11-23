# IR & EO Systems Handbook, vol. 5 (1993): Passive Electro-Optical Systems

**Source:** Stephen B. Campana (ed.), *Passive Electro-Optical Systems*,
vol. 5 of *The Infrared & Electro-Optical Systems Handbook*,
J. S. Accetta and D. L. Shumaker (executive eds.), ERIM (Ann Arbor) and
SPIE Optical Engineering Press (Bellingham), 1993.

**File:** `raw/sensing/irhb-1993-v5-passive-eo-systems.pdf` (362 PDF
pages; text layer). Read on 23 November 2026. Printed page = PDF page − 12.

**Table:** `data/irhb-1993-v5-passive-eo-systems--johnson-cycles.csv`
(Tables 1.6–1.9).

## What it is

Chapters on infrared line scanners (ch. 1), forward-looking infrared
(FLIR, ch. 2), staring focal-plane-array imagers (ch. 3), and search and
track, with the performance models behind them. We read it for the
**human performance criteria**: how much resolution on a target a task
needs.

## What we use

### The Johnson criteria (ch. 1, pp. 28–31 [40–43])

These are cycles across the target's **critical dimension** for a given
chance of doing a task, for army vehicles. The critical dimension is the
smaller one seen: the height from front, side or rear, the width from
above. One cycle is two pixels. The tables come from the Night Vision
Laboratory static performance model (Ratches et al.), after Johnson and
Lawson.

| Task | Cycles for 50% | Table |
|---|---|---|
| Detection | **1.0** (0.95 at 2.0) | 1.6 |
| Classification | average of detection and recognition, **2.25** | text, p. 28 |
| Recognition | **3.5** (pessimistic 4, optimistic 3) | 1.7 |
| Identification | **6** on film; **8** on a monitor | 1.8A, 1.8B |
| Ship on the open ocean, detection | 0.5 | 1.9 |

The identification values are "approximate", "not supported by any
studies of interpreter performance", and work in design "with good
results" (p. 29).

### Sampled imagers (ch. 3, pp. 183–184 [195–196])

For a staring array, recognition took 4.6 cycles per target height when
range is scaled to computed MRT. That is "only slightly" more than
Johnson's 3–4. A design example uses a 3.5 km recognition range, a
3.3 m target, a **1.5 K** target contrast, 4.8 cycles, and an
atmospheric extinction of **0.29 /km**.

## Applied

**The world layer's classification ladder for imaging sensors** (eye,
optic, thermal, EO camera, IRST) in `world/sensing.ts` (`johnsonRank`).
Before, the ladder was hand-set: identity inside 30% of reach, type
inside 60%, class inside 90%. Now it follows the cycles on target, which
scale as 1/range, against:

- `world/sensing.JOHNSON_CLASSIFY` = 2.25
- `world/sensing.JOHNSON_RECOGNISE` = 3.5
- `world/sensing.JOHNSON_IDENTIFY` = 6

The anchor `world/sensing.CYCLES_AT_REACH` (cycles at the sensor's
catalogue reach) is **declared**: 1.8 keeps identity at 30% of reach. The
ladder becomes identity < 30%, type < 51%, class < 80%. Setting it to
1.0 would read the catalogue reach as the 50% detection range, giving
17% / 29% / 44%. Which is right depends on what the catalogue's
`rangeMaxM` means, and that is not written down.

Radar and sonar keep the old ladder: Johnson is for images.

## Not applied

- The 1.5 K design contrast is for a target in the open, not cammed up.
  It is no source for `world/sensing.CAMO_DT_K` (2 K, from O'Kane).
- The 0.29 /km LWIR extinction is a single design value. Thermal sights
  in the world layer have no atmospheric loss in clear air; see the
  vol. 2 note for fog.
