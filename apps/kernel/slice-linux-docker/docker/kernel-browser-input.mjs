import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
// MD-3: document-bound physical input, sharing Room cancellation and document checks.
import { PointerOutsideViewport, UserDomainRefusal } from "./kernel-browser-refusal.mjs";
import { assertCurrentDocument, assertNotCancelled } from "./browser-controller-actions.mjs";
const viewport = { css_width: geometry.width, css_height: geometry.height };
// MP-08: Chromium uses virtual key codes for native caret/editing commands.
const keyCodes = { Tab: 9, Enter: 13, Escape: 27, Backspace: 8, Delete: 46,
  ArrowLeft: 37, ArrowRight: 39, ArrowUp: 38, ArrowDown: 40, Home: 36, End: 35 };
// MP-08/MP-10: X keysyms for owned-display viewer keys (printable ASCII maps 1:1).
export const keysyms = { Tab: 0xff09, Enter: 0xff0d, Space: 0x20, Escape: 0xff1b, Backspace: 0xff08, Delete: 0xffff,
  ArrowLeft: 0xff51, ArrowUp: 0xff52, ArrowRight: 0xff53, ArrowDown: 0xff54, Home: 0xff50, End: 0xff57 };
// MP-08/MP-10: mouse-wheel notches animate like a native wheel through the
// owned display (Selkies forwards every wheel event as X button notches).
// Viewers report a notch as 120 (wheelDelta), 100 (Chrome on Windows,
// automation) or fine trackpad deltas. An admitted native display starts
// a notch for each nonzero fine delta; other callers retain precise CDP deltas.

// MP-11: the focused element (through open shadow roots and, for admitted mirror
// input, same-origin frames) must not be a sensitive field. A cross-origin frame
// answers 'frame': its own world decides (see checkTextTarget).
export const sensitiveFocusExpression = observedFrameInput => `(() => { let e = document.activeElement; while(e) {
          if(e.type === 'password' || /password|one-time-code|cc-/i.test(e.autocomplete || '') || e.closest('[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected]')) return true;
          if(e.shadowRoot?.activeElement) { e = e.shadowRoot.activeElement; continue; }
          if(e.tagName === 'IFRAME') {
            if(!${observedFrameInput}) return true;
            let leaf = null; try { leaf = e.contentDocument?.activeElement; } catch {}
            if(!leaf) return e.contentDocument ? true : 'frame'; e = leaf; continue;
          }
          return false;
        } return true; })()`;

// MP-08/MP-11 (530 page drag): a drag may neither start on (select) nor drop onto a protected field.
// Same-origin frames are read through; a cross-origin frame cannot be inspected, so it fails closed.
export const protectedPointExpression = (x, y) => `(() => { let d = document, x = ${x}, y = ${y};
  for (let depth = 0; depth < 8; depth++) {
    let e = d.elementFromPoint(x, y);
    for (let n = 0; e?.shadowRoot && n < 32; n++) { const inner = e.shadowRoot.elementFromPoint(x, y); if (!inner || inner === e) break; e = inner; }
    if (!e) return false;
    if (e.type === 'password' || /password|one-time-code|cc-/i.test(e.autocomplete || '') || e.closest('[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected]')) return true;
    if (e.tagName !== 'IFRAME') return false;
    let inner = null; try { inner = e.contentDocument; } catch {} if (!inner) return true;
    const r = e.getBoundingClientRect(); x -= r.left + e.clientLeft; y -= r.top + e.clientTop; d = inner;
  } return true; })()`;

export const notches = delta => delta % 120 === 0 ? delta / 120 : Math.round(delta / 100) || Math.sign(delta);
// MP-11: native pointer input is at most once. An action sent to the native
// worker without a reply may have reached the page, so it counts as dispatched
// (the retired source re-observes the outcome) and is never replayed via CDP.
async function nativeOnce(dispatch) {
  try { return await dispatch(); }
  catch (error) { if (error?.code === "native_input_uncertain") return true; throw error; }
}

