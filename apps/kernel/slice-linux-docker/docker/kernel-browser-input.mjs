import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
// MD-3: document-bound physical input, sharing Room cancellation and document checks.
import { UserDomainRefusal } from "./kernel-browser-refusal.mjs";
import { assertCurrentDocument, assertNotCancelled } from "./browser-controller-actions.mjs";
const viewport = { css_width: geometry.width, css_height: geometry.height };
// MP-08: Chromium uses virtual key codes for native caret/editing commands.
const keyCodes = { Tab: 9, Enter: 13, Escape: 27, Backspace: 8, Delete: 46,
  ArrowLeft: 37, ArrowRight: 39, ArrowUp: 38, ArrowDown: 40, Home: 36, End: 35 };
// MP-08/MP-10: X keysyms for owned-display viewer keys (printable ASCII maps 1:1).
const keysyms = { Tab: 0xff09, Enter: 0xff0d, Space: 0x20, Escape: 0xff1b, Backspace: 0xff08, Delete: 0xffff,
  ArrowLeft: 0xff51, ArrowUp: 0xff52, ArrowRight: 0xff53, ArrowDown: 0xff54, Home: 0xff50, End: 0xff57 };
// MP-08/MP-10: mouse-wheel notches animate like a native wheel through the
// owned display (Selkies forwards every wheel event as X button notches).
// Viewers report a notch as 120 (wheelDelta), 100 (Chrome on Windows,
// automation) or other line-sized deltas; fine trackpad deltas (< 50 px)
// stay precise on CDP.
export const notches = delta => delta % 120 === 0 ? delta / 120 : Math.abs(delta) >= 50 ? Math.round(delta / 100) || Math.sign(delta) : null;
// MP-11: native pointer input is at most once. An action sent to the native
// worker without a reply may have reached the page, so it counts as dispatched
// (the retired source re-observes the outcome) and is never replayed via CDP.
async function nativeOnce(dispatch) {
  try { return await dispatch(); }
  catch (error) { if (error?.code === "native_input_uncertain") return true; throw error; }
}

