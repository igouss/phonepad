import base64
import hashlib
import http.server
import io
import enum
import json
import os
import signal
import socket
import struct
import subprocess
import sys
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import IO, Callable

import evdev
from PIL import Image
from evdev import ecodes as e

HOST: str = "127.0.0.1"
PORT: int = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
ALLOWED_LOGIN: str = sys.argv[2]
ROOT: Path = Path(__file__).parent
STATIC: dict[str, tuple[str, bytes]] = {
    "/": ("text/html; charset=utf-8", (ROOT / "pad.html").read_bytes()),
    "/manifest.webmanifest": ("application/manifest+json", (ROOT / "manifest.webmanifest").read_bytes()),
    "/icon-192.png": ("image/png", (ROOT / "icon-192.png").read_bytes()),
    "/icon-512.png": ("image/png", (ROOT / "icon-512.png").read_bytes()),
}
KEY_GAP_S: float = 0.004
CAPTURE_BIN: Path = Path(__file__).parent / "target" / "release" / "phonepad-capture"
FRAME_WAIT_S: float = 0.05
MAX_IN_FLIGHT: int = 4
LOCAL_MOUSE_POLL_S: float = 0.1
PHONE_QUIET_S: float = 0.3
JPEG_QUALITY: int = 60
BIG_POINTER: int = 64
NORMAL_POINTER: int = int(os.environ.get("XCURSOR_SIZE", "24"))
WS_GUID: bytes = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

BUTTONS: dict[str, int] = {"left": e.BTN_LEFT, "right": e.BTN_RIGHT, "middle": e.BTN_MIDDLE}
KEYS: list[int] = [code for name, code in e.ecodes.items() if name.startswith("KEY_") and 0 < code < 0x2FF]

device: evdev.UInput = evdev.UInput(
    {
        e.EV_REL: [e.REL_X, e.REL_Y, e.REL_WHEEL, e.REL_HWHEEL, e.REL_WHEEL_HI_RES, e.REL_HWHEEL_HI_RES],
        e.EV_KEY: sorted(set(KEYS + list(BUTTONS.values()))),
    },
    name="phonepad",
)


