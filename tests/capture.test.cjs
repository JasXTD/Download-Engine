const { test } = require("node:test");
const assert = require("node:assert/strict");
require("../extension/core.js");
require("../extension/capture.js");
function harness(options = {}) {
  const h = {
    item: { id: 7, url: "https://example.test/file.zip", finalUrl: "https://cdn.example.test/file.zip",
      startTime: "2026-10-09T00:00:00Z", filename: "C:\\Downloads\\file.zip",
      mime: "application/zip", totalBytes: 9000000, state: "in_progress",
      paused: false, incognito: false, danger: "safe", canResume: false },
    elapsed: 0, values: {}, calls: [], notes: [], receipt: "missing", enabled: true, queued: 0,
    rules: IDMCapture.rules.DEFAULTS, ...options
  };
  const clone = value => value === undefined ? undefined : structuredClone(value);
  const storage = {
    async get(key) { return key === null ? clone(h.values) : { [key]: clone(h.values[key]) }; },
    async set(values) {
      h.calls.push("save");
      if (h.failStorage) throw new Error("storage full");
      Object.assign(h.values, clone(values));
    },
    async remove(key) { delete h.values[key]; }
  };
  const downloads = {
    async search() { return h.item ? [clone(h.item)] : []; },
    async pause() { h.calls.push("pause"); if (h.pauseError) throw Error("cannot pause"); h.item.paused = true; h.item.canResume = true; if (h.firefox) h.item.state = "interrupted"; },
    async resume() { h.calls.push("resume"); if (h.resumeError) throw Error("cannot resume"); h.item.paused = false; h.item.canResume = false; if (h.firefox) h.item.state = "in_progress"; },
    async cancel() { h.calls.push("cancel"); if (h.cancelError) throw Error("cannot cancel"); if (h.cancelNoEffect) return; h.item.state = h.completeOnCancel ? "complete" : "interrupted"; h.item.paused = !!h.retainedPausedFlag; h.item.canResume = false; }
  };
  const native = request => IDMCore.handoff(async (_, req) => {
    h.calls.push(req.action);
    if (h.offline) throw Error("host missing");
    await h.before?.(req.action, h);
    if (req.action === "capture_prepare") {
      if (h.prepareError) throw Error("file needs browser cookies");
      if (h.receipt === "aborted") throw Error("aborted");
      if (h.receipt === "missing") h.receipt = "prepared";
    } else if (req.action === "capture_commit") {
      if (h.commitError) throw Error("app could not start");
      if (h.receipt !== "prepared" && h.receipt !== "committed") throw Error("not prepared");
      if (h.receipt === "prepared") { h.receipt = "committed"; h.queued++; }
      if (h.lostReply) throw Error("connection lost after commit");
    } else if (req.action === "capture_abort" && h.receipt !== "committed") {
      h.receipt = "aborted";
    }
    return { ok: true, version: 1, capture_state: h.receipt,
      job_id: ["prepared", "committed"].includes(h.receipt) ? "saved-job" : undefined };
  }, request);
  h.manager = () => new IDMCapture.Manager({
    downloads, storage, enabled: async () => h.enabled, rules: async () => h.rules, native,
    now: () => h.elapsed, wait: async ms => { h.elapsed += ms; await h.onWait?.(h); },
    report: async (ok, message) => { h.notes.push({ ok, message }); }, uuid: () => "unique"
  });
  h.record = phase => {
    h.values["capture.7"] = { id: 7, browserUrl: h.item.url, startTime: h.item.startTime,
      phase, request_id: "capture:unique" };
  };
  h.run = () => h.manager().consider(7);
  return h;
}
test("pause, preflight, durable commit, then cancel in that order", async () => {
  const h = harness(); await h.run();
  assert.deepEqual(h.calls.filter(v => v !== "save"), ["pause", "capture_prepare", "capture_commit", "cancel"]);
  assert.equal(h.calls[0], "save");
  assert.equal(h.queued, 1);
  assert.equal(h.item.state, "interrupted");
  assert.deepEqual(h.values, {});
});
test("active canResume=false is eligible; private, unsafe, generated and unknown files are not", async () => {
  for (const patch of [
    { paused: true }, { state: "complete" }, { incognito: true }, { danger: "file" },
    { byExtensionId: "another-extension" }, { url: "blob:https://example.test/1", finalUrl: "" },
    { totalBytes: -1 }, { totalBytes: 0 }, { mime: "text/html" }, { mime: "" },
    { finalUrl: "https://user:secret@example.test/x" }
  ]) {
    const h = harness(); Object.assign(h.item, patch); await h.run();
    assert.deepEqual(h.calls, [], JSON.stringify(patch));
  }
  const h = harness({ enabled: false }); await h.run(); assert.deepEqual(h.calls, []);
});
test("failed public-file check resumes browser without queueing or cancelling", async () => {
  const h = harness({ prepareError: true }); await h.run();
  assert.equal(h.item.paused, false);
  assert.equal(h.queued, 0);
  assert.equal(h.receipt, "aborted");
  assert(!h.calls.includes("cancel"));
  await h.run(); // A resume event must not capture the fallback again.
  assert.equal(h.calls.filter(v => v === "pause").length, 1);
});
test("desktop launch rejection keeps the browser file", async () => {
  const h = harness({ commitError: true }); await h.run();
  assert.equal(h.receipt, "aborted"); assert.equal(h.queued, 0);
  assert.equal(h.item.paused, false); assert(!h.calls.includes("cancel"));
});
test("lost commit acknowledgement is resolved from the durable receipt", async () => {
  const h = harness({ lostReply: true }); await h.run();
  assert.equal(h.queued, 1);
  assert(h.calls.includes("capture_abort")); assert(h.calls.includes("cancel"));
  assert(!h.calls.includes("resume"));
});
test("unreachable host resumes browser, keeps recovery journal, and retries later", async () => {
  const h = harness({ offline: true }); await h.run();
  assert.equal(h.item.paused, false); assert.equal(h.values["capture.7"].phase, "uncertain");
  assert(!h.calls.includes("cancel"));
  h.offline = false; await h.manager().recoverAll();
  assert.equal(h.receipt, "aborted");
  assert.equal(h.values["capture.7"].phase, "browser");
  h.item.state = "complete"; await h.manager().recoverAll(); assert.deepEqual(h.values, {});
});
test("worker restart resolves an already committed handoff without adding twice", async () => {
  const h = harness({ receipt: "committed", queued: 1 });
  h.item.paused = true; h.record("committing");
  await h.manager().recoverAll();
  assert.equal(h.item.state, "interrupted"); assert.equal(h.queued, 1);
  assert(!h.calls.includes("capture_commit"));
});
test("worker restart aborts an uncommitted handoff and resumes its paused file", async () => {
  for (const phase of ["pausing", "preparing", "committing"]) {
    const h = harness({ receipt: "prepared" });
    h.item.paused = true; h.record(phase);
    await h.manager().recoverAll();
    assert.equal(h.item.paused, false); assert.equal(h.receipt, "aborted");
    assert(!h.calls.includes("cancel"));
  }
});
test("files completed or resumed by user during preflight are not committed", async () => {
  for (const patch of [{ state: "complete" }, { paused: false }]) {
    const h = harness({ before: async (action, h) => {
      if (action === "capture_prepare") Object.assign(h.item, patch);
    } });
    await h.run();
    assert.equal(h.queued, 0); assert(!h.calls.includes("cancel"));
    assert(!h.calls.includes("resume")); // Respect the user's own action.
  }
});
test("turning capture off during preflight releases the browser file", async () => {
  const h = harness({ before: async (action, h) => { if (action === "capture_prepare") h.enabled = false; } });
  await h.run(); assert.equal(h.item.paused, false); assert.equal(h.queued, 0);
});
test("concurrent metadata events cannot start a second handoff", async () => {
  let release; const gate = new Promise(resolve => { release = resolve; });
  let entered; const atPrepare = new Promise(resolve => { entered = resolve; });
  const h = harness({ before: async action => { if (action === "capture_prepare") { entered(); await gate; } } });
  const m = h.manager(); const first = m.consider(7); await atPrepare;
  await m.consider(7); await m.recoverAll(); release(); await first;
  assert.equal(h.calls.filter(v => v === "pause").length, 1); assert.equal(h.queued, 1);
});
test("storage failure before pause leaves browser untouched", async () => {
  const h = harness({ failStorage: true }); await h.run();
  assert(!h.calls.includes("pause")); assert(!h.calls.includes("cancel")); assert.equal(h.queued, 0);
});
test("failed browser cancellation reports duplicate possibility and resumes fallback", async () => {
  const h = harness({ cancelError: true }); await h.run();
  assert.equal(h.queued, 1); assert.equal(h.item.paused, false);
  assert.match(h.notes.at(-1).message, /both download lists/);
});
test("failed resume remains recoverable", async () => {
  const h = harness({ prepareError: true, resumeError: true }); await h.run();
  assert.equal(h.item.paused, true); assert(h.values["capture.7"]);
  h.resumeError = false; await h.manager().recoverAll();
  assert.equal(h.item.paused, false); assert.equal(h.values["capture.7"].phase, "browser");
});
test("reused browser IDs are never cancelled or resumed", async () => {
  const h = harness({ receipt: "committed" }); h.record("committing");
  h.item.startTime = "2026-10-10T00:00:00Z"; h.item.paused = true;
  await h.manager().recoverAll();
  assert(!h.calls.includes("resume")); assert(!h.calls.includes("cancel"));
});
test("uncertain fallback that later proves committed does not interrupt user activity", async () => {
  const h = harness({ receipt: "committed", queued: 1 }); h.record("uncertain");
  await h.manager().recoverAll();
  assert(!h.calls.includes("cancel")); assert.match(h.notes.at(-1).message, /also received/);
});
test("malformed native capture acknowledgements cannot authorize cancellation", async () => {
  for (const reply of [{ ok: true, version: 1 }, { ok: true, version: 1, capture_state: "prepared", job_id: "x" },
    { ok: true, version: 1, capture_state: "committed" }]) {
    await assert.rejects(IDMCore.handoff(async () => reply, { action: "capture_commit" }));
  }
});


