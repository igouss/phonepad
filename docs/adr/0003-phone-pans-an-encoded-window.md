# 0003: The phone pans inside an encoded window; the pointer travels as its own message

Date: 2026-11-09. Status: accepted.

## Weakest part at the time

Every pointer move re-cropped the latest capture on the GPU and encoded it. The crop and colour conversion run on the render engine, which Hyprland and a GPU-heavy terminal keep 70–98% busy while the pointer moves, so each one waited 14–55 ms. Pipelining six conversions ahead of an encode thread raised the rate to 57 units/s, but at 6.5% of a core (about 1.1 ms of CPU per unit, mostly the VA-API driver), and the units reached the phone in bursts of two or three per display frame: replaying them gave 21 distinct presented views a second.

## Forces

- The operator's targets: 60 frames a second while the pointer moves, about 1% of one core, Hyprland's CPU as low as possible, at least 35 dB PSNR.
- Through FFmpeg and iHD a conversion plus an encode costs 0.2–0.5 ms of CPU even on an idle GPU, so 60 encodes a second cannot be near 1%.
- A pointer move over a static screen changes no pixel; only the view moves.
- The phone already decodes with WebCodecs and draws to a canvas.

## Move

Encode a window around the pointer, a quarter of the view larger on each side, and keep it while the view fits in it (`phonepad-core::geometry::window_around`). Send the pointer position to every phone as a 9-byte message outside flow control (a `watch` channel: latest value wins), at the rate the capture thread samples it (60/s while moving). The page draws the latest decoded window into an offscreen canvas and pans the view inside it on every pointer message. An encode happens only for a new capture, a window re-centre or a keyframe request.

The capture moved from `wlr-screencopy` to `ext-image-copy-capture` in the same series. Hyprland 0.56.2 never frees a wlr-screencopy session (one per client, monitor and region box, kept after the client disconnects) and scans them all on every frame. While the JPEG-era build captured regions on this desktop, a profile showed 31% of Hyprland's CPU as self time in `outputCopyFBState` and `nudgeTimers`; attributing that to leaked sessions is an inference from Hyprland's source, not a measured delta (150 full-output cycles on a nested Hyprland changed its CPU by less than the noise). ext sessions are destroyed with their resource. Scaled and rotated monitors stream: the window is cropped in panel pixels, scaled to its logical size by the same conversion, and the page undoes the output transform with a canvas transform.

## Flagged assumption

The operator asked for 60 frames a second to the phone. This move reads it as 60 distinct *presented views* a second on the phone, with encoded units at the rate the screen content changes. Both numbers are reported. The operator has not decided this reading.

## Evidence

Same desktop, alternating runs, one client, pointer circling over the terminal: units/s 15.5 → 25.5; presented views/s 11.5 → 39.7; pointer messages 60/s; phonepad CPU 1.3% → 3.4%. Against the pipelined crop-per-move build: 57 units/s at 6.5% CPU. With the pointer still and the wallpaper animating: 29 units/s at 4.1% (before: 22 at 1.5%), because Hyprland reports every capture as fully damaged and the window is 2.25 times the view's area.

## Preserved

Flow control and keyframe semantics (`phonepad-core::stream`), the one-encode-for-all-viewers broadcast, the auth model, input handling.

## What would reverse it

A phone that cannot keep a decoded window and pan it at display rate; or content-change detection (a Hyprland damage ring, or a cheap GPU comparison) that makes per-move encodes rare enough that encoding only the view costs less than encoding the window at capture rate.

## Operator decision

None: every policy question goes back to the operator. Open: the 60 frames/s reading above; the 25% window margin; the 500 ms keyframe interval (ADR 0002).
