"""Runs one harness build against a freshly started llama-server, so every run
starts from an empty prompt cache. Server arguments are the app's
(src/services.rs start_translation_server) except the port, as in the
fomc-glossary kit. Only the server this script starts is stopped."""
import argparse, json, os, subprocess, sys, time, urllib.request

ENGINE = r"M:\JimotoDemo-20261105\data\engine"
MODELS = {
    "8b": (ENGINE + r"\models\2234f168757f570714b333049c9d018161ec5277\plamo-3-translate-8b-Q4_K_M.gguf", "32768"),
    "0.8b": (ENGINE + r"\models\03891e907db8a43c97d3a7c75add125cc6d1f96a\plamo-3-translate-0.8b-Q4_K_M.gguf", "8192"),
}
WORK = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PORT = "17870"


def healthy():
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{PORT}/health", timeout=2) as r:
            return b'"ok"' in r.read()
    except Exception:
        return False


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--harness", required=True)
    ap.add_argument("--model", required=True, choices=MODELS)
    ap.add_argument("--corpus", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--glossary")
    ap.add_argument("--no-chain", action="store_true")
    ap.add_argument("--no-easy", action="store_true")
    a = ap.parse_args()
    assert not healthy(), "another server already answers on the port"
    model, ctx = MODELS[a.model]
    server = subprocess.Popen(
        [ENGINE + r"\runtime\llama-b12119-cuda\llama-server.exe", "-m", model, "--host", "127.0.0.1",
         "--port", PORT, "--alias", os.path.splitext(os.path.basename(model))[0], "-c", ctx,
         "--parallel", "2", "-ngl", "99", "--flash-attn", "auto", "--no-webui",
         "--chat-template-file", ENGINE + r"\templates\plamo-f2b4599dff9b3e45c62567eac15e83186b4cb64e.jinja",
         "--no-prefill-assistant"],
        cwd=WORK, stdout=subprocess.DEVNULL, stderr=open(os.path.join(WORK, "server.err.log"), "w"))
    try:
        for _ in range(240):
            if healthy():
                break
            if server.poll() is not None:
                sys.exit("server exited")
            time.sleep(0.5)
        else:
            sys.exit("server did not become healthy")
        cmd = [a.harness, "--corpus", a.corpus, "--out", a.out, "--endpoint", f"http://127.0.0.1:{PORT}"]
        if a.glossary:
            cmd += ["--glossary", a.glossary]
        if a.no_chain:
            cmd.append("--no-chain")
        if a.no_easy:
            cmd.append("--no-easy")
        env = dict(os.environ, PLAMO_DATA_DIR=os.path.join(WORK, "data"))
        subprocess.run(cmd, check=True, stderr=subprocess.DEVNULL, env=env)
    finally:
        server.terminate()
        server.wait(timeout=30)
    for _ in range(60):
        if not healthy():
            break
        time.sleep(0.5)
    print("ok", a.out, "server pid", server.pid, "exit", server.returncode)


main()