test("a browser completion race during cancel is reported as a possible second copy", async () => {
  const h = harness({ completeOnCancel: true });
  await h.run();
  assert.equal(h.queued, 1);
  assert.match(h.notes.at(-1).message, /both download lists/);
});
test("recovery must not resume a later pause chosen by the user", async () => {
  const h = harness({ offline: true }); await h.run();
  assert.equal(h.item.paused, false);
  h.item.paused = true; // User deliberately pauses after our fallback.
  await h.manager().recoverAll();
  assert.equal(h.item.paused, true);
  h.offline = false; await h.manager().recoverAll();
  assert.equal(h.item.paused, true);
});


test("Firefox metadata arriving without onChanged is captured once it is known", async () => {
  const h = harness({ onWait: async h => { h.item.totalBytes = 9000000; h.item.mime = "application/zip"; } });
  h.item.totalBytes = -1; h.item.mime = "";
  await h.run();
  assert.equal(h.queued, 1);
  assert.equal(h.calls.filter(v => v === "pause").length, 1);
  assert(h.elapsed > 0);
});
test("metadata wait is bounded and leaves unknown files running without a journal", async () => {
  const h = harness(); h.item.totalBytes = -1;
  const m = h.manager(); await m.consider(7);
  assert(h.elapsed >= 10000 && h.elapsed <= 10200);
  assert.deepEqual(h.calls, []); assert.deepEqual(h.values, {});
  h.item.totalBytes = 9000000;
  await m.consider(7); assert.equal(h.queued, 1);
});
test("metadata wait respects completion, user pause, unsafe files, ID reuse, and disabling", async () => {
  for (const change of [
    h => { h.item.state = "complete"; }, h => { h.item.paused = true; },
    h => { h.item.incognito = true; }, h => { h.item.danger = "file"; },
    h => { h.item.startTime = "2026-10-11T00:00:00Z"; },
    h => { h.item.url = "https://another.example/file"; },
    h => { h.enabled = false; }, h => { h.item = null; }
  ]) {
    const h = harness({ onWait: async h => { h.item.totalBytes = 9000000; change(h); } });
    h.item.totalBytes = -1; await h.run();
    assert.deepEqual(h.calls, []); assert.equal(h.queued, 0); assert.deepEqual(h.values, {});
  }
});
test("metadata events during the wait do not start a duplicate handoff", async () => {
  let release, entered;
  const gate = new Promise(r => { release = r; });
  const waiting = new Promise(r => { entered = r; });
  const h = harness({ onWait: async h => { entered(); await gate; h.item.totalBytes = 9000000; } });
  h.item.totalBytes = -1;
  const m = h.manager(), first = m.consider(7);
  await waiting; await m.consider(7); release(); await first;
  assert.equal(h.queued, 1); assert.equal(h.calls.filter(v => v === "pause").length, 1);
});

