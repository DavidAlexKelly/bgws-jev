# Real-time mode: what it takes to behave realistically

## 1. What was seen, and why

> They approached and engaged correctly. As soon as a unit was hit it withdrew, and kept withdrawing for ever. The unit that hit it held for ever.

Three causes, all in the real-time engine:

| Seen | Cause |
|---|---|
| **Withdraws for ever** | A broken unit is given "withdraw 1 km". When it arrives, its order becomes "hold". The next tick it is still broken and not withdrawing, so it is sent another 1 km, and this repeats indefinitely. Broken never recovers in real time, and a broken unit is never asked for a decision. |
| **Breaks after one or two hits** | The fire table's lethality (3 CS and 1 morale step per hit) was tuned for **one shot per 15-minute turn**. Real time gives a shot every **5 minutes**, often from several firers at once. The morale ladder is only four steps from *good* to *broken*, so a couple of results breaks a unit in minutes. |
| **The winner holds for ever** | Units are only asked for a decision when something *happens* to them. Once its target was out of sight, the winner went to "hold", and nothing happens to a unit that is holding with no enemy in view. So it was never asked again. Nothing brings a unit back to its mission. |

The first and third are bugs. The second is missing modelling, and the rest of this document is about that.

## 2. What established simulations do

| Source | What it does | What it suggests here |
|---|---|---|
| **Combat Mission** (Battlefront) — [morale wiki](https://combatmission.fandom.com/wiki/Morale), [community notes](https://community.battlefront.com/topic/102770-morale-suppression-levels/) | Tracks **suppression separately from morale**. The morale ladder is *Rattled → Pinned → Shaken → Panicked → Broken/Routed*. Pinned troops dash for safety. Shaken troops cower and don't fire. Panicked troops run. **Shaken and below cannot be given orders**: the tactical AI runs them until they recover. Morale **recovers over time**, faster with better motivation and leadership, and with a command unit nearby. | Split suppression from morale. Make breaking temporary and recoverable. Let the autopilot, not Jev, drive units that are too shaken to take orders. Leaders speed up recovery. |
| **The Dupuy Institute** — [breakpoints](https://dupuyinstitute.org/2018/04/27/what-is-a-breakpoint/), [the 40% rule](https://dupuyinstitute.org/2024/03/26/the-40-rule/), [suppression](https://dupuyinstitute.org/tag/suppression/), [TNDM](https://dupuyinstitute.org/products/tndm-qjm/) | A "breakpoint" is a **forced change of posture**, not a count of hits. Historical battalions broke at around 40% casualties on average, with a very wide range. US doctrine used about **20% for attackers and 40% for defenders**. **Suppression matters more to outcomes than attrition**, and the TNDM is almost the only model that represents it. | Break at **loss thresholds, adjusted for posture and quality**, instead of every hit knocking morale down a step. Model suppression as its own main effect of fire. |
| **Steel Beasts** — [routes](https://www.steelbeasts.com/sbwiki/index.php/Moving_Units_On_Routes), [tactics](https://www.steelbeasts.com/sbwiki/index.php/Tactics), [hull-down](https://www.steelbeasts.com/topic/6-ai-hull-down/) | Each route carries a **tactic** that decides how the unit reacts to contact. *March*: stay on route unless under direct fire or flanked, then stop and go hull-down facing the enemy. *Scout*: stop turret-down; if fired on, retreat along the route. *Retreat*: reverse, pop smoke, then run. Routes end in **battle positions**, where units find hull-down. **Units return fire if fired on**, unless ordered to hold fire. | Give orders a **movement mode** that sets the autopilot's contact drill. Add hull-down. Always return fire unless the order is "never". |
| **Arma 3** — [behaviour](https://community.bistudio.com/wiki/AI_Behaviour), [combat modes](https://community.bistudio.com/wiki/Combat_Modes) | **Behaviour** (*Safe / Aware / Combat / Stealth*) is separate from **combat mode** (*Hold fire / return fire / engage at will*). *Combat* means bounding movement, using cover, and frequent contact reports. | Separate **how a unit moves** (posture) from **when it fires** (the rules of engagement it already has). |
| **Command: Modern Operations** — [weapon release](https://command.matrixgames.com/?p=3598), [manual](https://ftp.matrixgames.com/pub/CommandModernOperations/CMO%20manual%20EBOOK.pdf) | **Doctrine** per side, group or unit: weapon control *Free / Tight / Hold*, per-target-type release rules, automatic firing ranges, and **self-defence always allowed**. | Units always defend themselves, even under restrictive rules of engagement. Pick targets by threat and type, not simply the nearest. |
| **US Army Battle Drill 1: React to Contact** — [FM 3-21.8 drills](https://infantrydrills.com/archive-atp-fm-3-21-8-2016-appendix-j-selected-battle-drills/), [bounding overwatch](https://en.wikipedia.org/wiki/Bounding_overwatch) | On contact: **return fire at once, take the nearest cover, locate the enemy, report**, and only then decide. Moving elements use traveling or bounding overwatch, so one element is always covering another. | The first seconds of contact are a **drill the autopilot runs instantly**. Jev decides what comes next. Paired units bound. |
| **Wargame / WARNO** — [discussion](https://steamcommunity.com/app/251060/discussions/0/1291817837622012124/) | Stress runs from *calm* to *panicked* to *routed*, and stressed units perform much worse. Players pull stressed units back and bring them in again later. | Morale is a resource: a unit is pulled back, **recovers, and returns**. |
| **Tank engagement models** — [DTIC ADA092350](https://apps.dtic.mil/sti/pdfs/ADA092350.pdf), [probability of kill](https://en.wikipedia.org/wiki/Probability_of_kill) | Detection, then acquisition time, then shots, with **P(hit)** depending on range and aim and **P(kill given hit)** separate. Rate of fire is bounded by exposure and acquisition time. | Give fire a real **rate of fire** and per-shot **P(hit) and P(kill)**, instead of one table result per 5 minutes. |

## 3. What to add, in order

### Tier 1: fixes what was seen

1. **Suppression separate from morale.**
   - *Suppression* comes from any incoming fire, misses included, and builds up from each shot. It fades within tens of seconds once the fire stops. It lowers accuracy and spotting, and at high levels pins the unit: it can't advance.
   - *Morale*, or cohesion, drops with casualties and with sustained suppression, and recovers slowly.
   - Replace the fire table's "one morale step per hit" with this in real time.
2. **Break at a loss threshold, not per hit.**
   - A unit makes a break test when it crosses a loss threshold: about 20% as the attacker and 40% as the defender, following Dupuy, adjusted for troop quality and cover.
   - If it fails, it becomes *shaken* (stops, won't advance, fires only in self-defence) or *broken*.
   - A side-level breakpoint (for example, 50% of the force broken or destroyed) ends the engagement.
3. **Breaking is temporary: fall back, rally, return.**
   - A broken unit retreats **once**, to a rally point: towards its own side, out of known enemy sight, near friends or its HQ. There it stops and makes rally checks over time.
   - Recovery is faster with better troop quality, an HQ nearby, and time out of contact.
   - While shaken or broken the autopilot runs it and Jev isn't asked, as in Combat Mission. Once it has rallied, Jev is asked again.
4. **Mission persistence, and asking again when idle.**
   - Every unit keeps its **mission** (its task and objective) separate from its current order.
   - If a unit is idle with no enemy in sight for about 60–90 seconds, it raises an **idle** event. Jev is then asked, with "resume the mission" as an option, and "resume the mission" becomes the rule when Jev isn't sure.
   - When the enemy breaks: **pursue** (keep contact and fire) or **consolidate** on the objective.
5. **Recalibrate fire for real time.**
   - Resolve fire at a real rate of fire, with per-shot P(hit), P(kill given hit) and suppression, calibrated so a 15-minute exchange produces roughly what one turn-based result would.
   - Alternatively, keep the table and scale its lethality to the engagement cycle.
   - Tune both against the turn game with the headless runner.

### Tier 2: the autopilot behaves like a crew

6. **React-to-contact drill, instantly.** Return fire, take the nearest cover or hull-down within about 150 m, and report to the side. All in the first seconds, before Jev answers. Jev then chooses among *assault / hold and fight / flank / break contact*.
7. **Movement modes per order**, following Steel Beasts and Arma:
   - *road march*: fast, doesn't stop;
   - *tactical*: slower, halts and goes hull-down on contact;
   - *assault*: fires on the move and closes;
   - *bounding overwatch*: pairs alternate, one covering while the other moves;
   - *withdraw*: reverse and pop smoke.

   The mode sets the contact drill in item 6.
8. **Posture and hull-down.** Moving, stationary, in cover, or hull-down. Posture affects how easily a unit is seen, the enemy's P(hit), and what the unit is exposed to. Battle positions at the end of a move look for hull-down.
9. **Doctrine and self-defence.** Weapon control Free / Tight / Hold. A unit always returns fire when fired on unless it is set to Hold. Targets are chosen by threat (who is shooting at me, what can kill me, what is flanking), not by nearest.
10. **Contact reports and last-known positions.** A sighting is shared with the side after a short reporting delay, not instantly. Contacts that have faded stay on the map as *last known positions*, with their age shown.

### Tier 3: depth

11. **Command and control.** HQ command radius. Orders reach units after a delay that grows outside that radius. Losing the HQ slows orders and rally.
12. **Ammunition and rate of fire**, with resupply. Also artillery and smoke, so a withdrawal can be covered.
13. **An operational layer.** When a side hits a breakpoint, or loses or takes its objective, the LLM re-plans the side's missions. Jev keeps making the unit-level calls.

## 4. Suggested order

Tier 1, items 1–5, as one piece of work. That fixes the behaviour you saw and gives Jev a sensible world to decide in. Each item comes with headless tests and a before-and-after run of the identical-forces balance check, plus a check that games end on a decision rather than on the time limit or an endless retreat.

Then items 6–7, which do more than anything else to make the map look like real units fighting. Items 8–13 follow as needed.
