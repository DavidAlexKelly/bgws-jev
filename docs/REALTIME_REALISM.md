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

## 5. What was built: Tiers 1 and 2

All of it is in `realtime/engine/`. The turn game is unchanged: its event logs are identical over the 72 seeded games of the equivalence check.

### Tier 1

| Item | How it works | Where |
|---|---|---|
| **Suppression separate from morale** | Every incoming shot adds suppression (0–100). A miss adds 8, a suppress result 18, a hit 30, and a damaging hit 15 more. That is ×0.6 in cover or hull-down, and less for better troops. After 10 s without fire it fades at 2/s. At 25 a unit is *suppressed*: it shoots worse, through the fire table's own modifier. At 60 it is *pinned*: it can't advance. | `engine.ts` §6–7, `timing.ts` |
| **Morale is derived** | Each element's `morale` is written from cohesion and suppression every tick: *broken*, *disrupted* (shaken), *suppressed2* (pinned), *suppressed1*, or *good*. The shared fire table and `canAdvance` read it unchanged. | `derivedMorale` |
| **Break tests at loss thresholds** | A unit tests its nerve when its losses reach 20% (attacker) or 40% (defender), then every further 20%. It also tests after a minute pinned. Score = d6 + ⌊TQ/2⌋, +1 in cover, +1 with an HQ within 1.5 km, −1 if pinned, −1 if a friend was lost in the last minute. 6+ passes; 4–5 shakes the unit; under 4, **losses** break it. Being pinned can only shake a unit. | `engine.ts` §8 |
| **Shaken** | Holds where it is and fires only in self-defence. | |
| **Broken → fall back once → rally** | A broken unit withdraws **once**, to a rally point: an HQ, then the nearest friend further from the enemy (preferring one out of the enemy's sight), otherwise 800 m away. It is never sent back again. Every 60 s, once it is out of fire (suppression under 25 and no incoming fire for 30 s), it rolls d6 + ⌊TQ/2⌋, +1 with an HQ nearby, +1 with no enemy in sight. 7+ moves it up a step: broken → shaken → steady. | `engine.ts` §8–9 |
| **Autopilot while shaken or broken** | The runner doesn't ask shaken or broken units. On rallying, a unit raises a severe `rallied` event and is asked again. | `runner.ts` |
| **Missions** | Every unit has a `mission` (take, hold or support; a place; a purpose), set from its opening order and kept when the order changes. | `types.ts`, `initialOrders.ts` |
| **Idle re-ask** | A steady unit that is off its mission and has been quiet for 75 s (no events, no incoming fire, no shots) raises `idle`. The rules answer **resume**. This is what fixes the winner holding for ever. | `engine.ts` §10, `deciders.ts` |
| **Pursue / consolidate** | When an enemy a unit can see breaks, that unit gets `enemyBroke`. The options include `pursue:X` (assault after it) and `consolidate` (overwatch). The rules pursue when the mission is to take ground, and consolidate otherwise. | `options.ts`, `deciders.ts` |
| **Side breakpoint** | A side is beaten when 50% of its starting strength is destroyed or broken. The time limit remains as a backstop. | `engine.ts` §11 |
| **Fire for real time** | A shot every 30 s. The fire table is rolled as it is, but a hit costs strength (3 CS) only with probability hits × `lethalityPerTurn` × 30 / 900. **`lethalityPerTurn` = 1.5** is the calibration knob. | `engine.ts` §5–6 |

### Tier 2

| Item | How it works |
|---|---|
| **React-to-contact drill** | Applies to a steady unit when a *new* shooter fires on it, or when it runs into the enemy. Its weapon is ready within 5 s (return fire). If it is moving in the open, it dashes to the nearest cover within 150 m, away from the threat; otherwise it halts and engages. A bounding unit halts to cover. An assault presses on. This all happens before Jev answers, and the event says what the crew did. |
| **Movement modes** | `march`: full speed, doesn't fire. `tactical`: 60% speed, fires within its ROE (the default). `assault`: 80% speed, fires at anything, closes to 150 m before halting. `bound`: 300 m bounds with 40 s cover halts, alternating with the nearest bounding friend, and fires like overwatch while covering. `withdraw`: full speed, doesn't fire. Pinned units can't advance. |
| **Posture and hull-down** | A unit is *moving*, *halted*, *settled* (still for 30 s) or *hull-down* (settled, and in cover or at least 5 m above its nearest known threat). Hull-down counts as cover for fire and suppression. Posture also sets the range at which a unit is seen without a roll: 800 m moving, 500 m halted or settled in the open, 300 m in cover or hull-down. |
| **Self-defence and threat targeting** | Whoever fired on a unit in the last minute is always a valid target unless the ROE is "never". Targets are chosen by threat: +3 if it fired at me, +2 if it can hit me, +1 for a flank shot, minus range/3 km. |
| **Contact reports and last-known positions** | A unit knows what it sees at once (`ownSeen`) and can fire on it. Its side learns after 15 s (`reports`). When a contact fades, its last-known position stays in `lastKnown`. It is drawn as a ring on the map and given to Jev as `lostContacts`, with its age. |

### Jev

Jev's state now includes each unit's cohesion, suppression, posture, mission, whether it is on its mission, and what it can see itself. It also includes visibly breaking enemies and lost contacts. The instructions describe the movement modes and say that the crew has already run its drill. New options include `resume`, `assault:X`, `pursue:X`, `consolidate`, and the objective by mode (`objective`, `objective:march`, `objective:bound`).

### Measured

These are headless runs with the rule decider on flat ground. Identical forces go to each side's objective in tactical mode.

| Scenario | Games | Blue / red | Ended on breakpoint | Mean length |
|---|---|---|---|---|
| symmetric-control-v1 | 40 | 20 / 20 | 40 | 19 sim-min |
| combined-arms-v1 | 40 | 22 / 18 | 40 | 25 sim-min |

Lethality 2 gave 14 minutes and 1 gave 28. Before pinned tests were limited to shaking a unit, games ended in 6–10 minutes on suppression alone, with under one damaging hit each.

The tests in `realtime.test.ts` cover each mechanic, plus balance (identical forces) and decisiveness. Decisiveness means no game reaches the time limit, and a broken unit falls back at most once per break.

## 6. Giving Jev the context to decide

**What was seen:** two units traded long-range misses for minutes, and Jev kept answering "carry on".

**Causes:**
- **Unsure became "carry on".** With 15–20 options, even a clear preference can score under the 0.25 confidence threshold, and every low-confidence answer fell back to "carry on".
- **No memory.** Each question is a fresh request, and the state didn't say how long the unit had been firing or what the fire had achieved.
- **Misleading odds.** Options showed the fire table's "% to hit". In real time only a small fraction of hits do damage, so long-range fire looked many times more effective than it is.
- **The firing unit was never asked.** Missing isn't an event, so nothing prompted a review of the fire.
- **No way to close in.** The only "move closer" option was a point-blank assault, and nothing told a unit that a friend was already covering the target.

**What was added:**

| | |
|---|---|
| **Low confidence → the rules** | An unsure Jev hands the decision to the rule decider. The trace records Jev's pick and its confidence. |
| **Memory per unit** | Each unit keeps a record of its current fire: target, how long, shots, rounds that struck, and damage done. It also keeps the fire it is taking, by shooter, and its last 4 decisions: Jev's, the rules' or its crew's drill, including "carry on". Each decision says what it was about and what has happened since ("lost 0 strength, did 0 damage"). |
| **Review triggers** | After 3 minutes of firing, a unit raises `ineffective` if it did no damage in that time, or `review` if it did. The rules answer `ineffective` by closing to effective range, shifting to a target it can hurt, or flanking, whichever is at least 1.5 times better. |
| **Honest odds** | Every fire option shows its chance per minute of doing damage and the expected minutes to knock the target out, using the real-time lethality and the damage this side has already done. Threats to a unit are shown the same way. |
| **`close:X`** | Close to effective range of the target: the shorter of the weapon's short range and half its maximum. The chosen spot is on the near side, with a line of sight, preferring cover, fewer watchers and a shorter move. The summary gives the move and its time, the odds from there against the odds from here, the enemy's fire on you there, and which friends are firing on the target to cover you. Offered for the three nearest known enemies. |
| **Coordination** | Each known enemy lists `engagedBy`, `firingOn` and `damageYouHaveDoneToIt`. Each unit being asked, and each unit in the same fights, gets its memory and its friends within 1.5 km. Each fire option says who else is on that target, or that nobody is. Each question names the other units being decided at the same time, so they can be ordered to work together. |
| **Instructions** | Jev is told to judge fire by what it has done and by the damage odds, to change something when a long exchange isn't working, and to use a friend's covering fire to move. |

**Measured:**
- A request covering one side's whole combined-arms force is about 30,000 characters, roughly 8,000 tokens.
- Balance is unchanged within chance: 74 / 86 over 160 symmetric games, and 20 / 20 over 40 combined-arms games.
- No game reached the time limit.