test("Firefox interrupted-but-paused files are committed and cancelled after acknowledgement", async () => {
  const h = harness({ firefox: true }); await h.run();
  assert.equal(h.queued, 1); assert.equal(h.item.paused, false);
  assert.deepEqual(h.calls.filter(v => v !== "save"), ["pause", "capture_prepare", "capture_commit", "cancel"]);
  assert.deepEqual(h.values, {});
});
test("Firefox interrupted-but-paused fallback resumes and retains its recovery journal", async () => {
  const h = harness({ firefox: true, offline: true }); await h.run();
  assert.equal(h.item.state, "in_progress"); assert.equal(h.item.paused, false);
  assert.equal(h.queued, 0); assert.equal(h.values["capture.7"].phase, "uncertain");
  h.item.paused = true; h.item.canResume = true; h.item.state = "interrupted"; // A later user pause.
  h.offline = false; await h.manager().recoverAll();
  assert.equal(h.item.paused, true); assert.equal(h.item.state, "interrupted");
});
test("Firefox restart recovery resumes a paused uncommitted handoff", async () => {
  const h = harness({ firefox: true, receipt: "prepared" });
  h.item.paused = true; h.item.canResume = true; h.item.state = "interrupted"; h.record("preparing");
  await h.manager().recoverAll();
  assert.equal(h.item.state, "in_progress"); assert.equal(h.item.paused, false);
  assert.equal(h.receipt, "aborted"); assert.equal(h.queued, 0);
});

