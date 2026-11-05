"""ASR transport and validation tests; no model import or download required."""
import http.client
import io
import json
import struct
import threading
import unittest
import wave
from http.server import ThreadingHTTPServer
from types import SimpleNamespace

import asr_server as asr


def wav(seconds=0.1, channels=1, rate=16000, width=2):
    output = io.BytesIO()
    with wave.open(output, "wb") as audio:
        audio.setnchannels(channels)
        audio.setsampwidth(width)
        audio.setframerate(rate)
        audio.writeframes(b"\0" * int(seconds * rate) * channels * width)
    return output.getvalue()


class ValidationTests(unittest.TestCase):
    def test_language_defaults_aliases_and_invalid_input(self):
        self.assertEqual(asr.parse_language(""), "en")
        self.assertIsNone(asr.parse_language("language=auto"))
        self.assertEqual(asr.parse_language("language=zh-hant"), "zh")
        self.assertEqual(asr.parse_language("language=zh-hans"), "zh")
        self.assertEqual(asr.parse_language("language=jv", "whisper"), "jw")
        self.assertEqual(asr.parse_language("language=tl"), "fil")
        self.assertEqual(len(asr.QWEN_LANGUAGES), 30)
        with self.assertRaises(asr.RequestError):
            asr.parse_language("language=be", "qwen")
        for query in ["language=", "language=xx", "language=en&language=ja", "file=C:/secret"]:
            with self.assertRaises(asr.RequestError):
                asr.parse_language(query)

    def test_body_length_is_required_and_bounded(self):
        self.assertEqual(asr.validate_content_length(["100"]), 100)
        for values in [None, [], ["0"], ["-1"], ["+5"], [" 5"], ["3", "3"], ["3,3"], [str(asr.MAX_BODY + 1)], ["9" * 5000], ["９"]]:
            with self.assertRaises(asr.RequestError):
                asr.validate_content_length(values)

    def test_pcm_wav_duration_and_format(self):
        self.assertAlmostEqual(asr.validate_wav(wav()), 0.1)
        self.assertAlmostEqual(asr.validate_wav(wav(channels=2, rate=48000)), 0.1)
        for body in [b"bad", wav(width=1), wav(width=3), wav(channels=3), wav(rate=1000), wav(seconds=0), wav(seconds=15.1), wav()[:-2]]:
            with self.assertRaises(asr.RequestError):
                asr.validate_wav(body)

    def test_qwen_audio_is_mono_float32_16khz(self):
        samples = asr.decode_wav(wav(seconds=0.1, channels=2, rate=48000))
        self.assertEqual(samples.shape, (1600,))
        self.assertEqual(str(samples.dtype), "float32")

    def test_qwen_language_mapping_and_silence_gate(self):
        class Model:
            def transcribe(self, **kwargs):
                self.kwargs = kwargs
                return [SimpleNamespace(language="Japanese", text=" こんにちは ")]
        model = Model()
        state = asr.ASRState(model, "fake-qwen", "cpu", "float32", "qwen")
        silent = state.transcribe(wav(), "en", 0.1)
        self.assertEqual(silent["text"], "")
        self.assertFalse(hasattr(model, "kwargs"))
        output = io.BytesIO()
        with wave.open(output, "wb") as audio:
            audio.setnchannels(1)
            audio.setsampwidth(2)
            audio.setframerate(16000)
            audio.writeframes(struct.pack("<h", 1000) * 1600)
        result = state.transcribe(output.getvalue(), "ja", 0.1)
        self.assertEqual(model.kwargs["language"], "Japanese")
        self.assertEqual(model.kwargs["audio"][1], 16000)
        self.assertEqual(result["language"], "ja")
        self.assertEqual(result["segments"], [{"start": 0.0, "end": 0.1, "text": "こんにちは"}])

    def test_native_qwen_request_and_generation_limit(self):
        import torch

        class Inputs(dict):
            def to(self, device, dtype):
                self.location = (device, dtype)
                return self

        class Processor:
            tokenizer = SimpleNamespace(eos_token_id=99)

            def apply_transcription_request(self, **kwargs):
                self.request = kwargs
                return Inputs(input_ids=torch.tensor([[10, 20]]))

            def decode(self, ids, **kwargs):
                self.decoded = ids.tolist(), kwargs
                return [{"language": "English", "transcription": "hello"}]

        class Model:
            device = torch.device("cpu")
            dtype = torch.float32
            generation_config = SimpleNamespace(eos_token_id=[99])
            output = [10, 20, 30, 99]

            def generate(self, **kwargs):
                self.request = kwargs
                return torch.tensor([self.output])

        model, processor = Model(), Processor()
        native = asr.NativeQwen(model, processor, max_new_tokens=2)
        result = native.transcribe(audio=([0.1], 16000), language="English")[0]
        self.assertEqual(result.text, "hello")
        self.assertEqual(processor.request["language"], "English")
        self.assertEqual(processor.decoded, ([[30, 99]], {"return_format": "parsed"}))
        self.assertFalse(model.request["do_sample"])
        model.output = [10, 20, 30, 31]
        with self.assertRaises(asr.RequestError) as error:
            native.transcribe(audio=([0.1], 16000))
        self.assertEqual(error.exception.status, 422)

    def test_inference_has_no_history_and_uses_memory(self):
        class Model:
            def transcribe(self, body, **kwargs):
                self.body, self.kwargs = body, kwargs
                return iter([SimpleNamespace(start=0, end=0.1, text=" hello ", no_speech_prob=0, avg_logprob=0)]), SimpleNamespace(language="en")
        model = Model()
        state = asr.ASRState(model, "small", "cpu", "int8")
        result = state.transcribe(wav(), "en", 0.1)
        self.assertEqual(result["text"], "hello")
        self.assertIsInstance(model.body, io.BytesIO)
        self.assertFalse(model.kwargs["condition_on_previous_text"])
        self.assertTrue(model.kwargs["vad_filter"])
        self.assertEqual(model.kwargs["beam_size"], 1)


class HttpTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.state = asr.ASRState(None, "fake-cached-model", "cpu", "int8")
        cls.state.transcribe = lambda body, language, duration: {"text": "test", "language": language, "segments": []}
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), asr.Handler)
        cls.server.daemon_threads = True
        cls.server.state = cls.state
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join(timeout=2)

    def request(self, method, path, body=None, headers=None):
        connection = http.client.HTTPConnection("127.0.0.1", self.server.server_port, timeout=3)
        connection.request(method, path, body=body, headers=headers or {})
        response = connection.getresponse()
        status, data = response.status, json.loads(response.read())
        connection.close()
        return status, data

    def test_health_and_local_transcription(self):
        status, data = self.request("GET", "/health")
        self.assertEqual(status, 200)
        self.assertTrue(data["loaded"])
        status, data = self.request("POST", "/transcribe?language=zh-hant", wav(), {"Content-Type": "application/octet-stream"})
        self.assertEqual(status, 200)
        self.assertEqual(data["language"], "zh")

    def test_origin_rejected_and_busy_never_queues(self):
        status, _ = self.request("GET", "/health", headers={"Origin": "https://example.com"})
        self.assertEqual(status, 403)
        self.state.lock.acquire()
        try:
            status, _ = self.request("POST", "/transcribe", wav(), {"Content-Type": "application/octet-stream"})
            self.assertEqual(status, 429)
        finally:
            self.state.lock.release()


if __name__ == "__main__":
    unittest.main()
