"""Private CPU Indra service: raw in-memory PNG only, no request/result logging."""
from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import struct
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MAX_BODY = 8 * 1024 * 1024
MAX_PIXELS = 16_000_000
PORT = 17866


def png_dimensions(data: bytes) -> tuple[int, int]:
    if len(data) < 33 or data[:8] != b"\x89PNG\r\n\x1a\n" or data[8:16] != b"\x00\x00\x00\rIHDR":
        raise ValueError("invalid_png")
    width, height = struct.unpack(">II", data[16:24])
    if not width or not height or width * height > MAX_PIXELS:
        raise ValueError("image_dimensions")
    return width, height


def content_length(value: str | None) -> int:
    if value is None or not value.isascii() or not value.isdecimal():
        raise ValueError("content_length_required")
    size = int(value)
    if not 0 < size <= MAX_BODY:
        raise ValueError("body_too_large")
    return size


def safe_path(path: Path) -> None:
    for candidate in (path, *path.parents):
        if candidate.is_symlink() or getattr(candidate, "is_junction", lambda: False)():
            raise ValueError("linked_installation_path")


def verify_installation(root: Path):
    safe_path(root)
    root = root.resolve(strict=True)
    raw = (root / "metadata.json").read_bytes()
    metadata = json.loads(raw)
    if metadata["version"] != 1 or sys.version_info[:3] != (3, 13, 14):
        raise ValueError("runtime_version")
    if Path(sys.prefix).resolve() != (root / ".venv").resolve() or not Path(sys.base_prefix).resolve().is_relative_to(root / "python"):
        raise ValueError("nonisolated_python")
    if sys.flags.isolated != 1:
        raise ValueError("isolated_python_required")
    cfg = (root / ".venv" / "pyvenv.cfg").read_text(encoding="utf-8").lower()
    if "include-system-site-packages = true" in cfg:
        raise ValueError("system_packages_enabled")
    lock = root / "runtime" / "indra-requirements.lock"
    safe_path(lock)
    if hashlib.sha256(lock.read_bytes()).hexdigest() != metadata["requirements_sha256"]:
        raise ValueError("requirements_changed")
    for wheel in metadata["wheels"]:
        if importlib.metadata.version(wheel["package"]) != wheel["version"]:
            raise ValueError("dependency_version")
    model = root / "models" / metadata["model_revision"]
    for artifact in metadata["models"]:
        path = model / artifact["name"]
        safe_path(path)
        if path.stat().st_size != artifact["bytes"] or hashlib.file_digest(path.open("rb"), "sha256").hexdigest() != artifact["sha256"]:
            raise ValueError("model_verification")
    return metadata, model, hashlib.sha256(raw).hexdigest()


class Engine:
    def __init__(self, root: Path):
        metadata, model, digest = verify_installation(root)
        # Both operands are explicit local paths. No HF model lookup/download.
        import cv2
        import numpy as np
        from indra_ocr import OrtOCR

        cv2.setNumThreads(1)
        self.cv2, self.np = cv2, np
        self.ocr = OrtOCR(model, threads=2, providers=["CPUExecutionProvider"], math=False)
        self.lock = threading.Lock()
        self.health = {"ready": True, "engine": "indra-ocr", "sdk_version": metadata["sdk_version"], "model_revision": metadata["model_revision"], "device": "cpu", "model_set": "ja-en", "metadata_sha256": digest}

    def recognize(self, data: bytes) -> dict:
        width, height = png_dimensions(data)
        gray = self.cv2.imdecode(self.np.frombuffer(data, self.np.uint8), self.cv2.IMREAD_GRAYSCALE)
        if gray is None or gray.shape != (height, width):
            raise ValueError("invalid_png")
        page = self.ocr(gray, turn=False)
        return {"text": page.text, "model_revision": self.health["model_revision"], "device": "cpu", "timings": page.timings}


