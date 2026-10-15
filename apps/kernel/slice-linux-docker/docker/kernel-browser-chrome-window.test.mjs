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
  await assert.rejects(send({ kind: "text", text: "x" }), { code: "user_domain_sensitive_requires_focus" }, "a password field focused in the page");
  await send({ kind: "click", x: 520, y: 63 }); // the omnibox
  assert.deepEqual(await send({ kind: "key", key: "a", modifiers: 2 }), { dispatched: true }, "Ctrl+A selects the address");
  assert.deepEqual(await send({ kind: "text", text: "wiki" }), { dispatched: true });
  assert.deepEqual(await send({ kind: "key", key: "Backspace" }), { dispatched: true });
  assert.deepEqual(await send({ kind: "key", key: "Enter" }), { dispatched: true });
  await assert.rejects(send({ kind: "text", text: "x" }), { code: "user_domain_sensitive_requires_focus" }, "Enter hands focus back to the page");
  assert.deepEqual(await send({ kind: "key", key: "l", modifiers: 2 }), { dispatched: true }, "Ctrl+L");
  assert.deepEqual(await send({ kind: "text", text: "a" }), { dispatched: true }, "Ctrl+L focuses the omnibox");
  arbiter.pageFocused();
  await assert.rejects(send({ kind: "text", text: "x" }), { code: "user_domain_sensitive_requires_focus" }, "a native click into the page took focus back");
  await send({ kind: "click", x: 520, y: 63 }); await send({ kind: "key", key: "Tab" });
  await assert.rejects(send({ kind: "text", text: "x" }), { code: "user_domain_sensitive_requires_focus" }, "Tab moves focus on");
  await send({ kind: "click", x: 300, y: 20 }); // a Tab in the strip: the page keeps its focus
  await assert.rejects(send({ kind: "text", text: "x" }), { code: "user_domain_sensitive_requires_focus" }, "a strip click does not focus Chrome's UI");
});

// MP-08/MP-10/MP-11: every attached mirror keeps focus emulation enabled.
// A native omnibox check must observe physical focus, not that logical wake-up.
test('MP-11 a held mirror does not make an unfocused page appear to own omnibox keys',async()=>{
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
  assert.equal(emulated,false,'native Chrome keeps the page blurred until page input');
  pageFocus=true;await assert.rejects(send({kind:'text',text:'x'}),{code:'user_domain_sensitive_requires_focus'},'real sensitive page focus still refuses');
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
    windows.refreshAuxiliary([{ ...tabbed, width: 2000 }]);
    assert.equal(windows.auxiliaryState().length, 1, "a changed tabbed window waits for the kernel's own rebind");
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
test("MP-08/MP-11 the omnibox dropdown drawn in the window is one more popup below the band, corners masked", async t => {
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
    assert.deepEqual(region.rect, { left: 238, top: 174, width: 2156, height: 176 });
    // Only the slivers outside the rounded corners (radius 32 px at scale 2), one rect per row: no black squares.
    assert.deepEqual(region.masks.slice(0, 2), [{ x: 0, y: 144, width: 1, height: 1 }, { x: 2155, y: 144, width: 1, height: 1 }]);
    assert.deepEqual(region.masks.slice(-2), [{ x: 0, y: 175, width: 27, height: 1 }, { x: 2129, y: 175, width: 27, height: 1 }]);
    assert(region.masks.every(m => m.y >= 144 && m.y < 176 && m.width <= 32), "within the corner squares");
    const binding = windows.auxiliary(popup.auxiliary_id);
    windows.omnibox = ["visible", 103, 25, 1078, 130, 1110, 218]; windows.refreshAuxiliary([tabbed]);
    assert.equal(binding.isCurrent(), false, "more suggestions: a new fixed-size popup");
    assert.equal(windows.auxiliaryState()[0].height, 128);
    windows.omnibox = ["hidden", 103, 25, 1078, 1, 1110, 178]; windows.refreshAuxiliary([tabbed]);
    assert.deepEqual(windows.auxiliaryState(), [], "closed");
    for (const state of [["visible", 103, 25, 1078, 90, 1120, 178], ["visible", 103, 100, 1078, 90, 1110, 178], ["visible", 103, 25, 1278, 90, 1310, 178], null]) {
      windows.omnibox = state; windows.refreshAuxiliary([tabbed]);
      assert.deepEqual(windows.auxiliaryState(), [], `not the pinned layout or not over the page: ${JSON.stringify(state)}`);
    }
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
  await assert.rejects(send(band, { kind: "text", text: "x" }), { code: "user_domain_sensitive_requires_focus" }, "band text still fenced");
  open = false;
  await assert.rejects(send(menu, { kind: "key", key: "Enter" }), { code: "user_domain_stale_reference" }, "a closed menu takes nothing");
  assert.deepEqual(keys.map(k => k[0]), ["menu", "menu", "menu"]);
});
