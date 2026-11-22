# Yeh et al. (2001): how much a cue shortens search, and what it costs

**Source:** Michelle Yeh (MITRE), MAJ James L. Merlo (US Military
Academy), Christopher D. Wickens (Aviation Research Lab, University of
Illinois at Urbana-Champaign) and David L. Brandenburg (North Carolina
State University), "Head-Up vs. Head-Down: Effects of Precision on Cue
Effectiveness and Display Signaling". *Proceedings of the 45th Annual
Meeting of the Human Factors and Ergonomics Society*, Santa Monica, CA:
HFES, 2001. DTIC ADA447823. Funded by the US Army Research Laboratory
(Federated Laboratory programme). Approved for public release.

**File:** `raw/sensing/yeh-2001-head-up-head-down-cue-effectiveness.pdf`
(7 PDF pages). Read on 22 November 2026. Page numbers are the paper's
own (1–6). PDF page 2 is an inserted DTIC form, so from printed p. 2
onwards the **PDF page is printed + 1**.

**Table:** `data/yeh-2001-head-up-head-down-cue-effectiveness--detection-time-by-cue-precision.csv`
(read from Figure 3; about ±0.3 s).

## What it is

Downloaded as "the cueing report". It is a short conference paper on
**display design**: does a cue to a target's position work better on a
helmet-mounted display (HMD, head-up) or a hand-held display (head-down),
and how do cue precision and display clutter change that. It is not about
ground observers in the field. But its first experiment measures exactly
the number the sim lacks: **how much faster soldiers find a target when
told where it is**, and how that falls off as the cue gets vaguer.

## Experiment 1 (pp. 2–4)

**Set-up (p. 2):**

- 16 military personnel (Illinois National Guard, p. 5).
- A CAVE virtual-reality room, 270° field of view, head-tracked shutter
  glasses. Static images of hilly terrain.
- Targets camouflaged in the terrain: tanks, soldiers, land mines
  (expected, 30% each) and a nuclear device (unexpected, 10%, always shown
  with a tank or soldier and never cued; reporting it took priority).
- A secondary task: monitoring a drifting radio-frequency gauge.
- Cues on half the trials. On the HMD, an arrow pointing toward the
  target from the current head direction, turning into a box reticle once
  the target was in the forward view. On the hand-held, a non-rotating
  map-style diagram.
- Cue precision, told to the soldier before each block:
  - **perfect:** 0°–7.5° from the target centre;
  - **degraded:** 7.5°–22.5°;
  - **poor:** 22.5°–45°.
- 10 blocks of 20 trials, then an 11th block with "catastrophic" cue
  errors, the cue more than 90° from the target (pp. 2–3).
- Target salience: tanks and soldiers were "high salience"; land mines,
  smaller in visual angle, "low salience" (p. 3).

**Detection time (p. 3, Fig. 3; read from the graph):**

| Target | Display | Perfect | Degraded | Poor | Uncued |
|---|---|---|---|---|---|
| Soldier (high salience) | HMD | 2.2 s | 3.9 s | 4.8 s | 6.5 s |
| Soldier | hand-held | ~3.5 s* | 2.9 s | 4.1 s | 7.7 s |
| Mine (low salience) | HMD | 2.7 s | 8.1 s | 10.5 s | 10.2 s |
| Mine | hand-held | 3.6 s | 6.4 s | 10.2 s | 8.9 s |

\* The soldier hand-held marker for "perfect" is hidden under the mine
marker; the value is uncertain.

**As ratios of the uncued time** (our arithmetic):

| Cue | Salient target (soldier) | Hard target (mine) |
|---|---|---|
| Perfect (≤7.5°) | 0.34–0.45 | 0.26–0.40 |
| Degraded (7.5–22.5°) | 0.38–0.60 | 0.72–0.79 |
| Poor (22.5–45°) | 0.53–0.74 | 1.03–1.15 (no gain) |

**What the paper says about these (pp. 3–4):**

- Cued targets were found faster than uncued, F(1,15) = 221.6, p < 0.01
  (p. 3).
- Time rose as precision fell (perfect vs degraded and perfect vs poor
  both p < .01) (p. 3).
- Imprecise cues hurt the low-salience mines much more than the
  soldiers. For soldiers, even the poor cue (up to 45° off) beat no cue
  (pp. 3–4). For mines it did not. The authors' explanation: a less
  visible target has a smaller useful field of view in the periphery, so
  it suffers more as the area to search widens (p. 4).
