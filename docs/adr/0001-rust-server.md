# 0001: Replace the Python server and capture helper with one Rust binary

Date: 2026-11-09. Status: accepted.

## Weakest part at the time

`server.py` braided the decisions (which frames to send, when the pointer is large, which screen region to capture, how text maps to keys) with the effects that carry them out (uinput, Hyprland IPC, gsettings, WebSocket, JPEG). None of the decisions could be tested without a desktop session. The screen capture lived in a second process, `phonepad-capture`, only because Python had no Wayland client; every frame crossed a pipe as raw pixels, about 140 MB/s at 50 fps.

## Forces

- The operator asked for all desktop-side code in Rust ("all code should be in rust, replace python with rust server").
- Dependencies point inward and the core compiles and tests with no I/O in its tree (cm-hkg).
- Every lifecycle with more than two states is a sum type driven by `decide` and `evolve` (cm-prk).
- Boundaries need a force (cm-xrl). The capture process boundary's only force was Python's missing Wayland client.

## Move

Two crates. `phonepad-core` holds the decisions as pure functions with zero dependencies: per-session frame flow control (`stream`), viewer count and pointer size (`desk`), capture region geometry, and the US-layout typing plan. `phonepad` is the binary: axum serves the page and WebSocket, actors own the core state machines and execute their commands, a capture thread speaks wlr-screencopy in-process, `jpeg-encoder` encodes frames. Operator choices: tokio + axum over plain threads, `jpeg-encoder` over libjpeg-turbo bindings.

## Preserved

- The wire protocol and `pad.html`, byte for byte: the installed phone app keeps working without reinstalling.
- The auth model: loopback bind, `tailscale serve`, 403 unless `Tailscale-User-Login` matches.
- Measured behaviour: at least 30 frames/s to the phone counting repeated images (4 captures in flight, up to 4 unacknowledged frames; distinct images are bounded by how Hyprland serves captures, see the README), capture only while the app is visible, pointer large only during phone control.

## What would reverse it

A need the Rust stack serves worse than Python did, for example a desktop where only a Python input library works. Reverting to the last Python commit restores the previous system, since the wire protocol did not change.
