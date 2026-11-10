# phonepad

Use an Android phone as a touchpad and keyboard for a Hyprland desktop, with a live view of the screen around the pointer. It installs on the phone as an app from Chrome and reaches the desktop over Tailscale.

- One finger moves the pointer, tap clicks, two-finger tap right-clicks, three-finger tap middle-clicks, two-finger drag scrolls, tap-then-drag drags.
- L/M/R buttons, sticky Ctrl/Alt/Shift/Super, Esc, Tab, arrows, Backspace, Enter.
- ⌨ opens the phone keyboard; typed text goes to the focused window.
- The touchpad shows the screen around the pointer, with a ring at the pointer. The zoom button cycles 1.5× / 2.5× / 4×.
- The desktop pointer grows to size 64 while you control it from the phone and returns to normal when you use the real mouse or keyboard.
- While the app is open, an animated `owe` wallpaper is paused, so an unchanging screen costs nothing to stream. If the pause is lifted while you watch (`owe resume`, owed restarting), it is set again within 2 s. A pause you set yourself before opening the app is left alone.

KDE Connect does not work for this on Hyprland: its Wayland input needs the `RemoteDesktop` portal, which `xdg-desktop-portal-hyprland` does not implement.

## How it works

```
phone (installed web app)  ──wss──▶  tailscale serve :443  ──▶  phonepad on 127.0.0.1:8765
                                                                  │
                                    uinput virtual mouse+keyboard ◀┤
                                    wtype (chars with no key)      ◀┤
                                    wlr-screencopy + VA-API thread ──▶ H.264 access units back to the phone
```

Three crates (see [ADR 0001](docs/adr/0001-rust-server.md)):