- **Accuracy was not changed by cueing.** Expected targets were detected
  about 95% of the time whatever the display or cue type (p. 3).

**Attentional tunnelling (pp. 3–4):**

- The unexpected, uncued nuclear device was found only **50%** of the
  time, against about 95% for expected targets (p. 3). Making the cue
  look imprecise (dashed) did not widen attention.
- There was a non-significant trend (p = 0.15) for the unexpected target
  to be missed more with precise cueing on the HMD (p. 3).
- **Catastrophic cue failures** (cue >90° from the target): the target was
  eventually found, but after **101.4 s** with the hand-held and **56.8 s**
  with the HMD (p. 3). Two soldiers reported false targets at the empty
  cued location (p. 3).
- After the first failure, detection of the unexpected target rose to
  100% (hand-held) and 82% (HMD) (p. 3); 91% against 50% overall (pp. 4,
  5). Lost trust widened attention.

**Head-up vs head-down (p. 3):** cued targets were found about 1 s
faster on the HMD. For uncued, low-salience mines, the HMD was about 2 s
slower: its symbology cluttered the view.

## Experiment 2 (pp. 4–5)

8 military personnel; same task; cues only at 45° precision; a much
busier HMD image (a translucent contour map) (p. 4).

- Cueing still helped, F(1,7) = 21.1, p < 0.01 (p. 4).
- The hand-held was better overall; the HMD clutter cost reached **6 s**
  for uncued mines (p. 4).
- Accuracy on the unexpected object was lower, but not changed by display
  or cue (p. 4).
- The authors' summary: cues that cut the search area to a diameter of
  **90° or less** could help detection on either display (p. 5).

## Does it transfer to a soldier told where a threat is?

Partly. What carries over:

- **Cueing shortens time, not whether the target is found.** Accuracy
  stayed at about 95% with or without a cue (p. 3). In model terms that
  is a cue scaling τ and leaving P∞ alone. That is how `acquire.rs`
  already treats it.
- **The size of the effect.** A precise cue (within 7.5°) cut search time
  to roughly 0.25–0.45 of uncued. The sim's unsourced
  `sensing.acquire.cue_factor` is 0.25, at the strong end of that range.
- **Precision matters, and more so for hard targets.** A verbal report
  ("tank, half left, by the barn") is probably degraded or poor in these
  terms (7.5°–45°). For a salient target that still cuts time to about
  0.4–0.75; for a small or well-hidden one, little or nothing.
- **Tunnelling.** A cued soldier is more likely to miss a second,
  unexpected threat (50% vs 95%). And a wrong cue is worse than none:
  56–101 s to recover.

What does not carry over:

- A lab room with static pictures, a 270° wrap-around display, and
  electronic cues with known precision. Not real terrain, ranges, optics
  or haze.
- Times include the head turn and button press; the floor of about 2 s
  is response time as much as search.
- The uncued searches took only 6.5–10 s across 270°. Real field searches
  are longer, so the absolute times do not transfer, only the ratios.
- Small samples: 16 and 8 soldiers. Values read from a graph.
- No numbers on a cue given by voice or by pointing.

## Limits

- Display-design study; the cue was a graphic, never a spoken report.
- Static images; no movement; no range or contrast variation reported.
- Figure 3 values are read off a small chart (±0.3 s); one marker is
  hidden.
- It gives no P(t) curves, only mean times, so τ ratios assume the mean
  time scales with τ.

## How we could use it

1. **Source `sensing.acquire.cue_factor`.** Replace the default 0.25 with
   a cited value. Suggested:
   - precise cue (a laser marker, a designator, an on-sight reticle, a
     battle-management system plot): **0.35** (range 0.25–0.45),
     Fig. 3, p. 3;
   - verbal or approximate cue (7.5°–45°): **0.6 for vehicles and
     standing men, 1.0 for small or well-concealed targets** (range
     0.4–1.0), Fig. 3, p. 3.

   That needs the cue to carry a precision, or two cue types.
2. **Keep P∞ unchanged by a cue.** The paper backs the current design
   (accuracy unaffected, p. 3).
3. **Optional: tunnelling.** While an observer is working a cued
   contact, halve (50/95) the chance of detecting a different, uncued
   threat. A wrong cue could add about a minute before the real target is
   found (56.8–101.4 s, p. 3). Both are rough, from one lab study; use
   only if the sim models false or stale reports.
