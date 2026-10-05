// MD-3: document-bound physical input, sharing Room cancellation and document checks.
import { assertCurrentDocument, assertNotCancelled } from "./browser-controller-actions.mjs";
import { protectedHostRegions } from "./kernel-browser-region-protection.mjs";
const viewport = { css_width: 1280, css_height: 800 };
// MP-11: value-free protection classification before physical agent input.
export async function sensitiveHostInput(browser, tab, input) {
  if (input.kind === "scroll") return false;
  const { connection, sessionId } = await browser.resolvePageTarget(tab.target_id);
  await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
  const { frameTree } = await connection.send("Page.getFrameTree", {}, sessionId);
  const { executionContextId } = await connection.send("Page.createIsolatedWorld", {
    frameId: frameTree.frame.id, worldName: "chariox-host-protection", grantUniveralAccess: false,
  }, sessionId);
  const point = input.kind === "click" ? `document.elementFromPoint(${JSON.stringify(input.x)},${JSON.stringify(input.y)})` : "document.activeElement";
  const { result } = await connection.send("Runtime.evaluate", {
    contextId: executionContextId,
    expression: `(() => {
      let e = ${point};
      if (!e) return true;
      while (e.shadowRoot?.activeElement) e = e.shadowRoot.activeElement;
      const protectedSelector = '[data-chariox-sensitive],[data-chariox-observation-protected],input[type="password"],[autocomplete="one-time-code"],[autocomplete^="cc-"],iframe,frame';
      if (e.matches(protectedSelector) || e.closest(protectedSelector) || e.shadowRoot) return true;
      const form = e.form || e.closest('form');
      if (form?.querySelector(protectedSelector)) return true;
      let action = e.closest('button,a,[role="button"],input[type="submit"],input[type="image"]');
      if (${JSON.stringify(input.kind === "key" && input.key === "Enter")}) {
        // Native implicit submission activates the first associated submit
        // control in tree order, including controls outside the form element.
        if (e.tagName === 'INPUT' && !['submit','image','button','reset'].includes(e.type)) {
          if (!form) return true;
          action = [...e.getRootNode().querySelectorAll('button,input')].find(control =>
            control.form === form && (control.type === 'submit' || control.type === 'image'));
          if (!action) return true; // Buttonless or unknown activation needs focus.
        } else if (!action && e.tagName !== 'TEXTAREA' && !e.isContentEditable) return true;
        if (action && ![action.textContent, action.getAttribute('aria-label'), action.value, action.getAttribute('alt')].some(label => label?.trim())) return true;
      }
      if (action?.matches(protectedSelector) || action?.closest(protectedSelector)) return true;
      const label = action ? [action.textContent, action.getAttribute('aria-label'), action.value, action.getAttribute('alt')].filter(Boolean).join(' ') : '';
      return /\\b(pay|purchase|buy|checkout|authorize|approve|confirm payment)\\b/i.test(label);
    })()`, returnByValue: true,
  }, sessionId);
  if (result?.value !== false) return true;
  // CDP sees closed shadow hosts too. Use the capture protection model rather
  // than treating an opaque page subtree as a routine physical target.
  try {
    const regions = await protectedHostRegions(connection, sessionId);
    return input.kind === "click"
      ? regions.some(r => input.x >= r.x && input.x <= r.x + r.width && input.y >= r.y && input.y <= r.y + r.height)
      : regions.length > 0;
  } catch { return true; }
}
export async function inputHostTab(browser, tab, input, { signal, onDispatch, requireRoutine = false } = {}) {
    assertNotCancelled(signal);
    const { connection, sessionId } = await browser.resolvePageTarget(tab.target_id);
    const check = async (navigationRelease = false) => {
      assertNotCancelled(signal);
      await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
      if (requireRoutine && !navigationRelease && await sensitiveHostInput(browser, tab, input)) {
        throw new Error("MP-11: sensitive user-domain action requires focus or human approval");
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
        if (!["Tab", "Enter", "Escape", "Backspace", "Delete", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(input.key)) throw new Error("MD-2: unsupported key");
        const key = { key: input.key, code: input.key,
          ...(input.key === "Enter" ? { windowsVirtualKeyCode: 13 } : {}) };
        await sendInput("Input.dispatchKeyEvent", { type: "keyDown", ...key,
          ...(input.key === "Enter" ? { text: "\r", unmodifiedText: "\r" } : {}) });
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
