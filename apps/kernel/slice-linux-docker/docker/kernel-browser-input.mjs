// MD-3: document-bound physical input, sharing Room cancellation and document checks.
import { assertCurrentDocument, assertNotCancelled } from "./browser-controller-actions.mjs";
const viewport = { css_width: 1280, css_height: 800 };
export async function inputHostTab(browser, tab, input, { signal, onDispatch, resolveMirror } = {}) {
    assertNotCancelled(signal);
    const { connection, sessionId } = await browser.resolvePageTarget(tab.target_id);
    const check = async () => {
      assertNotCancelled(signal);
      await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
      assertNotCancelled(signal);
    };
    let mirrorGuard;
    const sendInput = async (method, params) => {
      await check();
      await mirrorGuard?.();
      onDispatch?.();
      const result = await connection.send(method, params, sessionId);
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
        const { frameTree } = await connection.send("Page.getFrameTree", {}, sessionId);
        const { executionContextId } = await connection.send("Page.createIsolatedWorld", {
          frameId: frameTree.frame.id, worldName: "chariox-host-input", grantUniveralAccess: false,
        }, sessionId);
        const { result } = await connection.send("Runtime.evaluate", {
          contextId: executionContextId,
          expression: "(() => { let e = document.activeElement; while(e?.shadowRoot?.activeElement) e = e.shadowRoot.activeElement; return !!e && (e.type === 'password' || e.tagName === 'IFRAME' || /password|one-time-code/.test(e.autocomplete || '')); })()",
          returnByValue: true,
        }, sessionId);
        if (result?.value !== false) throw new Error("MD-2: secret field input requires the Vault path");
        await sendInput("Input.insertText", { text: input.text });
      } else if (input.kind === "key") {
        if (!["Tab", "Enter", "Escape", "Backspace", "Delete", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(input.key)) throw new Error("MD-2: unsupported key");
        const key = { key: input.key, code: input.key,
          ...(input.key === "Enter" ? { windowsVirtualKeyCode: 13 } : {}) };
        await sendInput("Input.dispatchKeyEvent", { type: "keyDown", ...key,
          ...(input.key === "Enter" ? { text: "\r", unmodifiedText: "\r" } : {}) });
        await sendInput("Input.dispatchKeyEvent", { type: "keyUp", ...key });
      } else {
        if (!Number.isInteger(input.x) || input.x < 0 || input.x >= viewport.css_width || !Number.isInteger(input.y) || input.y < 0 || input.y >= viewport.css_height) throw new Error("MD-2: pointer outside viewport");
        if (input.kind === "click") {
          await sendInput("Input.dispatchMouseEvent", { type: "mousePressed", x: input.x, y: input.y, button: "left", clickCount: 1 });
          await sendInput("Input.dispatchMouseEvent", { type: "mouseReleased", x: input.x, y: input.y, button: "left", clickCount: 1 });
        } else if (input.kind === "scroll" && Number.isInteger(input.delta_x) && Number.isInteger(input.delta_y) && Math.abs(input.delta_x) <= 10000 && Math.abs(input.delta_y) <= 10000) {
          await sendInput("Input.dispatchMouseEvent", { type: "mouseWheel", x: input.x, y: input.y, deltaX: input.delta_x, deltaY: input.delta_y });
        } else throw new Error("MD-2: unsupported input");
      }
    });
  }
