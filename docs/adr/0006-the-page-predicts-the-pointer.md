# 0006: The page predicts the pointer from the moves the desktop has not yet credited

Date: 2026-11-09. Status: accepted and implemented on branch `cell-5`.

## Weakest part at the time

On a touch the page sent a move and drew nothing until the pointer message came back: the ring and the pan inside the decoded window (ADR 0003) followed the finger one round trip late, 60–200 ms on cellular and longer behind a TCP loss stall. Nothing else the phone shows depends on the network as directly: the window is already on the phone.

## Forces

- A prediction is only as good as its knowledge of which moves a heard position already holds. Without it, every pointer message either drops the moves still in flight (the ring jumps back by them) or counts them twice.
- The pointer position comes from `hyprctl cursorpos` on the capture thread, while moves are written to uinput on the input thread. A position sampled while a move is being written may or may not hold it.
- Hyprland clamps the pointer to the monitor; a prediction that does not overshoots at every edge.
- Hyprland's libinput adaptive profile accelerates the phonepad device on top of the page's own acceleration (measured on this desktop: 8 moves of 2 px at 16 ms gaps moved 14 px, of 10 px 90, of 25 px 382), so the desktop's gain is not the page's and depends on the moves' arrival rate.
- The page is compiled into the binary and had no tests; the no-comments gate covered only `pad.html`.

## Move

Each move carries a sequence number; the input thread records a phone's last written move, and the pointer message (now 29 bytes) carries the position, the last move of that phone the position holds, and the monitor under the pointer. The records live in `marks::Marks`, a seqlock: the generation is odd while a move is written, and the sampler keeps a reading only if the generation was even and unchanged from before it copied the records until Hyprland answered, so a position is credited with exactly the moves written before it was asked for. The page (`pointer.js`, pure and tested under `node --test` by a new `page-tests` gate) predicts the last heard position plus the moves not yet credited, clamping each to the monitor, and on the frame after a touch redraws the ring and pans the view inside the monitor, drawing the part of the last decoded window the view overlaps.

The first version copied the records before the query without the generation. On lan, 4 of 256 pointer messages then jumped the ring back by more than 2 px, 3 of them by one whole move (6 px) that the message before had added twice: a move written between the copy and Hyprland's answer was in the position but not in the records, so the page counted it twice until the next message. With the generation, the same run had no backward jump.

## Flagged assumptions

- Hyprland answers a `cursorpos` query sent after a uinput write with a position that holds the write. On Hyprland 0.56.2, 200 of 200 queries sent right after a 10 px write saw it, and the harness saw no backward jump once the seqlock was in; if it stops holding, the page jumps back by the moves the position lacked, and `cargo xtask latency touch` reports it.
- The prediction uses the page's acceleration only. The libinput gain is left in place: making the phonepad device flat (`accel_profile flat` for its pointer) would make the prediction exact at speed but changes how the pointer feels, which is the operator's decision. Until then the ring jumps forward when the desktop accelerated more than the page predicted (51.9 px once on bad-lte, where moves arrive in bursts after a loss stall and libinput sees them as fast).
- A view that leaves the decoded window shows the pad colour there until a window around the new position arrives; the bead's optional margin around the view was not built.

## Evidence

`cargo xtask latency touch port=8811 seconds=10 lan wifi lte bad-lte`, one run, the page driven by CDP touches circling at 60 CSS px radius, about one move per display frame:

| profile | handler→redraw p50 / p95 / max ms | event→redraw p50 / p95 ms | pointer messages | largest backward jump px | largest jump px |
|---|---|---|---|---|---|
| lan | 0.2 / 0.4 / 0.7 | 32.9 / 47.9 | 290 | 0.0 | 0.0 |
| wifi | 0.2 / 0.5 / 1.0 | 32.9 / 40.7 | 293 | 0.0 | 0.0 |
| lte | 0.2 / 0.5 / 0.7 | 33.1 / 47.8 | 281 | 0.0 | 1.0 |
| bad-lte | 0.2 / 0.4 / 0.7 | 33.1 / 47.5 | 227 | 0.0 | 51.9 |

The redraw is the canvas draw in the animation frame that follows the touch handler, not the pixels on the screen. Event→redraw adds headless Chromium's input pipeline, about two frames before the handler runs, which is the same with or without the prediction. The circling speed is about 6 desktop px per move, where libinput's gain is close to 1, which is why lan to lte show no jump at all; faster fingers get forward jumps from the gain. Known cases for the instrument: a page that busy-waits 30 ms per paint fails the redraw check (p95 30.6 ms); a page that forgets its in-flight moves at each pointer message fails the jump check on lte (7.8 px).

## Preserved

The pointer message stays outside flow control and latest-value-wins; a phone gets one only when its position, its own last move or its monitor changed. Units, acks, flow control (ADR 0005) and the window of ADR 0003 are unchanged. Moves still go to the desk as phone input first.

## What would reverse it

A compositor whose position answer can lag a completed uinput write (the ring would jump back by the lagging moves), or a desktop gain the page cannot match and the operator will not make flat, if the forward jumps feel worse on a phone than the round-trip lag did.

## Operator decision

None beyond the epic's: every policy question goes back to the operator. Open: whether the phonepad device's pointer is made flat (and the page's acceleration retuned), and whether the server sends a margin around the view.
