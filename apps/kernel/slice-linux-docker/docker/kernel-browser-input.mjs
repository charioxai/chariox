import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
// MD-3: document-bound physical input, sharing Room cancellation and document checks.
import { PointerOutsideViewport, UserDomainRefusal } from "./kernel-browser-refusal.mjs";
import { BrowserActionError, assertCurrentDocument, assertNotCancelled } from "./browser-controller-actions.mjs";
const canonicalViewport = { css_width: geometry.width, css_height: geometry.height };
// MP-08: Chromium uses virtual key codes for native caret/editing commands.
const keyCodes = { Tab: 9, Enter: 13, Escape: 27, Backspace: 8, Delete: 46,
  ArrowLeft: 37, ArrowRight: 39, ArrowUp: 38, ArrowDown: 40, Home: 36, End: 35 };
// MP-08/MP-10: X keysyms for owned-display viewer keys (printable ASCII maps 1:1).
export const keysyms = { Tab: 0xff09, Enter: 0xff0d, Space: 0x20, Escape: 0xff1b, Backspace: 0xff08, Delete: 0xffff,
  ArrowLeft: 0xff51, ArrowUp: 0xff52, ArrowRight: 0xff53, ArrowDown: 0xff54, Home: 0xff50, End: 0xff57 };
// MP-08/MP-10: mouse-wheel notches animate like a native wheel through the
// owned display (Selkies forwards every wheel event as X button notches).
// A native X11 notch moves 120 pixels. Other viewer steps, including a
// 100-pixel Windows wheel step, must retain their distance through CDP.
// Fine trackpad motion must retain its pixel distance through CDP: turning
// each small delta into a whole notch amplifies a gesture and its momentum tail.

const FOCUS_GRACE_MS = 100;
export const notches = delta => delta % 120 === 0 ? delta / 120 : null;
// Chrome's own UI (strip, menus, popups) has no CDP path: X11 notches only. A wheel
// step of 100 or more turns per step; fine trackpad deltas accumulate to whole notches.
export class ChromeWheel {
  constructor() { this.rest = { x: 0, y: 0 }; }
  notches(axis, delta) {
    if (!Number.isFinite(delta)) return null;
    const whole = notches(delta);
    if (whole !== null || Math.abs(delta) >= 100) { this.rest[axis] = 0; return whole ?? Math.round(delta / 100); }
    const sum = Math.sign(this.rest[axis]) === -Math.sign(delta) ? delta : this.rest[axis] + delta, turned = Math.trunc(sum / 120);
    this.rest[axis] = sum - turned * 120; return turned;
  }
}
// MP-11: native pointer input is at most once. An action sent to the native
// worker without a reply may have reached the page, so it counts as dispatched
// (the retired source re-observes the outcome) and is never replayed via CDP.
async function nativeOnce(dispatch) {
  try { return await dispatch(); }
  catch (error) { if (error?.code === "native_input_uncertain") return true; throw error; }
}

