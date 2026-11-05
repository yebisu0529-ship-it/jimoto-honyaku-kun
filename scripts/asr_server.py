#!/usr/bin/env python3
"""Resident, loopback-only Qwen3 ASR (or explicitly selected Whisper).

Example: python asr_server.py --backend qwen
Explicit fallback: --backend whisper --model small --device cpu --compute-type int8
Downloads require an explicit --allow-download. No audio is sent off this PC.
"""
from __future__ import annotations

import argparse
import io
import json
import logging
import math
import os
from pathlib import Path
import socket
import threading
from types import SimpleNamespace
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit
import wave

MAX_BODY = 4 * 1024 * 1024
MAX_SECONDS = 15.0
SAMPLE_RATES = {8000, 16000, 22050, 24000, 32000, 44100, 48000}
# Whisper's language set, plus the translation UI's Chinese variants.
WHISPER_LANGUAGES = frozenset(
    "en zh de es ru ko fr ja pt tr pl ca nl ar sv it id hi fi vi he uk el ms cs ro da hu ta no th ur hr bg lt la mi ml cy sk te fa lv bn sr az sl kn et mk br eu is hy ne mn bs kk sq sw gl mr pa si km sn yo so af oc ka be tg sd gu am yi lo uz fo ht ps tk nn mt sa lb my bo tl mg as tt haw ln ha ba jw su yue".split()
)
QWEN_LANGUAGES = dict(zip(
    "zh en yue ar de fr es pt id it ko ru th vi ja tr hi ms nl sv da fi pl cs fil fa el hu mk ro".split(),
    ["Chinese", "English", "Cantonese", "Arabic", "German", "French", "Spanish", "Portuguese",
     "Indonesian", "Italian", "Korean", "Russian", "Thai", "Vietnamese", "Japanese", "Turkish",
     "Hindi", "Malay", "Dutch", "Swedish", "Danish", "Finnish", "Polish", "Czech", "Filipino",
     "Persian", "Greek", "Hungarian", "Macedonian", "Romanian"],
))


class RequestError(ValueError):
    def __init__(self, message: str, status: int = 400):
        super().__init__(message)
        self.status = status


def parse_language(query: str, backend: str = "qwen") -> str | None:
    args = parse_qs(query, keep_blank_values=True)
    if set(args) - {"language"} or len(args.get("language", [])) > 1:
        raise RequestError("Only one language parameter is accepted.")
    language = args.get("language", ["en"])[0]
    if language == "auto":
        return None
    aliases = {"zh-hans": "zh", "zh-hant": "zh"}
    aliases.update({"tl": "fil"} if backend == "qwen" else {"jv": "jw", "fil": "tl"})
    language = aliases.get(language, language)
    if language not in (QWEN_LANGUAGES if backend == "qwen" else WHISPER_LANGUAGES):
        raise RequestError(f"Audio language is not supported by the selected {backend} backend.")
    return language


def validate_content_length(values: list[str] | None) -> int:
    if not values:
        raise RequestError("Content-Length is required.", 411)
    if len(values) != 1 or not values[0].isascii() or not values[0].isdigit():
        raise RequestError("Invalid Content-Length.")
    if len(values[0]) > 10:
        raise RequestError("Audio body exceeds 4 MiB.", 413)
    length = int(values[0])
    if length <= 0:
        raise RequestError("Audio body is empty.")
    if length > MAX_BODY:
        raise RequestError("Audio body exceeds 4 MiB.", 413)
    return length