export async function inputHostTab(browser, tab, input, { signal, onDispatch, resolveMirror, asyncScroll = false, nativeWheel = null, nativeClick = null, nativeKey = null } = {}) {
    assertNotCancelled(signal);
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
    const sensitive = () => `(() => { let e = document.activeElement; while(e) {
          if(e.type === 'password' || /password|one-time-code|cc-/i.test(e.autocomplete || '') || e.closest('[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected]')) return true;
          if(e.shadowRoot?.activeElement) { e = e.shadowRoot.activeElement; continue; }
          if(e.tagName === 'IFRAME') {
            if(!${observedFrameInput}) return true;
            try { const leaf = e.contentDocument?.activeElement; if(!leaf) return true; e = leaf; continue; } catch { return true; }
          }
          return false;
        } return true; })()`;
    const evaluateText = async expression => {
      const evaluate = async contextId => (await connection.send("Runtime.evaluate", { contextId, expression, returnByValue: true }, sessionId)).result?.value;
      try { return await evaluate(await textWorld()); }
      catch { return await evaluate(await textWorld(true)); }
    };
    const checkTextTarget = async () => {
      if (await evaluateText(sensitive()) !== false) throw new UserDomainRefusal("sensitive_requires_focus");
    };
    let mirrorGuard;
    const sendInput = async (method, params) => {
      await check();
      if (method === "Input.insertText" || method === "Input.imeSetComposition" || (method === "Input.dispatchKeyEvent" && params.text)) {
        await checkTextTarget();
        await check();
      }
      // MP-11: live mirror focus is the final fence, after every text preflight.
      await mirrorGuard?.();
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
      await check();resolved=await resolveMirror(input);
      observedFrameInput=resolved.observedFrameInput===true;
    }
    return browser.inputCapture.run(connection, sessionId, async () => {
      await check();
      if(resolved) {
        mirrorGuard=resolved.guard;
        if(resolved.perform) {await check();await resolved.perform(sendInput,onDispatch);await check();return;}
        input=resolved.input;
      }
      if (input.kind === "text") {
        if (typeof input.text !== "string" || input.text.length > 16384) throw new Error("MD-2: input text exceeds limit");
        await sendInput("Input.insertText", { text: input.text });
      } else if (input.kind === "key") {
        const printable = typeof input.key === "string" && /^[^\p{C}]$/u.test(input.key);
        if (!printable && !["Tab", "Shift+Tab", "Enter", "Space", "Escape", "Backspace", "Delete", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(input.key)) throw new Error("MD-2: unsupported key");
        const name = input.key === "Shift+Tab" ? "Tab" : input.key;
        const key = { key: name === "Space" ? " " : name, code: name,
          ...(input.key === "Shift+Tab" ? { modifiers: 8 } : {}),
          windowsVirtualKeyCode: { Tab:9, Enter:13, Space:32, Escape:27, Backspace:8, Delete:46, ArrowLeft:37, ArrowUp:38, ArrowRight:39, ArrowDown:40, Home:36, End:35 }[name] };
        // MP-08/MP-10: a viewer key on the owned display goes through XTest
        // after the same document, text-target and mirror fences, and only
        // while the page itself has focus (never browser UI); otherwise CDP.
        const keysym = printable ? (/^[\x20-\x7e]$/.test(input.key) ? input.key.charCodeAt(0) : null) : keysyms[name];
        if (nativeKey && !resolved && keysym != null) {
          await check();
          const text = printable || ["Enter", "Space"].includes(input.key);
          const focused = await evaluateText(text ? `${sensitive()} ? 'sensitive' : document.hasFocus()` : "document.hasFocus()");
          if (focused === "sensitive") throw new UserDomainRefusal("sensitive_requires_focus");
          await check(); await mirrorGuard?.();
          if (focused === true && nativeKey(keysym, input.key === "Shift+Tab")) { onDispatch?.(); return; }
        }
        await sendInput("Input.dispatchKeyEvent", { type: "keyDown", ...key,
          ...(["Enter", "Space"].includes(input.key) || printable ? { text: printable ? input.key : input.key === "Enter" ? "\r" : " ", unmodifiedText: printable ? input.key : input.key === "Enter" ? "\r" : " " } : {}) });
        // MP-08: paired releases keep document and live cancellation checks.
        await sendInput("Input.dispatchKeyEvent", { type: "keyUp", ...key });
      } else {
        if (!Number.isInteger(input.x) || input.x < 0 || input.x >= viewport.css_width || !Number.isInteger(input.y) || input.y < 0 || input.y >= viewport.css_height) throw new Error("MD-2: pointer outside viewport");
        if (input.kind === "click") {
          // MP-08/MP-10: a viewer click on the owned display goes through
          // XTest after the same fences (no renderer acknowledgement wait).
          if (nativeClick && !resolved) {
            await check(); await mirrorGuard?.();
            if (await nativeOnce(() => nativeClick(input.x, input.y))) { onDispatch?.(); return; }
          }
          await sendInput("Input.dispatchMouseEvent", { type: "mousePressed", x: input.x, y: input.y, button: "left", clickCount: 1 });
          await sendInput("Input.dispatchMouseEvent", { type: "mouseReleased", x: input.x, y: input.y, button: "left", clickCount: 1 });
        } else if (input.kind === "scroll" && Number.isInteger(input.delta_x) && Number.isInteger(input.delta_y) && Math.abs(input.delta_x) <= 10000 && Math.abs(input.delta_y) <= 10000) {
          const nx = notches(input.delta_x), ny = notches(input.delta_y);
          if (nativeWheel && nx !== null && ny !== null && Math.abs(nx) <= 10 && Math.abs(ny) <= 10 && (nx || ny)) {
            await check(); await mirrorGuard?.();
            // MP-11: a retired source has dispatched nothing. Fall back to
            // fenced CDP input without claiming an uncertain native action.
            if (await nativeOnce(() => nativeWheel(input.x, input.y, nx, ny))) { onDispatch?.(); return; }
          }
          const params = { type: "mouseWheel", x: input.x, y: input.y, deltaX: input.delta_x, deltaY: input.delta_y };
          if (!asyncScroll || (typeof asyncScroll === 'function' && !asyncScroll())) { await sendInput("Input.dispatchMouseEvent", params); return; }
          // MP-08/MP-10: the document fence and dispatch stay serialized; the
          // caller does not hold the input lane for the renderer's
          // frame-aligned wheel ack. CDP preserves dispatch order.
          await check(); await mirrorGuard?.(); onDispatch?.();
          const ack = connection.send("Input.dispatchMouseEvent", params, sessionId);
          ack.catch(() => {});
          return { ack };
        } else throw new Error("MD-2: unsupported input");
      }
    });
  }