US_UNSHIFTED: str = "`1234567890-=[]\\;',./"
US_SHIFTED: str = '~!@#$%^&*()_+{}|:"<>?'
US_NAMES: list[str] = [
    "GRAVE", "1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "MINUS", "EQUAL",
    "LEFTBRACE", "RIGHTBRACE", "BACKSLASH", "SEMICOLON", "APOSTROPHE", "COMMA", "DOT", "SLASH",
]
US_KEYS: dict[str, tuple[int, bool]] = {
    **{c: (e.ecodes["KEY_" + c.upper()], False) for c in "abcdefghijklmnopqrstuvwxyz"},
    **{c: (e.ecodes["KEY_" + c], True) for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ"},
    **{c: (e.ecodes["KEY_" + n], False) for c, n in zip(US_UNSHIFTED, US_NAMES)},
    **{c: (e.ecodes["KEY_" + n], True) for c, n in zip(US_SHIFTED, ["GRAVE", *US_NAMES[1:]])},
    " ": (e.KEY_SPACE, False),
    "\t": (e.KEY_TAB, False),
    "\n": (e.KEY_ENTER, False),
}


def tap(code: int, shift: bool) -> None:
    if shift:
        device.write(e.EV_KEY, e.KEY_LEFTSHIFT, 1)
    device.write(e.EV_KEY, code, 1)
    device.syn()
    device.write(e.EV_KEY, code, 0)
    if shift:
        device.write(e.EV_KEY, e.KEY_LEFTSHIFT, 0)
    device.syn()
    time.sleep(KEY_GAP_S)


def type_text(text: str) -> None:
    pending: str = ""
    for ch in text:
        if ch in US_KEYS:
            if pending:
                subprocess.run(["wtype", "--", pending], check=False)
                pending = ""
            tap(*US_KEYS[ch])
        else:
            pending += ch
    if pending:
        subprocess.run(["wtype", "--", pending], check=False)


def key_code(name: str) -> int:
    code: int | list[int] = e.ecodes[name]
    if not name.startswith("KEY_") or not isinstance(code, int):
        raise ValueError(name)
    return code


def handle(msg: dict) -> None:
    kind: str = msg["t"]
    if kind == "move":
        device.write(e.EV_REL, e.REL_X, int(msg["dx"]))
        device.write(e.EV_REL, e.REL_Y, int(msg["dy"]))
    elif kind == "scroll":
        dx: int = int(msg["dx"])
        dy: int = int(msg["dy"])
        if dy:
            device.write(e.EV_REL, e.REL_WHEEL_HI_RES, -dy)
        if dx:
            device.write(e.EV_REL, e.REL_HWHEEL_HI_RES, dx)
    elif kind == "button":
        device.write(e.EV_KEY, BUTTONS[msg["b"]], 1 if msg["down"] else 0)
    elif kind == "keys":
        codes: list[int] = [key_code(n) for n in msg["k"]]
        for c in codes:
            device.write(e.EV_KEY, c, 1)
            device.syn()
        for c in reversed(codes):
            device.write(e.EV_KEY, c, 0)
            device.syn()
        return
    elif kind == "text":
        type_text(msg["s"])
        return
    device.syn()


def run(*args: str) -> str:
    return subprocess.run(list(args), capture_output=True, text=True, check=False).stdout


def hypr(command: str) -> object:
    path: Path = Path(os.environ["XDG_RUNTIME_DIR"]) / "hypr" / os.environ["HYPRLAND_INSTANCE_SIGNATURE"] / ".socket.sock"
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.connect(str(path))
        sock.sendall(f"j/{command}".encode())
        chunks: list[bytes] = []
        while chunk := sock.recv(65536):
            chunks.append(chunk)
    return json.loads(b"".join(chunks))


def cursor_pos() -> tuple[int, int]:
    pos: dict[str, int] = hypr("cursorpos")
    return pos["x"], pos["y"]


def pointer_theme() -> str:
    theme: str = run("gsettings", "get", "org.gnome.desktop.interface", "cursor-theme").strip().strip("'")
    index: Path = Path.home() / ".icons" / theme / "index.theme"
    if index.exists():
        for line in index.read_text().splitlines():
            if line.startswith("Inherits="):
                return line.split("=", 1)[1].split(",")[0].strip()
    return theme


class PointerSize(enum.Enum):
    NORMAL = enum.auto()
    BIG = enum.auto()


class Pointer:
    def __init__(self, theme: str) -> None:
        self.theme: str = theme
        self.size: PointerSize = PointerSize.NORMAL
        self.last_phone_input: float = 0.0
        self.lock: threading.Lock = threading.Lock()
        self.apply(NORMAL_POINTER)

    def apply(self, size: int) -> None:
        run("gsettings", "set", "org.gnome.desktop.interface", "cursor-size", str(size))
        run("hyprctl", "setcursor", self.theme, str(size))

    def phone_input(self) -> None:
        with self.lock:
            self.last_phone_input = time.monotonic()
            if self.size is PointerSize.BIG:
                return
            self.size = PointerSize.BIG
        self.apply(BIG_POINTER)
        threading.Thread(target=self.watch_local_mouse, daemon=True).start()

    def reset(self) -> None:
        with self.lock:
            if self.size is PointerSize.NORMAL:
                return
            self.size = PointerSize.NORMAL
        self.apply(NORMAL_POINTER)

    def watch_local_mouse(self) -> None:
        last: tuple[int, int] = cursor_pos()
        while True:
            time.sleep(LOCAL_MOUSE_POLL_S)
            with self.lock:
                if self.size is PointerSize.NORMAL:
                    return
                quiet: bool = time.monotonic() - self.last_phone_input > PHONE_QUIET_S
            now: tuple[int, int] = cursor_pos()
            if now != last and quiet:
                self.reset()
                return
            last = now


@dataclass(frozen=True)
class Monitor:
    name: str
    x: int
    y: int
    w: int
    h: int


def monitors() -> list[Monitor]:
    found: list[Monitor] = []
    for m in hypr("monitors"):
        w: int = round(m["width"] / m["scale"])
        h: int = round(m["height"] / m["scale"])
        if m["transform"] % 2:
            w, h = h, w
        found.append(Monitor(m["name"], m["x"], m["y"], w, h))
    return found


def region_line(cx: int, cy: int, w: int, h: int, outputs: list[Monitor]) -> str:
    mon: Monitor = next(
        (m for m in outputs if m.x <= cx < m.x + m.w and m.y <= cy < m.y + m.h),
        outputs[0],
    )
    w = min(w, mon.w)
    h = min(h, mon.h)
    x: int = min(max(cx - w // 2, mon.x), mon.x + mon.w - w)
    y: int = min(max(cy - h // 2, mon.y), mon.y + mon.h - h)
    return f"{mon.name} {x} {y} {x - mon.x} {y - mon.y} {w} {h}\n"


RAW_MODES: dict[int, str] = {0: "BGRX", 1: "BGRX", 0x34324241: "RGBX", 0x34324258: "RGBX"}


@dataclass(frozen=True)
class RawFrame:
    seq: int
    x: int
    y: int
    width: int
    height: int
    stride: int
    raw_mode: str
    y_invert: bool
    pixels: bytes

    def encode(self) -> bytes:
        image: Image.Image = Image.frombuffer(
            "RGB", (self.width, self.height), self.pixels, "raw", self.raw_mode, self.stride, -1 if self.y_invert else 1
        )
        out: io.BytesIO = io.BytesIO()
        image.save(out, "JPEG", quality=JPEG_QUALITY)
        cx, cy = cursor_pos()
        return struct.pack("!iiii", self.x, self.y, cx, cy) + out.getvalue()


class Capturer:
    def __init__(self) -> None:
        self.cond: threading.Condition = threading.Condition()
        self.process: subprocess.Popen[bytes] | None = None
        self.latest: RawFrame | None = None
        self.view: tuple[int, int] = (0, 0)

    def start(self) -> None:
        with self.cond:
            if self.process is not None:
                return
            self.process = subprocess.Popen([str(CAPTURE_BIN)], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
            self.latest = None
            process: subprocess.Popen[bytes] = self.process
        threading.Thread(target=self.read_frames, args=(process,), daemon=True).start()

    def stop(self) -> None:
        with self.cond:
            process: subprocess.Popen[bytes] | None = self.process
            self.process = None
            self.latest = None
            self.cond.notify_all()
        if process is not None:
            process.terminate()
            process.wait()

    def set_view(self, w: int, h: int) -> None:
        with self.cond:
            self.view = (w, h)

    def wanted_region(self) -> str | None:
        with self.cond:
            w, h = self.view
        if min(w, h) <= 0:
            return None
        cx, cy = cursor_pos()
        return region_line(cx, cy, w, h, monitors())

    def read_frames(self, process: subprocess.Popen[bytes]) -> None:
        stdin: IO[bytes] = process.stdin
        stdout: IO[bytes] = process.stdout
        sent: str | None = None
        seq: int = 0
        try:
            while process.poll() is None:
                region: str | None = self.wanted_region()
                if region is None:
                    time.sleep(FRAME_WAIT_S)
                    continue
                if region != sent:
                    stdin.write(region.encode())
                    stdin.flush()
                    sent = region
                header: bytes = stdout.read(28)
                if len(header) < 28:
                    break
                x, y, width, height, stride, fmt, y_invert = struct.unpack("<iiIIIII", header)
                pixels: bytes = stdout.read(stride * height)
                seq += 1
                frame: RawFrame = RawFrame(seq, x, y, width, height, stride, RAW_MODES[fmt], bool(y_invert), pixels)
                with self.cond:
                    if self.process is not process:
                        return
                    self.latest = frame
                    self.cond.notify_all()
        except (OSError, ValueError, KeyError) as err:
            print(f"capture reader stopped: {err!r}", file=sys.stderr)
        finally:
            if process.poll() is not None and process.returncode not in (0, -signal.SIGTERM):
                print(f"phonepad-capture exited {process.returncode}", file=sys.stderr)

    def next_frame(self, after: int, alive: "Callable[[], bool]") -> RawFrame | None:
        with self.cond:
            self.cond.wait_for(lambda: not alive() or (self.latest is not None and self.latest.seq > after))
            return self.latest if alive() else None


class Presence:
    def __init__(self) -> None:
        self.visible: int = 0
        self.lock: threading.Lock = threading.Lock()

    def shown(self) -> None:
        with self.lock:
            self.visible += 1
            first: bool = self.visible == 1
        if first:
            capturer.start()

    def hidden(self) -> None:
        with self.lock:
            self.visible -= 1
            last: bool = self.visible == 0
        if last:
            capturer.stop()
            pointer.reset()


@dataclass(frozen=True)
class Hidden:
    pass


@dataclass(frozen=True)
class Streaming:
    in_flight: int


Stream = Hidden | Streaming


class StreamEvent(enum.Enum):
    SHOWN = enum.auto()
    HIDDEN = enum.auto()
    SENT = enum.auto()
    ACKED = enum.auto()


def next_stream(state: Stream, event: StreamEvent) -> Stream:
    match state, event:
        case _, StreamEvent.HIDDEN:
            return Hidden()
        case Hidden(), StreamEvent.SHOWN:
            return Streaming(0)
        case Hidden(), StreamEvent.SENT | StreamEvent.ACKED:
            return state
        case Streaming(), StreamEvent.SHOWN:
            return state
        case Streaming(n), StreamEvent.SENT:
            return Streaming(n + 1)
        case Streaming(n), StreamEvent.ACKED:
            return Streaming(max(0, n - 1))


class Session:
    def __init__(self, sock: socket.socket) -> None:
        self.sock: socket.socket = sock
        self.send_lock: threading.Lock = threading.Lock()
        self.cond: threading.Condition = threading.Condition()
        self.stream: Stream = Hidden()
        self.closed: bool = False

    def send(self, opcode: int, payload: bytes) -> None:
        with self.send_lock:
            send_frame(self.sock, opcode, payload)

    def on(self, event: StreamEvent) -> None:
        with self.cond:
            was_visible: bool = isinstance(self.stream, Streaming)
            self.stream = next_stream(self.stream, event)
            now_visible: bool = isinstance(self.stream, Streaming)
            self.cond.notify_all()
        if now_visible and not was_visible:
            presence.shown()
        elif was_visible and not now_visible:
            presence.hidden()
        if event is StreamEvent.ACKED:
            with capturer.cond:
                capturer.cond.notify_all()

    def close(self) -> None:
        self.on(StreamEvent.HIDDEN)
        with self.cond:
            self.closed = True
        with capturer.cond:
            capturer.cond.notify_all()

    def can_send(self) -> bool:
        with self.cond:
            return not self.closed and isinstance(self.stream, Streaming) and self.stream.in_flight < MAX_IN_FLIGHT

    def alive(self) -> bool:
        return not self.closed

    def sender(self) -> None:
        last_seq: int = 0
        while not self.closed:
            frame: RawFrame | None = capturer.next_frame(last_seq, self.alive)
            if frame is None:
                return
            with capturer.cond:
                capturer.cond.wait_for(lambda: self.closed or self.can_send())
            if self.closed:
                return
            frame = capturer.latest or frame
            if frame.seq <= last_seq:
                continue
            last_seq = frame.seq
            try:
                payload: bytes = frame.encode()
                self.on(StreamEvent.SENT)
                self.send(2, payload)
            except OSError:
                return

    def handle(self, msg: dict) -> None:
        match msg["t"]:
            case "visible":
                self.on(StreamEvent.SHOWN if msg["v"] else StreamEvent.HIDDEN)
            case "view":
                capturer.set_view(int(msg["w"]), int(msg["h"]))
            case "ack":
                self.on(StreamEvent.ACKED)
            case _:
                pointer.phone_input()
                handle(msg)


def recv_exact(sock: socket.socket, n: int) -> bytes:
    buf: bytearray = bytearray()
    while len(buf) < n:
        chunk: bytes = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError
        buf += chunk
    return bytes(buf)


def send_frame(sock: socket.socket, opcode: int, payload: bytes) -> None:
    n: int = len(payload)
    if n < 126:
        header: bytes = struct.pack("!BB", 0x80 | opcode, n)
    elif n < 65536:
        header = struct.pack("!BBH", 0x80 | opcode, 126, n)
    else:
        header = struct.pack("!BBQ", 0x80 | opcode, 127, n)
    sock.sendall(header + payload)


def websocket_loop(session: Session) -> None:
    sock: socket.socket = session.sock
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    while True:
        b0, b1 = recv_exact(sock, 2)
        opcode: int = b0 & 0x0F
        n: int = b1 & 0x7F
        if n == 126:
            (n,) = struct.unpack("!H", recv_exact(sock, 2))
        elif n == 127:
            (n,) = struct.unpack("!Q", recv_exact(sock, 8))
        mask: bytes = recv_exact(sock, 4) if b1 & 0x80 else b"\0\0\0\0"
        payload: bytes = bytes(c ^ mask[i % 4] for i, c in enumerate(recv_exact(sock, n)))
        if opcode == 8:
            session.send(8, b"")
            return
        if opcode == 9:
            session.send(10, payload)
        elif opcode == 1:
            try:
                session.handle(json.loads(payload))
            except (KeyError, ValueError, TypeError) as err:
                print(f"bad message {payload!r}: {err!r}", file=sys.stderr)


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        if self.headers.get("Tailscale-User-Login") != ALLOWED_LOGIN:
            self.send_error(403)
            return
        if self.headers.get("Upgrade", "").lower() == "websocket":
            accept: str = base64.b64encode(
                hashlib.sha1(self.headers["Sec-WebSocket-Key"].encode() + WS_GUID).digest()
            ).decode()
            self.send_response(101)
            self.send_header("Upgrade", "websocket")
            self.send_header("Connection", "Upgrade")
            self.send_header("Sec-WebSocket-Accept", accept)
            self.end_headers()
            self.close_connection = True
            session: Session = Session(self.connection)
            sender: threading.Thread = threading.Thread(target=session.sender, daemon=True)
            sender.start()
            try:
                websocket_loop(session)
            except (ConnectionError, OSError):
                pass
            session.close()
            sender.join()
            for code in BUTTONS.values():
                device.write(e.EV_KEY, code, 0)
            device.syn()
            return
        path: str = self.path.split("?", 1)[0]
        if path not in STATIC:
            self.send_error(404)
            return
        content_type, body = STATIC[path]
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: object) -> None:
        pass


pointer: Pointer = Pointer(pointer_theme())
capturer: Capturer = Capturer()
presence: Presence = Presence()


def shutdown(signum: int, frame: object) -> None:
    capturer.stop()
    pointer.apply(NORMAL_POINTER)
    sys.exit(0)


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, shutdown)
    server: http.server.ThreadingHTTPServer = http.server.ThreadingHTTPServer((HOST, PORT), Handler)
    print(f"phonepad listening on http://{HOST}:{PORT}", flush=True)
    server.serve_forever()
