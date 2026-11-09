// Runs extension/content.js against a fake YouTube page and clock and checks
// what it sends while the page is hidden. Run: node scripts/test_extension_content.mjs
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const script = process.argv[2] ?? fileURLToPath(new URL("../extension/content.js", import.meta.url));
const source = readFileSync(script, "utf8");
let failures = 0;

function page() {
  let now = 0, nextId = 1, captionText = "", observer = null;
  const timers = new Map(), sent = [], videoHandlers = {};
  const segment = { get textContent() { return captionText; }, getClientRects() { return captionText ? [{}] : []; } };
  const captionRoot = { querySelectorAll() { return captionText ? [segment] : []; } };
  const video = {
    currentTime: 0, paused: false, ended: false, seeking: false,
    addEventListener(name, fn) { videoHandlers[name] = fn; }, removeEventListener(name) { delete videoHandlers[name]; }
  };
  const document = {
    hidden: false,
    querySelector(selector) {
      return selector.startsWith("video") ? video : selector === ".ytp-caption-window-container" ? captionRoot : null;
    },
    addEventListener() {}
  };
  const schedule = (fn, ms, repeat) => {
    const id = nextId++;
    timers.set(id, { at: now + Math.max(0, ms), fn, every: repeat ? ms : 0 });
    return id;
  };
  const context = {
    URL, location: { href: "https://www.youtube.com/watch?v=abc" }, document, window: { addEventListener() {} },
    MutationObserver: class { constructor(fn) { observer = fn; } observe() {} disconnect() {} },
    getComputedStyle: () => ({ visibility: "visible", display: "block" }),
    chrome: { runtime: { sendMessage(message) {
      sent.push({ t: now, text: message.payload.text, paused: message.payload.paused });
      return Promise.resolve();
    } } },
    setTimeout: (fn, ms) => schedule(fn, ms, false), setInterval: (fn, ms) => schedule(fn, ms, true),
    clearTimeout: id => timers.delete(id), clearInterval: id => timers.delete(id),
    Date: { now: () => now }, Number, Math, JSON, Promise
  };
  vm.createContext(context);
  vm.runInContext(source, context);
  const moveClock = to => {
    if (!video.paused && !video.seeking) video.currentTime += (to - now) / 1000;
    now = to;
  };
  return {
    advanceTo(target) {
      for (;;) {
        let due = null;
        for (const [id, timer] of timers) {
          if (timer.at <= target && (!due || timer.at < due[1].at || (timer.at === due[1].at && id < due[0]))) due = [id, timer];
        }
        if (!due) break;
        const [id, timer] = due;
        moveClock(timer.at);
        if (timer.every) timer.at += timer.every; else timers.delete(id);
        timer.fn();
      }
      moveClock(target);
    },
    caption(text) { captionText = text; observer?.(); },
    hide(hidden) { document.hidden = hidden; },
    fire(name, change = {}) { Object.assign(video, change); videoHandlers[name]?.(); },
    take() { return sent.splice(0); }
  };
}

const describe = messages => messages.map(m => `${(m.t / 1000).toFixed(1)}s ${JSON.stringify(m.text)}${m.paused ? " paused" : ""}`).join(", ");
function check(name, ok, messages) {
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${ok ? "" : `\n     sent: ${describe(messages)}`}`);
  if (!ok) failures += 1;
}

{
  // A minimized window: YouTube leaves the last cue on screen and stops changing it.
  const p = page();
  p.advanceTo(100); p.caption("Hello there"); p.advanceTo(1_000); p.take();
  p.hide(true); p.advanceTo(9_000);
  const sent = p.take();
  const clears = sent.filter(m => m.text === "");
  check("a frozen cue is cleared once, within 7 s of its last change", clears.length === 1 && clears[0].t <= 7_100, sent);
  check("a frozen cue is not reoffered after it is cleared", !sent.some(m => m.text && m.t > clears[0]?.t), sent);
  p.fire("waiting"); p.fire("playing"); p.advanceTo(13_000);
  const afterPlaying = p.take();
  check("playing again on a frozen page does not send the cue as live", !afterPlaying.some(m => m.text), afterPlaying);
  p.fire("seeking", { seeking: true }); p.advanceTo(13_200); p.fire("seeked", { seeking: false, currentTime: 60 }); p.advanceTo(17_000);
  const afterSeek = p.take();
  check("a seek on a frozen page does not send the old cue as live", !afterSeek.some(m => m.text), afterSeek);
}
{
  // A background tab that plays sound: YouTube keeps changing the captions.
  const p = page();
  p.hide(true); p.advanceTo(100); p.caption("Cue A is on screen"); p.advanceTo(1_000);
  p.fire("pause", { paused: true }); p.advanceTo(11_000);
  p.fire("play", { paused: false }); p.fire("playing"); p.advanceTo(13_500);
  p.caption("Cue B follows"); p.advanceTo(14_500);
  const sent = p.take();
  check("a pause does not age the cue on screen", !sent.some(m => m.t > 11_000 && m.text === "" && !m.paused), sent);
  check("changing captions are still sent while hidden", sent.some(m => m.text === "Cue B follows"), sent);
}
{
  const p = page();
  p.advanceTo(100); p.caption("A long visible cue"); p.advanceTo(11_000);
  const sent = p.take();
  check("a visible page keeps reoffering an unchanged cue",
    sent.filter(m => m.text === "A long visible cue").length >= 5 && !sent.some(m => m.t > 0 && m.text === ""), sent);
}
console.log(failures ? `${failures} failed` : "all passed");
process.exitCode = failures ? 1 : 0;
