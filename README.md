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

- `crates/phonepad-core`: the decisions, as pure functions with no dependencies. `stream` is the per-phone flow control of H.264 access units (up to 4 unacknowledged; a phone that falls behind gets no delta until the next keyframe, and asks for one only once it has credit to take it), `desk` the viewer count and pointer size, `wallpaper` who owns the wallpaper pause: this instance owns it while it holds an `flock` on `$XDG_RUNTIME_DIR/phonepad/wallpaper-paused` (the file names the owner's PID); a non-empty marker nobody holds is a crashed owner's pause, resumed at the next start or adopted by a viewing instance, and a held one belongs to a live instance and is never resumed by another, `geometry` the capture region, `keys` the typing plan over a character-to-key table. The state machines are sum types driven by `decide` and `evolve`.
- `crates/phonepad`: the binary. axum serves the page and the WebSocket; one actor per phone session and one desk actor own the core state machines and execute their commands; one thread owns the uinput device. A Wayland `ext_idle_notifier_v1` input-idle notification with a 300 ms timeout reports the first device input after a quiet spell; Hyprland raises it for keyboard keys and pointer, touch and tablet devices but not for cursor warps, so a report more than 300 ms after the last phone input is real mouse or keyboard use. While a phone views, the desk polls `owe status` every 2 s and re-asserts or takes over the pause. `owe`, `gsettings` and `hyprctl` are killed after 2 s without an answer, and shutdown ends within 6 s (3 s for the desk, 3 s for the runtime) even if a capture thread ignores the stop. For each typed text it reads the layout Hyprland applies to the `phonepad` keyboard (`j/devices`: rules, model, layout, variant, options, active layout) and builds the character-to-key table with libxkbcommon, keeping the last one until the layout changes; characters the layout has no key for go to `wtype`. If the layout cannot be read or compiled, the text is logged as not typed. While the app is visible on any phone, one capture thread keeps a long-lived `wlr-screencopy` capture (`copy_with_damage`) of the whole monitor under the pointer in a pool of three encoder surfaces exported as linear dmabufs, two in flight. It encodes when a new frame arrives, the pointer moves (re-cropping the latest frame on the GPU, no new capture) or a phone asks for a keyframe, at most 60 times a second. Each access unit is encoded once and broadcast to every phone; keyframe requests from all phones coalesce into one flag; a phone whose socket falls behind misses units and resyncs on a keyframe instead of slowing the others. A cursor sampler (60/s while the pointer moves, 20/s when still) keeps Hyprland IPC off the encode path. `crates/phonepad-h264` wraps FFmpeg's VA-API H.264 encoder and is the only crate allowed `unsafe` (see [ADR 0002](docs/adr/0002-h264-vaapi.md)); `PHONEPAD_RENDER_NODE` overrides the render node (default `/dev/dri/renderD128`).

The page (`pad.html`, `manifest.webmanifest`, icons) decodes the stream with WebCodecs `VideoDecoder` and is compiled into the binary, so changing it needs a rebuild.

## Security model

- phonepad binds `127.0.0.1` only. The phone reaches it through `tailscale serve`, which adds the caller's `Tailscale-User-Login` header.
- Every request whose `Tailscale-User-Login` differs from `PHONEPAD_LOGIN` gets 403. Tagged devices carry no user login, so other machines on the tailnet cannot inject input or view the screen.
- Anyone holding your unlocked phone with the app open can control and see the desktop.
- phonepad reads no input devices and needs no `input` group: it learns only that some input happened (ext-idle-notify), never which key or where.

## Requirements

- Hyprland (needs `zwlr_screencopy_manager_v1` v3 and `ext_idle_notifier_v1` v2), `hyprctl`, `gsettings`, `wtype`
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

- `cargo xtask views` regenerates `docs/views/`: the crate graph and the `stream` and `desk` state machines, produced by running core's `decide` and `evolve`. Never edit those files by hand.
- `cargo xtask check` runs the gates CI runs: `phonepad-core` has no dependencies, no comments in Rust or `pad.html`, and `docs/views/` matches the code.
- `target/debug/xtask profile <pid> <seconds>` writes a perf hot-path summary to `docs/views/hot-path.md`. Run it directly, not through `cargo`, if your cargo runner isolates PIDs. It needs a live Hyprland session, so CI never runs it.
- `cargo xtask e2e [--bin <phonepad>]` builds phonepad (or takes `--bin`), runs a test instance on port 8769 as `test@example.invalid` with its own runtime dir `$XDG_RUNTIME_DIR/ppe2e` and a stub `owe` first on `PATH`, and prints one PASS/FAIL line per check: the Hyprland and Wayland sockets connect through that dir, a wrong or missing login gets 403, one and then three viewers each receive units whose first is a keyframe and which ffmpeg decodes frame for frame, the instance survives the sessions, and SIGTERM sent while a fourth viewer is streaming ends it with code 0 within 5 s. A still desktop is enough: each viewer that joins requests a keyframe, which is encoded from the held frame without damage. The viewers send only `view`, `visible` and `ack`, never input, so the pointer and the desktop stay untouched. It needs a live Hyprland session, the iGPU, `/dev/uinput` and the ffmpeg CLI, so CI never runs it.
- CI (`.github/workflows/ci.yml`) runs fmt, clippy with `-D warnings`, the tests and `cargo xtask check`.

## Measurements

On an Intel UHD Graphics 630 (Hyprland renders there; AQ_DRM_DEVICES selects the iGPU), 2560×1440 at 60 Hz, a 648×1072 view, test clients on the same machine, a uinput mouse moving the pointer. CPU is % of one core, per thread from `/proc` over 5 s. A phone was streaming from a second phonepad instance on the same GPU during these runs.

| Clients | units/s per client | distinct frames/s | phonepad CPU | CPU per unit | Hyprland CPU |
|---|---|---|---|---|---|
| 1 | 23.0 | 23.0 | 1.4% | 0.61 ms | 11.6% |
| 2 | 23.4 | 23.4 | 1.4% | 0.60 ms | 11.2% |
| 4 | 23.2 | 23.2 | 1.4–1.8% | 0.60–0.78 ms | 11.4–12.0% |
| 8 | 23.6 | 23.6 | 1.8% | 0.76 ms | 12.0% |

- Capture and encode run once for all clients, so the cost does not grow with the number of phones. A client that stops acknowledging does not change the others' rate (23.0 units/s with one stalled client of two).
- Pointer still: phonepad 0.8%, Hyprland 4.0%.
- The rate is bounded by the encoder waiting 10–37 ms per unit for the shared GPU; each encode runs to completion before the next starts. `cargo xtask h264-check` measures the encoder alone: about 0.2 ms of CPU per unit, worst-frame PSNR about 39 dB.

## Limitations

- Real mouse or keyboard use that starts within 300 ms of phone input and continues without a 300 ms pause is taken for the phone's until it pauses; the pointer then shrinks on the next real input.
- `owe` has one manual-pause flag. A pause you set while phonepad already holds its own cannot be told apart from phonepad's, so it is resumed when the last viewer leaves; an `owe resume` while a phone is viewing is undone within 2 s.
- When the instance that owns the pause stops viewing while another instance still views, the wallpaper can animate for up to 2 s before the other takes the pause over.
