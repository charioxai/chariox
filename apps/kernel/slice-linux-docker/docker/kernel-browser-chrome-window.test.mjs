// MP-08/MP-10/MP-11 (§2b): native Chrome window binding and input arbiter.
import test from "node:test";
import assert from "node:assert/strict";
import { ChromeWindows, InputArbiter, bandChord, refusedChord } from "./kernel-browser-chrome-window.mjs";
import { OwnedDisplay } from "./kernel-browser-owned-display.mjs";

function fake({ rows, bounds = { left: 0, top: 0, width: 1280, height: 887, windowState: "normal" }, viewport } = {}) {
  const display = new OwnedDisplay("/unused");
  // ownsDisplay needs a live owned child; the fixture marks one without spawning.
  const host = { generation: 3, tabs: new Map([["host-tab-a", { tab_id: "host-tab-a", target_id: "ta", document_id: "da" }], ["host-tab-b", { tab_id: "host-tab-b", target_id: "tb", document_id: "db" }]]),
    chromium: { display, child: { pid: 4242, exitCode: null }, scale: 2 },
    browser: { connection: { send: async (method) => method === "Browser.getWindowForTarget" ? { windowId: 1, bounds } : {} } },
    tabGroups: { active: async () => "tb", viewport: async target_id => ({target_id:target_id??'tb', ...(viewport ?? {width:bounds.width,height:bounds.height-87})}) } };
  const inventory = async () => (rows ?? [
    { id: 0x400001, x: 0, y: 0, width: 128, height: 128, visible: true, focused: false, override_redirect: false, transient_for: 0 },
    { id: 0x600001, x: 0, y: 0, width: 2560, height: 1774, visible: true, focused: true, override_redirect: false, transient_for: 0 },
    { id: 0x600050, x: 200, y: 160, width: 1600, height: 401, visible: true, focused: false, override_redirect: true, transient_for: 0 },
  ]).map(r=>({...r,frame_insets:r.frame_insets??[0,0,0,0]}));
  return { host, windows: new ChromeWindows(host, { inventory }), display };
}
// Union seam (2026-10-15): on the shared owned desktop Chromium's window fills the 1280x800 screen at
// scale 1 and the page is clipped below the band, so "window − pinned page" gave no band at all.
test("MP-08/MP-10/MP-11 the band binds on the shared owned desktop where the window fills the screen", async t => {
  const window = { id: 0x400013, x: 0, y: 0, width: 1280, height: 800, visible: true, focused: true, override_redirect: false, transient_for: 0 };
  const { host, windows } = fake({ rows: [window], bounds: { left: 0, top: 0, width: 1280, height: 800, windowState: "normal" } });
  const real = new OwnedDisplay((await import("node:os")).tmpdir());
  let started = false;
  try { await real.start(); started = true; } catch {}
  if (!started) return t.skip("no Xvfb");
  try {
    Object.assign(host.chromium, { display: real, scale: 1 });
    const bound = await windows.resolve();
    assert.equal(bound.tabbed.content_top, 86, "Chrome's 87 DIP band, even device rows");
    assert.deepEqual(windows.band().rect, { left: 0, top: 0, width: 1280, height: 86 });
  } finally { await real.close(); }
});
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
});