class Handler(BaseHTTPRequestHandler):
    server_version = "LocalIndra/1"

    def log_message(self, *_args):
        pass

    def respond(self, status: int, payload: dict):
        raw = json.dumps(payload, ensure_ascii=False, allow_nan=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Connection", "close")
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        try:
            self.wfile.write(raw)
        except (BrokenPipeError, ConnectionResetError):
            pass
        self.close_connection = True

    def do_OPTIONS(self):
        self.respond(403, {"error": "cross_site_forbidden"})

    def do_GET(self):
        if self.headers.get("Origin") is not None:
            return self.respond(403, {"error": "cross_site_forbidden"})
        if self.path != "/health":
            return self.respond(404, {"error": "not_found"})
        self.respond(200, self.server.engine.health)

    def do_POST(self):
        if self.headers.get("Origin") is not None:
            return self.respond(403, {"error": "cross_site_forbidden"})
        if self.path != "/ocr":
            return self.respond(404, {"error": "not_found"})
        if self.headers.get("Transfer-Encoding") or self.headers.get_content_type() != "image/png":
            return self.respond(415, {"error": "raw_png_required"})
        try:
            if len(self.headers.get_all("Content-Length", [])) != 1:
                raise ValueError("content_length_required")
            size = content_length(self.headers.get("Content-Length"))
        except ValueError as error:
            return self.respond(400, {"error": str(error)})
        engine = self.server.engine
        if not engine.lock.acquire(blocking=False):
            return self.respond(429, {"error": "busy"})
        try:
            self.connection.settimeout(10)
            data = self.rfile.read(size)
            if len(data) != size:
                return self.respond(400, {"error": "truncated_body"})
            result = engine.recognize(data)
            self.respond(200, result)
        except ValueError as error:
            self.respond(400, {"error": str(error)})
        except (TimeoutError, ConnectionError):
            self.close_connection = True
        except Exception:
            # Never expose SDK exceptions that may contain image-derived text.
            self.respond(500, {"error": "ocr_failed"})
        finally:
            engine.lock.release()


class Server(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = False

    def __init__(self, engine, port=PORT):
        self.engine = engine
        self.slots = threading.BoundedSemaphore(4)
        super().__init__(("127.0.0.1", port), Handler)

    def process_request(self, request, address):
        if not self.slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        try:
            super().process_request(request, address)
        except Exception:
            self.slots.release()
            raise

    def process_request_thread(self, request, address):
        try:
            super().process_request_thread(request, address)
        finally:
            self.slots.release()

    def handle_error(self, *_args):
        # ThreadingHTTPServer's default traceback can expose request context.
        pass


def self_test():
    import unittest

    class Bounds(unittest.TestCase):
        def test_lengths(self):
            for value in (None, "", "-1", "0", "1,2", "９", str(MAX_BODY + 1)):
                with self.assertRaises(ValueError):
                    content_length(value)
            self.assertEqual(content_length("42"), 42)

        def test_png_header_and_pixels(self):
            def header(w, h):
                return b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR" + struct.pack(">II", w, h) + b"\x00" * 9
            self.assertEqual(png_dimensions(header(1920, 1080)), (1920, 1080))
            for data in (b"bad", header(0, 1), header(5000, 5000)):
                with self.assertRaises(ValueError):
                    png_dimensions(data)

        def test_http_origin_invalid_png_and_busy(self):
            import http.client
            class Stub:
                lock = threading.Lock()
                health = {"ready": True}
                def recognize(self, data):
                    png_dimensions(data)
                    return {"text": "test"}
            engine = Stub()
            server = Server(engine, port=0)
            worker = threading.Thread(target=server.serve_forever, daemon=True)
            worker.start()
            def request(method, route, body=None, headers=None):
                client = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=2)
                client.request(method, route, body, headers or {})
                response = client.getresponse()
                status = response.status
                response.read()
                client.close()
                return status
            try:
                self.assertEqual(request("GET", "/health"), 200)
                self.assertEqual(request("GET", "/health", headers={"Origin": "https://example.invalid"}), 403)
                self.assertEqual(request("POST", "/ocr", b"bad", {"Content-Type": "image/png"}), 400)
                engine.lock.acquire()
                try:
                    self.assertEqual(request("POST", "/ocr", b"bad", {"Content-Type": "image/png"}), 429)
                finally:
                    engine.lock.release()
            finally:
                server.shutdown()
                server.server_close()
                worker.join(timeout=2)

    suite = unittest.defaultTestLoader.loadTestsFromTestCase(Bounds)
    if not unittest.TextTestRunner().run(suite).wasSuccessful():
        raise SystemExit(1)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if not args.root or not args.root.is_absolute():
        parser.error("an absolute dedicated root is required")
    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"
    os.environ["HF_HUB_DISABLE_IMPLICIT_TOKEN"] = "1"
    engine = Engine(args.root)
    if args.check:
        print(json.dumps(engine.health))
        return
    with Server(engine) as server:
        server.serve_forever(poll_interval=0.1)


if __name__ == "__main__":
    main()
