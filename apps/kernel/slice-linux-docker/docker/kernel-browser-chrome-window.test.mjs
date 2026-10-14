// MP-08/MP-10/MP-11 (§2b): native Chrome window binding and input arbiter.
import test from "node:test";
import assert from "node:assert/strict";
import { ChromeWindows, InputArbiter, bandChord, refusedChord } from "./kernel-browser-chrome-window.mjs";
import { OwnedDisplay } from "./kernel-browser-owned-display.mjs";

function fake({ rows, bounds = { left: 0, top: 0, width: 1280, height: 887, windowState: "normal" } } = {}) {
  const display = new OwnedDisplay("/unused");
  // ownsDisplay needs a live owned child; the fixture marks one without spawning.
  const host = { generation: 3, tabs: new Map([["host-tab-a", { tab_id: "host-tab-a", target_id: "ta", document_id: "da" }], ["host-tab-b", { tab_id: "host-tab-b", target_id: "tb", document_id: "db" }]]),
    chromium: { display, child: { pid: 4242, exitCode: null }, scale: 2 },
    browser: { connection: { send: async (method) => method === "Browser.getWindowForTarget" ? { windowId: 1, bounds } : {} } },
    tabGroups: { active: async () => "tb" } };
  const inventory = async () => rows ?? [
    { id: 0x400001, x: 0, y: 0, width: 128, height: 128, visible: true, focused: false, override_redirect: false, transient_for: 0 },
    { id: 0x600001, x: 0, y: 0, width: 2560, height: 1774, visible: true, focused: true, override_redirect: false, transient_for: 0 },
    { id: 0x600050, x: 200, y: 160, width: 1600, height: 401, visible: true, focused: false, override_redirect: true, transient_for: 0 },
  ];
  return { host, windows: new ChromeWindows(host, { inventory }), display };
}
test("MP-11 resolution with an owned display: band, auxiliary, generations and retirement", async t => {
  const { host, windows } = fake();
  const real = new OwnedDisplay((await import("node:os")).tmpdir());
  let started = false;
  try { await real.start(); started = true; } catch {}
  if (!started) return t.skip("no Xvfb");
  try {
    host.chromium.display = real;
    const bound = await windows.resolve();
    assert.equal(bound.tabbed.xid, 0x600001);
    assert.equal(bound.tabbed.content_top, 174, "1774 − 800×2");
    assert.equal(bound.tabbed.role, "tabbed");
    assert.deepEqual(windows.band(), { surface: "browser_chrome", window: { id: 0x600001, width: 2560, height: 1774 }, rect: { left: 0, top: 0, width: 2560, height: 174 }, contentHeight: 1600, isCurrent: bound.tabbed.isCurrent });
    assert.equal(bound.auxiliary.length, 1, "the omnibox dropdown below the band is admitted; the keepalive popup is not");
    assert.deepEqual(windows.auxiliaryRegion(bound.auxiliary[0]).rect, { left: 0, top: 0, width: 1600, height: 400 });
    const retired = []; windows.onRetire(event => retired.push(event));
    assert.equal((await windows.resolve()).tabbed.window_generation, bound.tabbed.window_generation, "same window, same generation");
    host.generation = 4;
    assert.equal(bound.tabbed.isCurrent(), false, "a browser generation change retires the binding");
    assert.throws(() => windows.current(), { code: "user_domain_stale_reference" });
    const next = await windows.resolve();
    assert.equal(next.tabbed.window_generation, bound.tabbed.window_generation + 1);
    assert.deepEqual(retired.map(event => event.reason), ["window_changed"]);
  } finally { await real.close(); }
});

