# 0007: Transport spike: WebTransport against WebSocket over TCP under cellular loss

Date: 2026-11-10. Status: Proposed (spike r9c.6; the operator decides adoption). Branch `cell-6`.

## Weakest part at the time

Every byte between the phone and phonepad rides one TCP connection: units, acks, pointer messages and moves, through `tailscale serve`. A lost segment holds everything behind it until the retransmission lands, so one loss on cellular delays the picture, the pointer message and the moves together. ADR 0005 sized the window so a stall no longer costs a keyframe; the stall itself is left, and the harness's TCP emulator, replayed here (`ws-proxy` below), yields 43 gaps over 250 ms a minute at bad-lte.

## Forces

- H.264 deltas chain on their reference: the phone cannot decode unit N+1 before unit N, whatever the transport. Head-of-line blocking of the video is in the codec, not only in TCP; a transport removes it only between flows (video, pointer, moves, acks), or with redundancy that repairs a loss without a round trip.
- `tailscale serve` terminates only TCP (`--https`, `--http`, `--tcp`, `--tls-terminated-tcp`; `tailscale serve --help`, 1.104.2): it cannot carry HTTP/3 or any UDP. A UDP transport means phonepad listens on the tailnet address itself and authenticates the peer itself, instead of trusting the `Tailscale-User-Login` header that `tailscale serve` injects.
- Over a DERP relay, WireGuard packets travel over TCP (HTTPS) between the phone and the relay, so a UDP transport inside the tunnel gets back the head-of-line blocking it was chosen to avoid. It helps only on a direct path (r9c.7).
- The downlink TCP sender today is tailscaled's userspace netstack (it terminates `tailscale serve`'s TCP), not the Linux stack; how fast it recovers a loss was not measured here.
- Loss-based congestion control (Cubic, in Linux TCP and in quinn) reads random 1% loss as congestion and shrinks the window; the stream is app-limited (about 0.2 Mbit/s of deltas) so this bites only on keyframes.

## Experiment

`cargo xtask transport` (commit 13acebf, `docs/development.md`): one process replays the same synthetic unit stream through `phonepad-core`'s flow control (ADR 0005) to a viewer that releases a unit only when it and every unit sent before it have arrived (the decode order), acks it then, and sends a 12-byte ping every 20 ms standing in for moves and pointer messages. Send→draw is send to in-order release on one clock. A Rust client, not the page: the page's software decoder adds 5–90 ms (ADR 0005), which would hide the transport. Bad-lte everywhere: 150 ms ± 50 ms, 3 Mbit/s, 1% loss; 60 s measured after 3 s, each run started at loadavg < 3, 2 rounds.

Four paths, because the two userspace emulators do not model loss the same way:

- `ws-proxy`: WebSocket through the latency harness's TCP emulator, which never drops a segment but delays it and everything behind it by max(200 ms, 2 × RTT) = 300 ms.
- `wt-relay`: WebTransport (wtransport 0.7.2 on quinn 0.11; one unidirectional stream per unit, acks and pings on their own bidirectional streams) through the new UDP emulator, which really drops datagrams and lets QUIC recover.
- `ws-direct`, `wt-direct`: the same clients with no userspace emulator, inside `unshare -rn` with kernel netem on `lo` (`delay 75ms 25ms rate 3mbit loss 1%`, MTU 1280, GSO off): real Linux TCP and real quinn against the same real loss.

Known cases: `transport calibrate` passes (probe RTT +100.1 / +200.1 ms for 100 / 200 ms set; 1.14% of 20 000 datagrams lost at 1% set, within 3σ = 0.21%; 2.000 Mbit/s through a 2 Mbit/s cap with queue overflows; at 150 ± 50 ms every RTT within [100, 205]) and fails on planted mutants (half the delay: +50 / +100 ms; no loss: 0.000%). netem with the same knobs measured 0.86% one-way loss and probe RTT p50 181 ms against the UDP emulator's 169 ms (the UDP emulator keeps order by construction, which skews its median above 150; netem's ordering with `rate` set was not verified). The netem TCP arm met per-segment loss: in a 15 s `ws-direct` run netem sent 4994 packets and dropped 49 (0.98%), against 5031 TCP segments sent and 35 retransmitted, so TCP passed one segment per packet (no aggregated skbs). `ws-proxy` reproduces ADR 0005's acking viewer at bad-lte: 50 units/s, longest gap 348–357 ms (ADR 0005: 344–375), send→ack p95 456–470 ms (439–449).

