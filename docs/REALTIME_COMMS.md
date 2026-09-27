# Real-time: what each unit knows, and how units talk

This builds on the command design ([REALTIME_COMMAND.md](REALTIME_COMMAND.md)). Before this change there were three shortcuts:
- **Every report reached everyone.** A sighting reached the whole side after a flat 15 s.
- **Reports were perfect.** A reported enemy's position was where it really was, live.
- **Help came from nowhere.** D9 fired because the engine knew a friend was in trouble.

Now each unit knows only what it has seen and what reaches it by radio. Units ask each other for help.

## Who does what

| | Game logic (every second) | Jev (at decision points) | Claude (when paused) |
|---|---|---|---|
| Recording sightings; ageing reports; uncertainty | ✓ | | |
| Sending contact, under-fire and situation reports | ✓ (automatic, a drill) | | |
| Delays, range, HQ relay, one talker at a time | ✓ | | |
| Delivering reports into each unit's picture | ✓ | | |
| Raising D9 and D12 from messages that arrive | ✓ | | |
| Answering a friend's request (D12) | | ✓ | |
| Asking a friend for cover or fire (as part of an option) | | ✓ | |
| Weighing a friend's trouble against its orders (D9) | | ✓ | |
| Saying which unit supports which | | | ✓ |
| Planning from what has reached HQ | | | ✓ |

Jev never writes a message or chooses who hears it. It picks an option, and game logic sends whatever that option carries.

## Each unit's picture (`types.ts`, `knowledge.ts`, `engine.ts`)

- **Seen itself: `ownSeen`.** Level, when, and where the enemy was. Live while in sight; forgotten after 2 min out of sight.
- **Told: `picture`.** One track per enemy:
  - level;
  - where it was reported, and when it was seen;
  - who saw it, and which HQ passed it on;
  - how far off the report may be.

  The uncertainty starts at 50 m and grows with age: 6 m/s if the enemy was moving when seen, 0.5 m/s if it was still (`trackErrorM`). A track is dropped 2 min after the sighting it came from.
- **`knownTo`** is the unit's own sight, or its picture. It is never the side's.
- **A report says where to look, not where to shoot.**
  - A unit fires only on what it has seen itself (`seesItself`).
  - A reported enemy is spotted at three times the usual rate while it is near where it was reported (`observerCued`).
  - Finding a reported enemy does not raise a new-contact decision.
