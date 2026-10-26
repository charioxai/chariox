// MP-08/MP-10/MP-11 (§2b): native Chrome window binding and input arbiter.
import test from "node:test";
import assert from "node:assert/strict";
import { ChromeWindows, InputArbiter, OMNIBOX_LIST_HEIGHT, bandChord, refusedChord } from "./kernel-browser-chrome-window.mjs";
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
// MP-11 (r54): the first paint of a loading page changes its native rect (GTK insets, width) but not the band.
// The band's window generation and streams survive, so keys parked for the band are never made stale by it; every
// page-bound binding and stream is retired at once.
test("MP-11 a page-only geometry change keeps the band window generation and retires page-bound bindings", async t => {
  const viewport = { width: 1280, height: 800 };
  const rows = [{ id: 0x600001, x: 0, y: 0, width: 2560, height: 1774, visible: true, focused: true, override_redirect: false, transient_for: 0 }];
  const { host, windows } = fake({ rows, viewport });
  const real = new OwnedDisplay((await import("node:os")).tmpdir());
  try { await real.start(); } catch { return t.skip("no Xvfb"); }
  try {
    host.chromium.display = real;
    const bound = await windows.resolve();
    const band = windows.band(bound.tabbed), retired = []; windows.onRetire(event => retired.push(event));
    Object.assign(rows[0], { frame_insets: [4, 4, 0, 0] }); viewport.width = 1276;
    assert.equal(await bound.tabbed.isPageCurrent("tb"), false, "the page rect moved");
    await windows.resolving;
    const next = windows.current();
    assert.equal(next.tabbed.window_generation, bound.tabbed.window_generation, "same band, same window generation");
    assert.notDeepEqual(next.tabbed.page_rect, bound.tabbed.page_rect);
    assert.equal(band.isCurrent(), true, "the band source stays current");
    assert.equal(bound.tabbed.isCurrent(), false, "page-bound bindings and streams are retired");
    assert.equal(next.tabbed.isCurrent(), true);
    assert.deepEqual(retired.map(event => event.reason), ["page_geometry_changed"]);
    assert.equal(await next.tabbed.isPageCurrent("tb"), true);
    assert.equal(retired.length, 1, "one notification, not one per call");
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
    assert.deepEqual(windows.band(), { surface: "browser_chrome", window: { id: 0x600001, width: 2560, height: 1774 }, rect: { left: 0, top: 0, width: 2560, height: 174 }, contentHeight: 1600, isCurrent: bound.tabbed.isBandCurrent });
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
  host.displayTarget = async ({tab_id}) => host.tabs.get(tab_id);
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

test("MP-08/MP-10/MP-11 band preflight observes the live document before ledger admission", async () => {
  const cached = { tab_id: 't', target_id: 'target', document_id: 'old' };
  const calls = [], host = { generation: 3, tabs: new Map([['t', cached]]), tabGroups: { active: async () => 'target' },
    displayTarget: async command => { calls.push(command); return { ...cached, document_id: 'live' }; } };
  const binding = { window_generation: 2, isCurrent: () => true };
  const arbiter = new InputArbiter(host, { current: () => ({ tabbed: binding }) });
  assert.deepEqual(await arbiter.resolveDisplayInput({ window_generation: 2 }), { tab_id: 't', document_id: 'live', generation: 3 });
  assert.deepEqual(calls, [{ tab_id: 't', generation: 3, fresh: true }], 'input admission asks for a renderer proof');
});

test("MP-08/MP-10/MP-11 band preflight discovers a newly created native Tab", async () => {
  let discovers = 0;
  const tab = { tab_id: 'new', target_id: 'new-target', document_id: 'live' };
  const host = { generation: 3, tabs: new Map(), tabGroups: { active: async () => 'new-target' },
    reconcile: async () => { discovers++; host.tabs.set(tab.tab_id, tab); }, displayTarget: async () => tab };
  const binding = { window_generation: 2, isCurrent: () => true };
  const arbiter = new InputArbiter(host, { current: () => ({ tabbed: binding }) });
  assert.deepEqual(await arbiter.resolveDisplayInput({ window_generation: 2 }), { tab_id: 'new', document_id: 'live', generation: 3 });
  assert.equal(discovers, 1);
});

for (const retired of ['window', 'active-tab', 'generation']) test(`MP-11 band preflight rechecks ${retired} after its document read`, async () => {
  let live = true, active = 'target';
  const tab = { tab_id: 't', target_id: 'target', document_id: 'live' };
  const host = { generation: 3, tabs: new Map([['t', tab]]), tabGroups: { active: async () => active }, displayTarget: async () => {
    if (retired === 'window') live = false;
    if (retired === 'active-tab') active = 'other';
    if (retired === 'generation') host.generation++;
    return tab;
  } };
  const binding = { window_generation: 2, isCurrent: () => live };
  const arbiter = new InputArbiter(host, { current: () => ({ tabbed: binding }) });
  await assert.rejects(arbiter.resolveDisplayInput({ window_generation: 2 }), { code: 'user_domain_stale_reference' });
});

test("MP-11 band input: active Tab rechecked before dispatch, refused or retired input settles", async () => {
  let focus = false, document = "db";
  const evaluated = [];
  const connection = { send: async (method, params) => method === "Page.getFrameTree" ? { frameTree: { frame: { id: "f", loaderId: document } } }
    : method === "Runtime.evaluate" ? (evaluated.push(params), { result: { value: focus } }) : {} };
  const host = { generation: 3, tabs: new Map([["host-tab-b", { tab_id: "host-tab-b", target_id: "tb", document_id: "db" }]]), tabGroups: { active: async () => "tb" },
    browser: { inputCapture:{physicalFocus:async(_connection,_session,read)=>read()}, resolvePageTarget: async () => ({ connection, sessionId: "s" }), ensureFocusWorld: async () => ({ contextId: 77 }) } };
  const arbiter = new InputArbiter(host, {});
  const calls = [];
  const source = { surface: "browser_chrome", isCurrent: () => true, click: async (x, y) => (calls.push(["click", x, y]), true), wheel: async (...a) => (calls.push(["wheel", ...a]), true), chord: async (k, m) => (calls.push(["chord", k, m]), true) };
  const send = (input, extra = {}) => arbiter.input({ stream: { regionSource: source }, command: { tab_id: "host-tab-b", document_id: "db", generation: 3, input, ...extra } });
  assert.deepEqual(await send({ kind: "click", x: 300, y: 40 }), { dispatched: true });
  focus = true;
  assert.deepEqual(await send({ kind: "key", key: "l", modifiers: 2 }), { dispatched: true }, "Ctrl+L whatever the focus");
  focus = false;
  await assert.rejects(send({ kind: "key", key: "N", modifiers: 10 }), { code: "user_domain_not_granted" }, "Ctrl+Shift+N (incognito)");
  focus = true; arbiter.pageFocused(); // after Ctrl+L, a native click back into the page
  assert.deepEqual(await send({ kind: "text", text: "secret" }), { dispatched: true }, "A-L23: a person types into any focused field");
  assert.deepEqual(await send({ kind: "key", key: "Enter" }), { dispatched: true });
  assert.deepEqual(await send({ kind: "key", key: "v", modifiers: 2 }), { dispatched: true });
  focus = false;
  assert.deepEqual(await send({ kind: "text", text: "wiki" }), { dispatched: true });
  assert.deepEqual(await send({ kind: "key", key: "Enter" }), { dispatched: true });
  assert.deepEqual(await send({ kind: "scroll", x: 10, y: 10, delta_x: 0, delta_y: 120 }), { dispatched: true });
  await assert.rejects(send({ kind: "click", x: 1, y: 1 }, { tab_id: "host-tab-a" }), { code: "user_domain_stale_reference" }, "another Tab became active after admission");
  // Live drill r4band1: a strip click while the active Tab loads was dropped. Band clicks,
  // wheel and browser shortcuts reach only Chrome's UI (the crop holds no page pixels);
  // The owner's text also follows the native window; model observations keep their own document fences.
  document = "dc";
  assert.deepEqual(await send({ kind: "click", x: 1, y: 1 }), { dispatched: true }, "a strip click while the active Tab navigates");
  assert.deepEqual(await send({ kind: "scroll", x: 10, y: 10, delta_x: 0, delta_y: 120 }), { dispatched: true });
  assert.deepEqual(await send({ kind: "key", key: "t", modifiers: 2 }), { dispatched: true }, "Ctrl+T while the active Tab navigates");
  assert.deepEqual(await send({ kind: "text", text: "x" }), {dispatched:true}, "owner input survives the page commit after admission");
  assert.deepEqual(await send({ kind: "key", key: "Enter" }), {dispatched:true});
  document = "db";
  await assert.rejects(send({ kind: "click", x: 1, y: 1 }, { _agent_input: true }), { code: "user_domain_not_granted" });
  const retired = { ...source, click: async () => false };
  await assert.rejects(arbiter.input({ stream: { regionSource: retired }, command: { tab_id: "host-tab-b", document_id: "db", generation: 3, input: { kind: "click", x: 1, y: 1 } } }), { code: "user_domain_stale_reference" }, "a retired source settles as refused");
  const uncertain = { ...source, click: async () => { throw Object.assign(Error("x"), { code: "native_input_uncertain" }); } };
  assert.deepEqual(await arbiter.input({ stream: { regionSource: uncertain }, command: { tab_id: "host-tab-b", document_id: "db", generation: 3, input: { kind: "click", x: 1, y: 1 } } }), { dispatched: true }, "uncertain native input counts as dispatched, never replayed");
  await assert.rejects(send({ kind: "text", text: "é" }), /unsupported text/);
  assert.deepEqual(calls.map(c => c[0] === "chord" ? c[1] : c[0]), ["click", 0x6c, 0x73, 0x65, 0x63, 0x72, 0x65, 0x74, 0xff0d, 0x76, 0x77, 0x69, 0x6b, 0x69, 0xff0d, "wheel", "click", "wheel", 0x74, 0x78, 0xff0d]);
});

test("MP-08 browser chords and refused chords", () => {
  for (const [k, m] of [[0x6c, 2], [0x74, 2], [0x77, 2], [0xff09, 2], [0xff09, 10], [0x54, 10], [0xff51, 1], [0xff53, 1]]) assert(bandChord(k, m), `${k}/${m}`);
  for (const [k, m] of [[0x75, 2], [0x4e, 10], [0x6f, 2], [0xff1b, 8]]) assert(refusedChord(k, m), `${k}/${m}`); // view-source, incognito, open file, task manager
  // Miguel 2026-10-15: DevTools, print, save, downloads, history and clear data are allowed.
  for (const [k, m] of [[0x61, 0], [0x6c, 2], [0x63, 2], [0x76, 2], [0xffc9, 0], [0x49, 10], [0x4a, 10], [0x43, 10], [0x70, 2], [0x73, 2], [0x6a, 2], [0x68, 2], [0xffff, 10]]) assert(!refusedChord(k, m), `${k}/${m}`);
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
// culinux 12:10Z: focus emulation (on while a mirror is attached) makes document.hasFocus() always true.
test("MP-11 omnibox focus is told apart from the page's remembered sensitive focus", async () => {
  const connection = { send: async (method, params) => method === "Page.getFrameTree" ? { frameTree: { frame: { id: "f", loaderId: "db" } } }
    : method === "Runtime.evaluate" ? { result: { value: true } } : {} };
  const host = { generation: 3, tabs: new Map([["host-tab-b", { tab_id: "host-tab-b", target_id: "tb", document_id: "db" }]]), tabGroups: { active: async () => "tb" },
    browser: { inputCapture: { physicalFocus: async () => false /* emulated hasFocus() is always true; the physical read sees Chrome UI holding the keys */ }, resolvePageTarget: async () => ({ connection, sessionId: "s" }), ensureFocusWorld: async () => ({ contextId: 77 }) } };
  const arbiter = new InputArbiter(host, {}), calls = [];
  const source = { surface: "browser_chrome", isCurrent: () => true, click: async () => (calls.push("click"), true), chord: async k => (calls.push(k), true) };
  const send = input => arbiter.input({ stream: { regionSource: source }, command: { tab_id: "host-tab-b", document_id: "db", generation: 3, input } });
  assert.deepEqual(await send({ kind: "text", text: "x" }), { dispatched: true }, "a password field focused in the page");
  await send({ kind: "click", x: 520, y: 63 }); // the omnibox
  assert.deepEqual(await send({ kind: "key", key: "a", modifiers: 2 }), { dispatched: true }, "Ctrl+A selects the address");
  assert.deepEqual(await send({ kind: "text", text: "wiki" }), { dispatched: true });
  assert.deepEqual(await send({ kind: "key", key: "Backspace" }), { dispatched: true });
  assert.deepEqual(await send({ kind: "key", key: "Enter" }), { dispatched: true });
  assert.deepEqual(await send({ kind: "text", text: "x" }), { dispatched: true }, "Enter hands focus back to the page");
  assert.deepEqual(await send({ kind: "key", key: "l", modifiers: 2 }), { dispatched: true }, "Ctrl+L");
  assert.deepEqual(await send({ kind: "text", text: "a" }), { dispatched: true }, "Ctrl+L focuses the omnibox");
  arbiter.pageFocused();
  assert.deepEqual(await send({ kind: "text", text: "x" }), { dispatched: true }, "a native click into the page took focus back");
  await send({ kind: "click", x: 520, y: 63 }); await send({ kind: "key", key: "Tab" });
  assert.deepEqual(await send({ kind: "text", text: "x" }), { dispatched: true }, "Tab moves focus on");
  await send({ kind: "click", x: 300, y: 20 }); // a Tab in the strip: the page keeps its focus
  assert.deepEqual(await send({ kind: "text", text: "x" }), { dispatched: true }, "a strip click does not focus Chrome's UI");
});

// Protocol 540: the person's keys in the address bar (a click on it, Ctrl+L, typing there) keep its list
// stream warm; Enter, Tab, another toolbar button, the strip or another shortcut end it.
test("MP-08/MP-11 the arbiter tells the windows when the address bar has the keys", async () => {
  const connection = { send: async (method) => method === "Page.getFrameTree" ? { frameTree: { frame: { id: "f", loaderId: "db" } } }
    : method === "Runtime.evaluate" ? { result: { value: true } } : {} };
  const host = { generation: 3, tabs: new Map([["host-tab-b", { tab_id: "host-tab-b", target_id: "tb", document_id: "db" }]]), tabGroups: { active: async () => "tb" },
    browser: { inputCapture: { physicalFocus: async () => false }, resolvePageTarget: async () => ({ connection, sessionId: "s" }), ensureFocusWorld: async () => ({ contextId: 77 }) } };
  const focus = [];
  const arbiter = new InputArbiter(host, { focusOmnibox: on => focus.push(on), onOmnibox: x => x >= 300 && x < 1000 });
  const source = { surface: "browser_chrome", isCurrent: () => true, click: async () => true, chord: async () => true };
  const send = input => arbiter.input({ stream: { regionSource: source }, command: { tab_id: "host-tab-b", document_id: "db", generation: 3, input } });
  const step = async (input, expected, why) => { focus.length = 0; await send(input); assert.deepEqual(focus, [expected], why); };
  await step({ kind: "click", x: 520, y: 63 }, true, "a click on the address bar");
  await step({ kind: "text", text: "w" }, true, "typing there");
  await step({ kind: "key", key: "Enter" }, false, "Enter navigates");
  await step({ kind: "key", key: "l", modifiers: 2 }, true, "Ctrl+L");
  await step({ kind: "click", x: 80, y: 63 }, false, "Reload is not the address bar");
  await step({ kind: "key", key: "l", modifiers: 2 }, true);
  await step({ kind: "click", x: 300, y: 20 }, false, "a Tab in the strip");
  await step({ kind: "key", key: "l", modifiers: 2 }, true);
  await step({ kind: "key", key: "t", modifiers: 2 }, false, "another browser shortcut");
});

// r24 (cold first list): before a window's first list its address-bar span is unknown. A toolbar click may
// be the address bar, so the keys after it wait for the list they open; the click itself does not wait.
test("MP-08/MP-11 a toolbar click before the window's first list arms the list wait for the keys, not for the click", async () => {
  const connection = { send: async (method) => method === "Page.getFrameTree" ? { frameTree: { frame: { id: "f", loaderId: "db" } } }
    : method === "Runtime.evaluate" ? { result: { value: true } } : {} };
  const host = { generation: 3, tabs: new Map([["host-tab-b", { tab_id: "host-tab-b", target_id: "tb", document_id: "db" }]]), tabGroups: { active: async () => "tb" },
    browser: { inputCapture: { physicalFocus: async () => false }, resolvePageTarget: async () => ({ connection, sessionId: "s" }), ensureFocusWorld: async () => ({ contextId: 77 }) } };
  const focus = [];
  const arbiter = new InputArbiter(host, { focusOmnibox: (on, ago = 0) => focus.push([on, ago]), onOmnibox: () => null });
  const source = { surface: "browser_chrome", isCurrent: () => true, click: async () => true, chord: async () => true };
  const send = input => arbiter.input({ stream: { regionSource: source }, command: { tab_id: "host-tab-b", document_id: "db", generation: 3, input } });
  await send({ kind: "click", x: 520, y: 63 });
  assert.equal(focus.length, 1);
  assert.equal(focus[0][0], true, "the toolbar click may be the address bar");
  assert.ok(focus[0][1] < -250, "backdated past the list wait: the click's own reply does not wait for a list");
  focus.length = 0;
  await send({ kind: "text", text: "w" });
  assert.deepEqual(focus, [[true, 0]], "a key in the toolbar waits for the list it opens");
  const { windows } = fake();
  assert.equal(windows.onOmnibox(520), null, "no list seen in this window: the span is unknown, not 'not the address bar'");
});

// r24 (cold first list): the list rect Chrome showed in a window is remembered for the same X window, size, scale
// and Chrome process, so a window rebound at that geometry keeps its warm list; any other geometry starts unknown.
test("MP-08/MP-11 a window rebound at the same geometry keeps the list rect it proved, another geometry does not", () => {
  const { windows } = fake();
  const at = (generation, over = {}) => ({ browser_generation: 3, xid: 0x600001, window_generation: generation, scale: 1, content_top: 87, page_rect: { left: 0, top: 87, width: 1280, height: 800 }, bounds: { width: 1280, height: 887, ...over.bounds }, ...over });
  assert.equal(windows.omniboxRect(at(1)), null);
  windows.noteOmnibox(at(1), { x: 100, y: 87, width: 995, height: 344 });
  windows.omniboxLast = { x: 100, y: 87, width: 995, height: 344, window_generation: 1 };
  assert.deepEqual(windows.omniboxRect(at(2)), { x: 100, y: 87, width: 995, height: 344 }, "same window, size, scale and Chrome");
  assert.equal(windows.onOmnibox.call({ omniboxRect: () => windows.omniboxRect(at(2)) }, 500), true);
  assert.equal(windows.omniboxRect(at(2, { bounds: { width: 1280, height: 700 } })), null, "another size");
  assert.equal(windows.omniboxRect(at(2, { scale: 2 })), null, "another scale");
  assert.equal(windows.omniboxRect(at(2, { xid: 0x600002 })), null, "another X window");
  assert.equal(windows.omniboxRect(at(2, { browser_generation: 4 })), null, "another Chrome process");
  assert.equal(windows.omniboxRect(at(2, { content_top: 127, page_rect: { left: 0, top: 127, width: 1280, height: 760 } })), null, "same outer window, another page boundary");
});

// MP-08/MP-10/MP-11: every attached mirror keeps focus emulation enabled.
// A native omnibox check must observe physical focus, not that logical wake-up.
test('MP-08/MP-10 owner UI typing does not wait for or change held mirror focus',async()=>{
 const {BrowserInputCapture}=await import('./browser-controller-input.mjs');
 const capture=new BrowserInputCapture();let pageFocus=true,emulated=false;
 const connection={send:async(method,p)=>{
  if(method==='Page.getFrameTree')return{frameTree:{frame:{id:'f',loaderId:'doc'}}};
  if(method==='Emulation.setFocusEmulationEnabled'){emulated=p.enabled;return{}};
  return{result:{value:p.expression==='document.hasFocus()'?pageFocus||emulated:true}};
 }};
 const tab={tab_id:'tab',target_id:'target',document_id:'doc'};
 const host={generation:3,tabs:new Map([['tab',tab]]),tabGroups:{active:async()=> 'target'},browser:{inputCapture:capture,resolvePageTarget:async()=>({connection,sessionId:'s'}),ensureFocusWorld:async()=>({contextId:77})}};
 const arbiter=new InputArbiter(host,{}),source={surface:'browser_chrome',isCurrent:()=>true,chord:async()=>{pageFocus=false;return true}};
 const send=input=>arbiter.input({stream:{regionSource:source},command:{tab_id:'tab',document_id:'doc',generation:3,input}});
 const release=await capture.hold(connection,'s');
 try{
  await send({kind:'key',key:'l',modifiers:2});assert.equal(emulated,true);
  assert.deepEqual(await send({kind:'text',text:'wiki'}),{dispatched:true});
  assert.equal(emulated,true,'native owner input does not read or mutate the mirror focus emulation');
  pageFocus=true;assert.deepEqual(await send({kind:'text',text:'x'}),{dispatched:true},'A-L23: page focus on any field is the person\'s typing');
 }finally{await release()}
});

// Protocol 530 (BROWSER-UX-14): the band behaves like Chromium's own strip: middle-click closes a Tab,
// right-click opens Chrome's context menu, a drag inside the strip reorders Tabs. Popup windows (menus,
// the omnibox dropdown, extension popups) take clicks, wheel and keys; only the band takes buttons/drags.
test("MP-08/MP-11 band middle/right click and strip drag; popups take primary input only", async () => {
  const connection = { send: async (method) => method === "Page.getFrameTree" ? { frameTree: { frame: { id: "f", loaderId: "db" } } }
    : method === "Runtime.evaluate" ? { result: { value: false } } : {} };
  const host = { generation: 3, tabs: new Map([["host-tab-b", { tab_id: "host-tab-b", target_id: "tb", document_id: "db" }]]), tabGroups: { active: async () => "tb" },
    browser: { inputCapture:{physicalFocus:async(_connection,_session,read)=>read()}, resolvePageTarget: async () => ({ connection, sessionId: "s" }), ensureFocusWorld: async () => ({ contextId: 77 }) } };
  const arbiter = new InputArbiter(host, {}), calls = [];
  const source = surface => ({ surface, isCurrent: () => true, click: async (...a) => (calls.push([surface, "click", ...a]), true),
    drag: async (...a) => (calls.push([surface, "drag", ...a]), true), wheel: async (...a) => (calls.push([surface, "wheel", ...a]), true), chord: async (...a) => (calls.push([surface, "chord", ...a]), true) });
  const send = (surface, input, extra = {}) => arbiter.input({ stream: { regionSource: source(surface) }, command: { tab_id: "host-tab-b", document_id: "db", generation: 3, input, ...extra } });
  assert.deepEqual(await send("browser_chrome", { kind: "click", x: 120, y: 20, button: "middle" }), { dispatched: true });
  assert.deepEqual(await send("browser_chrome", { kind: "click", x: 120, y: 20, button: "right" }), { dispatched: true });
  assert.deepEqual(await send("browser_chrome", { kind: "drag", x: 120, y: 20, to_x: 380, to_y: 22 }), { dispatched: true });
  await assert.rejects(send("browser_chrome", { kind: "drag", x: 120, y: 20, to_x: 380, to_y: 60 }), /unsupported input/, "leaving the strip would tear the Tab into a new window");
  await assert.rejects(send("browser_chrome", { kind: "drag", x: 120, y: 60, to_x: 380, to_y: 20 }), /unsupported input/);
  await assert.rejects(send("browser_chrome", { kind: "click", x: 1, y: 1, button: "left" }), /unsupported input/);
  await assert.rejects(send("browser_chrome", { kind: "click", x: 1, y: 1, button: "middle" }, { _agent_input: true }), { code: "user_domain_not_granted" });
  assert.deepEqual(await send("browser_auxiliary", { kind: "click", x: 30, y: 40 }), { dispatched: true }, "a menu item");
  assert.deepEqual(await send("browser_auxiliary", { kind: "scroll", x: 30, y: 40, delta_x: 0, delta_y: 120 }), { dispatched: true });
  assert.deepEqual(await send("browser_auxiliary", { kind: "key", key: "Escape" }), { dispatched: true });
  await assert.rejects(send("browser_auxiliary", { kind: "click", x: 30, y: 40, button: "right" }), /unsupported input/);
  await assert.rejects(send("browser_auxiliary", { kind: "drag", x: 1, y: 1, to_x: 2, to_y: 2 }), /unsupported input/);
  await assert.rejects(send("page", { kind: "click", x: 1, y: 1 }), { code: "user_domain_stale_reference" }, "a page region is not Chrome's UI");
  assert.deepEqual(calls.map(c => c.slice(0, 2).concat(c[1] === "click" ? [c[4]] : [])), [["browser_chrome", "click", 2], ["browser_chrome", "click", 3], ["browser_chrome", "drag"],
    ["browser_auxiliary", "click", 1], ["browser_auxiliary", "wheel"], ["browser_auxiliary", "chord"]]);
});

test("MP-08/MP-11 popups of the band's window get kernel ids, follow the band's inventory and retire", async t => {
  const { host, windows } = fake();
  const real = new OwnedDisplay((await import("node:os")).tmpdir());
  try { await real.start(); } catch { return t.skip("no Xvfb"); }
  try {
    host.chromium.display = real;
    const bound = await windows.resolve();
    const tabbed = { id: 0x600001, x: 0, y: 0, width: 2560, height: 1774, visible: true, focused: true, override_redirect: false, transient_for: 0 };
    const [dropdown] = bound.auxiliary;
    assert.deepEqual(windows.auxiliaryState(), [{ auxiliary_id: dropdown.auxiliary_id, x: 100, y: 80, width: 800, height: 200 }], "CSS px from the window's top-left");
    const menu = { id: 0x600070, x: 2000, y: 140, width: 497, height: 801, visible: true, focused: false, override_redirect: true, transient_for: 0 };
    const foreign = { id: 0x700001, x: 2600, y: 0, width: 50, height: 50, visible: true, focused: false, override_redirect: true, transient_for: 0 };
    windows.refreshAuxiliary([tabbed, { id: 0x600050, x: 200, y: 160, width: 1600, height: 401, visible: true, focused: false, override_redirect: true, transient_for: 0 }, menu, foreign]);
    const state = windows.auxiliaryState();
    assert.equal(state[0].auxiliary_id, dropdown.auxiliary_id, "an unchanged popup keeps its id");
    assert.deepEqual(state[1], { auxiliary_id: state[1].auxiliary_id, x: 1000, y: 70, width: 248, height: 400 });
    assert.equal(state.length, 2, "a window outside the band's window is not its popup");
    const binding = windows.auxiliary(state[1].auxiliary_id);
    assert.deepEqual(windows.auxiliaryRegion(binding).rect, { left: 0, top: 0, width: 496, height: 800 });
    windows.refreshAuxiliary([tabbed, { ...menu, height: 900 }]);
    assert.equal(binding.isCurrent(), false, "a resized popup retires its stream (fixed-size video)");
    assert.notEqual(windows.auxiliaryState()[0].auxiliary_id, state[1].auxiliary_id);
    assert.equal(dropdown.isCurrent(), false, "a closed popup is retired");
    assert.throws(() => windows.auxiliary(state[1].auxiliary_id), { code: "user_domain_stale_reference" });
    // r9 hosted drill: a native page capture fits Chrome's window (fitNativeViewport); the band kept the
    // old geometry, so every band readback was refused ("native_region_readback_unavailable") for good.
    const generation = windows.current().tabbed.window_generation;
    windows.refreshAuxiliary([{ ...tabbed, width: 2000 }]);
    assert.throws(() => windows.current(), { code: "user_domain_stale_reference" }, "a resized tabbed window retires the band at once");
    await windows.resolving;
    assert.notEqual(windows.current().tabbed.window_generation, generation, "...and is rebound at its real geometry");
    let reads = 0;
    const stop = windows.watch(async () => (reads++, [tabbed]), 5);
    await new Promise(resolve => setTimeout(resolve, 40));
    stop();
    assert(reads >= 2 && windows.auxiliaryState().length === 0, "a watched band follows its popups");
    const after = reads; await new Promise(resolve => setTimeout(resolve, 20));
    assert.equal(reads, after, "the last viewer stops the watch");
    host.generation = 4;
    assert.deepEqual(windows.auxiliaryState(), [], "no popups without a current band window");
  } finally { await real.close(); }
});

// r12 quiet host: the band's native capture retired (a transient window size change) while the window
// stayed bound; the viewer subscribed a new band, but the poll kept reading the dead first band, so a
// closed Tab menu stayed listed and the omnibox dropdown never appeared.
test("MP-08/MP-11 popups follow a live band when an earlier band's reader has died", async t => {
  const { host, windows } = fake();
  const real = new OwnedDisplay((await import("node:os")).tmpdir());
  try { await real.start(); } catch { return t.skip("no Xvfb"); }
  try {
    host.chromium.display = real;
    await windows.resolve();
    const tabbed = { id: 0x600001, x: 0, y: 0, width: 2560, height: 1774, visible: true, focused: true, override_redirect: false, transient_for: 0 };
    const menu = { id: 0x600070, x: 2000, y: 140, width: 497, height: 801, visible: true, focused: false, override_redirect: true, transient_for: 0 };
    let rows = [tabbed, menu];
    const stopDead = windows.watch(async () => { throw Error("MP-11: native window inventory lease"); }, 5);
    const stopLive = windows.watch(async () => rows, 5);
    try {
      await new Promise(resolve => setTimeout(resolve, 40));
      assert.deepEqual(windows.auxiliaryState().map(({x, y}) => [x, y]), [[1000, 70]], "the live band's menu is listed");
      rows = [tabbed];
      await new Promise(resolve => setTimeout(resolve, 40));
      assert.deepEqual(windows.auxiliaryState(), [], "the closed menu is retired");
    } finally { stopDead(); stopLive(); }
  } finally { await real.close(); }
});

// r17 quiet host: a menu stayed listed after a long sequence with every band reader gone. Fixed-label marks
// (no data) name that state: all watched bands unreadable (once per streak) and each popup count change.
test("MP-08/MP-11 popup tracking marks when no band can be read and when the popup count changes", async t => {
  const { host, windows } = fake();
  const real = new OwnedDisplay((await import("node:os")).tmpdir());
  try { await real.start(); } catch { return t.skip("no Xvfb"); }
  try {
    host.chromium.display = real;
    const marks = []; host.timing = label => marks.push(label);
    await windows.resolve();
    const tabbed = { id: 0x600001, x: 0, y: 0, width: 2560, height: 1774, visible: true, focused: true, override_redirect: false, transient_for: 0 };
    const menu = { id: 0x600070, x: 2000, y: 140, width: 497, height: 801, visible: true, focused: false, override_redirect: true, transient_for: 0 };
    let rows = [tabbed, menu], dead = false;
    const stop = windows.watch(async () => { if (dead) throw Error("MP-11: native window inventory lease"); return rows; }, 5);
    try {
      await new Promise(resolve => setTimeout(resolve, 40));
      rows = [tabbed]; await new Promise(resolve => setTimeout(resolve, 40));
      dead = true; await new Promise(resolve => setTimeout(resolve, 60));
      assert.deepEqual(marks.filter(label => label.startsWith("aux_")), ["aux_count 1", "aux_count 0", "aux_poll_unreadable"], "one mark per streak");
    } finally { stop(); }
  } finally { await real.close(); }
});

// BROWSER-UX-14 (accbrowser r1: a menu stayed listed 4-5 s after its item was clicked): with no band able to read the
// inventory, a state read re-reads the owner's own inventory, so a popup Chrome closed no longer stays listed.
test("MP-08/MP-11 a state read retires a closed popup when no band can read the inventory", async t => {
  const tabbed = { id: 0x600001, x: 0, y: 0, width: 2560, height: 1774, visible: true, focused: true, override_redirect: false, transient_for: 0 };
  const menu = { id: 0x600070, x: 2000, y: 140, width: 497, height: 801, visible: true, focused: false, override_redirect: true, transient_for: 0 };
  const rows = [tabbed, menu];
  const { host, windows } = fake({ rows });
  const real = new OwnedDisplay((await import("node:os")).tmpdir());
  try { await real.start(); } catch { return t.skip("no Xvfb"); }
  try {
    host.chromium.display = real;
    await windows.resolve();
    assert.deepEqual(windows.auxiliaryState().map(({ x, y }) => [x, y]), [[1000, 70]], "the menu is listed");
    rows.splice(1, 1);
    const woken = []; host.stateSignal = { notify: () => woken.push(1) };
    const stopDead = windows.watch(async () => { throw Error("MP-11: native window inventory lease"); }, 5);
    try {
      await new Promise(resolve => setTimeout(resolve, 30));
      assert.equal(windows.auxiliaryState().length, 1, "no band reads: the closed menu is still listed");
      await windows.refreshStale();
      assert.deepEqual(windows.auxiliaryState(), [], "the owner's inventory retires it");
      assert.equal(woken.length, 1, "a state reader parked on the popup list wakes when the stale popup is retired");
      await windows.omniboxShown();
      assert.deepEqual(windows.auxiliaryState(), [], "the cached inventory no longer lists it, so the next refresh cannot bring it back");
    } finally { stopDead(); }
  } finally { await real.close(); }
});

// MP-08/MP-10/MP-11: Caps Lock/automation can report upper-case key labels
// without Shift. Native X11 must receive the caller's explicit modifier chord.
test('MP-08/MP-10/MP-11 band accelerators preserve explicit modifiers despite upper-case labels', async () => {
  const connection = {send:async method=>method==='Page.getFrameTree'?{frameTree:{frame:{id:'f',loaderId:'doc'}}}:{result:{value:false}}};
  const host={generation:1,tabs:new Map([['tab',{tab_id:'tab',target_id:'target',document_id:'doc'}]]),tabGroups:{active:async()=> 'target'},
    browser:{resolvePageTarget:async()=>({connection,sessionId:'s'}),ensureFocusWorld:async()=>({contextId:1})}};
  const arbiter=new InputArbiter(host,{}),calls=[];
  const source={surface:'browser_chrome',isCurrent:()=>true,chord:async(k,m)=>(calls.push([k,m]),true)};
  const send=input=>arbiter.input({stream:{regionSource:source},command:{tab_id:'tab',document_id:'doc',generation:1,input}});
  for(const [key,modifiers,expected] of [['A',2,0x61],['Z',10,0x7a],['I',2,0x69],['A',1,0x61],['A',4,0x61],['ArrowLeft',2,0xff51]]){
    await send({kind:'key',key,modifiers});assert.deepEqual(calls.at(-1),[expected,modifiers],`${key}/${modifiers} must not acquire implicit Shift`);
  }
  await send({kind:'text',text:'A'});assert.deepEqual(calls.at(-1),[0x41,0],'plain text retains its requested case');
  await assert.rejects(send({kind:'key',key:'N',modifiers:10}),{code:'user_domain_not_granted'},'explicit Ctrl+Shift+N remains refused');
});

// Coordinator 12:22Z ask 1: Chrome 155 draws the omnibox dropdown inside its main window (no X window).
// Real Chrome 155 probe (r8/overlay-probe/probe3*.json): the popup WebUI reports
// ["visible", screenX 113, screenY 35, list 1078x90, widget 1110x178] for a window at (10,10) DIP,
// list body pixels end 28 DIP above the widget bottom and sit 16 DIP inside its sides (DPR 1 and 2).
test("MP-08/MP-11 the omnibox dropdown drawn in the window is one more popup below the band", async t => {
  const { host, windows } = fake();
  const real = new OwnedDisplay((await import("node:os")).tmpdir());
  try { await real.start(); } catch { return t.skip("no Xvfb"); }
  try {
    host.chromium.display = real;
    await windows.resolve();
    const tabbed = { id: 0x600001, x: 0, y: 0, width: 2560, height: 1774, visible: true, focused: true, override_redirect: false, transient_for: 0 };
    windows.omnibox = ["visible", 103, 25, 1078, 90, 1110, 178];
    windows.refreshAuxiliary([tabbed]);
    const [popup] = windows.auxiliaryState();
    assert.deepEqual(popup, { auxiliary_id: popup.auxiliary_id, x: 119, y: 87, width: 1078, height: 88 }, "the list below the band, CSS px");
    const region = windows.auxiliaryRegion(windows.auxiliary(popup.auxiliary_id));
    assert.equal(region.inWindow, true); assert.equal(region.window.id, 0x600001, "a rect of the tabbed window itself");
    assert.deepEqual(region.pageRect, windows.current().tabbed.page_rect, "over the page: its masks use the page origin");
    // r11: the list's stream opens at the list's full height (Chrome 155: at most 8 rows of 40 DIP), so a
    // growing list never replaces it; each new stream left the person without a list for 0.4-1.3 s while typing.
    assert.deepEqual(region.rect, { left: 238, top: 174, width: 2156, height: 2 * OMNIBOX_LIST_HEIGHT });
    assert.equal(region.masks, undefined, "around its rounded corners are page pixels under the active Tab's masks");
    const grown = windows.auxiliary(popup.auxiliary_id);
    windows.omnibox = ["visible", 103, 25, 1078, 130, 1110, 218]; windows.refreshAuxiliary([tabbed]);
    assert.equal(grown.isCurrent(), true, "more suggestions: the same popup");
    assert.equal(windows.auxiliaryState()[0].height, 128);
    windows.omnibox = ["visible", 103, 25, 1078, 90, 1110, 178]; windows.refreshAuxiliary([tabbed]);
    assert.equal(grown.isCurrent(), true, "a shorter list keeps the same popup");
    // r10final 1920x1080@2: the viewer shows only the live list; its stream keeps its size (no page copy below it).
    assert.equal(windows.auxiliaryState()[0].height, 88, "the live list's height");
    assert.equal(windows.auxiliaryRegion(grown).rect.height, 2 * OMNIBOX_LIST_HEIGHT, "the stream keeps its size");
    windows.host.arbiter = { chromeFocus: { generation: windows.host.generation, window_generation: windows.bound.tabbed.window_generation } };
    windows.readOmnibox = async () => ["visible", 103, 25, 1078, 90, 1110, 178];
    const source = { isCurrent: grown.isCurrent, region: { window_region: windows.auxiliaryRegion(grown).rect } };
    assert.equal(await windows.inWindowCurrent(source, { x: 40, y: 80 }), true, "on the live list");
    assert.equal(await windows.inWindowCurrent(source, { x: 40, y: 100 }), false, "below it is the page: refused");
    assert.equal(await windows.inWindowCurrent(source), true, "keys go to Chrome's UI");
    delete windows.readOmnibox;
    windows.omnibox = ["hidden", 103, 25, 1078, 1, 1110, 178]; windows.refreshAuxiliary([tabbed]);
    assert.deepEqual(windows.auxiliaryState(), [], "closed");
    for (const state of [["visible", 103, 25, 1078, 90, 1120, 178], ["visible", 103, 100, 1078, 90, 1110, 178], ["visible", 103, 25, 1278, 90, 1310, 178], null]) {
      windows.omnibox = state; windows.refreshAuxiliary([tabbed]);
      assert.deepEqual(windows.auxiliaryState(), [], `not the pinned layout or not over the page: ${JSON.stringify(state)}`);
    }
    // Review #1016 P1: input re-reads the dropdown; a closed one retires before the next poll.
    windows.omnibox = ["visible", 103, 25, 1078, 90, 1110, 178]; windows.refreshAuxiliary([tabbed]);
    const open = windows.auxiliary(windows.auxiliaryState()[0].auxiliary_id);
    windows.host.arbiter = { chromeFocus: { generation: windows.host.generation, window_generation: windows.bound.tabbed.window_generation } };
    windows.readOmnibox = async () => ["visible", 103, 25, 1078, 90, 1110, 178];
    assert.equal(await windows.inWindowCurrent(open), true, "still open: admitted");
    windows.readOmnibox = async () => ["hidden", 103, 25, 1078, 1, 1110, 178];
    assert.equal(await windows.inWindowCurrent(open), false, "closed: refused");
    assert.deepEqual(windows.auxiliaryState(), [], "and retired at once");
  } finally { await real.close(); }
});

// Protocol 540 (r12 quiet pair: list first paint ~0.9 s vs Selkies ~0.16 s, mostly a new stream per open):
// while the person's keys are in the address bar, the last list's stream stays listed with no visible
// height under the same kernel id, so the viewer keeps it subscribed and the next list paints from it.
test("MP-08/MP-11 the omnibox list stream stays warm while the address bar has focus", async t => {
  const { host, windows } = fake();
  const real = new OwnedDisplay((await import("node:os")).tmpdir());
  try { await real.start(); } catch { return t.skip("no Xvfb"); }
  try {
    host.chromium.display = real;
    await windows.resolve();
    const tabbed = { id: 0x600001, x: 0, y: 0, width: 2560, height: 1774, visible: true, focused: true, override_redirect: false, transient_for: 0 };
    const open = ["visible", 103, 25, 1078, 90, 1110, 178], closed = ["hidden", 103, 25, 1078, 1, 1110, 178];
    const show = state => { windows.omnibox = state; windows.refreshAuxiliary([tabbed]); return windows.auxiliaryState(); };
    windows.focusOmnibox(false);
    assert.deepEqual(show(closed), [], "no list seen yet: nothing to keep warm");
    windows.focusOmnibox(true);
    assert.deepEqual(show(closed), [], "a list never seen in this window has no rect yet");
    const [list] = show(open);
    assert.equal(list.height, 88);
    const binding = windows.auxiliary(list.auxiliary_id);
    assert.deepEqual(show(closed), [{ ...list, height: 0 }], "closed while focused: same id, nothing visible");
    assert.equal(binding.isCurrent(), true, "its stream stays current");
    assert.equal(windows.auxiliaryRegion(binding).rect.height, 2 * OMNIBOX_LIST_HEIGHT);
    assert.equal(windows.inWindowOver({ left: 0, top: 174, width: 2560, height: 1600 }), false, "a closed list covers no page region");
    windows.readOmnibox = async () => closed;
    const source = { isCurrent: binding.isCurrent, region: { window_region: windows.auxiliaryRegion(binding).rect } };
    assert.equal(await windows.inWindowCurrent(source, { x: 40, y: 20 }), false, "a click on the warm stream reaches nothing");
    assert.equal(await windows.inWindowCurrent(source), false, "a warm capture without a live Chrome list grants no keyboard exemption");
    windows.readOmnibox = async () => open;
    await windows.refreshWarm();
    assert.deepEqual(windows.auxiliaryState(), [list], "a state read sees the list Chrome just opened, on the same stream");
    // r13 b3: the read that follows a key often arrived before Chrome opened the list (then a 250 ms poll).
    show(closed); windows.focusOmnibox(true);
    let reads = 0; windows.readOmnibox = async () => ++reads >= 3 ? open : closed;
    await windows.refreshWarm();
    assert.deepEqual(windows.auxiliaryState(), [list], "a read right after a key waits briefly for the list");
    show(closed); windows.focusOmnibox(true, -1000);
    reads = 0; windows.readOmnibox = async () => (++reads, closed);
    await windows.refreshWarm();
    assert.equal(reads, 1, "no wait long after the last key");
    windows.focusOmnibox(true);
    delete windows.readOmnibox;
    assert.deepEqual(show(closed), [{ ...list, height: 0 }]);
    assert.deepEqual(show(open), [list], "the next list paints from the same stream");
    windows.focusOmnibox(false);
    assert.deepEqual(show(closed), [], "focus left: the warm stream retires");
    assert.equal(binding.isCurrent(), false);
    windows.focusOmnibox(true, -60_000);
    assert.deepEqual(show(closed), [], "an address bar idle for a minute is not kept warm");
    windows.focusOmnibox(true);
    host.generation = 4;
    assert.deepEqual(windows.auxiliaryState(), [], "no warm list without a current window");
  } finally { await real.close(); }
});

test("MP-11 the kernel reads only the omnibox popup's visibility and geometry", async () => {
  const sent = [];
  const connection = { send: async (method, params) => {
    sent.push([method, params]);
    if (method === "Target.getTargets") return { targetInfos: [{ type: "browser_ui", targetId: "aim", url: "chrome://omnibox-popup.top-chrome/omnibox_popup_aim.html" }, { type: "browser_ui", targetId: "pop", url: "chrome://omnibox-popup.top-chrome/" }] };
    if (method === "Target.attachToTarget") return { sessionId: "ps" };
    if (method === "Runtime.evaluate") return { result: { value: '["visible",113,35,1078,90,1110,178]' } };
    return {};
  } };
  const windows = new ChromeWindows({ browser: { connection } });
  assert.deepEqual(await windows.readOmnibox(), ["visible", 113, 35, 1078, 90, 1110, 178]);
  assert.deepEqual(sent[1], ["Target.attachToTarget", { targetId: "pop", flatten: true }]);
  assert.equal(sent[2][1].expression, "JSON.stringify([document.visibilityState,screenX,screenY,innerWidth,innerHeight,outerWidth,outerHeight])");
  await windows.readOmnibox();
  assert.equal(sent.filter(([method]) => method === "Target.attachToTarget").length, 1, "one session, reused");
});

// Coordinator 12:22Z ask 1: the Web Store's browser-modal 'Add "…"?' dialog dims the whole window (real
// Chrome 155: the band's toolbar row 255 -> 102) and hangs from the toolbar; no X window or CDP event.
test("MP-08/MP-11 a scrim over the band's toolbar row offers the page area as one more popup until it clears", async t => {
  const { host, windows } = fake();
  const real = new OwnedDisplay((await import("node:os")).tmpdir());
  try { await real.start(); } catch { return t.skip("no Xvfb"); }
  try {
    host.chromium.display = real;
    await windows.resolve();
    windows.refreshAuxiliary([{ id: 0x600001, x: 0, y: 0, width: 2560, height: 1774, visible: true, focused: true, override_redirect: false, transient_for: 0 }]);
    const width = 2560, frame = (value, dialog = null, strip = value) => {
      const pixels = Buffer.alloc(width * 174 * 4, value);
      pixels.fill(strip, 0, width * 40 * 4); // the tab strip rows
      if (dialog) for (let y = 160; y < 174; y++) pixels.fill(255, (y * width + dialog[0]) * 4, (y * width + dialog[1]) * 4);
      return { width, height: 174, readRegion: (x, y, w, h) => Buffer.from(pixels.subarray((y * width + x) * 4, (y * width + x + w) * 4)) };
    };
    windows.observeBand(frame(255));
    assert.deepEqual(windows.auxiliaryState(), [], "clear toolbar");
    windows.observeBand(frame(242));
    assert.deepEqual(windows.auxiliaryState(), [], "a light change (the dropdown's shadow) is not a scrim");
    windows.observeBand(frame(90, null, 255));
    assert.deepEqual(windows.auxiliaryState(), [], "a dark toolbar row under a clear tab strip (the omnibox list) is not a scrim");
    windows.observeBand(frame(102, [832, 1728]));
    const [overlay] = windows.auxiliaryState();
    assert.deepEqual(overlay, { auxiliary_id: overlay.auxiliary_id, x: 0, y: 87, width: 1280, height: 800 }, "the whole page area below the band");
    const region = windows.auxiliaryRegion(windows.auxiliary(overlay.auxiliary_id));
    assert.equal(region.pageOverlay, true); assert.equal(region.window.id, 0x600001);
    assert.deepEqual(region.rect, { left: 0, top: 174, width: 2560, height: 1600 });
    windows.observeBand(frame(255));
    assert.deepEqual(windows.auxiliaryState(), [], "the dialog closed");
    windows.observeBand(frame(102)); windows.relearnToolbar();
    assert.deepEqual(windows.auxiliaryState(), [], "no scrim found behind it (a darker theme): learned again");
    windows.observeBand(frame(102));
    assert.deepEqual(windows.auxiliaryState(), [], "the darker toolbar is the new clear colour");
  } finally { await real.close(); }
});

// Review #1016 @4e2aaff3d P2: a password field (or iframe) keeps the page's activeElement; the person opens
// the Tab's context menu in the band. While Chrome's menu owns the keys, ArrowDown/Enter/Escape on it must
// reach it; keys for the page (band text with no Chrome UI focus) keep the sensitive-field fence.
test("MP-11 keys on an admitted native popup reach Chrome's UI whatever the page remembers", async () => {
  const connection = { send: async (method) => method === "Page.getFrameTree" ? { frameTree: { frame: { id: "f", loaderId: "db" } } }
    : method === "Runtime.evaluate" ? { result: { value: true } } : {} };
  const host = { generation: 3, tabs: new Map([["host-tab-b", { tab_id: "host-tab-b", target_id: "tb", document_id: "db" }]]), tabGroups: { active: async () => "tb" },
    browser: { resolvePageTarget: async () => ({ connection, sessionId: "s" }), ensureFocusWorld: async () => ({ contextId: 77 }) } };
  const arbiter = new InputArbiter(host, {}), keys = [];
  let open = true;
  const band = { surface: "browser_chrome", isCurrent: () => true, click: async () => true, chord: async k => (keys.push(["band", k]), true) };
  const menu = { surface: "browser_auxiliary", isCurrent: () => open, click: async () => true, chord: async k => (keys.push(["menu", k]), true) };
  const send = (source, input) => arbiter.input({ stream: { regionSource: source }, command: { tab_id: "host-tab-b", document_id: "db", generation: 3, input } });
  await send(band, { kind: "click", x: 300, y: 20, button: "right" }); // the Tab's context menu
  for (const key of ["ArrowDown", "Enter", "Escape"]) assert.deepEqual(await send(menu, { kind: "key", key }), { dispatched: true }, key);
  assert.deepEqual(await send(band, { kind: "text", text: "x" }), { dispatched: true }, "band text reaches the page");
  open = false;
  await assert.rejects(send(menu, { kind: "key", key: "Enter" }), { code: "user_domain_stale_reference" }, "a closed menu takes nothing");
  assert.deepEqual(keys.map(k => k[0]), ["menu", "menu", "menu", "band"]);
});

// Review #1016 @53ad3e893 P1: the omnibox dropdown is drawn in the tabbed window and closes without an X event;
// its rect is then the page itself. Input on it re-reads Chrome's dropdown before dispatch, never the last poll.
test("MP-11 input on a closed in-window dropdown is refused before the next poll", async () => {
  const host = { generation: 3, tabs: new Map([["host-tab-b", { tab_id: "host-tab-b", target_id: "tb", document_id: "db" }]]), tabGroups: { active: async () => "tb" } };
  let live = true, reads = 0;
  const windows = { inWindowCurrent: async source => (reads++, live && source.isCurrent()) };
  const arbiter = new InputArbiter(host, windows), sent = [];
  const dropdown = { surface: "browser_auxiliary", overPage: true, isCurrent: () => true, click: async (x, y) => (sent.push(["click", x, y]), true), chord: async k => (sent.push(["key", k]), true) };
  const send = input => arbiter.input({ stream: { regionSource: dropdown }, command: { tab_id: "host-tab-b", document_id: "db", generation: 3, input } });
  assert.deepEqual(await send({ kind: "click", x: 40, y: 20 }), { dispatched: true }, "an open dropdown takes a click");
  live = false; // Escape closed it; the 150 ms poll has not run yet
  await assert.rejects(send({ kind: "click", x: 40, y: 20 }), { code: "user_domain_stale_reference" }, "the page under it takes nothing");
  await assert.rejects(send({ kind: "key", key: "Enter" }), { code: "user_domain_stale_reference" });
  assert.deepEqual(sent, [["click", 40, 20]]);
  assert.equal(reads, 3, "every input re-reads the dropdown");
});

test("MP-08/MP-11 a page region under in-window Chrome UI is known by geometry only", async t => {
  const { windows } = fake();
  const real = new OwnedDisplay((await import("node:os")).tmpdir());
  try { await real.start(); } catch { return t.skip("no Xvfb"); }
  try {
    windows.host.chromium.display = real;
    await windows.resolve();
    const tabbed = { id: 0x600001, x: 0, y: 0, width: 2560, height: 1774, visible: true, focused: true, override_redirect: false, transient_for: 0 };
    const video = { left: 1900, top: 220, width: 500, height: 280 };
    assert.equal(windows.inWindowOver(video), false, "no dropdown");
    windows.omnibox = ["visible", 103, 25, 1078, 90, 1110, 178]; windows.refreshAuxiliary([tabbed]); // list x 238..2394, y 174..350 (physical)
    assert.equal(windows.inWindowOver(video), true, "the list covers the video's top rows");
    assert.equal(windows.inWindowOver({ ...video, top: 360 }), false, "below the list");
    assert.equal(windows.inWindowOver({ ...video, left: 2400 }), false, "beside it");
    // r11: the list's stream is taller than a shorter list; the page below the LIVE list keeps streaming.
    windows.omnibox = ["visible", 103, 25, 1078, 50, 1110, 138]; windows.refreshAuxiliary([tabbed]); // list y 174..270
    assert.equal(windows.inWindowOver({ ...video, top: 290 }), false, "below a shorter list, inside its stream");
    assert.equal(windows.inWindowOver({ ...video, top: 250 }), true);
    windows.omnibox = ["hidden", 103, 25, 1078, 1, 1110, 178]; windows.refreshAuxiliary([tabbed]);
    assert.equal(windows.inWindowOver(video), false, "closed");
    // r10 union2: Chrome paints the list before the 150 ms poll sees it; a readback checks it live.
    windows.host.arbiter = { chromeFocus: { generation: windows.host.generation, window_generation: windows.bound.tabbed.window_generation } };
    windows.readOmnibox = async () => ["visible", 103, 25, 1078, 90, 1110, 178];
    assert.equal(windows.inWindowOver(video), false, "the poll has not seen it");
    assert.equal(await windows.inWindowOverNow(video), true, "the live read has");
  } finally { await real.close(); }
});

// MP-08/MP-10/MP-11: New Tab's home navigation can commit while a person types in Chrome's omnibox.
for(const focus of ['chrome','page','unavailable','other-tab','window','generation','retired'])test(`MP-11 omnibox input across a home-document commit focus=${focus}`,async()=>{
 const tab={tab_id:'tab',target_id:'target',document_id:'before'};let physical=false;const sent=[],marks=[];
 const connection={send:async(method,p)=>method==='Page.getFrameTree'?{frameTree:{frame:{id:'f',loaderId:tab.document_id}}}:{result:{value:p.expression==='document.hasFocus()'?physical:true}}};
 const host={generation:3,tabs:new Map([['tab',tab]]),tabGroups:{active:async()=>tab.target_id},timing:s=>marks.push(s),browser:{inputCapture:{physicalFocus:async(_c,_s,read)=>read()},resolvePageTarget:async()=>({connection,sessionId:'s'}),ensureFocusWorld:async()=>({contextId:77})}};
 const arbiter=new InputArbiter(host,{}),source={surface:'browser_chrome',isCurrent:()=>true,chord:async key=>(sent.push(key),true)},regionWindow={window_generation:1};
 const send=input=>arbiter.input({stream:{regionSource:source,regionWindow},command:{tab_id:'tab',document_id:'before',generation:3,input}});
 await send({kind:'key',key:'l',modifiers:2});sent.length=0;tab.document_id='home';
 if(focus==='page')physical=true;
 if(focus==='unavailable')delete host.browser.inputCapture.physicalFocus;
 if(focus==='window'){regionWindow.window_generation=2;source.isCurrent=()=>false;}
 if(focus==='generation')host.generation=4;
 if(focus==='retired')source.isCurrent=()=>false;
 if(focus==='other-tab'){host.tabs.set('other',{tab_id:'other',target_id:'other',document_id:'home'});host.tabGroups.active=async()=> 'other'}
 if(['other-tab','window','generation','retired'].includes(focus)){await assert.rejects(send({kind:'text',text:'h'}),{code:'user_domain_stale_reference'});assert.deepEqual(sent,[],'retired window, generation and active Tab bindings still refuse');}
 else{await send({kind:'text',text:'h'});assert.deepEqual(sent,[104],'owner keys stay with the native window across page commits');}
});

test('MP-08/MP-10/MP-11 a fresh ledger observation preserves omnibox text after the home commit', async () => {
 const tab={tab_id:'tab',target_id:'target',document_id:'before'},sent=[];let liveDocument='before';
 const connection={send:async(method,p)=>method==='Page.getFrameTree'?{frameTree:{frame:{id:'f',loaderId:liveDocument}}}:{result:{value:p.expression==='document.hasFocus()'?false:true}}};
 const host={generation:3,tabs:new Map([['tab',tab]]),tabGroups:{active:async()=>tab.target_id},
  displayTarget:async()=>Object.assign(tab,{document_id:liveDocument}),
  browser:{inputCapture:{physicalFocus:async(_c,_s,read)=>read()},resolvePageTarget:async()=>({connection,sessionId:'s'}),ensureFocusWorld:async()=>({contextId:77})}};
 const binding={window_generation:1,isCurrent:()=>true},arbiter=new InputArbiter(host,{current:()=>({tabbed:binding})});
 const source={surface:'browser_chrome',isCurrent:()=>true,chord:async key=>(sent.push(key),true)},stream={regionSource:source,regionWindow:binding};
 const send=async input=>arbiter.input({stream,command:{...await arbiter.resolveDisplayInput({window_generation:1}),input}});
 await send({kind:'key',key:'l',modifiers:2});sent.length=0;
 liveDocument='home';
 await send({kind:'text',text:'https'});
 assert.equal(String.fromCharCode(...sent),'https');
});

// MP-11 (r54): band typing never depends on a busy page renderer. The person's own toolbar click or Ctrl+L is the
// kernel's record that Chrome's UI owns the keys; the page's physical focus is read only while the page answers.
for (const hang of ['focus read', 'focus world']) test(`MP-11 omnibox typing is not parked behind a page that cannot answer (${hang})`, async () => {
 const tab={tab_id:'tab',target_id:'target',document_id:'before'};const sent=[];
 const never=new Promise(()=>{});
 const connection={send:async(method)=>method==='Page.getFrameTree'?{frameTree:{frame:{id:'f',loaderId:tab.document_id}}}:{result:{value:false}}};
 const host={generation:3,tabs:new Map([['tab',tab]]),tabGroups:{active:async()=>tab.target_id},browser:{
  inputCapture:{physicalFocus:async(_c,_s,read)=>hang==='focus read'?never:read()},resolvePageTarget:async()=>({connection,sessionId:'s'}),
  ensureFocusWorld:async()=>hang==='focus world'?never:({contextId:77}),currentPageFrame:()=>({connection,sessionId:'s',frame:{id:'f',loaderId:tab.document_id}})}};
 const arbiter=new InputArbiter(host,{}),source={surface:'browser_chrome',isCurrent:()=>true,chord:async k=>(sent.push(k),true),click:async()=>true},regionWindow={window_generation:1};
 const send=input=>Promise.race([arbiter.input({stream:{regionSource:source,regionWindow},command:{tab_id:'tab',document_id:'before',generation:3,input}}),
  new Promise((_,reject)=>setTimeout(()=>reject(new Error('parked behind the page renderer')),1500))]);
 await send({kind:'key',key:'l',modifiers:2});sent.length=0;
 const started=performance.now();
 await send({kind:'text',text:'a'});
 const second=performance.now();
 await send({kind:'text',text:'bc'});
 assert.equal(String.fromCharCode(...sent),'abc');
 assert.ok(performance.now()-second<100,'only the first key of a claim waits for the page; the rest use the kernel record');
 assert.ok(second-started<1200,'the first key waits a bounded time');
});

// MP-08/MP-10/MP-11: a physical focus proof belongs to one document, including its asynchronous read.
for (const physical of [false,true]) test(`MP-08/MP-10 native owner typing never queries a retired page focus proof (${physical})`,async()=>{
 const tab={tab_id:'tab',target_id:'target',document_id:'before'};const sent=[];
 const connection={send:async(method,p)=>{
  if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'f',loaderId:tab.document_id}}};
  if(method==='Runtime.evaluate'&&p.expression==='document.hasFocus()'){tab.document_id='later';return {result:{value:physical}};}
  return {result:{value:false}};
 }};
 const host={generation:3,tabs:new Map([['tab',tab]]),tabGroups:{active:async()=>tab.target_id},browser:{inputCapture:{physicalFocus:async(_c,_s,read)=>read()},resolvePageTarget:async()=>({connection,sessionId:'s'}),ensureFocusWorld:async()=>({contextId:77})}};
 const arbiter=new InputArbiter(host,{}),source={surface:'browser_chrome',isCurrent:()=>true,chord:async k=>(sent.push(k),true)},regionWindow={window_generation:1};
 const send=input=>arbiter.input({stream:{regionSource:source,regionWindow},command:{tab_id:'tab',document_id:'before',generation:3,input}});
 await send({kind:'key',key:'l',modifiers:2});sent.length=0;
 tab.document_id='later';await send({kind:'text',text:'h'});
 assert.deepEqual(sent,[104],'a page navigation never consumes the native omnibox key');
});

// MP-08/MP-10/MP-11 review#1023: no steady-state CDP/auxiliary mutation per page frame.
for (const bound of [false, true]) test(`MP-10 region occlusion skips Chrome reads without UI focus bound=${bound}`, async () => {
  const host = { generation: 3, arbiter: { chromeFocus: null } }, windows = new ChromeWindows(host)
  windows.bound = bound ? { tabbed: { isCurrent: () => true, window_generation: 7, scale: 1 }, auxiliary: [] } : null
  let reads = 0, refreshes = 0
  windows.readOmnibox = async () => { reads++; return ["hidden"] }
  windows.refreshAuxiliary = () => { refreshes++ }
  assert.equal(await windows.inWindowOverNow({ left: 0, top: 100, width: 20, height: 20 }), false)
  assert.equal(reads, 0); assert.equal(refreshes, 0)
})

test("MP-11 concurrent occlusion probes share a focused read and never mutate window bindings", async () => {
  const host = { generation: 3, arbiter: { chromeFocus: { generation: 3, window_generation: 7 } } }, windows = new ChromeWindows(host)
  windows.bound = { tabbed: { isCurrent: () => true, window_generation: 7, scale: 1, bounds: { x: 0, y: 0, width: 1280, height: 887 }, page_rect: { top: 87 } }, auxiliary: [] }
  let reads = 0, refreshes = 0, release
  const gate = new Promise(resolve => { release = resolve })
  windows.readOmnibox = async () => { reads++; await gate; return ["hidden"] }
  windows.refreshAuxiliary = () => { refreshes++ }
  const rect = { left: 0, top: 100, width: 20, height: 20 }, pending = [windows.inWindowOverNow(rect), windows.inWindowOverNow(rect)]
  release(); assert.deepEqual(await Promise.all(pending), [false, false]); assert.equal(reads, 1); assert.equal(refreshes, 0)
})

test("MP-11 UI dispatch keeps occlusion live through focus release, then steady frames use the result", async () => {
  const host = { generation: 3, arbiter: { chromeFocus: null } }, windows = new ChromeWindows(host)
  windows.bound = { tabbed: { isCurrent: () => true, window_generation: 7, scale: 1, bounds: { x: 0, y: 0, width: 1280, height: 887 }, page_rect: { top: 87 } }, auxiliary: [] }
  let reads = 0; windows.readOmnibox = async () => { reads++; return ["hidden"] }
  const rect = { left: 0, top: 100, width: 20, height: 20 }, finish = windows.uiInput()
  await windows.inWindowOverNow(rect); assert.equal(reads, 1, "in-flight native input can still show Chrome UI after its focus proof clears")
  finish(); await windows.inWindowOverNow(rect); assert.equal(reads, 2, "the completed native dispatch gets one fresh observation")
  await windows.inWindowOverNow(rect); assert.equal(reads, 2, "steady page frames keep that observation")
})

// MP-08/MP-10/MP-11: toolbar focus is not proof that a delayed list cannot open.
// Steady frames resume the cached path once the physical page has its focus back.
test('MP-10 page focus release ends live occlusion checks after one fresh probe', async () => {
 const host={generation:3,arbiter:{chromeFocus:{generation:3,window_generation:7}}},windows=new ChromeWindows(host);
 windows.bound={tabbed:{isCurrent:()=>true,window_generation:7,scale:1,bounds:{x:0,y:0,width:1280,height:887},page_rect:{top:87}},auxiliary:[]};
 let reads=0;windows.readOmnibox=async()=>{reads++;return ['hidden']};
 const rect={left:0,top:100,width:20,height:20},finish=windows.uiInput();
 await windows.inWindowOverNow(rect);finish();await windows.inWindowOverNow(rect);
 host.arbiter.chromeFocus=null;
 for(let n=0;n<10;n++)assert.equal(await windows.inWindowOverNow(rect),false);
 assert.equal(reads,2,'page focus release makes the cached post-dispatch negative usable');
});

test('MP-11 delayed omnibox opening after a negative probe still fences the next readback', async () => {
 const host={generation:3,arbiter:{chromeFocus:{generation:3,window_generation:7}}},windows=new ChromeWindows(host);
 windows.bound={tabbed:{isCurrent:()=>true,window_generation:7,scale:1,bounds:{x:0,y:0,width:1280,height:887},page_rect:{top:87}},auxiliary:[]};
 let visible=false;windows.readOmnibox=async()=>visible?['visible',103,25,1078,90,1110,178]:['hidden'];
 const rect={left:120,top:100,width:20,height:20},finish=windows.uiInput();finish();
 assert.equal(await windows.inWindowOverNow(rect),false);
 visible=true;
 assert.equal(await windows.inWindowOverNow(rect),true,'no watcher tick or new input between the hidden and visible states');
});

test('MP-11 an occlusion probe cannot be shared across native input revisions', async () => {
 const host={generation:3,arbiter:{chromeFocus:{generation:3,window_generation:7}}},windows=new ChromeWindows(host);
 windows.bound={tabbed:{isCurrent:()=>true,window_generation:7,scale:1,bounds:{x:0,y:0,width:1280,height:887},page_rect:{top:87}},auxiliary:[]};
 const old=Promise.withResolvers();let reads=0;
 windows.readOmnibox=async()=>++reads===1?old.promise:['visible',103,25,1078,90,1110,178];
 const rect={left:120,top:100,width:20,height:20},first=windows.inWindowOverNow(rect),finish=windows.uiInput(),second=windows.inWindowOverNow(rect);
 old.resolve(['hidden']);
 assert.equal(await first,true,'a read crossed by new input holds its pixels conservatively');
 assert.equal(await second,true);assert.equal(reads,2);finish();
});

// PERF_AUDIT 4.2.7 (C2), r53: a key for the page reads no frame tree while the ordered navigation events prove the document,
// so a renderer busy with a long parse is not asked before every key (the focus world and the sensitive-field check still run).
test("MP-08/MP-11 a band key reads no page frame tree while navigation events prove the document", async () => {
  const sent = [];
  const connection = { send: async (method) => (sent.push(method), method === "Page.getFrameTree" ? { frameTree: { frame: { id: "f", loaderId: "db" } } }
    : method === "Runtime.evaluate" ? { result: { value: false } } : {}) };
  const host = { generation: 3, tabs: new Map([["host-tab-b", { tab_id: "host-tab-b", target_id: "tb", document_id: "db" }]]), tabGroups: { active: async () => "tb" },
    browser: { currentPageFrame: () => ({connection,sessionId:"s",frame:{id:"f",loaderId:"db"}}), inputCapture: { physicalFocus: async (_c, _s, read) => read() }, resolvePageTarget: async () => ({ connection, sessionId: "s" }), ensureFocusWorld: async () => ({ contextId: 77 }) } };
  const arbiter = new InputArbiter(host, {});
  const source = { surface: "browser_chrome", isCurrent: () => true, chord: async () => true };
  assert.deepEqual(await arbiter.input({ stream: { regionSource: source }, command: { tab_id: "host-tab-b", document_id: "db", generation: 3, input: { kind: "text", text: "a" } } }), { dispatched: true });
  assert.equal(sent.filter(m => m === "Page.getFrameTree").length, 0);
  assert.equal(sent.filter(m => m === "Runtime.evaluate").length, 0, "A-L23: no page read before a band key");
});

// r13 b3 union (browserux-r13union3-mvalvidm, 1440x900@2 -> 1920x1080@1): a page region saw the page geometry
// change and retired the window without rebinding it; every band subscribe then failed (241 stale references)
// and the band never painted again. A geometry retirement rebinds at the real geometry.
test("MP-08/MP-11 a page geometry change rebinds the band window", async t => {
  let viewport;
  const { windows } = fake({ viewport: { get width() { return viewport?.width ?? 1280; }, get height() { return viewport?.height ?? 800; } } });
  const real = new OwnedDisplay((await import("node:os")).tmpdir());
  try { await real.start(); } catch { return t.skip("no Xvfb"); }
  try {
    windows.host.chromium.display = real;
    const bound = await windows.resolve();
    assert.equal(await bound.tabbed.isPageCurrent(), true);
    viewport = { width: 1280, height: 780 };
    assert.equal(await bound.tabbed.isPageCurrent(), false, "the old page rect is refused");
    await windows.resolving;
    const next = windows.current();
    assert.equal(next.tabbed.window_generation, bound.tabbed.window_generation + 1, "rebound at once");
    windows.retire("missed");
    await windows.heal();
    assert.equal(windows.current().tabbed.window_generation, next.tabbed.window_generation + 1, "a state read heals a missed rebind");
    windows.retire("missed");
    await windows.heal();
    assert.throws(() => windows.current(), { code: "user_domain_stale_reference" }, "at most once a second");
  } finally { await real.close(); }
});

test('MP-08/MP-10/MP-11 active target in one fresh viewport query keeps a foreign Tab fenced',async()=>{
 const {host,windows}=fake(),display=new OwnedDisplay(process.env.TMPDIR??'/tmp');await display.start();
 try{
  host.chromium.display=display;const b=(await windows.resolve()).tabbed;let reads=0;
  const viewport=host.tabGroups.viewport;host.tabGroups.viewport=async(...args)=>{reads++;return viewport(...args)};
  assert.equal(await b.isPageCurrent('ta'),false);assert.equal(reads,1);assert.equal(b.isCurrent(),true,'a different active Tab does not retire the unchanged browser window');
  assert.equal(await b.isPageCurrent('tb'),true);assert.equal(reads,2);
  host.tabGroups.viewport=async()=>{throw Error('unavailable')};assert.equal(await b.isPageCurrent('tb'),false);assert.equal(b.isCurrent(),false);
 }finally{await display.close()}
});

// PERF_AUDIT 4.2.2/4.2.6 (protocol 540): a band input's reply carries the window it left, so the viewer
// follows in that round trip instead of at its next 250 ms state read (r12/r13 quiet: Tab click and list
// first paint waited for the poll and a second round trip).
const followHost = (active) => {
  const connection = { send: async (method) => method === "Page.getFrameTree" ? { frameTree: { frame: { id: "f", loaderId: "da" } } }
    : method === "Runtime.evaluate" ? { result: { value: false } } : {} };
  const host = { generation: 3, tabs: new Map([["host-tab-a", { tab_id: "host-tab-a", target_id: "ta", document_id: "da" }], ["host-tab-b", { tab_id: "host-tab-b", target_id: "tb", document_id: "db" }]]),
    tabGroups: { active: async () => active.target }, retireNativeForeground: () => null,
    chromeWindow: () => ({ window_generation: 7, active_tab_id: "host-tab-a", auxiliary: active.list ? [{ auxiliary_id: 1, x: 80, y: 40, width: 600, height: 120 }] : [] }),
    browser: { inputCapture: { physicalFocus: async (_c, _s, read) => read() }, resolvePageTarget: async () => ({ connection, sessionId: "s" }), ensureFocusWorld: async () => ({ contextId: 77 }) } };
  return host;
};
test("MP-08/MP-10 a Tab click's reply carries the Tab the strip now shows", async () => {
  const active = { target: "ta" };
  const arbiter = new InputArbiter(followHost(active), {});
  const source = { surface: "browser_chrome", isCurrent: () => true, click: async () => { setTimeout(() => { active.target = "tb"; }, 30); return true; } };
  const reply = await arbiter.input({ stream: { regionSource: source }, command: { tab_id: "host-tab-a", document_id: "da", generation: 3, input: { kind: "click", x: 300, y: 12 } } });
  assert.deepEqual(reply, { dispatched: true, _follow: { tab_id: "host-tab-a", switches: true } }, "answered at once: the serial input lane never waits for Chrome");
  assert.deepEqual((await arbiter.follow(reply._follow)).chrome_window, { window_generation: 7, active_tab_id: "host-tab-b", auxiliary: [] });
});
test("MP-08/MP-10 an address-bar key's reply carries the omnibox list it opened", async () => {
  const active = { target: "ta", list: false };
  let warm = false;
  const windows = { onOmnibox: () => true, focusOmnibox: on => { warm = on; }, refreshWarm: async () => { if (warm) active.list = true; } };
  const arbiter = new InputArbiter(followHost(active), windows);
  const source = { surface: "browser_chrome", isCurrent: () => true, click: async () => true, chord: async () => true };
  const stream = { regionSource: source }, command = input => ({ tab_id: "host-tab-a", document_id: "da", generation: 3, input });
  await arbiter.input({ stream, command: command({ kind: "click", x: 300, y: 60 }) });
  const reply = await arbiter.input({ stream, command: command({ kind: "text", text: "w" }) });
  assert.deepEqual(reply, { dispatched: true, _follow: { tab_id: "host-tab-a", switches: false } });
  assert.deepEqual((await arbiter.follow(reply._follow)).chrome_window.auxiliary, [{ auxiliary_id: 1, x: 80, y: 40, width: 600, height: 120 }]);
});
test("MP-08/MP-10 a click that opens a Tab the kernel has not adopted yet leaves the window to the state read", async () => {
  const active = { target: "ta" };
  const arbiter = new InputArbiter(followHost(active), {});
  const source = { surface: "browser_chrome", isCurrent: () => true, click: async () => { active.target = "t-new"; return true; } };
  const reply = await arbiter.input({ stream: { regionSource: source }, command: { tab_id: "host-tab-a", document_id: "da", generation: 3, input: { kind: "click", x: 600, y: 12 } } });
  assert.deepEqual(await arbiter.follow(reply._follow), {});
});

// Review #1016 @40979b97d P1: the warm omnibox entry (540) stays current while the list is closed; it must not
// carry keys past the page's fences. Closed list + a page field focused through the Computer surface: the
// retained popup binding's text and chords get the sensitive-field fence; a shown list keeps the keys in Chrome.
test("MP-11 a closed warm omnibox list hands its keys to the page, a shown list keeps them in Chrome", async () => {
  const evaluated = [];
  const connection = { send: async (method, params) => method === "Page.getFrameTree" ? { frameTree: { frame: { id: "f", loaderId: "da" } } }
    : method === "Runtime.evaluate" ? (evaluated.push(params.expression), { result: { value: true } }) : {} };
  const host = { generation: 3, tabs: new Map([["host-tab-a", { tab_id: "host-tab-a", target_id: "ta", document_id: "da" }]]), tabGroups: { active: async () => "ta" },
    browser: { inputCapture: { physicalFocus: async (_c, _s, read) => read() }, resolvePageTarget: async () => ({ connection, sessionId: "s" }), ensureFocusWorld: async () => ({ contextId: 77 }) } };
  let shown = false;
  const windows = { inWindowCurrent: async source => source.isCurrent(), omniboxShown: async () => shown, focusOmnibox() {} };
  const arbiter = new InputArbiter(host, windows);
  const chords = [];
  const source = { surface: "browser_auxiliary", overPage: true, isCurrent: () => true, chord: async (...args) => (chords.push(args), true) };
  const send = input => arbiter.input({ stream: { regionSource: source }, command: { tab_id: "host-tab-a", document_id: "da", generation: 3, input } });
  assert.equal((await send({ kind: "text", text: "s" })).dispatched, true);
  assert.equal((await send({ kind: "key", key: "a", modifiers: 4 })).dispatched, true);
  assert.equal(chords.length, 2, "A-L23: a closed list's keys reach the page field");
  shown = true; evaluated.length = 0;
  assert.equal((await send({ kind: "text", text: "w" })).dispatched, true);
  assert.equal(evaluated.length, 0, "a shown list owns the keys (Chrome's UI)");
});

// vidlat 07:52Z (C10): a band input marks its dispatch at the final native action boundary (after every
// fence, before XTest), once per native action, never for a refused one, so band frames can be graded as
// input-triggered exactly as page frames are.
test("MP-08/MP-10 a band input marks each native dispatch after its fences", async () => {
  const order = [];
  const active = { target: "ta" };
  const arbiter = new InputArbiter(followHost(active), {});
  const source = { surface: "browser_chrome", isCurrent: () => true, click: async () => (order.push("click"), true), chord: async () => (order.push("chord"), true) };
  const send = (input, tab_id = "host-tab-a") => arbiter.input({ stream: { regionSource: source }, command: { tab_id, document_id: "da", generation: 3, input }, onDispatch: () => order.push("dispatch") });
  await send({ kind: "click", x: 300, y: 12 });
  await send({ kind: "key", key: "t", modifiers: 4 });
  assert.deepEqual(order, ["dispatch", "click", "dispatch", "chord"]);
  await assert.rejects(send({ kind: "click", x: 300, y: 12 }, "host-tab-b"), { code: "user_domain_stale_reference" });
  assert.equal(order.length, 4, "a refused input marks nothing");
});

// The follow-up reads are best effort and never part of the input: the kernel answers inputs in one
// serial lane, and a follow-up wait there queued every later key (b3 r14union2 / vidlat r20: typed URLs lost
// keys, "kernel request timed out"). The input answers at once; a stalled follow-up answers empty.
test("MP-08/MP-10 a stalled follow-up read never holds a band input's reply", async () => {
  const active = { target: "ta" };
  const windows = { onOmnibox: () => true, focusOmnibox() {}, refreshWarm: () => new Promise(() => {}) };
  const arbiter = new InputArbiter(followHost(active), windows);
  const source = { surface: "browser_chrome", isCurrent: () => true, chord: async () => true };
  const started = performance.now(), alive = setInterval(() => {}, 50); // the host's event loop is never idle
  const reply = await arbiter.input({ stream: { regionSource: source }, command: { tab_id: "host-tab-a", document_id: "da", generation: 3, input: { kind: "key", key: "a" } } });
  assert(performance.now() - started < 100, "the input itself never waits for the follow-up");
  assert.deepEqual(await arbiter.follow(reply._follow).finally(() => clearInterval(alive)), {});
  assert(performance.now() - started < 1000, "answered at the follow bound");
});

// r54 (audit lever 3): a person's band key is admitted from the native observer's record of the active Tab while no window
// event retired it; a browser process that is slow to answer a fresh query holds nothing.
test("MP-11 band input admission uses the cached active Tab and asks the browser only when none is recorded", async () => {
  const tab = { tab_id: "tab", target_id: "target", document_id: "doc" };
  let fresh = 0, cached = "target";
  const host = { generation: 3, tabs: new Map([["tab", tab]]), tabGroups: { cachedActive: () => cached, active: async () => { fresh++; return "target"; } } };
  const arbiter = new InputArbiter(host, {});
  assert.equal((await arbiter.activeTab()).tab_id, "tab");
  assert.equal(fresh, 0, "no native query while the record stands");
  cached = null;
  assert.equal((await arbiter.activeTab()).tab_id, "tab");
  assert.equal(fresh, 1, "a retired record is read again");
});

// vidlat r22 (coordinator 22:55Z): source publication may reuse the native owner verdict only while nothing
// it depends on changed. Every owner/resize/detach/geometry/window trigger forces the next publication fresh.
const publishFixture=async()=>{
 const {ChromeTabGroups,TAB_GROUPS_EXTENSION_ID}=await import('./kernel-browser-tab-groups.mjs');
 let listener,viewport={target_id:'tb',width:1280,height:800},reads=0;
 const connection={subscribe(fn){listener=fn;return()=>{listener=null}},isOpen:()=>true,async send(method,params){
  if(method==='Target.getTargets')return {targetInfos:[{type:'service_worker',targetId:'worker',url:`chrome-extension://${TAB_GROUPS_EXTENSION_ID}/sw.js`}]};
  if(method==='Target.attachToTarget')return {sessionId:'worker-session'};
  if(method==='Runtime.evaluate'){if(params.expression==='charioxTabs.revision === 2')return{result:{value:true}};reads++;return {result:{value:viewport}};}
  if(method==='Browser.getWindowForTarget')return {windowId:1,bounds:{left:0,top:0,width:1280,height:887,windowState:'normal'}};
  return {};
 }};
 const {host,windows}=fake(),display=new OwnedDisplay(process.env.TMPDIR??'/tmp');await display.start();
 host.chromium.display=display;host.browser.connection=connection;host.tabGroups=new ChromeTabGroups(connection);
 const b=(await windows.resolve()).tabbed;
 return {b,windows,reads:()=>reads,event:m=>listener?.(m),move:next=>{viewport=next},close:async()=>{host.tabGroups.close();await display.close()}};
};
const ownerChanged={method:'Runtime.bindingCalled',sessionId:'worker-session',params:{name:'__charioxNativeWindowChanged',payload:''}};
test('MP-08/MP-10/MP-11 source publication reuses the native owner verdict until an owner trigger retires it',async()=>{
 const f=await publishFixture();
 try{
  assert.equal(await f.b.isPagePublishable('tb'),true);const first=f.reads();
  for(let n=0;n<5;n++)assert.equal(await f.b.isPagePublishable('tb'),true);
  assert.equal(f.reads(),first,'no extension read while no owner event arrived');
  assert.equal(await f.b.isPageCurrent('tb'),true);assert.equal(f.reads(),first+1,'input and admission stay fresh');
  for(const [name,trigger] of [['owner API event',()=>f.event(ownerChanged)],['page resize',()=>f.event({method:'Page.frameResized',sessionId:'page'})],
   ['extension detached',()=>f.event({method:'Target.detachedFromTarget',params:{sessionId:'worker-session'}})],['browser disconnected',()=>f.event({method:'Chariox.browserDisconnected'})]]){
   assert.equal(await f.b.isPagePublishable('tb'),true);const before=f.reads();
   trigger();assert.equal(await f.b.isPagePublishable('tb'),true,name);assert.equal(f.reads(),before+1,`${name}: next publication reads fresh once`);
   assert.equal(await f.b.isPagePublishable('tb'),true);assert.equal(f.reads(),before+1,`${name}: and then reuses it`);
  }
 }finally{await f.close()}
});
test('MP-08/MP-10/MP-11 a geometry or Tab change behind an owner event is refused and rebinds the window',async()=>{
 const f=await publishFixture();
 try{
  assert.equal(await f.b.isPagePublishable('tb'),true);
  f.move({target_id:'tb',width:1280,height:780});f.event(ownerChanged);
  assert.equal(await f.b.isPagePublishable('tb'),false,'the old page rect is refused after the event');
  assert.equal(f.b.isCurrent(),false,'and the window binding retires');
 }finally{await f.close()}
 const g=await publishFixture();
 try{
  assert.equal(await g.b.isPagePublishable('tb'),true);g.move({target_id:'ta',width:1280,height:800});g.event(ownerChanged);
  assert.equal(await g.b.isPagePublishable('tb'),false,'a foreign active Tab is refused');assert.equal(g.b.isCurrent(),true);
 }finally{await g.close()}
});
test('MP-08/MP-10/MP-11 a retired window or an unsupported owner never publishes from a cache',async()=>{
 const f=await publishFixture();
 try{
  assert.equal(await f.b.isPagePublishable('tb'),true);f.windows.retire('test');
  assert.equal(await f.b.isPagePublishable('tb'),false,'window generation changed');
 }finally{await f.close()}
 const g=await publishFixture();
 try{
  await g.b.isPagePublishable('tb');
  const {host}=g.windows;host.tabGroups.nativeStateSupported=()=>false;
  assert.equal(await (await g.windows.resolve()).tabbed.isPagePublishable('tb'),true);
  const before=g.reads();assert.equal(await (await g.windows.resolve()).tabbed.isPagePublishable('tb'),true);assert(g.reads()>before,'no native observer: always fresh');
 }finally{await g.close()}
});

// Protocol 556: a parked state read wakes on a window retirement or a change of the popups, not on every poll.
test('MP-08/MP-10 window retirement and popup changes wake a parked state read once per change',async()=>{
 const f=await publishFixture(),woken=[];
 try{
  f.windows.host.stateSignal={notify:()=>woken.push(1)};
  const rows=[...f.windows.rows];
  assert.equal(f.windows.bound.auxiliary.length,1,'the fixture window has a popup');
  f.windows.refreshAuxiliary(rows);const first=woken.length;f.windows.refreshAuxiliary(rows);f.windows.refreshAuxiliary(rows);
  assert.equal(woken.length,first,'an unchanged inventory poll wakes nothing after the first report');
  f.windows.refreshAuxiliary(rows.filter(r=>!r.override_redirect));
  assert.equal(woken.length,first+1,'a popup closing wakes it');
  f.windows.refreshAuxiliary(rows.filter(r=>!r.override_redirect));assert.equal(woken.length,first+1);
  f.windows.retire('test');assert.equal(woken.length,first+2,'a retirement wakes it');
 }finally{await f.close()}
});

// L1: the inventory poll is a backstop; the worker's change header reads at once and coalesces.
test("L1 a watched band reads on the worker's change report, not on a timer", async () => {
  const { windows } = fake();
  let reads = 0, wake = null, release = null;
  const read = async () => { reads++; if (release) await new Promise(r => { release = r; }); return []; };
  const stop = windows.watch(read, 10_000, fn => { wake = fn; return () => { wake = null; }; });
  await new Promise(r => setTimeout(r, 120));
  assert.equal(reads, 0, "an idle band is not polled between backstops");
  wake(); await new Promise(r => setTimeout(r, 20));
  assert.equal(reads, 1, "a report reads within 20 ms");
  release = () => {}; wake();                       // a read in flight; two more reports arrive meanwhile
  await new Promise(r => setTimeout(r, 5)); wake(); wake();
  const pending = release; release = null; pending();
  await new Promise(r => setTimeout(r, 30));
  assert.equal(reads, 3, "reports during a read coalesce into one more read");
  stop(); assert.equal(wake, null, "the last viewer drops the subscription");
  const before = reads; await new Promise(r => setTimeout(r, 30));
  assert.equal(reads, before, "and the watch stops");
});
test("L1 the backstop still reads without any report", async () => {
  const { windows } = fake();
  let reads = 0;
  const stop = windows.watch(async () => (reads++, []), 20, () => () => {});
  await new Promise(r => setTimeout(r, 100));
  stop();
  assert(reads >= 3 && reads <= 6, `backstop cadence, got ${reads}`);
});


test('MP-08/MP-10 owner omnibox keys survive a home-document commit after admission', async()=>{
 const tab={tab_id:'tab',target_id:'target',document_id:'before'},sent=[];
 const host={generation:3,tabs:new Map([['tab',tab]]),tabGroups:{active:async()=>tab.target_id},browser:{resolvePageTarget:()=>assert.fail('Chrome UI input must not read the page renderer')}};
 const arbiter=new InputArbiter(host,{}),source={surface:'browser_chrome',isCurrent:()=>true,chord:async key=>(sent.push(key),true)},stream={regionSource:source,regionWindow:{window_generation:1}};
 const command=input=>({tab_id:'tab',document_id:'before',generation:3,input});
 await arbiter.input({stream,command:command({kind:'key',key:'l',modifiers:2})});sent.length=0;
 tab.document_id='consent';
 await arbiter.input({stream,command:command({kind:'text',text:'https://www.wikipedia.org/'})});
 await arbiter.input({stream,command:command({kind:'key',key:'Enter'})});
 assert.equal(String.fromCharCode(...sent.slice(0,-1)),'https://www.wikipedia.org/');assert.equal(sent.at(-1),0xff0d);
});