## Evidence

Deltas 200–600 bytes (a still screen with a moving pointer):

| arm | loss model | stalls > 250 / > 500 ms per 60 s | longest gap ms | send→draw p95 ms | send→ack p95 ms | ping RTT p95 ms | units/s |
|---|---|---|---|---|---|---|---|
| ws-proxy | modelled 300 ms stall | 43 / 0, 43 / 0 | 357, 348 | 323, 324 | 456, 470 | 456, 456 | 50.1, 50.1 |
| wt-relay, Cubic | UDP emulator drops | 6 / 0, 5 / 0 | 319, 303 | 199, 217 | 320, 330 | 336, 321 | 49.9, 50.0 |
| wt-relay, BBR | UDP emulator drops | 1 / 0, 1 / 0 | 342, 256 | 172, 177 | 304, 309 | 302, 314 | 50.0, 50.0 |
| ws-direct (Linux TCP) | netem drops | 3 / 0, 1 / 0 | 337, 251 | 248, 242 | 367, 370 | 374, 367 | 50.0, 50.0 |
| wt-direct, Cubic | netem drops | 11 / 1, 10 / 0 | 505, 302 | 230, 230 | 353, 344 | 346, 346 | 49.9, 50.0 |
| wt-direct, BBR | netem drops | 0 / 0, 0 / 0 | 246, 229 | 212, 135 | 341, 304 | 337, 321 | 50.0, 50.0 |

Deltas 1000–4000 bytes (busy content), one round:

| arm | stalls > 250 / > 500 | units/s | send→draw p95 ms | keyframes (desyncs) |
|---|---|---|---|---|
| ws-proxy | 118 / 32 | 21.8 | 406 | 101 (100) |
| wt-relay, Cubic | 69 / 58 | 11.7 | 870 | 73 (73) |
| wt-relay, BBR | 116 / 27 | 20.5 | 325 | 125 (125) |
| ws-direct | 113 / 29 | 20.3 | 351 | 122 (122) |
| wt-direct, Cubic | 64 / 47 | 10.4 | 1135 | 65 (64) |
| wt-direct, BBR | 93 / 17 | 25.7 | 330 | 104 (103) |

Readings:

- Against the harness's TCP model WebTransport looks 7–40 times better (43 stalls a minute against 1–6). Against real Linux TCP under the same real loss it is not: WebSocket had 1–3 stalls a minute, WebTransport with Cubic 10–11, with BBR 0. The model's fixed 300 ms stall is what made TCP look bad; Linux TCP repairs a loss in about one round trip (RACK and fast retransmit, with 50 packets a second behind the loss to trigger them).
- The best alternative, WebTransport with BBR, gains over real TCP: 1–3 stalls a minute to 0, send→draw p95 242–248 to 135–212 ms, ping p95 367–374 to 321–337 ms. With quinn's default Cubic it loses on stalls.
- With busy content both transports collapse the same way: units desync about twice a second and every arm falls to 10–26 units/s. The likely cause, not verified here, is that the units held behind one repaired loss (about 2.5 KB each) plus a 40 KB keyframe exceed ADR 0005's 64 KiB floor while the rate estimate is still low. Either way it is flow control, not transport, and it costs far more than the transport difference.
- `serve-wt`: headless Chromium 155 opened a WebTransport session to wtransport's self-signed certificate (ECDSA P-256, 14 days, the `serverCertificateHashes` requirements) and acked 477 units in order through a page. The Pixel 7a's Chrome was not tried.

## Costs of adopting WebTransport

