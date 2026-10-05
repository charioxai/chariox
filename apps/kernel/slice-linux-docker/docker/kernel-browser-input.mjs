// MD-3: document-bound physical input, sharing Room cancellation and document checks.
import { assertCurrentDocument, assertNotCancelled, BrowserActionError } from "./browser-controller-actions.mjs";
import { sensitiveHostInput } from "./kernel-browser-input-protection.mjs";
export { sensitiveHostInput };
const viewport = { css_width: 1280, css_height: 800 };
export async function inputHostTab(browser, tab, input, { signal, onDispatch, requireRoutine = false } = {}) {
    assertNotCancelled(signal);
    const { connection, sessionId } = await browser.resolvePageTarget(tab.target_id);
    const check = async (navigationRelease = false) => {
      assertNotCancelled(signal);
      await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
      if (requireRoutine && !navigationRelease && await sensitiveHostInput(browser, tab, input)) {
        throw new BrowserActionError("sensitive_requires_focus", "MP-11: sensitive user-domain action requires focus or human approval");
      }
      assertNotCancelled(signal);
    };
    const sendInput = async (method, params, navigationRelease = false) => {
      await check(navigationRelease);
      onDispatch?.();
      const result = await connection.send(method, params, sessionId);
      assertNotCancelled(signal);
      return result;
    };
    return browser.inputCapture.run(connection, sessionId, async () => {
      await check();
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
        if (!["Tab", "Enter", "Space", "Escape", "Backspace", "Delete", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(input.key)) throw new Error("MD-2: unsupported key");
        const key = { key: input.key === "Space" ? " " : input.key, code: input.key,
          ...(["Enter", "Space"].includes(input.key) ? { windowsVirtualKeyCode: input.key === "Enter" ? 13 : 32 } : {}) };
        await sendInput("Input.dispatchKeyEvent", { type: "keyDown", ...key,
          ...(["Enter", "Space"].includes(input.key) ? { text: input.key === "Enter" ? "\r" : " ", unmodifiedText: input.key === "Enter" ? "\r" : " " } : {}) });
        // Tab's paired release cannot natively activate the newly focused
        // button. Still check document, cancellation and grant authority.
        await sendInput("Input.dispatchKeyEvent", { type: "keyUp", ...key }, input.key === "Tab");
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