export async function inputHostTab(browser, tab, input, { signal, onDispatch, resolveMirror, asyncScroll = false, nativeWheel = null, nativeClick = null, nativeKey = null, viewScale = 1 } = {}) {
    assertNotCancelled(signal);
    // MP-08/MP-11 protocol498: CDP bits Alt=1, Control=2, Meta=4, Shift=8.
    if (input.kind === 'key' && input.modifiers !== undefined && (!Number.isInteger(input.modifiers) || input.modifiers < 0 || input.modifiers > 15)) throw new Error('MP-11: invalid key modifiers');
    const { connection, sessionId } = await browser.resolvePageTarget(tab.target_id);
    let observedFrameInput = false;
    const check = async () => {
      assertNotCancelled(signal);
      await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
      assertNotCancelled(signal);
    };
    // MP-11: all text-producing paths share the Vault-only target fence.
    // The public key string and MCP schema are not security boundaries.
    // MP-08/MP-10 (2.3): one isolated world per document, reused by every
    // keystroke; a destroyed context (navigation, reload) is recreated once.
    const textWorld = async (fresh = false) => {
      const cache = browser.chariox_text_worlds ??= new Map();
      const cached = cache.get(tab.target_id);
      if (!fresh && cached?.document === tab.document_id) return cached.contextId;
      const { frameTree } = await connection.send("Page.getFrameTree", {}, sessionId);
      const { executionContextId } = await connection.send("Page.createIsolatedWorld", {
        frameId: frameTree.frame.id, worldName: "chariox-host-input", grantUniveralAccess: false,
      }, sessionId);
      if (frameTree.frame.loaderId === tab.document_id) cache.set(tab.target_id, { document: tab.document_id, contextId: executionContextId });
      return executionContextId;
    };
    // MP-08/MP-11: admitted mirrors may target observed same-origin frame
    // descendants. Inspect the live leaf in this isolated world; direct
    // frame input and inaccessible/protected frames still fail closed.
    const sensitive = () => sensitiveFocusExpression(observedFrameInput);
    const evaluateText = async expression => {
      const evaluate = async contextId => (await connection.send("Runtime.evaluate", { contextId, expression, returnByValue: true }, sessionId)).result?.value;
      try { return await evaluate(await textWorld()); }
      catch { return await evaluate(await textWorld(true)); }
    };
    const checkTextTarget = async () => {
      let verdict = await evaluateText(sensitive());
      // An admitted mirror focus inside a cross-origin frame: the same rules run
      // in that frame's own world (fail closed without a resolver).
      if (verdict === 'frame') verdict = resolved?.frameSensitive ? await resolved.frameSensitive(sensitiveFocusExpression(false)) : true;
      if (verdict !== false) throw new UserDomainRefusal("sensitive_requires_focus");
    };
    let mirrorGuard;
    const admitInput = async (method, params) => {
      await check();
      if (method === "Input.insertText" || method === "Input.imeSetComposition" || (method === "Input.dispatchKeyEvent" && (params.text || (params.modifiers & 7)))) {
        await checkTextTarget();
        // MP-11: a fenced mirror guard below checks the live document and
        // focused ancestry after this shared secret preflight. No mutation or
        // await separates a duplicate document read from that final guard.
        if (!resolved?.fenced) await check();
      }
      // MP-11: live mirror focus is the final fence, after every text preflight.
      await mirrorGuard?.();
    };
    const sendInput = async (method, params) => {
      await admitInput(method, params);
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
      observedFrameInput=resolved.observedFrameInput===true;
    }
    return browser.inputCapture.run(connection, sessionId, async () => {
      // MP-08/MP-10/MP-11: a fenced (v2) resolution has its final document
      // fence in perform/sendInput/its guard below. No mutation or await
      // separates this redundant query from that final fence.
      if (!resolved?.fenced) await check();
      if(resolved) {
        mirrorGuard=resolved.guard;
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
        // after the same document, text-target and mirror fences, and only
        // while the page itself has focus (never browser UI); otherwise CDP.
        const keysym = printable ? (/^[\x20-\x7e]$/.test(input.key) ? input.key.charCodeAt(0) : null) : keysyms[name];
        if (nativeKey && !resolved && !chord && keysym != null) {
          await check();
          const text = printable || ["Enter", "Space"].includes(input.key);
          const focused = await evaluateText(text ? `${sensitive()} ? 'sensitive' : document.hasFocus()` : "document.hasFocus()");
          if (focused === "sensitive") throw new UserDomainRefusal("sensitive_requires_focus");
          await check(); await mirrorGuard?.();
          if (focused === true && nativeKey(keysym, Boolean(modifiers & 8))) { onDispatch?.(); return; }
        }
        const down = { type: "keyDown", ...key,
          ...(!chord && (["Enter", "Space"].includes(input.key) || printable) ? { text: printable ? input.key : input.key === "Enter" ? "\r" : " ", unmodifiedText: printable ? input.key : input.key === "Enter" ? "\r" : " " } : {}) };
        await admitInput("Input.dispatchKeyEvent", down);
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
          if (await evaluateText(`${protectedPointExpression(...from)} || ${protectedPointExpression(...to)}`) !== false) throw new UserDomainRefusal("sensitive_requires_focus");
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
    });
  }