test("Chromium cancellation with a retained paused flag is terminal and never resumed", async () => {
  const h = harness({ retainedPausedFlag: true }); await h.run();
  assert.equal(h.item.state, "interrupted"); assert.equal(h.item.paused, true);
  assert.equal(h.item.canResume, false); assert.equal(h.queued, 1);
  assert.equal(h.calls.filter(v => v === "cancel").length, 1);
  assert(!h.calls.includes("resume")); assert.deepEqual(h.values, {});
  assert.deepEqual(h.notes.at(-1), { ok: true, message: "Download sent to Download Engine." });
});

test("Firefox cancellation that leaves the file paused must fall back and report the second copy", async () => {
  const h = harness({ firefox: true, cancelNoEffect: true }); await h.run();
  assert.equal(h.queued, 1); assert.equal(h.item.paused, false);
  assert.equal(h.item.state, "in_progress"); assert(h.calls.includes("resume"));
  assert.equal(h.notes.at(-1).ok, false);
  assert.match(h.notes.at(-1).message, /both download lists/);
});
test("restart recovery clears an already cancelled Chromium copy without resuming it", async () => {
  const h = harness({ receipt: "committed", queued: 1 });
  h.item.state = "interrupted"; h.item.paused = true; h.item.canResume = false;
  h.record("committing"); await h.manager().recoverAll();
  assert(!h.calls.includes("cancel")); assert(!h.calls.includes("resume"));
  assert.deepEqual(h.values, {}); assert.equal(h.notes.at(-1).ok, true);
});

const RULES = IDMCapture.rules;
const rules = (patch = {}) => ({ ...RULES.DEFAULTS, ...patch });
const form = (patch = {}) => ({ types: "zip", minSize: "", unit: "MB", sites: "", ...patch });
const item = patch => ({ url: "https://example.test/get", finalUrl: "https://cdn.example.test/get",
  referrer: "https://pages.example.test/list", filename: "C:\\Downloads\\file.zip", totalBytes: 9000000, ...patch });