test("MP-11 a fullscreen window never binds", async t => {
  const { host, windows } = fake({ bounds: { left: 0, top: 0, width: 1280, height: 887, windowState: "fullscreen" } });
  const real = new OwnedDisplay((await import("node:os")).tmpdir());
  try { await real.start(); } catch { return t.skip("no Xvfb"); }
  try { host.chromium.display = real; await assert.rejects(windows.resolve(), { code: "user_domain_stale_reference" }); }
  finally { await real.close(); }
});
test("MP-11 two equal tabbed windows or none never bind", async t => {
  const twin = { x: 0, y: 0, width: 2560, height: 1774, visible: true, focused: false, override_redirect: false, transient_for: 0 };
  for (const rows of [[{ ...twin, id: 5 }, { ...twin, id: 6 }], []]) {
    const { host, windows } = fake({ rows });
    const real = new OwnedDisplay((await import("node:os")).tmpdir());
    try { await real.start(); } catch { return t.skip("no Xvfb"); }
    try { host.chromium.display = real; await assert.rejects(windows.resolve(), { code: "user_domain_stale_reference" }); }
    finally { await real.close(); }
  }
});

test("MP-11 band input acts on the kernel-observed active Tab; agents and stale windows are refused", async () => {
  const host = { generation: 3, tabs: new Map([["host-tab-b", { tab_id: "host-tab-b", target_id: "tb", document_id: "db" }]]), tabGroups: { active: async () => "tb" } };
  const binding = { window_generation: 2, isCurrent: () => true };
  const arbiter = new InputArbiter(host, { current: () => ({ tabbed: binding }) });
  assert.deepEqual(await arbiter.resolveDisplayInput({ window_generation: 2 }), { tab_id: "host-tab-b", document_id: "db", generation: 3 });
  await assert.rejects(arbiter.resolveDisplayInput({ window_generation: 1 }), { code: "user_domain_stale_reference" });
  await assert.rejects(arbiter.resolveDisplayInput({ window_generation: 2, _agent_input: true }), { code: "user_domain_not_granted" });
  assert.deepEqual(await arbiter.resolveDisplayInput({ tab_id: "x", document_id: "d", generation: 3 }), { tab_id: "x", document_id: "d", generation: 3 }, "page input keeps its own target");
  assert.equal(await arbiter.isActiveTab({ target_id: "tb" }, binding), true);
  assert.equal(await arbiter.isActiveTab({ target_id: "ta" }, binding), false);
  host.tabGroups = null;
  assert.equal(await arbiter.isActiveTab({ target_id: "tb" }, binding), false, "no extension: fail closed");
  const calls = [];
  const source = { surface: "browser_chrome", isCurrent: () => true, click: async (x, y) => calls.push(["click", x, y]), wheel: async () => true, chord: async (k, m) => calls.push(["chord", k, m]) };
  assert.deepEqual(await arbiter.input({ stream: { regionSource: source }, command: { input: { kind: "chord", keysym: 0x6c, modifiers: 2 } } }), { dispatched: true });
  await assert.rejects(arbiter.input({ stream: { regionSource: source }, command: { input: { kind: "chord", keysym: 0x49, modifiers: 10 } } }), { code: "user_domain_not_granted" }, "Ctrl+Shift+I");
  await assert.rejects(arbiter.input({ stream: { regionSource: { ...source, isCurrent: () => false } }, command: { input: { kind: "click", x: 1, y: 1 } } }), { code: "user_domain_stale_reference" });
  await assert.rejects(arbiter.input({ stream: { regionSource: source }, command: { _agent_input: true, input: { kind: "click", x: 1, y: 1 } } }), { code: "user_domain_not_granted" });
  assert.deepEqual(calls, [["chord", 0x6c, 2]]);
});

test("MP-08 browser chords and refused chords", () => {
  for (const [k, m] of [[0x6c, 2], [0x74, 2], [0x77, 2], [0xff09, 2], [0xff09, 10], [0x54, 10], [0xff51, 1], [0xff53, 1]]) assert(bandChord(k, m), `${k}/${m}`);
  for (const [k, m] of [[0xffc9, 0], [0x49, 10], [0x4a, 10], [0x43, 10], [0x75, 2], [0x4e, 10], [0x70, 2], [0x73, 2], [0x6f, 2], [0x6a, 2], [0x68, 2]]) assert(refusedChord(k, m), `${k}/${m}`);
  for (const [k, m] of [[0x61, 0], [0x6c, 2], [0x63, 2], [0x76, 2]]) assert(!refusedChord(k, m), `${k}/${m}`);
});
