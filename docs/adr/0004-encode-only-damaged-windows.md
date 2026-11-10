# 0004: Encode a capture only when its damage touches the encoded window

Date: 2026-11-09. Status: accepted.

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

## Cost accepted

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

None: every policy question goes back to the operator. Open: whether the duplicate encode per change (two sessions) is worth a capture of extra latency to remove.