test("MP-11 band input: active Tab rechecked before dispatch, page focus fences keys/text, refused or retired input settles", async () => {
  let focus = false, document = "db";
  const evaluated = [];
  const connection = { send: async (method, params) => method === "Page.getFrameTree" ? { frameTree: { frame: { id: "f", loaderId: document } } }
    : method === "Runtime.evaluate" ? (evaluated.push(params), { result: { value: focus } }) : {} };
  const host = { generation: 3, tabs: new Map([["host-tab-b", { tab_id: "host-tab-b", target_id: "tb", document_id: "db" }]]), tabGroups: { active: async () => "tb" },
    browser: { resolvePageTarget: async () => ({ connection, sessionId: "s" }), ensureFocusWorld: async () => ({ contextId: 77 }) } };
  const arbiter = new InputArbiter(host, {});
  const calls = [];
  const source = { surface: "browser_chrome", isCurrent: () => true, click: async (x, y) => (calls.push(["click", x, y]), true), wheel: async (...a) => (calls.push(["wheel", ...a]), true), chord: async (k, m) => (calls.push(["chord", k, m]), true) };
  const send = (input, extra = {}) => arbiter.input({ stream: { regionSource: source }, command: { tab_id: "host-tab-b", document_id: "db", generation: 3, input, ...extra } });
  assert.deepEqual(await send({ kind: "click", x: 300, y: 40 }), { dispatched: true });
  focus = true;
  assert.deepEqual(await send({ kind: "key", key: "l", modifiers: 2 }), { dispatched: true }, "Ctrl+L whatever the focus");
  focus = false;
  await assert.rejects(send({ kind: "key", key: "I", modifiers: 10 }), { code: "user_domain_not_granted" }, "Ctrl+Shift+I");
  focus = true;
  await assert.rejects(send({ kind: "text", text: "secret" }), { code: "user_domain_sensitive_requires_focus" }, "a sensitive field focused: no band text");
  await assert.rejects(send({ kind: "key", key: "Enter" }), { code: "user_domain_sensitive_requires_focus" });
  await assert.rejects(send({ kind: "key", key: "v", modifiers: 2 }), { code: "user_domain_sensitive_requires_focus" }, "Ctrl+V is not a browser shortcut: it follows the text-target fence");
  focus = "frame";
  await assert.rejects(send({ kind: "text", text: "x" }), { code: "user_domain_sensitive_requires_focus" }, "a cross-origin frame fails closed");
  assert(evaluated.every(params => params.contextId === 77), "the fence runs in the isolated world");
  focus = false;
  assert.deepEqual(await send({ kind: "text", text: "wiki" }), { dispatched: true });
  assert.deepEqual(await send({ kind: "key", key: "Enter" }), { dispatched: true });
  assert.deepEqual(await send({ kind: "scroll", x: 10, y: 10, delta_x: 0, delta_y: 120 }), { dispatched: true });
  await assert.rejects(send({ kind: "click", x: 1, y: 1 }, { tab_id: "host-tab-a" }), { code: "user_domain_stale_reference" }, "another Tab became active after admission");
  // Live drill r4band1: a strip click while the active Tab loads was dropped. Band clicks,
  // wheel and browser shortcuts reach only Chrome's UI (the crop holds no page pixels);
  // text and other keys may reach the page and keep the document fence.
  document = "dc";
  assert.deepEqual(await send({ kind: "click", x: 1, y: 1 }), { dispatched: true }, "a strip click while the active Tab navigates");
  assert.deepEqual(await send({ kind: "scroll", x: 10, y: 10, delta_x: 0, delta_y: 120 }), { dispatched: true });
  assert.deepEqual(await send({ kind: "key", key: "t", modifiers: 2 }), { dispatched: true }, "Ctrl+T while the active Tab navigates");
  await assert.rejects(send({ kind: "text", text: "x" }), { code: "stale_document_reference" }, "text may reach the page: the active Tab navigated after admission");
  await assert.rejects(send({ kind: "key", key: "Enter" }), { code: "stale_document_reference" });
  document = "db";
  await assert.rejects(send({ kind: "click", x: 1, y: 1 }, { _agent_input: true }), { code: "user_domain_not_granted" });
  const retired = { ...source, click: async () => false };
  await assert.rejects(arbiter.input({ stream: { regionSource: retired }, command: { tab_id: "host-tab-b", document_id: "db", generation: 3, input: { kind: "click", x: 1, y: 1 } } }), { code: "user_domain_stale_reference" }, "a retired source settles as refused");
  const uncertain = { ...source, click: async () => { throw Object.assign(Error("x"), { code: "native_input_uncertain" }); } };
  assert.deepEqual(await arbiter.input({ stream: { regionSource: uncertain }, command: { tab_id: "host-tab-b", document_id: "db", generation: 3, input: { kind: "click", x: 1, y: 1 } } }), { dispatched: true }, "uncertain native input counts as dispatched, never replayed");
  await assert.rejects(send({ kind: "text", text: "é" }), /unsupported text/);
  assert.deepEqual(calls.map(c => c[0] === "chord" ? c[1] : c[0]), ["click", 0x6c, 0x77, 0x69, 0x6b, 0x69, 0xff0d, "wheel", "click", "wheel", 0x74]);
});