- Auth: `tailscale serve` cannot carry it, so phonepad would bind a UDP port on its tailnet address (100.x, today it binds only 127.0.0.1) and decide itself who may connect. Two ways, neither built: tailscaled's LocalAPI whois on the QUIC peer's address (`tailscale whois --json <ip:port>`, which also returns the tags, so tagged devices must be refused by tag; this desktop is itself tagged `tag:workstation`), or a short-lived token minted by the page that `tailscale serve` already authenticates by header and presented in the WebTransport CONNECT. The token keeps the trusted boundary where it is today; whois adds a dependency on the LocalAPI socket. ACLs then decide which nodes reach the port at all.
- Certificates: `tailscale serve` renews the ts.net certificate today. WebTransport needs either `tailscale cert <host>.ts.net` renewed by phonepad (90 days, publicly trusted, no hashes) or a self-signed ECDSA certificate of at most 14 days whose hash the authenticated page hands to `serverCertificateHashes` (regenerated at each start; a page older than 14 days must reload).
- Path: over DERP the tunnel is TCP, so WebTransport gains nothing there (r9c.7). The page needs a WebSocket fallback for that path and for networks that block UDP, so both transports stay.
- Code: a transport seam in `session.rs` (today it owns an axum `WebSocket`), unit streams with in-order reassembly on the page, acks and pointer messages on their own streams, a UDP listener with its own auth, roughly 300–500 lines of server and 100 of page against the spike's 1 400 lines of both ends.
- Dependencies: wtransport pulls quinn, rustls, ring and their trees: 105 lockfile entries in the spike (some platform-only; production would drop rcgen and x509-parser).
- Congestion control must be chosen: quinn's default Cubic was worse than Linux TCP here; BBR in quinn is marked experimental.

## Not prototyped

- Unreliable datagrams per unit: a unit is 200 bytes to 40 KB, a datagram at most about 1200, so units would need fragmentation, and one lost fragment loses the unit and, with it, the reference chain until a keyframe: at 1% loss about one 40 KB keyframe every 2 s. It only pays with forward error correction or intra refresh, which is codec work, not transport.
- WebRTC: a data channel (SCTP over DTLS, unordered/unreliable modes) has the same reference-chain problem as datagrams; a video track hands the stream to the browser's RTP stack (NACK, keyframe requests, loss-tolerant delay-based congestion control) but out of the page's WebCodecs decoder that ADR 0003 and 0006 draw from. Its auth is the cheapest of the options: the SDP and DTLS fingerprint travel over the WebSocket that `tailscale serve` already authenticates. The stack (str0m is sans-IO; webrtc-rs is heavier) is a larger build than a one-session spike.

## Recommendation (for the operator)

Do not adopt WebTransport now. Against real TCP under the same loss it removes 1–3 stalls a minute and 30–110 ms of send→draw p95, only with BBR, only on a direct path, at the cost of a second listener outside `tailscale serve`, its own auth and certificates, and a WebSocket fallback kept alongside. Two cheaper moves come first: make the latency harness's TCP emulator stop overstating loss (it counts 43 stalls a minute where Linux TCP shows 1–3, or run the harness under netem), and fix the flow-control collapse with busy content, which costs 20 units/s and about 100 desyncs a minute on either transport.

## What would reverse it

The real downlink TCP sender is tailscaled's userspace netstack, not Linux. If it recovers losses like the harness's model rather than like Linux (measurable from the phone on a direct cellular path, or by putting netem under a `tailscale serve` path on a test tailnet node: more than about 10 stalls over 250 ms a minute at 1% loss), WebTransport with BBR (0 a minute here) wins by the margin the model shows, and the costs above are worth paying for a direct path. Also reverse if r9c.7 finds the phone's path is direct on cellular and loss there is well above 1%, where loss-based TCP and quinn's Cubic degrade and BBR does not.

## Flagged assumptions

Chosen by the implementing agent, not the operator: the synthetic stream (a unit every 20 ms, deltas uniform 200–600 bytes, 40 KB keyframes at least 500 ms apart: small deltas are what the live desktop with a moving hardware pointer produces, and they reproduce ADR 0005's acking-viewer shape); one unidirectional stream per unit with older units at higher priority; acks and pings each on their own stream; QUIC payloads of 1200 bytes with MTU discovery off (the tailnet MTU is 1280); netem's jitter and rate (rate shared by both directions on `lo`) in place of the userspace emulators for the kernel-loss arms; quinn's BBR as the alternative to its default Cubic; ping every 20 ms as the stand-in for moves and pointer messages.

## Operator decision

None: every policy question goes back to the operator. Open: adopt WebTransport or not; if adopted, whois or page-minted token for auth, and `tailscale cert` or 14-day self-signed hashes.