test("capture rules default to common archives, programs and media; existing capture users keep every type", () => {
  for (const type of ["zip", "7z", "rar", "exe", "msi", "iso", "mp4", "mkv", "mp3", "pdf", "r0*"]) assert(RULES.DEFAULT_TYPES.includes(type), type);
  for (const type of ["html", "htm", "jpg", "png", "txt", "csv", "json", "docx"]) assert(!RULES.DEFAULT_TYPES.includes(type), type);
  assert.equal(RULES.DEFAULT_TYPES.length, 59);
  assert.equal(new Set(RULES.DEFAULT_TYPES).size, RULES.DEFAULT_TYPES.length);
  assert.deepEqual(RULES.normalize(undefined), RULES.DEFAULTS);
  assert.deepEqual(RULES.normalize(undefined, true), { version: 1, types: ["*"], minSizeKB: 0, sites: [] });
  // A saved choice always wins over the legacy default.
  assert.deepEqual(RULES.normalize({ version: 1, types: ["zip"], minSizeKB: 0, sites: [] }, true).types, ["zip"]);
  assert(Object.isFrozen(RULES.DEFAULTS) && Object.isFrozen(RULES.DEFAULTS.types));
});
test("damaged stored capture rules fall back field by field to safe values", () => {
  for (const saved of [null, "zip", 7, []]) assert.deepEqual(RULES.normalize(saved), RULES.DEFAULTS, JSON.stringify(saved));
  assert.deepEqual(RULES.normalize({ types: "zip", minSizeKB: "5", sites: "example.test" }), RULES.DEFAULTS);
  assert.deepEqual(RULES.normalize({ types: [], minSizeKB: -1, sites: [] }).types, RULES.DEFAULT_TYPES);
  for (const minSizeKB of [-1, 1.5, NaN, Infinity, 1024 ** 3 + 1, "10"]) assert.equal(RULES.normalize({ types: ["zip"], minSizeKB }).minSizeKB, 0);
  assert.equal(RULES.normalize({ types: ["zip"], minSizeKB: 1024 ** 3 }).minSizeKB, 1024 ** 3);
  const repaired = RULES.normalize({ types: ["ZIP", ".mp4", "zip", "bad/type", 5, "x".repeat(17)], minSizeKB: 2048,
    sites: ["Example.TEST", "bad site", "*", 9, "*.Example.ORG"] });
  assert.deepEqual(repaired, { version: 1, types: ["zip", "mp4"], minSizeKB: 2048, sites: ["example.test", "*.example.org"] });
  const many = RULES.normalize({ types: Array.from({ length: 300 }, (_, i) => "t" + i), sites: Array.from({ length: 150 }, (_, i) => `s${i}.test`) });
  assert.equal(many.types.length, 200); assert.equal(many.sites.length, 100);
});
test("matching file types use the browser filename, then the URL, with * wildcards", () => {
  const types = list => rules({ types: list });
  assert.equal(RULES.matches(rules(), item()), true);
  assert.equal(RULES.matches(rules(), item({ filename: "C:\\Downloads\\report.docx" })), false);
  assert.equal(RULES.matches(rules(), item({ filename: "/home/me/Movie.MKV" })), true);
  assert.equal(RULES.matches(rules(), item({ filename: "C:\\Downloads\\archive.tar.gz" })), true);
  // Chromium may not know the filename yet: the final URL path decides until it does.
  assert.equal(RULES.matches(rules(), item({ filename: "", finalUrl: "https://cdn.example.test/v/Setup%20File.EXE?sig=1#x" })), true);
  assert.equal(RULES.matches(rules(), item({ filename: "", finalUrl: "", url: "https://example.test/files/a.zip" })), true);
  assert.equal(RULES.matches(rules(), item({ filename: "" })), false, "No extension in the URL");
  assert.equal(RULES.matches(rules(), item({ filename: "", finalUrl: "https://cdn.example.test/%E0%A4%A.zip" })), false, "Malformed escapes never match");
  assert.equal(RULES.matches(rules(), item({ filename: "C:\\Downloads\\part.r07" })), true);
  assert.equal(RULES.matches(rules(), item({ filename: "C:\\Downloads\\part.r7" })), false);
  assert.equal(RULES.matches(types(["*"]), item({ filename: "", finalUrl: "https://cdn.example.test/download" })), true);
  assert.equal(RULES.matches(types(["*"]), item({ filename: "C:\\Downloads\\notes.txt" })), true);
  assert.equal(RULES.matches(types(["mp*"]), item({ filename: "a.mp3" })), true);
  assert.equal(RULES.matches(types(["mp*"]), item({ filename: "a.m4a" })), false);
  assert.equal(RULES.matches(types(["*4"]), item({ filename: "a.mp4" })), true);
  assert.equal(RULES.matches(types(["zip"]), item({ filename: "C:\\Downloads\\zip" })), false, "A name without a dot has no type");
});
test("minimum size keeps smaller files in the browser and accepts the exact limit", () => {
  assert.equal(RULES.matches(rules({ minSizeKB: 0 }), item({ totalBytes: 1 })), true);
  assert.equal(RULES.matches(rules({ minSizeKB: 10 }), item({ totalBytes: 10 * 1024 - 1 })), false);
  assert.equal(RULES.matches(rules({ minSizeKB: 10 }), item({ totalBytes: 10 * 1024 })), true);
  assert.equal(RULES.matches(rules({ minSizeKB: 1024 }), item({ totalBytes: undefined })), false);
});
test("site exceptions match download, final and referring hosts with subdomains and * wildcards", () => {
  const sites = list => rules({ sites: list });
  const only = url => ({ url, finalUrl: "", referrer: "" });
  for (const [list, patch, expected] of [
    [["example.test"], {}, false],
    [["cdn.example.test"], {}, false],
    [["pages.example.test"], {}, false],
    [["other.test"], {}, true],
    [["ample.test"], {}, true],
    [["example.test"], only("https://notexample.test/a.zip"), true],
    [["*.example.org"], only("https://a.b.example.org/x.zip"), false],
    [["*.example.org"], only("https://example.org/x.zip"), true],
    [["cdn*.example.net"], only("https://cdn42.example.net/x.zip"), false],
    [["cdn*.example.net"], only("https://www.example.net/x.zip"), true],
    [["example.*"], only("https://example.co.uk/x.zip"), false],
    [["*.drive.*"], only("https://files.drive.example.test/x.zip"), false],
    [["xn--bcher-kva.example"], only("https://www.bücher.example/x.zip"), false],
    [["[::1]"], only("http://[::1]:8080/x.zip"), false],
    [["example.test"], { referrer: undefined, url: "https://a.test/x.zip", finalUrl: "not a url" }, true]
  ]) assert.equal(RULES.matches(sites(list), item(patch)), expected, JSON.stringify([list, patch]));
});
test("popup input is validated, bounded and normalized before saving", () => {
  assert.deepEqual(RULES.parse(form({ types: " .ZIP, *.mp4;r0*  zip\n**  ", minSize: "1.5",
    sites: "https://User:pw@WWW.Example.TEST:8443/a?b\n*.cdn.example.org, %2a.media.test bücher.example." })).rules,
  { version: 1, types: ["zip", "mp4", "r0*", "*"], minSizeKB: 1536, sites: ["www.example.test", "*.cdn.example.org", "*.media.test", "xn--bcher-kva.example"] });
  assert.equal(RULES.parse(form({ minSize: "100", unit: "KB" })).rules.minSizeKB, 100);
  assert.equal(RULES.parse(form({ minSize: "0.4", unit: "KB" })).rules.minSizeKB, 0);
  assert.equal(RULES.parse(form({ minSize: " ", unit: "KB" })).rules.minSizeKB, 0);
  assert.equal(RULES.parse(form({ minSize: String(1024 * 1024), unit: "MB" })).rules.minSizeKB, 1024 ** 3);
  const invalid = /^Invalid capture rules\.$/, size = /^Enter a minimum size between 0 and 1 TB\.$/;
  for (const [input, message] of [
    [null, invalid], [form({ unit: "GB" }), invalid], [form({ sites: undefined }), invalid],
    [form({ types: 5 }), invalid], [form({ types: "zip ".repeat(9000) }), invalid],
    [form({ types: "zip tar.gz a/b x.y.z" }), /^Check these file types: tar\.gz, a\/b, x\.y\.z$/],
    [form({ types: "toolongextension1" }), /^Check these file types: toolongextension1$/],
    [form({ types: " ,; " }), /^Add at least one file type, or enter \* to capture every type\.$/],
    [form({ types: Array.from({ length: 201 }, (_, i) => "t" + i).join(" ") }), /^Use up to 200 file types\.$/],
    [form({ minSize: "-1" }), size], [form({ minSize: "ten" }), size], [form({ minSize: "Infinity" }), size],
    [form({ minSize: String(1024 * 1024 + 1) }), size],
    [form({ sites: "good.test bad_site!.test * *.* http:// a..b" }), /^Check these sites: bad_site!\.test, \*, \*\.\*, http:\/\/, a\.\.b$/],
    [form({ sites: Array.from({ length: 101 }, (_, i) => `s${i}.test`).join("\n") }), /^Use up to 100 sites\.$/]
  ]) {
    const result = RULES.parse(input);
    assert.equal(result.rules, undefined, String(JSON.stringify(input)).slice(0, 80));
    assert.match(result.error, message);
  }
  assert.equal(RULES.parse(form({ types: Array.from({ length: 9 }, (_, i) => "bad/" + i).join(" ") })).error.split(", ").length, 5, "Errors list at most five entries");
  assert.equal(RULES.parse(form({ types: Array.from({ length: 200 }, (_, i) => "t" + i).join(" ") })).rules.types.length, 200);
  assert.equal(RULES.parse(form({ sites: Array.from({ length: 100 }, (_, i) => `s${i}.test`).join("\n") })).rules.sites.length, 100);
});
test("wildcard sites survive Chromium's escaped * in hostnames", () => {
  const NodeURL = globalThis.URL;
  // Chromium's URL parser reports * in a host as an upper-case %2A escape.
  globalThis.URL = class extends NodeURL { get hostname() { return super.hostname.replaceAll("*", "%2A"); } };
  try {
    assert.deepEqual(RULES.parse(form({ sites: "*.cdn.example.org cdn*.bücher.example" })).rules?.sites, ["*.cdn.example.org", "cdn*.xn--bcher-kva.example"]);
    assert.deepEqual(RULES.normalize({ types: ["zip"], sites: ["*.example.org"] }).sites, ["*.example.org"]);
  } finally { globalThis.URL = NodeURL; }
});
test("capture rule messages are translated catalog entries", () => {
  const catalog = require("../locales/extension.json");
  const source = require("node:fs").readFileSync(require.resolve("../extension/capture.js"), "utf8");
  const keys = [...source.matchAll(/__xdL\.[tf]\("([^"]+)"/g)].map(match => match[1]);
  for (const key of ["Invalid capture rules.", "Check these file types: {0}", "Use up to {0} sites.", "Enter a minimum size between 0 and 1 TB."]) assert(keys.includes(key), key);
  for (const key of keys) assert(catalog[key], key);
});
test("downloads outside the capture rules stay in the browser untouched", async () => {
  for (const [patch, itemPatch] of [
    [{}, { filename: "C:\\Downloads\\report.docx", mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document" }],
    [{ minSizeKB: 10 * 1024 }, {}],
    [{ sites: ["example.test"] }, {}],
    [{ sites: ["*.example.test"] }, {}],
    [{ sites: ["referrer.test"] }, { referrer: "https://www.referrer.test/page" }]
  ]) {
    const h = harness({ rules: rules(patch) }); Object.assign(h.item, itemPatch);
    await h.run();
    assert.deepEqual(h.calls, [], JSON.stringify(patch)); assert.deepEqual(h.values, {});
    assert.deepEqual(h.notes, []); assert.equal(h.item.paused, false); assert.equal(h.item.state, "in_progress");
  }
});
test("capture rules are read for every download and a later filename can qualify it", async () => {
  const h = harness({ rules: rules({ types: ["exe"] }) });
  Object.assign(h.item, { filename: "", url: "https://example.test/get?id=5", finalUrl: "https://cdn.example.test/get?id=5" });
  const m = h.manager();
  await m.consider(7); assert.deepEqual(h.calls, []);
  h.item.filename = "C:\\Downloads\\setup.exe"; // A Content-Disposition name arrives through onChanged.
  await m.consider(7); assert.equal(h.queued, 1);
  const next = harness({ rules: rules({ sites: ["example.test"] }) }), manager = next.manager();
  await manager.consider(7); assert.deepEqual(next.calls, []);
  next.rules = rules(); await manager.consider(7); assert.equal(next.queued, 1);
});
test("capture rules are required and their storage failures leave the browser file untouched", async () => {
  const missing = harness(), unruled = missing.manager(); unruled.rules = undefined;
  await unruled.consider(7);
  assert.deepEqual(missing.calls, []); assert.match(missing.notes[0].message, /Capture rules are required/);
  const h = harness(); const m = h.manager(); m.rules = async () => { throw Error("storage unavailable"); };
  await m.consider(7);
  assert.deepEqual(h.calls, []); assert.equal(h.item.paused, false); assert.deepEqual(h.values, {});
  assert.equal(h.notes.length, 1); assert.match(h.notes[0].message, /storage unavailable/);
});
