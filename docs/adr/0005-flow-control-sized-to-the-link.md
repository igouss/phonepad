# 0005: Flow control sized to the link instead of a fixed 4 units

Date: 2026-11-09. Status: accepted and implemented on branch `cell-2`.

## Weakest part at the time

Each phone could have 4 access units unacknowledged (`phonepad-core::stream`, ADR 0002). The cost was not 4 units per round trip but a keyframe interval: a fifth unit that found no credit desynced the phone, which then took no delta until a keyframe, asked for one only after an ack, and a requested keyframe comes no sooner than 500 ms after the previous one. So once a round trip was long enough for 4 units to be in flight, the phone got about 4 units, then waited half a second. Known case (`cargo xtask latency calibrate`, acking viewer, no decoder): 39.9 units/s at 0 ms added RTT, 10.7 at 100 ms, 8.0 at 200 ms; 4/RTT alone would allow 40 and 20.

## Forces

- A desync costs a keyframe: about 40 KB against a few hundred bytes for a delta, and up to 500 ms of waiting (the interval of ADR 0002, kept).
- A cellular link has a long and varying round trip, and loss stalls a TCP stream for max(200 ms, 2 × RTT) behind the lost segment. Units sent during a stall are not lost; dropping them only adds a keyframe.
- Unacknowledged bytes beyond what the link carries in a round trip wait in a queue and add their drain time to every later unit.
- The stream is app-limited: it carries far less than the link most of the time, so a delivery rate measured from it underestimates the link.
- The phone acks at decode, so a round trip includes its decoder.
- `phonepad-core` has no dependencies and no clock; the views draw each state machine from a finite list of states.

## Move

The window is bytes, sized from measurement (BBR's estimators, without its pacing or probing): on each ack the pipe measures the unit's round trip and a delivery rate (bytes acked since that unit was sent, over the time since the delivery that preceded its send, or since its own send into an empty pipe), keeps the minimum round trip over 10 s and the highest rate over 2 s, and admits a unit while the bytes in flight stay within rate × (min RTT + 60 ms), between 64 KiB and 4 MiB and at most 256 units. A unit into an empty pipe always goes, however large. An ack lands its unit and every unit sent before it (TCP delivers in order). A phone that is out of sync asks for a keyframe only when its pipe is empty, since no keyframe size is known before it is encoded, so the request it makes can always be taken.

`Stream` keeps its three phases; `Unsynced` and `Synced` carry the `Pipe` (`stream/pipe.rs`), a value folded by `evolve` from the `KeyframeSent`, `DeltaSent` and `Acked` facts, which now carry the unit's id, bytes and the session's monotonic time in microseconds. `stream::phase` reduces a state to its phase and an empty, roomy or full pipe; the views draw that projection (`xtask::machine::Labels`).

## Flagged assumptions

Chosen by the implementing agent under the operator's "accept all", not by the operator: the 60 ms queueing cap (the bead's p95 margin), the 64 KiB floor (a keyframe and a 300 ms loss stall of deltas fit, so a slow or app-limited estimate never refuses a delta), the 4 MiB ceiling, the 256-unit limit (also the timing ledger's size), the 10 s min-RTT and 2 s rate windows, the cumulative ack, and asking for a keyframe only on an empty pipe.

## Preserved

- A phone never receives a delta whose reference it did not receive (`a_viewer_never_receives_a_delta_whose_reference_it_did_not_receive`).
- A stalled phone never affects capture, encode or other phones: its flow control is its own, and a session that falls 8 units behind the broadcast misses units and resyncs.
- Keyframe requests only when the phone can take one; requested keyframes at least 500 ms apart.

## Evidence

`cargo xtask latency`, alternating main and this move on the same desktop, port 8781, two rounds of 15 s per profile, then 60 s of bad-lte. Units/s follow the screen; lan in the same run is the ceiling.

| client, profile | main units/s | this move units/s | main longest gap ms | this move longest gap ms | this move send→ack p95 ms |
|---|---|---|---|---|---|
| acking viewer, lan | 46.3, 44.3 | 50.3, 48.9 | 67, 90 | 147, 187 | 8, 10 |
| acking viewer, lte | 27.7, 23.7 | 45.2, 48.1 | 458, 480 | 63, 62 | 80, 80 |
| acking viewer, bad-lte | 7.8, 7.8 | 47.5, 49.8 | 746, 580 | 358, 375 | 449, 439 |
| acking viewer, bad-lte 60 s | 7.7 | 50.0 | 925 | 344 | 442 |
| page, lan | 43.7, 49.9 | 34.1, 48.9 | 71, 49 | 123, 55 | 20, 12 |
| page, lte | 11.9, 10.6 | 26.7, 50.1 | 481, 479 | 550, 48 | 90, 193 |
| page, bad-lte 60 s | 7.3 | 45.5 | 1114 | 731 | 551 |

The page decodes in software: at 50 units/s its receive→decode p95 reached 120 ms on lte and 211 ms on bad-lte (max 547), which is where its send→ack and its longest bad-lte gap come from; the acking viewer, which has no decoder, shows the link and the flow control alone. The page's software decoder cannot be removed from that measurement here; a phone decodes in hardware (ADR 0002: median 15 ms).

## What would reverse it

A phone whose queueing delay grows by more than the cap while the window admits units: the rate estimate holding a capacity the link no longer has. Then the window needs a delay signal (latest RTT against min RTT) as well as the rate, or the rate window must shrink.
