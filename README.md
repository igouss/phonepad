# phonepad

Use an Android phone as a touchpad and keyboard for a Hyprland desktop, with a live view of the screen around the pointer. It installs on the phone as an app from Chrome and reaches the desktop over Tailscale.

- One finger moves the pointer, tap clicks, two-finger tap right-clicks, three-finger tap middle-clicks, two-finger drag scrolls, tap-then-drag drags.
- L/M/R buttons, sticky Ctrl/Alt/Shift/Super, Esc, Tab, arrows, Backspace, Enter.
- ⌨ opens the phone keyboard; typed text goes to the focused window.
- The touchpad shows the screen around the pointer, with a ring at the pointer. The zoom button cycles 1.5× / 2.5× / 4×.
- The desktop pointer grows to size 64 while you control it from the phone and returns to normal when the real mouse moves.

KDE Connect does not work for this on Hyprland: its Wayland input needs the `RemoteDesktop` portal, which `xdg-desktop-portal-hyprland` does not implement.

## How it works

```
phone (installed web app)  ──wss──▶  tailscale serve :443  ──▶  phonepad on 127.0.0.1:8765
                                                                  │
                                    uinput virtual mouse+keyboard ◀┤
                                    wtype (non-ASCII text)         ◀┤
                                    wlr-screencopy capture thread  ──▶ JPEG frames back to the phone
```

Two crates (see [ADR 0001](docs/adr/0001-rust-server.md)):

- `crates/phonepad-core`: the decisions, as pure functions with no dependencies. `stream` is the per-phone frame flow control (up to 4 unacknowledged frames), `desk` the viewer count and pointer size, `geometry` the capture region, `keys` the US-layout typing plan. The state machines are sum types driven by `decide` and `evolve`.
- `crates/phonepad`: the binary. axum serves the page and the WebSocket; one actor per phone session and one desk actor own the core state machines and execute their commands; one thread owns the uinput device; a capture thread keeps four `wlr-screencopy` region captures in flight and runs only while the app is visible on the phone; a cursor sampler keeps Hyprland IPC off the send path; `jpeg-encoder` encodes frames.

The page (`pad.html`, `manifest.webmanifest`, icons) is compiled into the binary, so changing it needs a rebuild.

## Security model

- phonepad binds `127.0.0.1` only. The phone reaches it through `tailscale serve`, which adds the caller's `Tailscale-User-Login` header.
- Every request whose `Tailscale-User-Login` differs from `PHONEPAD_LOGIN` gets 403. Tagged devices carry no user login, so other machines on the tailnet cannot inject input or view the screen.
- Anyone holding your unlocked phone with the app open can control and see the desktop.

## Requirements

- Hyprland (needs `zwlr_screencopy_manager_v1` v3), `hyprctl`, `gsettings`, `wtype`
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
- CI (`.github/workflows/ci.yml`) runs fmt, clippy with `-D warnings`, the tests and `cargo xtask check`.

## Measurements

On an AMD Radeon HD 7970, 2560×1440 at 60 Hz, with a test client on the same machine:

| Captures in flight | Capture rate |
|---|---|
| 1 | 20 fps |
| 2 | 34 fps |
| 3 | 42 fps |
| 4 | 53 fps |

Each Hyprland screencopy takes about 50 ms regardless of region size, so throughput comes from pipelining.

Frames sent are not all distinct images. Hyprland serves several pending captures from the same render, so many frames repeat the previous image; each still carries the current pointer position for the ring. With text changing continuously under the pointer, phonepad measured 42–89 frames/s, of which 11–21/s were distinct images, across runs. On a static screen it sends 46–95 frames/s with almost no distinct images. The Python version this replaced measured 52–59 frames/s on a static screen and about 14 distinct/s with changing content.

## Limitations

- Typing on the real keyboard alone does not shrink the pointer: real keyboard input is not visible without joining the `input` group, which would expose all keystrokes to every program.
- A Hyprland pointer warp more than 300 ms after the last phone input counts as real mouse use.
- ASCII typing assumes a US keyboard layout.
