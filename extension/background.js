"use strict";
const BRIDGE = "http://127.0.0.1:17862";
let activeRequest = null;
let lastStatus = "";
async function status(state, detail) {
  const key = state + ":" + detail;
  if (key === lastStatus) return;
  lastStatus = key;
  await chrome.storage.local.set({ bridgeStatus: { state, detail, checkedAt: Date.now() } });
}
function validCaption(p) {
  return p && typeof p.text === "string" && p.text.length <= 4000
    && typeof p.video_id === "string" && p.video_id.length <= 200
    && Number.isFinite(p.position) && p.position >= 0 && typeof p.paused === "boolean"
    && Number.isSafeInteger(p.epoch) && p.epoch >= 0;
}
async function request(path, payload) {
  // Abort superseded updates instead of building an inference/request backlog.
  activeRequest?.abort();
  const controller = new AbortController(); activeRequest = controller;
  const timeout = setTimeout(() => controller.abort("timeout"), 2500);
  try {
    const { bridgeToken } = await chrome.storage.local.get("bridgeToken");
    if (controller.signal.aborted) return { ok: false };
    if (!bridgeToken) {
      await status("missing-token", "アプリの接続キーを設定してください。");
      return { ok: false };
    }
    const response = await fetch(BRIDGE + path, {
      method: payload ? "POST" : "GET", cache: "no-store", credentials: "omit",
      headers: { "Content-Type": "application/json", "X-Plamo-Token": bridgeToken },
      ...(payload ? { body: JSON.stringify(payload) } : {}), signal: controller.signal
    });
    if (!response.ok) {
      const detail = response.status === 401 || response.status === 403
        ? "認証に失敗しました。アプリのトークンを貼り直してください。"
        : `アプリが HTTP ${response.status} を返しました。`;
      await status("error", detail); return { ok: false };
    }
    await status("connected", "ローカルアプリに接続しました。");
    return { ok: true };
  } catch (error) {
    if (!controller.signal.aborted || controller.signal.reason === "timeout") {
      await status("offline", "接続できません。PLaMoアプリでブラウザー連携を開始してください。");
    }
    return { ok: false };
  } finally {
    clearTimeout(timeout);
    if (activeRequest === controller) activeRequest = null;
  }
}
chrome.runtime.onMessage.addListener((message, sender, reply) => {
  if (sender.id !== chrome.runtime.id) return false;
  if (message?.type === "health" && !sender.tab) {
    request("/health").then(reply); return true;
  }
  if (message?.type === "caption" && sender.tab && validCaption(message.payload)) {
    let origin;
    try { origin = new URL(sender.url || "").origin; } catch (_) { return false; }
    if (!["https://www.youtube.com", "https://m.youtube.com"].includes(origin)) return false;
    request("/caption", message.payload).then(reply); return true;
  }
  return false;
});
chrome.action.onClicked.addListener(() => chrome.runtime.openOptionsPage());
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === "local" && changes.bridgeToken) {
    activeRequest?.abort();
    lastStatus = "";
    status(changes.bridgeToken.newValue ? "unchecked" : "missing-token",
      changes.bridgeToken.newValue ? "接続確認を押してください。" : "アプリの接続キーを設定してください。");
  }
});
