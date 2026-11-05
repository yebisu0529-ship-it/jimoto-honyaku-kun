(() => {
  "use strict";
  let video = null, captionRoot = null, videoId = "", epoch = 0;
  let timer = null, previous = "", lastObserved = "";
  const crop = text => text.slice(0, 4000).replace(/[\uD800-\uDBFF]$/, "");
  const idFromUrl = () => {
    const url = new URL(location.href);
    return url.searchParams.get("v") || url.pathname.match(/^\/(?:shorts|live)\/([^/]+)/)?.[1] || "";
  };
  function send(text, force = false) {
    const payload = {
      text: crop(text), video_id: videoId,
      position: Number.isFinite(video?.currentTime) ? Math.max(0, video.currentTime) : 0,
      paused: !video || video.paused || video.ended, epoch
    };
    const key = JSON.stringify([payload.text, payload.video_id, payload.paused, epoch]);
    if (!force && previous === key) return;
    previous = key;
    // Fire and forget; no caption history, cookies, requests to YouTube, or retries.
    try { chrome.runtime.sendMessage({ type: "caption", payload }).catch(() => {}); } catch (_) {}
  }
  function visibleText() {
    if (!captionRoot || !videoId || video?.seeking) return "";
    return crop([...captionRoot.querySelectorAll(".ytp-caption-segment")]
      .filter(node => node.getClientRects().length && getComputedStyle(node).visibility !== "hidden"
        && getComputedStyle(node).display !== "none")
      .map(node => node.textContent || "").join(" ").trim());
  }
  function observeText() {
    const text = visibleText();
    if (text === lastObserved) return;
    lastObserved = text;
    clearTimeout(timer);
    timer = null;
    if (!text || video?.paused) { send(text); return; }
    timer = setTimeout(() => { timer = null; send(visibleText()); }, 300);
  }
  const captions = new MutationObserver(observeText);
  function reset() {
    clearTimeout(timer); timer = null; lastObserved = ""; epoch += 1;
    send("", true);
  }
  const onSeeking = () => reset();
  const onSeeked = () => { lastObserved = ""; observeText(); };
  const onPause = () => { clearTimeout(timer); timer = null; send("", true); };
  const onPlaying = () => { lastObserved = ""; observeText(); };
  const onEnded = () => reset();
  function refresh() {
    const nextId = idFromUrl();
    const nextVideo = document.querySelector("video.html5-main-video") || document.querySelector("video");
    if (videoId !== nextId || video !== nextVideo) {
      if (video) {
        for (const [name, handler] of events) video.removeEventListener(name, handler);
      }
      videoId = nextId; video = nextVideo;
      if (video) for (const [name, handler] of events) video.addEventListener(name, handler);
      reset();
    }
    const root = document.querySelector(".ytp-caption-window-container");
    if (root !== captionRoot) {
      captions.disconnect(); captionRoot = root;
      if (root) captions.observe(root, { subtree: true, childList: true, characterData: true, attributes: true });
      lastObserved = "";
    }
    observeText();
  }
  const events = [["seeking", onSeeking], ["seeked", onSeeked], ["pause", onPause], ["playing", onPlaying], ["ended", onEnded]];
  document.addEventListener("yt-navigate-finish", refresh);
  window.addEventListener("pagehide", () => { clearTimeout(timer); send("", true); });
  setInterval(refresh, 500);
  // Reoffer the current stable caption after app toggles or a busy backend.
  // The app deduplicates accepted text; source position is sampled by send().
  setInterval(() => {
    if (!video || video.paused || video.ended || video.seeking || timer !== null) return;
    const text = visibleText();
    if (text && text === lastObserved) send(text, true);
  }, 2000);
  refresh();
})();
