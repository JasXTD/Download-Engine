const { test } = require("node:test");
const assert = require("node:assert/strict");
require("../extension/core.js");
const { normalizeUrl, makeRequest, handoff } = globalThis.IDMCore;
test("validates links and removes fragments", () => {
  assert.equal(normalizeUrl(" https://example.com/a.zip#x "), "https://example.com/a.zip");
  for (const url of ["javascript:alert(1)", "file:///C:/x", "data:abc", "blob:https://x/y", "https://user:password@example.com/x", "x"]) assert.throws(() => normalizeUrl(url));
});
test("handoff requires a durable queue acknowledgement", async () => {
  const request = makeRequest("https://example.com/a", "request-123");
  const reply = await handoff(async (host, body) => {
    assert.equal(host, "com.xetro.downloadengine");
    assert.deepEqual(body, request);
    return { ok: true, version: 1, job_id: "saved", message: "Added" };
  }, request);
  assert.equal(reply.job_id, "saved");
  for (const response of [null, {}, {ok:true,version:1}, {ok:false,version:1,message:"Disk full"}, {ok:true,version:2,job_id:"x"}]) await assert.rejects(handoff(async()=>response,request));
});
test("failed native connection is not acknowledged", async () => {
  await assert.rejects(handoff(async () => { throw new Error("host missing"); }, makeRequest("https://example.com/a", "r")), /host missing/);
});
test("automatic capture and detection have install access; login cookies stay optional", () => {
  for (const browser of ["chromium", "firefox"]) {
    const manifest = require("../extension/manifest." + browser + ".json");
    const origins = ["https://*/*", "http://*/*"];
    const permissions = ["activeTab", "alarms", "contextMenus", "nativeMessaging", "scripting", "storage", "downloads", "webRequest"];
    assert.deepEqual([...manifest.permissions].sort(), [...permissions, ...(browser === "firefox" ? origins : [])].sort());
    assert.deepEqual(manifest.optional_permissions, ["cookies"]);
    if (browser === "chromium") assert.deepEqual(manifest.host_permissions, origins);
  }
});

test("Firefox declares local data transfers and optional login consent for AMO", () => {
  const settings = require("../extension/manifest.firefox.json").browser_specific_settings;
  const gecko = settings.gecko;
  assert.equal(gecko.id, "download-engine@xetro", "Use the Download Engine identity for the first public listing");
  assert(Number.parseInt(gecko.strict_min_version, 10) >= 142,
    "AMO checks data consent against both desktop 140 and Android 142");
  assert.equal(settings.gecko_android, undefined,
    "The native Windows companion does not support Firefox for Android");
  assert.deepEqual(gecko.data_collection_permissions, {
    required: ["browsingActivity", "websiteContent"], optional: ["authenticationInfo"]
  });
});

test("store and toolbar icons exist at the sizes declared by both manifests", () => {
  const fs = require("node:fs");
  const path = require("node:path");
  for (const variant of ["chromium", "firefox"]) {
    const manifest = require("../extension/manifest." + variant + ".json");
    const action = manifest.action || manifest.browser_action;
    assert(manifest.icons, `${variant}: store icons are required`);
    assert(action.default_icon, `${variant}: toolbar icons are required`);
    for (const size of [16, 32, 48, 128]) {
      const file = manifest.icons[size];
      assert.equal(typeof file, "string", `${variant}: missing ${size}px icon`);
      const png = fs.readFileSync(path.join(__dirname, "../extension", file));
      assert.equal(png.subarray(0, 8).toString("hex"), "89504e470d0a1a0a");
      assert.equal(png.readUInt32BE(16), size);
      assert.equal(png.readUInt32BE(20), size);
    }
    for (const size of [16, 32]) assert.equal(action.default_icon[size], manifest.icons[size]);
  }
});

test("extension and native registration use the same branded identities", () => {
  const fs = require("node:fs");
  const native = fs.readFileSync(require.resolve("../src/browser_setup.rs"), "utf8");
  const host = /pub const HOST_NAME: &str = "([^"]+)"/.exec(native)?.[1];
  const firefoxId = /pub const FIREFOX_ID: &str = "([^"]+)"/.exec(native)?.[1];
  assert.equal(globalThis.IDMCore.HOST, "com.xetro.downloadengine");
  assert.equal(host, globalThis.IDMCore.HOST);
  assert.equal(firefoxId, require("../extension/manifest.firefox.json").browser_specific_settings.gecko.id);
});



test("extension pages permit only local scripts and no web requests or embedded objects", () => {
  for (const variant of ["chromium", "firefox"]) {
    const manifest = require("../extension/manifest." + variant + ".json");
    const policy = variant === "chromium" ? manifest.content_security_policy.extension_pages : manifest.content_security_policy;
    for (const directive of ["default-src 'none'", "script-src 'self'", "object-src 'none'",
      "connect-src 'none'", "base-uri 'none'", "form-action 'none'"]) assert(policy.includes(directive));
    assert(!policy.includes("unsafe-inline") && !policy.includes("unsafe-eval"));
    assert.equal(manifest.externally_connectable, undefined);
    assert.equal(manifest.web_accessible_resources, undefined);
    assert.equal(manifest.content_scripts, undefined);
  }
});
