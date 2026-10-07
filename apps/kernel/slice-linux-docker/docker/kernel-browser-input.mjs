import {displayGeometry as geometry} from './kernel-browser-geometry.mjs';
// MD-3: document-bound physical input, sharing Room cancellation and document checks.
import { UserDomainRefusal } from "./kernel-browser-refusal.mjs";
import { assertCurrentDocument, assertNotCancelled } from "./browser-controller-actions.mjs";
const viewport = { css_width: geometry.width, css_height: geometry.height };
// MP-08: Chromium uses virtual key codes for native caret/editing commands.
const keyCodes = { Tab: 9, Enter: 13, Space: 32, Escape: 27, Backspace: 8, Delete: 46,
  ArrowLeft: 37, ArrowRight: 39, ArrowUp: 38, ArrowDown: 40, Home: 36, End: 35 };
export async function inputHostTab(browser, tab, input, { signal, onDispatch, resolveMirror, coordinateScale = 1 } = {}) {
    assertNotCancelled(signal);
    if(![.5,1].includes(coordinateScale))throw new Error('MD-2: unsupported native pointer scale');
    const { connection, sessionId } = await browser.resolvePageTarget(tab.target_id);
    const check = async () => {
      assertNotCancelled(signal);
      await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
      assertNotCancelled(signal);
    };
    // MP-11: all text-producing paths share the Vault-only target fence.
    // The public key string and MCP schema are not security boundaries.
    const checkTextTarget = async () => {
      const { frameTree } = await connection.send("Page.getFrameTree", {}, sessionId);
      const { executionContextId } = await connection.send("Page.createIsolatedWorld", {
        frameId: frameTree.frame.id, worldName: "chariox-host-input", grantUniveralAccess: false,
      }, sessionId);
      const { result } = await connection.send("Runtime.evaluate", {
        contextId: executionContextId,
        expression: "(() => { let e = document.activeElement; while(e?.shadowRoot?.activeElement) e = e.shadowRoot.activeElement; return !!e && (e.type === 'password' || e.tagName === 'IFRAME' || /password|one-time-code/.test(e.autocomplete || '')); })()",
        returnByValue: true,
      }, sessionId);
      if (result?.value !== false) throw new UserDomainRefusal("sensitive_requires_focus");
    };
    let mirrorGuard;
    const sendInput = async (method, params) => {
      await check();
      if (method === "Input.insertText" || (method === "Input.dispatchKeyEvent" && params.text)) {
        await checkTextTarget();
        await check();
      }
      await mirrorGuard?.();
      onDispatch?.();
      // Headed canonical DPR1 uses Emulation.scale=.5 on the DPR2 host.
      // CDP pointer positions address that scaled viewport; semantic admission
      // above always uses the original CSS coordinates. Wheel deltas stay CSS.
      const nativeParams=method==='Input.dispatchMouseEvent'&&coordinateScale!==1
        ? {...params,x:params.x*coordinateScale,y:params.y*coordinateScale}:params;
      const result = await connection.send(method, nativeParams, sessionId);
      assertNotCancelled(signal);
      return result;
    };
    // MP-11: sequence-only refusals precede even focus emulation. No page
    // focus/selection/physical input may run before mirror epoch admission.
    let resolved;
    if(input.kind==='mirror') {
      if(!resolveMirror)throw new Error('MP-11: mirror input resolver unavailable');
      await check();resolved=await resolveMirror(input);
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
        const name = input.key === "Shift+Tab" ? "Tab" : input.key;
        if (!printable && !Object.hasOwn(keyCodes, name)) throw new Error("MD-2: unsupported key");
        const key = { key: name === "Space" ? " " : name, code: name,
          ...(input.key === "Shift+Tab" ? { modifiers: 8 } : {}),
          windowsVirtualKeyCode: keyCodes[name] };
        await sendInput("Input.dispatchKeyEvent", { type: "keyDown", ...key,
          ...(["Enter", "Space"].includes(input.key) || printable ? { text: printable ? input.key : input.key === "Enter" ? "\r" : " ", unmodifiedText: printable ? input.key : input.key === "Enter" ? "\r" : " " } : {}) });
        // MP-08: paired releases keep document and live cancellation checks.
        await sendInput("Input.dispatchKeyEvent", { type: "keyUp", ...key });
      } else {
        if (!Number.isInteger(input.x) || input.x < 0 || input.x >= viewport.css_width || !Number.isInteger(input.y) || input.y < 0 || input.y >= viewport.css_height) throw new Error("MD-2: pointer outside viewport");
        if (input.kind === "click") {
          await sendInput("Input.dispatchMouseEvent", { type: "mousePressed", x: input.x, y: input.y, button: "left", clickCount: 1 });
          await sendInput("Input.dispatchMouseEvent", { type: "mouseReleased", x: input.x, y: input.y, button: "left", clickCount: 1 });
        } else if (input.kind === "scroll" && Number.isInteger(input.delta_x) && Number.isInteger(input.delta_y) && Math.abs(input.delta_x) <= (mirrorGuard ? 1000000 : 10000) && Math.abs(input.delta_y) <= (mirrorGuard ? 1000000 : 10000)) {
          await sendInput("Input.dispatchMouseEvent", { type: "mouseWheel", x: input.x, y: input.y, deltaX: input.delta_x, deltaY: input.delta_y });
        } else throw new Error("MD-2: unsupported input");
      }
    });
  }
