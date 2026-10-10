// MD-3: document-bound physical input, sharing Room cancellation and document checks.
import { UserDomainRefusal } from "./kernel-browser-refusal.mjs";
import { assertCurrentDocument, assertNotCancelled } from "./browser-controller-actions.mjs";
const viewport = { css_width: 1280, css_height: 800 };
// MP-08: Chromium uses virtual key codes for native caret/editing commands.
const keyCodes = { Tab: 9, Enter: 13, Escape: 27, Backspace: 8, Delete: 46,
  ArrowLeft: 37, ArrowRight: 39, ArrowUp: 38, ArrowDown: 40, Home: 36, End: 35 };
export async function inputHostTab(browser, tab, input, { signal, onDispatch, resolveMirror } = {}) {
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
    const checkTextTarget = async () => {
      const { frameTree } = await connection.send("Page.getFrameTree", {}, sessionId);
      const { executionContextId } = await connection.send("Page.createIsolatedWorld", {
        frameId: frameTree.frame.id, worldName: "chariox-host-input", grantUniveralAccess: false,
      }, sessionId);
      const { result } = await connection.send("Runtime.evaluate", {
        contextId: executionContextId,
        // MP-08/MP-11: admitted mirrors may target observed same-origin frame
        // descendants. Inspect the live leaf in this isolated world; direct
        // frame input and inaccessible/protected frames still fail closed.
        expression: `(() => { let e = document.activeElement; while(e) {
          if(e.type === 'password' || /password|one-time-code|cc-/i.test(e.autocomplete || '') || e.closest('[data-chariox-secret],[data-chariox-observation-protected],[data-observation-protected]')) return true;
          if(e.shadowRoot?.activeElement) { e = e.shadowRoot.activeElement; continue; }
          if(e.tagName === 'IFRAME') {
            if(!${observedFrameInput}) return true;
            try { const leaf = e.contentDocument?.activeElement; if(!leaf) return true; e = leaf; continue; } catch { return true; }
          }
          return false;
        } return true; })()`,
        returnByValue: true,
      }, sessionId);
      if (result?.value !== false) throw new UserDomainRefusal("sensitive_requires_focus");
    };
    let mirrorGuard;
    const sendInput = async (method, params) => {
      await check();
      await mirrorGuard?.();
      if (method === "Input.insertText" || (method === "Input.dispatchKeyEvent" && params.text)) {
        await checkTextTarget();
        await check();
      }
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
        await sendInput("Input.dispatchKeyEvent", { type: "keyDown", ...key,
          ...(["Enter", "Space"].includes(input.key) || printable ? { text: printable ? input.key : input.key === "Enter" ? "\r" : " ", unmodifiedText: printable ? input.key : input.key === "Enter" ? "\r" : " " } : {}) });
        // MP-08: paired releases keep document and live cancellation checks.
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
