"""A minimum-viable Chrome DevTools Protocol client, in the standard library.

Why this file exists, since it is the last thing anyone wants in a docs folder:
Chrome's command-line PDF export does not wait for JavaScript. Paged.js turns
this book into numbered pages asynchronously, and every CLI flag that sounds
like it would help - --virtual-time-budget, --timeout, --run-all-compositor-
stages-before-draw - dumps the page while Paged.js is still two pages in. The
result is a PDF that is silently three pages long and looks perfectly valid.

Waiting properly needs the DevTools protocol, which speaks WebSocket, which the
standard library does not. So the ~80 lines below are a WebSocket client with
exactly the features one CDP session needs: a handshake, masked text frames out,
fragmented frames in, and ping/pong. No pip install, no node_modules, no
vendored browser - the build runs on a clean checkout with the Chrome that is
already on the machine.
"""

from __future__ import annotations

import base64
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.request


class WebSocket:
    """Client-side WebSocket, text frames only (RFC 6455, the useful third)."""

    def __init__(self, url: str, timeout: float = 300.0):
        if not url.startswith("ws://"):
            raise ValueError(f"expected a ws:// url, got {url}")
        hostport, _, path = url[len("ws://"):].partition("/")
        host, _, port = hostport.partition(":")

        self.sock = socket.create_connection((host, int(port or 80)), timeout=30)
        self.sock.settimeout(timeout)
        self.buf = b""

        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall(
            f"GET /{path} HTTP/1.1\r\n"
            f"Host: {hostport}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n".encode()
        )
        while b"\r\n\r\n" not in self.buf:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise ConnectionError("Chrome closed the connection during the handshake")
            self.buf += chunk
        head, _, self.buf = self.buf.partition(b"\r\n\r\n")
        if b" 101 " not in head.split(b"\r\n")[0]:
            raise ConnectionError(head.decode("utf-8", "replace"))

    def _read(self, n: int) -> bytes:
        while len(self.buf) < n:
            chunk = self.sock.recv(1 << 20)
            if not chunk:
                raise ConnectionError("Chrome closed the connection")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def _frame(self, payload: bytes, opcode: int) -> bytes:
        header = bytearray([0x80 | opcode])
        n = len(payload)
        if n < 126:
            header.append(0x80 | n)
        elif n < 1 << 16:
            header.append(0x80 | 126)
            header += struct.pack(">H", n)
        else:
            header.append(0x80 | 127)
            header += struct.pack(">Q", n)
        mask = os.urandom(4)
        header += mask
        return bytes(header) + bytes(b ^ mask[i % 4] for i, b in enumerate(payload))

    def send(self, text: str) -> None:
        self.sock.sendall(self._frame(text.encode("utf-8"), 0x1))

    def recv(self) -> str:
        parts = []
        while True:
            b0, b1 = self._read(2)
            fin, opcode, masked, n = b0 & 0x80, b0 & 0x0F, b1 & 0x80, b1 & 0x7F
            if n == 126:
                n = struct.unpack(">H", self._read(2))[0]
            elif n == 127:
                n = struct.unpack(">Q", self._read(8))[0]
            key = self._read(4) if masked else b""
            payload = self._read(n) if n else b""
            if key:
                payload = bytes(b ^ key[i % 4] for i, b in enumerate(payload))

            if opcode == 0x9:                                    # ping
                self.sock.sendall(self._frame(payload, 0xA))
                continue
            if opcode == 0xA:                                    # pong
                continue
            if opcode == 0x8:                                    # close
                raise ConnectionError("Chrome closed the WebSocket")

            parts.append(payload)
            if fin:
                return b"".join(parts).decode("utf-8", "replace")

    def close(self) -> None:
        try:
            self.sock.sendall(self._frame(b"", 0x8))
        except OSError:
            pass
        self.sock.close()


class Chrome:
    """A headless Chrome, launched on an ephemeral debugging port."""

    def __init__(self, binary: str, url: str, quiet: bool = True):
        self.profile = tempfile.mkdtemp(prefix="handbook-chrome-")
        self.proc = subprocess.Popen(
            [
                binary,
                "--headless=new",
                "--disable-gpu",
                "--no-sandbox",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-extensions",
                "--disable-dev-shm-usage",
                "--allow-file-access-from-files",
                "--remote-debugging-port=0",
                f"--user-data-dir={self.profile}",
                url,
            ],
            stdout=subprocess.DEVNULL if quiet else None,
            stderr=subprocess.DEVNULL if quiet else None,
        )
        self.ws = WebSocket(self._page_socket())
        self.next_id = 0

    def _port(self, deadline: float) -> int:
        marker = os.path.join(self.profile, "DevToolsActivePort")
        while time.time() < deadline:
            if os.path.exists(marker):
                try:
                    with open(marker, encoding="utf-8") as fh:
                        return int(fh.readline().strip())
                except (OSError, ValueError):
                    pass
            if self.proc.poll() is not None:
                raise RuntimeError(f"Chrome exited early (code {self.proc.returncode})")
            time.sleep(0.05)
        raise TimeoutError("Chrome never reported a debugging port")

    def _page_socket(self, timeout: float = 45.0) -> str:
        deadline = time.time() + timeout
        port = self._port(deadline)
        while time.time() < deadline:
            try:
                with urllib.request.urlopen(
                    f"http://127.0.0.1:{port}/json/list", timeout=5
                ) as resp:
                    targets = json.load(resp)
            except OSError:
                time.sleep(0.1)
                continue
            for t in targets:
                if t.get("type") == "page" and t.get("webSocketDebuggerUrl"):
                    return t["webSocketDebuggerUrl"]
            time.sleep(0.1)
        raise TimeoutError("Chrome never opened a page target")

    def call(self, method: str, **params):
        self.next_id += 1
        want = self.next_id
        self.ws.send(json.dumps({"id": want, "method": method, "params": params}))
        while True:
            message = json.loads(self.ws.recv())
            if message.get("id") != want:
                continue                                  # an event; not ours
            if "error" in message:
                raise RuntimeError(f"{method}: {message['error']}")
            return message.get("result", {})

    def eval(self, expression: str):
        result = self.call("Runtime.evaluate", expression=expression, returnByValue=True)
        return result.get("result", {}).get("value")

    def wait_for(self, expression: str, timeout: float, what: str) -> None:
        """Poll a JavaScript expression until it is truthy."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                if self.eval(expression):
                    return
            except RuntimeError:
                pass
            time.sleep(0.25)
        raise TimeoutError(f"timed out after {timeout:.0f}s waiting for {what}")

    def print_to_pdf(self, **options) -> bytes:
        result = self.call("Page.printToPDF", **options)
        return base64.b64decode(result["data"])

    def close(self) -> None:
        try:
            self.ws.close()
        finally:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.proc.kill()
            shutil.rmtree(self.profile, ignore_errors=True)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
        return False


if __name__ == "__main__":                                # tiny self-test
    with Chrome(sys.argv[1], "data:text/html,<h1 id=x>ok</h1>") as browser:
        browser.wait_for("!!document.getElementById('x')", 10, "the test element")
        print("evaluate:", browser.eval("document.getElementById('x').textContent"))
        print("pdf bytes:", len(browser.print_to_pdf(printBackground=True)))