- `crates/phonepad-core`: the decisions, as pure functions with no dependencies. `stream` is the per-phone flow control of H.264 access units, sized to the link ([ADR 0005](docs/adr/0005-flow-control-sized-to-the-link.md)): a phone may have unacknowledged as many bytes as its delivery rate times its minimum round trip plus a 60 ms queueing cap, at least 64 KiB, at most 4 MiB and 256 units, and a unit into an empty pipe always goes; the round trip and the rate are measured from the acks (an ack lands its unit and every unit sent before it), the minimum over 10 s, the rate as the highest of 2 s. A phone that falls behind gets no delta until the next keyframe, and asks for one only once its pipe is empty, so the keyframe is sure to be taken; `stream::phase` reduces a state to Hidden, Unsynced or Synced and an empty, roomy or full pipe for the views. `desk` the viewer count and pointer size, `wallpaper` who owns the wallpaper pause: this instance owns it while it holds an `flock` on `$XDG_RUNTIME_DIR/phonepad/wallpaper-paused` (the file names the owner's PID); a non-empty marker nobody holds is a crashed owner's pause, resumed at the next start or adopted by a viewing instance, and a held one belongs to a live instance and is never resumed by another, `geometry` the capture region, `keys` the typing plan over a character-to-key table. The state machines are sum types driven by `decide` and `evolve`.
- `crates/phonepad`: the binary. axum serves the page and the WebSocket; one actor per phone session and one desk actor own the core state machines and execute their commands; one thread owns the uinput device. A Wayland `ext_idle_notifier_v1` input-idle notification with a 50 ms timeout reports the first device input after each quiet spell of 50 ms; Hyprland raises it for keyboard keys and pointer, touch and tablet devices but not for cursor warps, so a report more than 300 ms after the last phone input is real mouse or keyboard use. The timeout is short because a report comes only after a quiet spell: with 300 ms, a mouse taken up within 300 ms of the last phone input and moved without a 300 ms pause never produced one, and the pointer stayed large. While a phone views, the desk polls `owe status` every 2 s and re-asserts or takes over the pause. `owe`, `gsettings` and `hyprctl` are killed after 2 s without an answer, and shutdown ends within 6 s (3 s for the desk, 3 s for the runtime) even if a capture thread ignores the stop. For each typed text it reads the layout Hyprland applies to the `phonepad` keyboard (`j/devices`: rules, model, layout, variant, options, active layout) and builds the character-to-key table with libxkbcommon, keeping the last one until the layout changes; characters the layout has no key for go to `wtype`. If the layout cannot be read or compiled, the text is logged as not typed. While the app is visible on any phone, one capture thread keeps two `ext-image-copy-capture` sessions of the whole monitor under the pointer, into a pool of four encoder surfaces exported as linear dmabufs, and requests a frame only while no other frame waits for the compositor: a frame waits until an output commit copies it, which `presentation_time` reports, and is copying until `ready` (`capture::plan::Slot`, `capture::plan::to_issue`). Hyprland copies every waiting frame in the same commit, so two waiting frames would be one image encoded twice. The second session lets the next frame wait for the next commit while the last one is still copying: Hyprland allows one frame per session, and on the nested harness `ready` came 12–15 ms after `presentation_time`, most of a 60 Hz frame. The sessions end with the capture thread. Hyprland 0.56.2 never frees a `wlr-screencopy` session (one per client, monitor and region box, kept even after the client disconnects; `ScreenshareManager.cpp` erases one only when it is stopped, which nothing does), and scans all of them on every frame. On this desktop, while the JPEG-era build was capturing regions, a profile showed 31% of Hyprland's CPU as self time in that scan (`outputCopyFBState`) and in the event loop's timer scan (`nudgeTimers`, one timer per session); that those are the leaked sessions is read from Hyprland's source, not measured as a delta (150 full-output cycles on a nested Hyprland changed its CPU by less than the noise). It encodes a *window* around the pointer, a quarter of the phone's view larger on each side, and keeps that window while the view still fits in it; the phone pans inside the latest decoded window itself. So a pointer move costs no encode: the capture thread asks Hyprland for the pointer position (60 times a second while it moves, 20 when still) and every phone gets the position as a 29-byte message outside flow control, latest value wins. An encode happens only for a captured frame that changed the encoded window, a window re-centre or a keyframe request, at most 60 times a second. Each capture asks for the whole buffer (`damage_buffer`), and the compositor reports with each frame the `damage` that session saw since its own previous frame; a frame changes the window if it is newer than the last encoded frame and any of its damage overlaps the window's crop (`capture::plan::changes_shown`). A frame no newer than the encoded one shows nothing the phone lacks, whichever session took it; a failed frame changes nothing, since the compositor keeps its damage for the next. Hyprland 0.56.2 reports every frame as fully damaged, so there every newer frame is encoded, one per output commit; a compositor that reports real damage gets no encode while only content outside the window changes. Its damage covers the commits since the same session's previous frame, so a change inside the window, encoded from one session's frame, is reported again by the other session's next frame and encoded once more if that frame is newer (see [ADR 0004](docs/adr/0004-encode-only-damaged-windows.md)). The crop and colour conversion are submitted on the GPU and handed to an encode thread, up to six in flight, because one conversion can wait 55 ms behind Hyprland on the render engine; a capture surface is not reused while a frame cropped from it is still being encoded. Each access unit is encoded once and broadcast to every phone; keyframe requests from all phones coalesce into one flag, and a requested keyframe comes no sooner than 500 ms after the previous one, so a slow phone that keeps falling behind and asking cannot make every phone download a keyframe per frame (with one such phone the healthy one got 40 keyframes in 8.5 s before, 17 after); a phone whose socket falls behind misses units and resyncs on a keyframe instead of slowing the others.

Messages to the phone: `0`, the unit id (big-endian u32, counting encodes), `window x, window y` (big-endian i32, global logical desktop pixels), the output's `wl_output` transform (one byte), then the access unit; `1`, `pointer x, pointer y` (big-endian i32), the sequence number of this phone's last move the pointer position includes (big-endian u32, 0 for none), and the monitor under the pointer: `x, y` (big-endian i32) and `width, height` (big-endian u32), in global logical pixels. Each move the page sends carries a sequence number (`{"t":"move","dx","dy","s"}`, counting from 1 per connection). The input thread records a phone's sequence number after it writes the move to uinput, inside a generation count that is odd while a move is being written (`marks::Marks`, a seqlock). The capture thread copies the records, asks Hyprland for the pointer position, and keeps the answer only if the generation was even and unchanged across both, so a position is credited with exactly the moves written before it was asked for; an answer a move overlapped is dropped and the pointer is asked for again one sample period later. A phone gets a pointer message only when its position, its own last move or its monitor changed. The order this relies on, a `cursorpos` query sent after a uinput write reflecting that write, is Hyprland's, not phonepad's (200 of 200 queries right after a 10 px write saw it, on Hyprland 0.56.2). The page acks each unit by its id once it is decoded or dropped (`{"t":"ack","id":…}`), and after the first paint that shows a decoded unit it reports `{"t":"drawn","id","rx","dec","at"}`: when it received, decoded and drew it, in the page's `performance.now()` milliseconds. With `PHONEPAD_TIMINGS` set, each session prints one `timing` JSON line to stdout per acked unit (its bytes, capture→send and send→ack in milliseconds; capture is Hyprland's presentation time of the frame, send is the hand-off to the socket) and per drawn report (receive→decode, decode→draw and the gap since the previous drawn unit, on the page's clock). The service leaves it unset. The window is cropped from the capture buffer in panel pixels and orientation, scaled to its logical size on the GPU, and the page turns or mirrors it by the transform, so scaled and rotated monitors stream (checked against `grim` on a nested Hyprland at scale 2 with transforms 1 and 5: the sent transform matched with 28–32 dB, every other orientation 20–22 dB).

`crates/phonepad-h264` wraps FFmpeg's VA-API H.264 encoder and is the only crate allowed `unsafe` (see [ADR 0002](docs/adr/0002-h264-vaapi.md)); `PHONEPAD_RENDER_NODE` overrides the render node (default `/dev/dri/renderD128`).

The page (`pad.html`, `pointer.js`, `manifest.webmanifest`, icons) decodes the stream with WebCodecs `VideoDecoder` and is compiled into the binary, so changing it needs a rebuild. `pointer.js` holds the page's pure pointer arithmetic, with no DOM, so `pointer.test.js` runs it under Node's test runner. The page does not wait a round trip to show a move: on the display frame after a touch it predicts the pointer as the last heard position plus every move it sent that the server has not yet credited, each move clamped to the monitor under the pointer as Hyprland clamps it, and redraws the ring and pans the view inside the last decoded window, showing the pad colour where the view leaves the window. A heard position drops the moves it already holds (by sequence number), so a prediction is corrected only by what the desktop did differently: the prediction applies the page's own acceleration, but Hyprland's libinput adaptive profile accelerates the phonepad device again (gain 0.9 at 2 px per 16 ms, 1.1 at 10, 2.0 at 25, measured on this desktop), so at speed the ring jumps forward when the desktop's position arrives. See [ADR 0006](docs/adr/0006-the-page-predicts-the-pointer.md).

## Security model

- phonepad binds `127.0.0.1` only. The phone reaches it through `tailscale serve`, which adds the caller's `Tailscale-User-Login` header.
- Every request whose `Tailscale-User-Login` differs from `PHONEPAD_LOGIN` gets 403. Tagged devices carry no user login, so other machines on the tailnet cannot inject input or view the screen.
- Anyone holding your unlocked phone with the app open can control and see the desktop.
- phonepad reads no input devices and needs no `input` group: it learns only that some input happened (ext-idle-notify), never which key or where.

## Requirements

- Hyprland (needs `ext_image_copy_capture_manager_v1`, `ext_output_image_capture_source_manager_v1` and `ext_idle_notifier_v1` v2), `hyprctl`, `gsettings`, `wtype`
- libxkbcommon and xkeyboard-config
- Rust (stable, pinned in `rust-toolchain.toml`)
- Read/write access to `/dev/uinput` for your user
- Tailscale on the desktop and on the phone, desktop with `tailscale set --operator=$USER`
- Chrome on Android

## Install

```sh
git clone https://github.com/igouss/phonepad ~/.local/share/phonepad
cd ~/.local/share/phonepad && cargo build --release

mkdir -p ~/.config/phonepad
echo "PHONEPAD_LOGIN=you@example.com" > ~/.config/phonepad/env

cp phonepad.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now phonepad

tailscale serve --bg --https=443 http://127.0.0.1:8765
```

`PHONEPAD_LOGIN` is your Tailscale login (`tailscale whois <phone-ip>` shows it). On the phone, open `https://<desktop>.<tailnet>.ts.net/` in Chrome, then ⋮ → Install. For it to reconnect after a phone restart, enable Always-on VPN for Tailscale in Android's VPN settings.

After changing code or the page: `cargo build --release && systemctl --user restart phonepad`.

## Development

Gates, generated views, the end-to-end check and the latency harness: [docs/development.md](docs/development.md).

## Measurements

On an Intel UHD Graphics 630 (Hyprland renders there; AQ_DRM_DEVICES selects the iGPU), 2560×1440 at 60 Hz, a 648×1072 view, test clients on the same machine, a uinput mouse circling the pointer over a terminal. CPU is % of one core, per thread from `/proc` over 8 s; GPU engine time from DRM `fdinfo`. The animated wallpaper was playing (the test instance's `owe` is a stub) and another phonepad instance was streaming to a phone, so Hyprland and the render engine were busier than on a quiet desktop. *Presented views/s* replays each client's messages (latest decoded window × latest pointer, one view per 60 Hz display frame) and counts views that differ from the previous one by more than 200 pixels; a pan of a few pixels on flat content counts as no change, so it undercounts.

| Build, pointer moving | units/s | presented views/s | pointer messages/s | KB/unit | phonepad CPU | Hyprland CPU |
|---|---|---|---|---|---|---|
| before (crop per move, encode on the capture thread) | 15.2–15.8 | 10.8–12.1 | – | 6.0 | 1.2–1.4% | 10.5–11.2% |
| now, 1 client | 25.5 | 38.7–40.7 | 59.9 | 2.9–3.0 | 3.2–3.5% | 10.5–10.8% |
| now, 4 clients (each) | 26.1 | 38.4–38.8 | 59.9 | 2.1 | 6.0% | 10.5% |

- Pointer still: before 22 units/s, 1.5% CPU, Hyprland 6.2%; now 29 units/s, 4.1%, Hyprland 9.4%, with the wallpaper playing behind the window. Every capture is encoded because Hyprland 0.56.2 reports every capture as fully damaged; with real damage reports the still pointer costs no encode (ADR 0004).
- The phonepad CPU is mostly the VA-API driver preparing each conversion and encode (about 0.3–0.8 ms of CPU per unit); it scales with units/s, which follow Hyprland's commits. Each extra client costs about 0.9% for its socket writes (60 pointer messages and 26 units a second).
- The conversion (`scale_vaapi`) runs on the render engine, which Hyprland and the terminal keep 70–98% busy while the pointer moves, so one conversion waits 14–55 ms in the queue; the encode itself takes about 1 ms on the video engine. `cargo xtask h264-check` measures the encoder alone: 0.2–0.5 ms of CPU per unit, worst-frame PSNR 38.9 dB, and the rate with three conversions in flight (77–299 units/s, load-dependent).

## Limitations

- Real mouse or keyboard use that starts within 300 ms of phone input shrinks the pointer only at its first 50 ms pause that ends more than 300 ms after the last phone input.
- `owe` has one manual-pause flag. A pause you set while phonepad already holds its own cannot be told apart from phonepad's, so it is resumed when the last viewer leaves; an `owe resume` while a phone is viewing is undone within 2 s.
- When the instance that owns the pause stops viewing while another instance still views, the wallpaper can animate for up to 2 s before the other takes the pause over.
