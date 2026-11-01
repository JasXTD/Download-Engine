const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const source = fs.readFileSync(require('node:path').join(__dirname, '../extension/torrent-links.js'), 'utf8');
const magnet = 'magnet:?xt=urn:btih:0123456789012345678901234567890123456789';
const flush = () => new Promise(setImmediate);
async function fixture(enabled = true, send = async () => ({ ok: true })) {
  const calls = [], fallback = [];
  let click, change;
  class Anchor { constructor(href) { this.href = href; } }
  vm.runInNewContext(source, {
    HTMLAnchorElement: Anchor,
    document: { addEventListener(name, handler, capture) { assert.equal(name, 'click'); assert.equal(capture, true); click = handler; } },
    chrome: { storage: { local: { get: async () => ({ autoCapture: enabled }) }, onChanged: { addListener(handler) { change = handler; } } },
      runtime: { sendMessage(message) { calls.push(message); return send(message); } } },
    location: { assign(url) { fallback.push(url); } },
  });
  await flush();
  return { calls, fallback, change, click(overrides = {}, href = magnet) {
    const event = { isTrusted: true, button: 0, defaultPrevented: false, composedPath: () => [new Anchor(href)],
      preventDefault() { this.prevented = true; }, stopImmediatePropagation() { this.stopped = true; }, ...overrides };
    click(event); return event;
  } };
}
test('trusted magnet click is offered once and regular links are untouched', async () => {
  const f = await fixture();
  const ordinary = f.click({}, 'https://example.invalid/file.zip');
  assert.equal(ordinary.prevented, undefined);
  const offered = f.click(); f.click();
  await flush();
  assert.equal(offered.prevented, true); assert.equal(offered.stopped, true);
  assert.equal(f.calls.length, 1); assert.equal(f.calls[0].action, 'on_page_torrent'); assert.equal(f.calls[0].url, magnet);
  assert.deepEqual(f.fallback, []);
  f.click(); await flush(); assert.equal(f.calls.length, 2);
});
test('synthetic, modified and already handled clicks never create offers', async () => {
  const f = await fixture();
  for (const overrides of [{ isTrusted: false }, { button: 1 }, { ctrlKey: true }, { shiftKey: true }, { altKey: true }, { metaKey: true }, { defaultPrevented: true }]) {
    assert.equal(f.click(overrides).prevented, undefined);
  }
  f.click({}, 'magnet:' + 'a'.repeat(16384)); await flush();
  assert.equal(f.calls.length, 0);
});
test('automatic capture changes apply immediately', async () => {
  const f = await fixture(false); assert.equal(f.click().prevented, undefined);
  f.change({ autoCapture: { newValue: true } }, 'local'); f.click(); await flush();
  assert.equal(f.calls.length, 1);
  f.change({ autoCapture: { newValue: false } }, 'local'); assert.equal(f.click().prevented, undefined);
});
test('failed native handoff returns control to the normal protocol handler', async () => {
  for (const send of [async () => ({ ok: false }), async () => { throw new Error('Native host unavailable'); }]) {
    const f = await fixture(true, send); f.click(); await flush();
    assert.deepEqual(f.fallback, [magnet]);
    f.click(); await flush(); assert.equal(f.calls.length, 2);
  }
});