test("MP-08 browser chords and refused chords", () => {
  for (const [k, m] of [[0x6c, 2], [0x74, 2], [0x77, 2], [0xff09, 2], [0xff09, 10], [0x54, 10], [0xff51, 1], [0xff53, 1]]) assert(bandChord(k, m), `${k}/${m}`);
  for (const [k, m] of [[0xffc9, 0], [0x49, 10], [0x4a, 10], [0x43, 10], [0x75, 2], [0x4e, 10], [0x70, 2], [0x73, 2], [0x6f, 2], [0x6a, 2], [0x68, 2]]) assert(refusedChord(k, m), `${k}/${m}`);
  for (const [k, m] of [[0x61, 0], [0x6c, 2], [0x63, 2], [0x76, 2]]) assert(!refusedChord(k, m), `${k}/${m}`);
});

test('MP-08/MP-10/MP-11 page origin uses native Tab dimensions and GTK insets; band keeps even rows',async()=>{
 const rows=[{id:42,x:0,y:0,width:1280,height:800,visible:true,focused:true,override_redirect:false,transient_for:0,frame_insets:[4,4,0,4]}];
 const {host,windows}=fake({rows,bounds:{left:0,top:0,width:1280,height:800,windowState:'normal'},viewport:{width:1272,height:709}});
 const display=new OwnedDisplay(process.env.TMPDIR??'/tmp');await display.start();
 try{Object.assign(host.chromium,{display,scale:1});const b=(await windows.resolve()).tabbed;
 assert.deepEqual(b.page_rect,{left:4,top:87,width:1272,height:709});assert.equal(b.content_top,86);assert.equal(await b.isPageCurrent(),true);
 host.tabGroups.viewport=async target_id=>({target_id:target_id??'tb',width:1272,height:700});assert.equal(await b.isPageCurrent(),false);assert.equal(b.isCurrent(),false);
 }finally{await display.close()}
});

// MP-11 (review #1016 P2): Chromium keeps the page's (sensitive) activeElement while its own omnibox
// owns the keys. After the person puts focus in Chrome's UI, typing and Enter reach the omnibox; once
// the page has focus again (or nothing says Chrome's UI has it), the page's text-target fence applies.
test("MP-11 omnibox focus is told apart from the page's remembered sensitive focus", async () => {
  let pageFocus = true;
  const connection = { send: async (method, params) => method === "Page.getFrameTree" ? { frameTree: { frame: { id: "f", loaderId: "db" } } }
    : method === "Runtime.evaluate" ? { result: { value: params.expression === "document.hasFocus()" ? pageFocus : true } } : {} };
  const host = { generation: 3, tabs: new Map([["host-tab-b", { tab_id: "host-tab-b", target_id: "tb", document_id: "db" }]]), tabGroups: { active: async () => "tb" },
    browser: { resolvePageTarget: async () => ({ connection, sessionId: "s" }), ensureFocusWorld: async () => ({ contextId: 77 }) } };
  const arbiter = new InputArbiter(host, {}), calls = [];
  const source = { surface: "browser_chrome", isCurrent: () => true, click: async () => { pageFocus = false; calls.push("click"); return true; }, chord: async k => (calls.push(k), true) };
  const send = input => arbiter.input({ stream: { regionSource: source }, command: { tab_id: "host-tab-b", document_id: "db", generation: 3, input } });
  await assert.rejects(send({ kind: "text", text: "x" }), { code: "user_domain_sensitive_requires_focus" }, "a password field focused in the page");
  await send({ kind: "click", x: 520, y: 63 }); // the omnibox
  assert.deepEqual(await send({ kind: "text", text: "wiki" }), { dispatched: true });
  assert.deepEqual(await send({ kind: "key", key: "Backspace" }), { dispatched: true });
  assert.deepEqual(await send({ kind: "key", key: "Enter" }), { dispatched: true });
  await assert.rejects(send({ kind: "text", text: "x" }), { code: "user_domain_sensitive_requires_focus" }, "Enter hands focus back to the page");
  pageFocus = true;
  assert.deepEqual(await send({ kind: "key", key: "l", modifiers: 2 }), { dispatched: true }, "Ctrl+L");
  pageFocus = false;
  assert.deepEqual(await send({ kind: "text", text: "a" }), { dispatched: true }, "Ctrl+L focuses the omnibox");
  pageFocus = true;
  await assert.rejects(send({ kind: "text", text: "x" }), { code: "user_domain_sensitive_requires_focus" }, "the page took focus back");
  await send({ kind: "click", x: 300, y: 20 }); // a Tab in the strip: the page keeps its focus
  await assert.rejects(send({ kind: "text", text: "x" }), { code: "user_domain_sensitive_requires_focus" }, "a strip click does not focus Chrome's UI");
});