export async function inputHostTab(browser, tab, input, { viewport=canonicalViewport, signal, onDispatch, resolveMirror, asyncScroll = false, nativeWheel = null, nativeClick = null, nativeKey = null, viewScale = 1, inputGuard, documentCurrent, chromeFocused = () => false, slow = (_name, operation) => operation } = {}) {
    assertNotCancelled(signal);
    // MP-08/MP-11 protocol498: CDP bits Alt=1, Control=2, Meta=4, Shift=8.
    if (input.kind === 'key' && input.modifiers !== undefined && (!Number.isInteger(input.modifiers) || input.modifiers < 0 || input.modifiers > 15)) throw new Error('MP-11: invalid key modifiers');
    const { connection, sessionId } = await slow('page_target', browser.resolvePageTarget(tab.target_id));
    const cachedDocument = () => {
      if (!documentCurrent()) throw new BrowserActionError("stale_document_reference", "browser document retired before native input");
    };
    // The page session's ordered navigation events name its document: a busy renderer is not asked (a person's first
    // wheel waited 5 s for it, r55). A navigation in flight leaves no proof, then the renderer is asked as before.
    const eventDocument = () => {
      const current = browser.currentPageFrame?.(tab.target_id);
      return current && current.connection === connection && current.sessionId === sessionId ? current.frame.loaderId : null;
    };
    const check = async (fresh = false) => {
      assertNotCancelled(signal);
      const proven = fresh || documentCurrent ? null : eventDocument();
      if (documentCurrent && !fresh) cachedDocument();
      else if (proven === tab.document_id) { /* current by ordered events */ }
      else if (proven) throw new BrowserActionError("stale_document_reference", "browser document retired before input");
      else await slow('document_fence', assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id));
      assertNotCancelled(signal);
    };
    // MP-08/MP-10 (2.3): one isolated world per document, reused by every
    // keystroke; a destroyed context (navigation, reload) is recreated once.
    const textWorld = async (fresh = false) => {
      const cache = browser.chariox_text_worlds ??= new Map();
      const cached = cache.get(tab.target_id);
      if (!fresh && cached?.document === tab.document_id) return cached.contextId;
      const { frameTree } = await connection.send("Page.getFrameTree", {}, sessionId);
      if (frameTree?.frame?.loaderId !== tab.document_id) throw new BrowserActionError("stale_document_reference", "browser document retired before focus world creation");
      const { executionContextId } = await connection.send("Page.createIsolatedWorld", {
        frameId: frameTree.frame.id, worldName: "chariox-host-input", grantUniveralAccess: false,
      }, sessionId);
      if (frameTree.frame.loaderId === tab.document_id) cache.set(tab.target_id, { document: tab.document_id, contextId: executionContextId });
      return executionContextId;
    };
    const evaluateText = async expression => {
      const evaluate = async contextId => (await connection.send("Runtime.evaluate", { contextId, expression, returnByValue: true }, sessionId)).result?.value;
      try { return await evaluate(await textWorld()); }
      catch (error) { if (error?.code === "stale_document_reference") throw error; return await evaluate(await textWorld(true)); }
    };
    // A renderer busy with a snapshot answers late. Once a read proved this document holds focus, the next keys go on
    // that proof for as long as Chrome's own UI holds no claim on the keyboard; a late read still refreshes it.
    const pageFocus = () => {
      const proven = (browser.chariox_page_focus ??= new Map());
      const read = evaluateText("document.hasFocus()").then(value => { if (value === true) proven.set(tab.target_id, tab.document_id); else proven.delete(tab.target_id); return value; });
      if (proven.get(tab.target_id) !== tab.document_id || chromeFocused()) return read;
      read.catch(() => {});
      return Promise.race([read, new Promise(resolve => setTimeout(resolve, FOCUS_GRACE_MS, true))]);
    };
    // The guards are awaited right before a physical dispatch: a retirement (parked retry) that arrives inside one must stop it.
    const guarded = guard => guard && (async () => { assertNotCancelled(signal); await slow('input_guard', guard()); assertNotCancelled(signal); });
    let mirrorGuard=guarded(inputGuard);
    const admitInput = async () => {
      // CDP fallbacks retain their fresh document barriers; live mirror focus is the final fence.
      await check(true);
      await mirrorGuard?.();
    };
    const sendInput = async (method, params) => {
      await admitInput();
      const endDispatch = onDispatch?.();
      try {
        const result = await connection.send(method, params, sessionId);
        assertNotCancelled(signal);
        return result;
      } finally { endDispatch?.(); }
    };
    // MP-11: sequence-only refusals precede even focus emulation. No page
    // focus/selection/physical input may run before mirror epoch admission.
    let resolved;
    if(input.kind==='mirror') {
      if(!resolveMirror)throw new Error('MP-11: mirror input resolver unavailable');
      // MP-08/MP-10/MP-11: resolvers fence the document before their first
      // page call (point, ancestry, focus), so no separate query here.
      resolved=await resolveMirror(input);
    }
    // A wheel needs no page focus: it does not wait for the renderer to resume focus emulation.
    return browser.inputCapture.run(connection, sessionId, async () => {
      // MP-08/MP-10/MP-11: a fenced (v2) resolution has its final document
      // fence in perform/sendInput/its guard below. No mutation or await
      // separates this redundant query from that final fence.
      if (!resolved?.fenced) await check();
      if(resolved) {
        mirrorGuard=guarded(inputGuard?async()=>{await inputGuard();await resolved.guard?.();}:resolved.guard);
        if(resolved.perform) {if(!resolved.fenced)await check();await resolved.perform(sendInput,onDispatch);await check();return;}
        input=resolved.input;
      }
      if (input.kind === "text") {
        if (typeof input.text !== "string" || input.text.length > 16384) throw new Error("MD-2: input text exceeds limit");
        await sendInput("Input.insertText", { text: input.text });
      } else if (input.kind === "key") {
        const printable = typeof input.key === "string" && /^[^\p{C}]$/u.test(input.key);
        // Shift only with focus/selection keys: Shift+Tab moves focus back, Shift+Arrow/Home/End extend the selection.
        const shift = !printable && typeof input.key === "string" && /^Shift\+(Tab|ArrowLeft|ArrowRight|ArrowUp|ArrowDown|Home|End)$/.test(input.key);
        // Ctrl only with word deletion (CDP only: the native path presses no Ctrl).
        const ctrl = !printable && typeof input.key === "string" && /^Ctrl\+(Backspace|Delete)$/.test(input.key);
        if (!printable && !shift && !ctrl && !["Tab", "Enter", "Space", "Escape", "Backspace", "Delete", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(input.key)) throw new Error("MD-2: unsupported key");
        const modifiers = (input.modifiers ?? 0) | (shift ? 8 : ctrl ? 2 : 0);
        const chord = (modifiers & 7) !== 0;
        const name = shift ? input.key.slice(6) : ctrl ? input.key.slice(5) : input.key;
        const key = { key: name === "Space" ? " " : name, code: /^[a-z]$/i.test(name) ? `Key${name.toUpperCase()}` : name,
          ...(modifiers ? { modifiers } : {}),
          windowsVirtualKeyCode: { Tab:9, Enter:13, Space:32, Escape:27, Backspace:8, Delete:46, ArrowLeft:37, ArrowUp:38, ArrowRight:39, ArrowDown:40, Home:36, End:35 }[name] ?? (/^[a-z0-9]$/i.test(name) ? name.toUpperCase().charCodeAt(0) : undefined) };
        // MP-08/MP-10: a viewer key on the owned display goes through XTest
        // after the same document and mirror fences, and only
        // while the page itself has focus (never browser UI); otherwise CDP.
        const keysym = printable ? (/^[\x20-\x7e]$/.test(input.key) ? input.key.charCodeAt(0) : null) : keysyms[name];
        if (nativeKey && !resolved && !chord && keysym != null) {
          await check();
          await mirrorGuard?.();
          assertNotCancelled(signal);
          if (documentCurrent) cachedDocument();
          // MP-08/MP-10/MP-11: all asynchronous owner/region admission precedes
          // the final live focus check; no later await on the fast path.
          const focused = await pageFocus();
          assertNotCancelled(signal);
          if (documentCurrent) cachedDocument(); else await check();
          if (focused === true && nativeKey(keysym, Boolean(modifiers & 8))) { onDispatch?.(); return; }
        }
        const down = { type: "keyDown", ...key,
          ...(!chord && (["Enter", "Space"].includes(input.key) || printable) ? { text: printable ? input.key : input.key === "Enter" ? "\r" : " ", unmodifiedText: printable ? input.key : input.key === "Enter" ? "\r" : " " } : {}) };
        await admitInput();
        // MP-08/MP-10/MP-11: Enter may navigate before its acknowledgement.
        // Admit the press once, then order its non-text release on the same
        // session before waiting, just as for a mouse click. Cancellation or
        // navigation cannot strand the key or replay the press.
        const endDispatch = onDispatch?.();
        try {
          const press = connection.send("Input.dispatchKeyEvent", down, sessionId);
          const release = connection.send("Input.dispatchKeyEvent", { type: "keyUp", ...key }, sessionId);
          const replies = await Promise.allSettled([press, release]);
          const failed = replies.find(reply => reply.status === "rejected");
          if (failed) throw failed.reason;
          assertNotCancelled(signal);
        } finally { endDispatch?.(); }
      } else {
        if (!Number.isInteger(input.x) || input.x < 0 || input.x >= viewport.css_width || !Number.isInteger(input.y) || input.y < 0 || input.y >= viewport.css_height) throw new PointerOutsideViewport();
        // Protocol 530: middle/right buttons are the band's (Chrome's own UI), never a page click.
        if (input.kind === "click" && input.button !== undefined) throw new Error("MD-2: unsupported input");
        if (input.kind === "click") {
          // MP-08/MP-10: a viewer click on the owned display goes through
          // XTest after the same fences (no renderer acknowledgement wait).
          if (nativeClick && !resolved) {
            await check(); await mirrorGuard?.();
            if (await nativeOnce(() => nativeClick(input.x, input.y))) { onDispatch?.(); return; }
          }
          // MP-08/MP-10: CDP pointer coordinates are in the emulated view's
          // space: a DPR1 page on a scale-2 window has view scale 1/2.
          const x = input.x * viewScale, y = input.y * viewScale;
          // MP-11: a fenced mirror click's guard is the final document and
          // live hit fence (fenced evaluate) immediately before the press.
          if (resolved?.fenced) await mirrorGuard(); else { await check(); await mirrorGuard?.(); }
          // MP-08/MP-10/MP-11: CDP orders commands on this session. Queue the
          // matching release before awaiting the frame-paced press reply. Once
          // admitted, navigation/cancellation must not strand a pressed button.
          onDispatch?.();
          const press = connection.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 }, sessionId);
          const release = connection.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 }, sessionId);
          const replies = await Promise.allSettled([press, release]);
          const failed = replies.find(reply => reply.status === "rejected");
          if (failed) throw failed.reason;
          assertNotCancelled(signal);
        } else if (input.kind === "scroll" && Number.isInteger(input.delta_x) && Number.isInteger(input.delta_y) && Math.abs(input.delta_x) <= 10000 && Math.abs(input.delta_y) <= 10000) {
          const nx = notches(input.delta_x), ny = notches(input.delta_y);
          if (nativeWheel && nx !== null && ny !== null && Math.abs(nx) <= 10 && Math.abs(ny) <= 10 && (nx || ny)) {
            await check(); await mirrorGuard?.();
            // MP-11: a retired source has dispatched nothing. Fall back to
            // fenced CDP input without claiming an uncertain native action.
            if (await nativeOnce(() => nativeWheel(input.x, input.y, nx, ny))) { onDispatch?.(); return; }
          }
          const params = { type: "mouseWheel", x: input.x * viewScale, y: input.y * viewScale, deltaX: input.delta_x, deltaY: input.delta_y };
          if (!asyncScroll || (typeof asyncScroll === 'function' && !asyncScroll())) { await sendInput("Input.dispatchMouseEvent", params); return; }
          // MP-08/MP-10: the document fence and dispatch stay serialized; the
          // caller does not hold the input lane for the renderer's
          // frame-aligned wheel ack. CDP preserves dispatch order.
          await check(); await mirrorGuard?.(); onDispatch?.();
          const ack = connection.send("Input.dispatchMouseEvent", params, sessionId);
          ack.catch(() => {});
          return { ack };
        } else if (input.kind === "drag" && !resolved) {
          // Protocol 530: a primary-button drag in the page (text selection for Notes on the video path).
          if (!Number.isInteger(input.to_x) || input.to_x < 0 || input.to_x >= viewport.css_width || !Number.isInteger(input.to_y) || input.to_y < 0 || input.to_y >= viewport.css_height) throw new Error("MD-2: pointer outside viewport");
          const from = [input.x * viewScale, input.y * viewScale], to = [input.to_x * viewScale, input.to_y * viewScale];
          await check();
          await check();
          // Once pressed, the moves and the release are queued with it: cancellation or navigation
          // cannot strand the button (as for a click).
          onDispatch?.();
          const steps = 8, at = i => ({ x: from[0] + (to[0] - from[0]) * i / steps, y: from[1] + (to[1] - from[1]) * i / steps });
          const sent = [connection.send("Input.dispatchMouseEvent", { type: "mousePressed", ...at(0), button: "left", buttons: 1, clickCount: 1 }, sessionId)];
          for (let i = 1; i <= steps; i++) sent.push(connection.send("Input.dispatchMouseEvent", { type: "mouseMoved", ...at(i), button: "left", buttons: 1 }, sessionId));
          sent.push(connection.send("Input.dispatchMouseEvent", { type: "mouseReleased", ...at(steps), button: "left", buttons: 0, clickCount: 1 }, sessionId));
          const failed = (await Promise.allSettled(sent)).find(reply => reply.status === "rejected");
          if (failed) throw failed.reason;
          assertNotCancelled(signal);
        } else throw new Error("MD-2: unsupported input");
      }
    }, { resumeFocus: input.kind !== 'scroll' });
  }
