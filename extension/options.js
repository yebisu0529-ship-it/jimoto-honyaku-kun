"use strict";
const token = document.getElementById("token");
const status = document.getElementById("status");
async function render() {
  const { bridgeToken, bridgeStatus } = await chrome.storage.local.get(["bridgeToken", "bridgeStatus"]);
  if (!bridgeToken) { status.textContent = "未設定：アプリの接続キーを貼り付けて保存してください。"; return; }
  const time = bridgeStatus?.checkedAt ? new Date(bridgeStatus.checkedAt).toLocaleTimeString() : "";
  status.textContent = bridgeStatus ? `${bridgeStatus.detail}${time ? `（${time}）` : ""}` : "接続キーは保存済みです。「接続確認」を押してください。";
}
document.getElementById("save").addEventListener("click", async () => {
  const value = token.value.trim();
  if (!value) { status.textContent = "保存する接続キーを入力してください。"; return; }
  if (value.length > 4096 || /[\r\n]/.test(value)) { status.textContent = "接続キーの形式を確認してください。"; return; }
  await chrome.storage.local.set({ bridgeToken: value }); token.value = "";
  status.textContent = "保存しました。「接続確認」を押してください。";
});
document.getElementById("clear").addEventListener("click", async () => {
  await chrome.storage.local.remove(["bridgeToken", "bridgeStatus"]); token.value = ""; await render();
});
document.getElementById("check").addEventListener("click", async () => {
  status.textContent = "接続を確認しています…";
  try { await chrome.runtime.sendMessage({ type: "health" }); await render(); }
  catch (_) { status.textContent = "拡張機能を再読み込みしてください。"; }
});
chrome.storage.onChanged.addListener((_changes, area) => { if (area === "local") render(); });
render();
