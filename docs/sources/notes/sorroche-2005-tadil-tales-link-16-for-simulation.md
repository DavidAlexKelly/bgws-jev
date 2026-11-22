# Sorroche (2005): simulating Link 16 (TADIL-TALES)

**Source:** Joe Sorroche (ASRC Communications, Distributed Mission
Operations Center, Kirtland AFB), "Tactical Digital Information Link –
Technical Advice and Lexicon for Enabling Simulation (TADIL-TALES)".
Paper 41, 10th International Command and Control Research and Technology
Symposium, 2005. DTIC ADA461555.

**File:** `raw/simulation/sorroche-2005-tadil-tales-link-16-for-simulation.pdf`
(22 pages). Read on 22 November 2026.

## What it is

A standard for representing Link 16 (JTIDS, TADIL J) in DIS and HLA
simulations: how to encode messages, and five levels of timing fidelity.

## Figures

- **Timing (PDF p. 4):** time slots of 7.8125 ms, 128 per second, in three
  interleaved sets (A, B, C). An epoch is 12.8 minutes, of 32K slots per
  set (98,304 in all; p. 5). 112.5 epochs a day. Slots are assigned to
  Network Participation Groups in a network data load fixed in advance;
  a group's repetition rate is a power of 2.
- **Message size (p. 5):** each slot carries one 35-bit header (padded to
  48) and a number of 75-bit J-words (padded to 80).
- **Throughput ceiling (p. 5):** at most **1,536 J-words per second**:
  12 J-words per Pack-4 single-pulse slot × 128 slots per second.
  Enhanced-throughput (LET) packets are exempt.
- **Fidelity levels (p. 5, Table 2):** TSA 0 (message content only, no
  rate limit) to TSA 4 (full timing, sensitive to network latency). TSA 1
  throttles bandwidth to Link 16 rates without assigning slots. Network
  delays under 3 ms let the receive buffer be set to zero (p. 5).

## Limits

It gives no data rates in kbit/s and no radio range. The
`research/modern-warfare-reference.md` paragraph on Link 16 cites this
report alongside Wikipedia for "31.6, 57.6 or 115.2 kbps" and "ranges
exceed 300 miles"; those figures are not in this paper and need their own
source.

## How we could use it

- **Comms model:** the throughput ceiling (1,536 J-words/s per net) and
  slot timing bound how fast a Link 16 net can pass tracks.
- **Interop:** the TSA levels say what fidelity the interop crate should
  claim if it ever carries Link 16 Signal PDUs.