def validate_wav(body: bytes) -> float:
    if not body or len(body) > MAX_BODY:
        raise RequestError("Audio body must be between 1 byte and 4 MiB.", 413)
    try:
        with wave.open(io.BytesIO(body), "rb") as audio:
            channels = audio.getnchannels()
            rate = audio.getframerate()
            width = audio.getsampwidth()
            frames = audio.getnframes()
            if audio.getcomptype() != "NONE" or width != 2:
                raise RequestError("WAV must contain uncompressed PCM16 audio.")
            if channels not in (1, 2):
                raise RequestError("WAV must be mono or stereo.")
            if rate not in SAMPLE_RATES:
                raise RequestError("Unsupported WAV sample rate.")
            if frames <= 0:
                raise RequestError("WAV contains no audio frames.")
            duration = frames / rate
            if duration > MAX_SECONDS:
                raise RequestError("Audio must be 15 seconds or shorter.", 413)
            if len(audio.readframes(frames)) != frames * width * channels:
                raise RequestError("WAV data is truncated.")
    except (wave.Error, EOFError, OSError) as exc:
        raise RequestError("Invalid PCM WAV audio.") from exc
    return duration


def decode_wav(body: bytes):
    """Decode already-validated PCM16 to mono float32/16 kHz without file IO."""
    import numpy as np
    from scipy.signal import resample_poly
    with wave.open(io.BytesIO(body), "rb") as audio:
        rate, channels = audio.getframerate(), audio.getnchannels()
        samples = np.frombuffer(audio.readframes(audio.getnframes()), dtype="<i2").astype(np.float32) / 32768.0
    if channels == 2:
        samples = samples.reshape(-1, 2).mean(axis=1)
    if rate != 16000:
        divisor = math.gcd(rate, 16000)
        samples = resample_poly(samples, 16000 // divisor, rate // divisor).astype(np.float32)
    return samples


class NativeQwen:
    """Small adapter around the native Transformers Qwen3-ASR processor/model."""

    def __init__(self, model, processor, max_new_tokens: int = 256):
        self.model = model
        self.processor = processor
        self.max_new_tokens = max_new_tokens

    def transcribe(self, audio, language=None):
        import torch
        samples, rate = audio
        if rate != 16000:
            raise ValueError("Native Qwen input must already be resampled to 16 kHz.")
        inputs = self.processor.apply_transcription_request(audio=samples, language=language)
        inputs = inputs.to(self.model.device, self.model.dtype)
        with torch.inference_mode():
            generated = self.model.generate(**inputs, max_new_tokens=self.max_new_tokens, do_sample=False)
        sequences = getattr(generated, "sequences", generated)
        new_ids = sequences[:, inputs["input_ids"].shape[1]:]
        # An output budget is a safety limit, not evidence of a complete transcript.
        eos = getattr(getattr(self.model, "generation_config", None), "eos_token_id", None)
        if eos is None:
            eos = getattr(getattr(self.processor, "tokenizer", None), "eos_token_id", None)
        eos_ids = {eos} if isinstance(eos, int) else set(eos or [])
        if new_ids.shape[-1] >= self.max_new_tokens and not any(token in eos_ids for token in new_ids[0].tolist()):
            raise RequestError("Speech recognition reached its output limit; use a shorter clip.", 422)
        parsed = self.processor.decode(new_ids, return_format="parsed")
        if not isinstance(parsed, list) or not parsed or not isinstance(parsed[0], dict):
            raise RuntimeError("Native Qwen returned an invalid parsed result.")
        first = parsed[0]
        if not isinstance(first.get("transcription"), str):
            raise RuntimeError("Native Qwen result contains no transcription.")
        return [SimpleNamespace(text=first["transcription"],
                                language=first.get("language") or language or "auto")]


class ASRState:
    def __init__(self, model, model_name: str, device: str, compute_type: str, backend: str = "whisper"):
        self.model = model
        self.model_name = model_name
        self.device = device
        self.compute_type = compute_type
        self.backend = backend
        self.lock = threading.Lock()

    def transcribe(self, body: bytes, language: str | None, duration: float) -> dict:
        if self.backend == "qwen":
            import numpy as np
            samples = decode_wav(body)
            # Qwen has no Whisper-style no-speech score. Suppress actual silence
            # before inference; this is not a claim of perfect speech detection.
            if not samples.size or float(np.sqrt(np.mean(samples * samples))) < 0.0001:
                return {"text": "", "language": language or "auto", "segments": []}
            result = self.model.transcribe(audio=(samples, 16000),
                                           language=QWEN_LANGUAGES[language] if language else None)[0]
            text = result.text.strip()
            detected = next((code for code, name in QWEN_LANGUAGES.items()
                             if name.casefold() == result.language.casefold()), result.language)
            # Without a forced-aligner, only the clip's interval is known.
            return {"text": text, "language": detected,
                    "segments": [{"start": 0.0, "end": duration, "text": text}] if text else []}
        # faster-whisper decodes/resamples this in-memory WAV to 16 kHz via PyAV.
        segments, info = self.model.transcribe(
            io.BytesIO(body), language=language, task="transcribe",
            beam_size=1, best_of=1, temperature=0.0,
            vad_filter=True, vad_parameters={"min_silence_duration_ms": 300},
            condition_on_previous_text=False, initial_prompt=None,
            no_speech_threshold=0.6, log_prob_threshold=-1.0,
            compression_ratio_threshold=2.4, max_new_tokens=224,
            hallucination_silence_threshold=1.0,
        )
        result = []
        for segment in segments:
            text = segment.text.strip()
            if not text:
                continue
            if getattr(segment, "no_speech_prob", 0.0) > 0.6 and getattr(segment, "avg_logprob", 0.0) < -1.0:
                continue
            start = max(0.0, min(float(segment.start), duration))
            end = max(start, min(float(segment.end), duration))
            result.append({"start": start, "end": end, "text": text})
        # Keep segments authoritative; no session state or previous-text history.
        return {"text": " ".join(s["text"] for s in result),
                "language": info.language, "segments": result}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "LocalASR/1"

    def log_message(self, fmt, *args):
        # Do not log request URLs, audio, or recognized content.
        logging.info("ASR HTTP %s", args[1] if len(args) > 1 else "request")

    def reply(self, status: int, value: dict):
        data = json.dumps(value, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Connection", "close")
        self.end_headers()
        self.close_connection = True
        try:
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError, socket.timeout):
            pass

    def local_request(self) -> bool:
        # A native client sends no Origin. Browser page requests are rejected.
        if self.headers.get_all("Origin"):
            self.reply(403, {"error": "Cross-site requests are not accepted."})
            return False
        hosts = {"127.0.0.1", "localhost", f"127.0.0.1:{self.server.server_port}",
                 f"localhost:{self.server.server_port}"}
        if self.headers.get("Host", "").lower() not in hosts:
            self.reply(403, {"error": "Only local Host headers are accepted."})
            return False
        return True

    def do_OPTIONS(self):
        self.reply(403, {"error": "Browser CORS access is disabled."})

    def do_GET(self):
        if not self.local_request():
            return
        if self.path != "/health":
            self.reply(404, {"error": "Not found."})
            return
        state = self.server.state
        self.reply(200, {"status": "ok", "loaded": True, "model": state.model_name,
                         "backend": state.backend,
                         "device": state.device, "compute_type": state.compute_type,
                         "busy": state.lock.locked()})

    def do_POST(self):
        if not self.local_request():
            return
        locked = False
        try:
            path = urlsplit(self.path)
            if path.path != "/transcribe" or path.scheme or path.netloc or path.fragment:
                raise RequestError("Not found.", 404)
            language = parse_language(path.query, self.server.state.backend)
            if self.headers.get("Transfer-Encoding"):
                raise RequestError("Chunked bodies are not accepted.")
            length = validate_content_length(self.headers.get_all("Content-Length"))
            if self.headers.get_content_type() != "application/octet-stream":
                raise RequestError("Content-Type must be application/octet-stream.", 415)
            state = self.server.state
            locked = state.lock.acquire(blocking=False)
            if not locked:
                raise RequestError("ASR is busy; retry after the current clip.", 429)
            self.connection.settimeout(15)
            body = self.rfile.read(length)
            if len(body) != length:
                raise RequestError("Audio body is truncated.")
            duration = validate_wav(body)
            result = state.transcribe(body, language, duration)
            self.reply(200, result)
        except RequestError as exc:
            self.reply(exc.status, {"error": str(exc)})
        except (socket.timeout, TimeoutError):
            self.reply(408, {"error": "Audio upload timed out."})
        except Exception:
            logging.exception("Local ASR inference failed")
            self.reply(500, {"error": "Local speech recognition failed. See the sidecar log."})
        finally:
            if locked:
                self.server.state.lock.release()


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", choices=["qwen", "whisper"], default="qwen")
    parser.add_argument("--model", default=None)
    parser.add_argument("--device", choices=["cpu", "cuda", "auto"], default="auto")
    parser.add_argument("--compute-type", default=None)
    parser.add_argument("--port", type=int, default=17861)
    parser.add_argument("--allow-download", action="store_true", help="Explicitly permit downloading missing model files")
    parser.add_argument("--check", action="store_true", help="Check dependencies and cached model without loading inference")
    args = parser.parse_args(argv)
    if not 1 <= args.port <= 65535:
        parser.error("port must be between 1 and 65535")
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    if not args.allow_download:
        # Enforce offline operation for the model and auxiliary VAD artifacts.
        os.environ["HF_HUB_OFFLINE"] = "1"
        os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"
    try:
        args.model = args.model or ("Qwen/Qwen3-ASR-0.6B-hf" if args.backend == "qwen" else "small")
        if args.backend == "qwen":
            import torch
            import transformers
            from transformers import AutoProcessor, AutoModelForMultimodalLM
            from huggingface_hub import snapshot_download
            device = ("cuda" if torch.cuda.is_available() else "cpu") if args.device == "auto" else args.device
            compute_type = args.compute_type or (("bfloat16" if torch.cuda.is_bf16_supported() else "float16") if device == "cuda" else "float32")
            if compute_type not in {"float32", "float16", "bfloat16"}:
                raise ValueError("Qwen compute type must be float32, float16 or bfloat16.")
            model_path = args.model if Path(args.model).is_dir() else snapshot_download(
                repo_id=args.model, local_files_only=not args.allow_download)
            package_version = transformers.__version__
        else:
            import faster_whisper
            from faster_whisper import WhisperModel
            from faster_whisper.utils import download_model
            device = "cpu" if args.device == "auto" else args.device
            compute_type = args.compute_type or "int8"
            model_path = args.model if Path(args.model).is_dir() else download_model(
                args.model, local_files_only=not args.allow_download)
            package_version = faster_whisper.__version__
        if args.check:
            print(json.dumps({"ok": True, "backend": args.backend, "package_version": package_version,
                              "model": args.model, "model_path": str(model_path),
                              "device": device, "compute_type": compute_type,
                              "offline": not args.allow_download}))
            return 0
        logging.info("Loading cached %s model %s on %s (%s)", args.backend, args.model, device, compute_type)
        if args.backend == "qwen":
            processor = AutoProcessor.from_pretrained(model_path, local_files_only=True)
            native_model = AutoModelForMultimodalLM.from_pretrained(
                model_path, dtype=getattr(torch, compute_type),
                device_map="cuda:0" if device == "cuda" else "cpu",
                attn_implementation="sdpa", local_files_only=True)
            native_model.eval()
            model = NativeQwen(native_model, processor)
        else:
            model = WhisperModel(model_path, device=device, compute_type=compute_type,
                                 local_files_only=not args.allow_download, num_workers=1)
        state = ASRState(model, args.model, device, compute_type, args.backend)
        with ThreadingHTTPServer(("127.0.0.1", args.port), Handler) as server:
            server.daemon_threads = True
            server.state = state
            logging.info("ASR ready at http://127.0.0.1:%s", args.port)
            server.serve_forever(poll_interval=0.25)
    except KeyboardInterrupt:
        return 0
    except Exception as exc:
        logging.error("ASR startup failed: %s", exc)
        if not args.allow_download:
            logging.error("No model was downloaded. Cache the model explicitly or use --allow-download.")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
