# 0004: Encode a capture only when its damage touches the encoded window

Date: 2026-11-09. Status: accepted; the duplicate encode per change it accepted is gone since [One frame waits for a commit](#one-frame-waits-for-a-commit-2026-11-10) below.

## Weakest part at the time

Every capture that arrived newer than the held one was encoded, whatever it showed. With the pointer still, phonepad encoded about 29 units/s on this desktop (4% of a core, mostly the VA-API driver), although the window the phone shows is a sixth of the monitor and often nothing in it changed. Hyprland 0.56.2 reports every capture as fully damaged, so phonepad had no way to tell; a patched Hyprland (branch `capture-damage` in `~/Projects/forks/Hyprland`) reports, with each frame, the damage that session saw since its own previous ready frame.

## Forces

- The operator's target of about 1% of one core (ADR 0003).
- Correct on both compositors: stock Hyprland must behave as before, the patched one must never lose a change.
- Two sessions, one frame in flight on each: each session's damage is relative to its own last frame, and a frame's timestamp is when the compositor shared it, not when the content was rendered.
- Damage arrives as `damage` events before `ready`; a failed frame's damage is kept by the compositor for the next frame of that session.

## Move

Each capture asks for the whole buffer (`damage_buffer`). The damage events of a frame are kept on its capturing slot (`Slot::Capturing { damage }`, `capture::plan::damaged`); the slot's damage dies with it when the frame fails. At `ready`, a frame changes the encoded window when nothing has been encoded yet, or when it is newer than the last encoded frame and any of its damage overlaps that frame's crop (`plan::changes_shown`, `Crop::overlaps`). Only those frames set the flag that schedules an encode; keyframes and window re-centres encode regardless, as before.

The dirty region the damage protocol suggests reduces to one bit here: a crop change already forces an encode, so the only crop a pending change is ever tested against is the last encoded one, fixed until the next encode. A frame no newer than the encoded one shows only content that is already encoded, whichever session took it, so its damage can be ignored. Every change reaches the phone: a change after the last encoded frame was taken is reported by the next frame of either session, and that frame is newer than the encoded one.

## Preserved

On stock Hyprland the full-damage event overlaps every crop, so the flag is set exactly when a frame newer than the last encode arrives, which is what the code did before. Flow control, keyframe pacing, the held-frame choice and the window geometry are unchanged.

## Cost accepted (removed, see below)

A change inside the window is encoded twice: once from the first session's frame, once from the other session's next frame, which reports the same change and carries a later timestamp. The second unit is a P-frame of 22–29 bytes. Telling the two apart needs the render time of a frame, which the protocol does not carry; skipping the other session's frame when its damage window straddles the last encode would instead delay some changes by up to a capture.

## Evidence

Nested 1254×1414 Hyprland (the host pointer disabled in it), a 400×400 view (a 600×600 window), a layer surface repainting a 50×40 rectangle ten times a second, 12 s measured per phase, phonepad CPU as % of one core:

| Phase | stock Hyprland, before | stock, after | patched Hyprland, before | patched, after |
| --- | --- | --- | --- | --- |
| nothing changes, pointer still | 23.8 units/s, 1.9% | 21.3, 2.0% | 23.6, 2.4% | 0, 0.4% |
| repaint outside the window | 20.1, 1.9% | 20.1, 1.6% | 20.1, 1.0% | 0, 0.3% |
| repaint inside the window | 20.1, 1.8% | 20.1, 1.5% | 20.0, 1.3% | 20.1, 1.6% |
| repaint inside, pointer moving in the window | 20.1, 2.8% | 20.1, 2.4% | 20.0, 2.1% | 20.0, 2.0% |
| pointer moving far (window re-centres), no repaint | 5.8, 1.6% | 6.5, 1.0% | 6.9, 0.7% | 6.2, 1.1% |

Decoding each stream with ffmpeg, every inside phase shows all 150 repaints of its 15 s. The same phases on an output with transform 1 (rotated 90°) give the same result on the patched compositor: 0 units/s outside, all 150 repaints inside, so its damage and the crop share buffer coordinates. Scaled outputs were not measured. On the live stock desktop (648×1072 view, back-to-back runs), before and after: pointer still 29.9 and 29.4 units/s against 28.4 and 29.3; pointer circling 25.6 and 26.2 against 26.3 and 25.8.

## What would reverse it

A compositor whose damage reports miss changes (a stale phone window would show it); or frames that carry render time, which would make the one-bit rule drop the duplicate encode too.

## Operator decision

None: every policy question goes back to the operator. The open question, whether the duplicate encode per change was worth a capture of extra latency to remove, lapsed: the move below removes the duplicate without adding that latency. The move was made under the epic `phonepad-cellular-latency-r9c`'s operator decision (bead r9c.8).

## One frame waits for a commit (2026-11-10)

### Weakest part at the time

The capture loop gave a frame to every idle session at once, so both sessions' frames waited for the same output commit. Hyprland 0.56.2 copies every waiting frame in each commit (`ScreenshareManager::onOutputCommit`) and stamps each with the time its own copy starts (`Time::steadyNow()` on `RESULT_TIMESTAMP`, `ImageCopyCapture.cpp`), so the two frames held one image under stamps 0.1–0.2 ms apart, and the later one counted as newer and was encoded again. A trace of main on the nested harness at 125 Hz content showed it: both frames requested within 15 µs, their `presentation_time` 0.1–0.2 ms apart, `ready` 12–15 ms after `presentation_time`, then both requested again. A pair took 17–33 ms, so main copied about 35 commits a second and encoded each twice (at 20 Hz content: 40 units/s for 20 changes/s).

### Forces

- One encode per content change, and up to 60 changes/s to the phone at 60 Hz.
- Hyprland holds one frame per session, and a frame copies for 12–15 ms between its commit and `ready` on the nested harness, most of a 60 Hz frame.
- A frame's stamp cannot identify its commit: it is when that frame's copy started, distinct for every frame.
- Stock Hyprland reports every frame as fully damaged, so damage cannot tell two frames of one commit apart.

### Move

Request a frame only while no other frame waits for a commit. A slot's lifecycle names the stages (`capture::plan::Slot`): `Requested` (waiting for a commit), `Copying` (its `presentation_time` came; it carries the stamp), `Held`, `Free`. `capture::plan::to_issue` returns at most one slot and session, and none while a slot is `Requested`; a session is busy while its frame is `Requested` or `Copying`. The second session stays: it lets the next frame wait for the next commit while the last one still copies. The stamp moved from an array beside the slots, where a frame without `presentation_time` read the stamp of the slot's previous frame, into `Copying`.

Alternatives evaluated:
- Deduplicate frames of one commit by their stamps: the protocol carries no commit identity and Hyprland's stamps differ per frame, so it would be a threshold on stamp distance against an unspecified copy order.
- Deduplicate by damage: stock Hyprland reports full damage every frame.
- One session (r9c.2's experiment): its next request goes out only at `ready`, 12–15 ms into the frame: 58.5 and 50.6 changes/s.
- Three sessions under the same rule: 50.4 changes/s in one run, against 59.0 for two in the run before it.

### Flagged assumption

A `ready` without a `presentation_time` before it (the protocol says the event is sent before `ready`; Hyprland always sends it) is held at stamp 0: kept only when nothing is held, and never encoded over a shown frame. On a compositor that sends `presentation_time` in the same flush as `ready`, the rule degenerates to one frame in flight.

### Preserved

The damage rule (`plan::changes_shown`), the held-frame choice, the encode schedule, keyframe pacing, flow control, the pool of four surfaces and the two sessions.

### Evidence

`cargo xtask latency nested calibrate` (lan, acking viewer; changes/s seen, units/s in brackets) and `nested hz=125 viewer lte`, binaries alternating back to back, every run started at 1-minute load < 3 (start / end load 1.4–3.0 / 1.4–3.8). main is 9b37443.

| Run | main 0 Hz | main 20 Hz | main 125 Hz | main lte 125 Hz | this move 0 Hz | 20 Hz | 125 Hz | lte 125 Hz |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 0.0 (45.6) | 20.0 (39.9) | 41.1 (57.1) | 33.6 (51.8) | 0.0 (42.1) | 20.0 (20.0) | 52.0 (52.6) | 58.7 (58.7) |
| 2 | 0.0 (44.9) | 20.0 (40.0) | 41.5 (56.7) | 36.1 (52.5) | 0.0 (41.7) | 20.0 (20.0) | 51.7 (52.3) | 59.0 (59.0) |
| 3 | 0.0 (41.3) | 19.9 (39.8) | 36.0 (52.1) | 31.3 (50.3), 34.5 (52.5) | 0.0 (41.9), 0.0 (46.7) | 20.0 (20.0), 20.0 (20.0) | 51.8 (52.3), 59.1 (59.1) | 51.2 (51.5), 50.5 (51.0) |

Single runs before the A/B: this move 58.9 (58.9) at 125 Hz; the uncommitted prototype 59.0 (59.0), with three sessions 50.4 (51.0); main 32.9 (50.1). Units equal changes at 20 Hz in every run of this move, against two units per change on main. The second calibrate of round 3 on this move overlapped a 30 s compile.

At 125 Hz this move reads either about 59 or about 52 changes/s, with no dependence on order or phase. A trace of a 52 run shows the next frame requested within 0.1 ms of the previous `presentation_time` and still copied 33 ms later in about 10% of intervals. A run that also logged the content client's frame callbacks (which Hyprland sends once per render) matched 545 of 551 copies to a render within 2 ms, and as many renders to a copy: every render of the nested compositor was copied, and the rate was its render rate (36.7 renders/s in that run, its last seconds while it was being stopped). So the 59/52 split is how often the nested compositor rendered, which follows the live compositor's pacing and the shared GPU, not the request cadence.

### Remaining

- The bead's 125 Hz clause (60 ± 10% in 2 of 2 runs) holds only while the nested compositor renders at 60/s: 3 of the 8 runs of this move at 125 Hz in the table passed, and the single run before it; none of main's 8.
- On a compositor that reports real damage, a session's damage covers the commits since its own previous frame, which with alternating sessions spans two commits. A change encoded from one session's frame is reported again by the other session's next frame and encoded once more if that frame is newer. Not measured: no patched Hyprland was built for this move.
- With nothing changing on stock Hyprland, units/s stays at 41–47 on both binaries (each request damages the output and makes the next commit); outside this move.

### What would reverse it

A compositor that copies one waiting frame per commit across sessions, or that sends `presentation_time` only with `ready` (the second session then idles); or a `ready` latency of a few milliseconds, which would let one session keep up and make the second cost a compositor session for nothing.
