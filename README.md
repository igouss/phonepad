# phonepad

Use an Android phone as a touchpad and keyboard for a Hyprland desktop, with a live view of the screen around the pointer. It installs on the phone as an app from Chrome and reaches the desktop over Tailscale.

- One finger moves the pointer, tap clicks, two-finger tap right-clicks, two-finger drag scrolls, tap-then-drag drags.
- L/M/R buttons, sticky Ctrl/Alt/Shift/Super, Esc, Tab, arrows, Backspace, Enter.
- ⌨ opens the phone keyboard; typed text goes to the focused window.
- The touchpad shows the screen around the pointer (about 45 fps), with a ring at the pointer. The zoom button cycles 1.5× / 2.5× / 4×.
- The desktop pointer grows to size 64 while you control it from the phone and returns to normal when the real mouse moves.

KDE Connect does not work for this on Hyprland: its Wayland input needs the `RemoteDesktop` portal, which `xdg-desktop-portal-hyprland` does not implement.

## How it works

```
phone (installed web app)  ──wss──▶  tailscale serve :443  ──▶  server.py on 127.0.0.1:8765
                                                                   │
                                     uinput virtual mouse+keyboard ◀┤
                                     wtype (non-ASCII text)         ◀┤
                                     capture/ (Rust, screencopy)    ──▶ JPEG frames back to the phone
```

- `server.py` serves the page and a WebSocket. Input becomes events on a uinput device named `phonepad`. ASCII is typed through that device using the US layout; other characters go through `wtype`.
- `capture/` keeps four `wlr-screencopy` region captures in flight and streams raw frames to `server.py`, which encodes JPEG and sends the newest frame, with at most four unacknowledged frames per phone.
- The capture helper runs only while the app is visible on the phone, and is stopped when the app is backgrounded or closed.

## Security model

- `server.py` binds `127.0.0.1` only. The phone reaches it through `tailscale serve`, which adds the caller's `Tailscale-User-Login` header.
- Every request whose `Tailscale-User-Login` differs from `PHONEPAD_LOGIN` gets 403. Tagged devices carry no user login, so other machines on the tailnet cannot inject input or view the screen.
- Anyone holding your unlocked phone with the app open can control and see the desktop.

## Requirements

- Hyprland (needs `zwlr_screencopy_manager_v1` v3), `hyprctl`, `gsettings`
- Python 3.10+ with `python-evdev` and `Pillow`; `wtype`
- Rust toolchain
- Read/write access to `/dev/uinput` for your user
- Tailscale on the desktop and on the phone, desktop with `tailscale set --operator=$USER`
- Chrome on Android

## Install

```sh
git clone https://github.com/igouss/phonepad ~/.local/share/phonepad
cd ~/.local/share/phonepad/capture && cargo build --release

mkdir -p ~/.config/phonepad
echo "PHONEPAD_LOGIN=you@example.com" > ~/.config/phonepad/env

cp ~/.local/share/phonepad/phonepad.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now phonepad

tailscale serve --bg --https=443 http://127.0.0.1:8765
```

`PHONEPAD_LOGIN` is your Tailscale login (`tailscale whois <phone-ip>` shows it). On the phone, open `https://<desktop>.<tailnet>.ts.net/` in Chrome, then ⋮ → Install. For it to reconnect after a phone restart, enable Always-on VPN for Tailscale in Android's VPN settings.

`server.py` reads the page files at startup: run `systemctl --user restart phonepad` after editing them.

## Measurements

On an AMD Radeon HD 7970, 2560×1440 at 60 Hz, Pixel 7a over Tailscale on the same Wi-Fi:

| Captures in flight | Capture rate |
|---|---|
| 1 | 20 fps |
| 2 | 34 fps |
| 3 | 42 fps |
| 4 | 53 fps |

Each Hyprland screencopy takes about 50 ms regardless of region size, so throughput comes from pipelining. `grim` per frame gives about 8 fps. End to end the phone receives 43–47 fps; the image lags the screen by about 50 ms.

## Limitations

- Typing on the real keyboard alone does not shrink the pointer: real keyboard input is not visible without joining the `input` group, which would expose all keystrokes to every program.
- A Hyprland pointer warp more than 300 ms after the last phone input counts as real mouse use.
- ASCII typing assumes a US keyboard layout.