- **A report of an enemy it knew nothing of** raises D1 (new contact).
- **The side's picture** is the best any of its units knows (`sidePictureLevels`). It drives the map's "blue eyes" and red eyes and the last-known rings.
- **The commander's picture** is `hqPicture`: what the side's HQ has seen and been told, or the best any unit knows if there is no HQ. Positions are where each enemy was seen, with how old that is and how far it may have moved. This is what Claude's orders prompt is built from.
- **Contacts on the board at the start** (a scenario's briefing) count as seen by every unit of that side.
- **Friendly positions are true.** Units know where their friends are. Situation reports tell friends what a unit is doing and how it stands.

## The radio net (`comms.ts`)

**Messages** are structured, not free text, so they are cheap and replayable:

| Message | Sent | Priority |
|---|---|---|
| under fire | automatically when fired on by a new shooter or when losing a vehicle; at most every 20 s. Carries the shooter if located, else the bearing; and vehicles left | 1st |
| request / reply | when a chosen option carries one: "cover me", "engage X"; "complying", "can't", "partly" | 2nd |
| contact | automatically on a new or better sighting, and every 60 s while still in sight. Not sent if a friend reported the same in the last 30 s | 3rd |
| situation report | every 2 min per unit, staggered (radio net only) | 4th |

**Two modes** (`RtConfig.comms`, "Communications" in setup):
- **Radio net** (the app's default):
  - A unit takes 10 s to compose a message: 20 s under fire, and 15 s with no HQ in range.
  - A pinned crew gets out only "under fire" and calls for help.
  - Then it waits for its side's net, which carries one transmission at a time (4 s each), most urgent first.
  - With an HQ in radio range (5 km), the message goes to the HQ, and the HQ passes it on after 5 s to everyone in its own range: two hops.
  - With no HQ, it goes straight to every friend within 5 km. A friend out of range never hears it.
  - A unit that is destroyed takes its unsent messages with it. An HQ that is destroyed or broken stops relaying, and units fall back to direct.
- **Perfect.** Every message reaches its recipients 15 s after it is sent (`reportDelayS`), whatever the range: the old side-wide sharing. This is the engine's default, so the tests and the harness are unchanged.

HQs are units with a command rating: some force lists include them, and the placement screen has an "HQ" tick box.

Nothing on the net rolls a die. Every delay is a fixed, declared figure, so a game still replays exactly from its seed and its decisions.

## Asking for help (`decisions.ts`, `runner.ts`)

**D9, friend needs help,** is raised when a friend's "under fire" report arrives, if all of these hold:
- the receiver is within 1.5 km of the friend;
- it is steady and not firing itself;
- it knows the attacker;
- it hasn't been asked about that friend in the last minute.

**Options that ask a friend.** An option can carry a request to a friend. The friend it goes to is:
- the unit whose orders say it supports the asker;
- else the nearest steady friend within 3 km, preferring one that knows the enemy.

The options that carry one:
- **D2:** `callFire` (return fire and ask the friend to engage it too), and `pullBackCovered` (pull back, asking the friend to cover the move).
- **D6:** `fireAndMove`, asking a friend to cover the move.
- **D0:** `covered`, which asks a friend to cover by radio. It used to order the friend directly.

**D12, request received.** The friend is asked how to answer:
- **comply:** engage the enemy from here, or move to get a shot, then engage;
- **partly:** overwatch from here, without moving;
- **can't:** carry on, saying why.

The reply goes back to the asker, and the answered request is cleared.

The rules comply, except in two cases:
- a unit told to avoid fights only partly complies;
- a unit already in its own fight with nothing to hit declines.

A unit that supports the asker always complies.

**Support links.** Claude's orders can say `"supports": "B1"`. B1's requests go to that unit first. Jev sees it in the orders, and at D12 it is told that supporting B1 is part of its orders.

## What Jev sees

- **Each enemy with its source:** "in sight"; "seen a few minutes ago"; or "reported by B2 (passed on by HQ1), seen under a minute ago; within a few hundred metres".
- **heardOnTheRadio:** what it heard in the last 3 minutes (friends under fire, requests, replies).
- **Its orders' support link**, if it has one.
- **D12's situation:** who asked, what, and whether supporting them is part of its orders.

## On screen

- **Setup:**
  - "Communications": radio net or perfect;
  - an "HQ" tick box when placing a unit.
- **Feed:** radio traffic as its own lines, e.g. `📻 B1 → HQ1: contact: …` or `📻 B1 → 3 units (passed on by HQ1): …`.
- **Unit inspector:**
  - each enemy with its source and age;
  - requests it has not answered, what it heard, and what friends last reported about themselves;
  - its support link.
- **Map:** with a unit selected, each enemy it knows only by report is drawn where it was reported, ringed by how far off that may be now.
- **Event stream:** a `message` entry type (see [EVENT_STREAM.md](EVENT_STREAM.md)).

## Declared figures

| Figure | Value |
|---|---|
| Compose a message; pass one on at an HQ; one transmission | 10 s (×2 under fire, ×1.5 with no HQ); 5 s; 4 s |
| Radio range | 5 km |
| Report position error; drift moving / still | 50 m; 6 m/s / 0.5 m/s |
| Re-report a contact still in sight; skip if a friend reported it within | 60 s; 30 s |
| Under-fire report at most every; situation report every | 20 s; 2 min |
| Spotting a reported enemy near where it was reported | ×3 |
| A friend's trouble is ours within; asked again after | 1.5 km; 60 s |
| Ask friends within | 3 km |
| Perfect comms delay | 15 s |

## Measured (rules deciding, 24 games each on generated relief)

- **Balance** is unchanged either way. Symmetric forces went 6–6 with perfect comms and 7–5 on the radio net. Every game ended on a breakpoint, none on time.
- **The radio net makes reactions slower and patchier.** "Friend needs help" decisions (D9) fell from 163 to 72: a friend hears later, or not at all.
- **Few requests get through on the radio net:** 8 delivered, against 27 with perfect comms. These fights last one to three minutes. A crew under fire takes about 20 s to compose, and many callers are knocked out before their call goes out.
- **Situation reports are the bulk of the traffic**, and load the net as they would a real one.

## Not yet

- Jamming and lost messages.
- Friendly positions from situation reports, rather than true positions.
- A hidden unit's choice to report and risk being heard, or stay silent.
- The map for one side's eyes still draws enemies where they really are. Only the selected unit's picture shows reported positions.
