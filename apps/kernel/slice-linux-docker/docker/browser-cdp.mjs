#!/usr/bin/env node

const command = process.argv[2];
const args = process.argv.slice(3);

async function readStdin() {
  const chunks = [];
  for await (const chunk of process.stdin) {
    chunks.push(Buffer.from(chunk));
  }
  return Buffer.concat(chunks).toString("utf8");
}

async function getPage() {
  const response = await fetch("http://127.0.0.1:9222/json/list");
  const targets = await response.json();
  const pages = targets.filter((target) => target.type === "page" && target.webSocketDebuggerUrl);
  const page =
    pages.find((target) => target.url?.includes("arroba-slice-screen-test")) ??
    pages.find((target) => target.url?.startsWith("file:///workspace/")) ??
    pages.find((target) => target.url && target.url !== "about:blank") ??
    pages.at(-1);
  if (!page) {
    throw new Error("No Chromium page target found");
  }
  return page;
}

async function withSocket(callback) {
  const page = await getPage();
  const socket = new WebSocket(page.webSocketDebuggerUrl);
  let nextId = 1;
  const pending = new Map();

  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    const resolver = pending.get(message.id);
    if (resolver) {
      pending.delete(message.id);
      resolver(message);
    }
  });

  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });

  const send = async (method, params = {}) => {
    const id = nextId++;
    socket.send(JSON.stringify({ id, method, params }));
    const response = await new Promise((resolve) => pending.set(id, resolve));
    if (response.error) {
      throw new Error(`${method}: ${response.error.message}`);
    }
    return response.result;
  };

  try {
    const result = await callback(send);
    await new Promise((resolve) => setTimeout(resolve, 1000));
    return result;
  } finally {
    socket.close();
  }
}

async function closeBrowser() {
  try {
    await withSocket(async (send) => {
      await send("Browser.close");
    });
  } catch (error) {
    const message = String(error?.message ?? error);
    if (
      !message.includes("WebSocket") &&
      !message.includes("terminated") &&
      !message.includes("closed")
    ) {
      throw error;
    }
  }
}

async function navigate(url) {
  await withSocket(async (send) => {
    await send("Page.navigate", { url });
  });
}

function evaluateExpression(source, args = {}) {
  return `
    (() => {
      const __arrobaArgs = ${JSON.stringify(args)};
      ${source}
    })()
  `;
}

async function evaluate(send, source, args = {}) {
  const result = await send("Runtime.evaluate", {
    expression: evaluateExpression(source, args),
    awaitPromise: false,
    returnByValue: true,
  });
  return result.result?.value;
}

async function showAgentCursor(send, x, y, clicked = false) {
  if (!Number.isFinite(x) || !Number.isFinite(y)) return;
  await evaluate(send, `
    const cursorId = "__arroba-agent-cursor";
    let cursor = document.getElementById(cursorId);
    if (!cursor) {
      cursor = document.createElement("div");
      cursor.id = cursorId;
      cursor.setAttribute("aria-hidden", "true");
      cursor.style.cssText = [
        "position:fixed",
        "left:0",
        "top:0",
        "width:28px",
        "height:36px",
        "border:3px solid #151515",
        "border-radius:999px",
        "background:#f09352",
        "box-shadow:0 2px 7px rgba(0,0,0,.55)",
        "z-index:2147483647",
        "pointer-events:none",
        "opacity:0",
        "transition:transform 90ms cubic-bezier(.2,.8,.2,1),opacity 160ms ease",
      ].join(";");
      cursor.innerHTML = \`
        <svg data-arroba-pointer viewBox="0 0 19 25" aria-hidden="true" style="position:absolute;left:-4px;top:-4px;width:28px;height:37px;filter:drop-shadow(0 2px 2px rgba(0,0,0,.72))">
          <path d="M1.5 1.7v18.1l4.7-4.2 3.2 7.4 3.2-1.4-3.1-7.2 6.4-.1L1.5 1.7Z" fill="#f09352" stroke="#151515" stroke-width="1.5" stroke-linejoin="round"/>
        </svg>
        <span data-arroba-tag style="position:absolute;left:24px;top:22px;padding:5px 9px;border:1px solid rgba(255,255,255,.72);border-radius:5px;background:rgba(18,18,18,.92);color:#f4f1ec;font:700 16px/1 ui-monospace,SFMono-Regular,Menlo,monospace;letter-spacing:.08em;text-transform:uppercase;white-space:nowrap;box-shadow:0 2px 7px rgba(0,0,0,.38)">agent</span>
        <span data-arroba-ring style="position:absolute;left:-16px;top:-16px;width:32px;height:32px;border:3px solid #f09352;border-radius:999px;opacity:0"></span>
      \`;
      document.documentElement.appendChild(cursor);
    }
    cursor.style.transform = \`translate3d(\${__arrobaArgs.x}px, \${__arrobaArgs.y}px, 0)\`;
    cursor.style.setProperty("opacity", "1", "important");
    cursor.style.setProperty("display", "block", "important");
    cursor.style.setProperty("visibility", "visible", "important");
    window.clearTimeout(window.__arrobaAgentCursorFadeTimer);
    window.__arrobaAgentCursorFadeTimer = window.setTimeout(() => {
      cursor.style.setProperty("opacity", "0", "important");
    // Keep the last automated pointer visible long enough for the agent's
    // response and the remote screen stream to settle in the controlling UI.
    }, 45000);
    if (__arrobaArgs.clicked) {
      const ring = cursor.querySelector?.("[data-arroba-ring]");
      ring?.animate?.([
        { transform: "scale(.35)", opacity: .95 },
        { transform: "scale(1.8)", opacity: 0 },
      ], { duration: 420, easing: "cubic-bezier(.2,.8,.2,1)" });
    }
    return { ok: true };
  `, { x, y, clicked });
}

async function typeIntoBrowser(text, options = {}) {
  await withSocket(async (send) => {
    if (options.useNativeInput) {
      const focusResult = await evaluate(send, `
        ${browserDomScript()}
        const selector = __arrobaArgs.selector;
        const target = selector
          ? resolveTarget(selector, ["field"])
          : document.activeElement?.matches?.("input, textarea, select, [contenteditable=true]")
            ? document.activeElement
            : null;
        if (!target) return { ok: false, error: selector ? "target_not_found" : "no_focused_fillable_element" };
        if (target.disabled || target.readOnly) return { ok: false, error: "target_not_editable" };
        if (!selector) {
          target.focus();
          return { ok: true };
        }
        target.scrollIntoView({ block: "center", inline: "center" });
        const rect = target.getBoundingClientRect();
        if (rect.width <= 0 || rect.height <= 0) return { ok: false, error: "target_not_visible" };
        return {
          ok: true,
          click: true,
          x: rect.left + rect.width / 2,
          y: rect.top + rect.height / 2,
        };
      `, { selector: options.selector ?? null });
      if (!focusResult?.ok) {
        throw new Error(focusResult?.error ?? "browser_focus_failed");
      }
      if (focusResult.click) {
        await showAgentCursor(send, focusResult.x, focusResult.y, true);
        await send("Input.dispatchMouseEvent", {
          type: "mouseMoved",
          x: focusResult.x,
          y: focusResult.y,
        });
        await send("Input.dispatchMouseEvent", {
          type: "mousePressed",
          x: focusResult.x,
          y: focusResult.y,
          button: "left",
          clickCount: 1,
        });
        await send("Input.dispatchMouseEvent", {
          type: "mouseReleased",
          x: focusResult.x,
          y: focusResult.y,
          button: "left",
          clickCount: 1,
        });
        await send("Runtime.evaluate", {
          expression: "new Promise((resolve) => setTimeout(resolve, 250))",
          awaitPromise: true,
          returnByValue: true,
        });
      }
      if (options.append !== true) {
        await send("Input.dispatchKeyEvent", {
          type: "keyDown",
          key: "Control",
          code: "ControlLeft",
          windowsVirtualKeyCode: 17,
          nativeVirtualKeyCode: 17,
          modifiers: 2,
        });
        await send("Input.dispatchKeyEvent", {
          type: "keyDown",
          key: "a",
          code: "KeyA",
          windowsVirtualKeyCode: 65,
          nativeVirtualKeyCode: 65,
          modifiers: 2,
        });
        await send("Input.dispatchKeyEvent", {
          type: "keyUp",
          key: "a",
          code: "KeyA",
          windowsVirtualKeyCode: 65,
          nativeVirtualKeyCode: 65,
          modifiers: 2,
        });
        await send("Input.dispatchKeyEvent", {
          type: "keyUp",
          key: "Control",
          code: "ControlLeft",
          windowsVirtualKeyCode: 17,
          nativeVirtualKeyCode: 17,
          modifiers: 0,
        });
        await send("Input.dispatchKeyEvent", {
          type: "keyDown",
          key: "Backspace",
          code: "Backspace",
          windowsVirtualKeyCode: 8,
          nativeVirtualKeyCode: 8,
        });
        await send("Input.dispatchKeyEvent", {
          type: "keyUp",
          key: "Backspace",
          code: "Backspace",
          windowsVirtualKeyCode: 8,
          nativeVirtualKeyCode: 8,
        });
      }
      await send("Input.insertText", { text });
      const verified = await evaluate(send, `
        ${browserDomScript()}
        const selector = __arrobaArgs.selector;
        const active = selector
          ? resolveTarget(selector, ["field"])
          : document.activeElement?.matches?.("input, textarea, select, [contenteditable=true]")
            ? document.activeElement
            : null;
        if (!active) return { ok: false, error: selector ? "target_not_found" : "no_focused_fillable_element" };
        const value = "value" in active ? active.value : active.textContent;
        const current = String(value || "");
        return {
          ok: __arrobaArgs.append
            ? current.length >= __arrobaArgs.minLength
            : current === __arrobaArgs.text,
        };
      `, { selector: options.selector ?? null, minLength: text.length, text, append: options.append === true });
      if (!verified?.ok) {
        throw new Error(verified?.error ?? "browser_type_not_applied");
      }
      if (options.submit === true) {
        await send("Input.dispatchKeyEvent", {
          type: "keyDown",
          key: "Enter",
          code: "Enter",
          windowsVirtualKeyCode: 13,
          nativeVirtualKeyCode: 13,
        });
        await send("Input.dispatchKeyEvent", {
          type: "keyUp",
          key: "Enter",
          code: "Enter",
          windowsVirtualKeyCode: 13,
          nativeVirtualKeyCode: 13,
        });
      }
      return;
    }

    const result = await evaluate(send, `
          ${browserDomScript()}
          const text = __arrobaArgs.text;
          const selector = __arrobaArgs.selector;
          const allowFallback = __arrobaArgs.allowFallback;
          const active = selector
            ? resolveTarget(selector, ["field"])
            : document.activeElement?.matches?.("input, textarea, select, [contenteditable=true]")
              ? document.activeElement
              : allowFallback
                ? document.querySelector("input[type=password], input:not([type]), input[type=text], textarea, select, [contenteditable=true]")
                : null;
          if (!active) {
            return { ok: false, error: selector ? "target_not_found" : "no_focused_fillable_element" };
          }
          if (active.disabled || active.readOnly) {
            return { ok: false, error: "target_not_editable" };
          }
          active.focus();
          if (active.tagName === "SELECT") {
            active.value = text;
            active.dispatchEvent(new Event("input", { bubbles: true }));
            active.dispatchEvent(new Event("change", { bubbles: true }));
            const rect = active.getBoundingClientRect();
            return { ok: true, x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
          }
          if ("value" in active) {
            const expectedLength = (__arrobaArgs.append ? active.value.length : 0) + text.length;
            active.value = __arrobaArgs.append ? active.value + text : text;
            active.dispatchEvent(new Event("input", { bubbles: true }));
            active.dispatchEvent(new Event("change", { bubbles: true }));
            const rect = active.getBoundingClientRect();
            return { ok: true, selector: selector || null, filled: String(active.value || "").length >= expectedLength, x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
          }
          active.textContent = __arrobaArgs.append ? (active.textContent || "") + text : text;
          active.dispatchEvent(new Event("input", { bubbles: true }));
          const rect = active.getBoundingClientRect();
          return { ok: true, selector: selector || null, filled: String(active.textContent || "").length >= text.length, x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
    `, {
      text,
      selector: options.selector ?? null,
      allowFallback: Boolean(options.allowFallback),
      append: options.append !== false,
    });
    if (!result?.ok) {
      throw new Error(result?.error ?? "browser_type_failed");
    }
    await showAgentCursor(send, result.x, result.y);
    if (!result.filled) {
      const focusResult = await evaluate(send, `
        ${browserDomScript()}
        const selector = __arrobaArgs.selector;
        const active = selector
          ? resolveTarget(selector, ["field"])
          : document.activeElement?.matches?.("input, textarea, select, [contenteditable=true]")
            ? document.activeElement
            : null;
        if (!active) return { ok: false, error: selector ? "target_not_found" : "no_focused_fillable_element" };
        if (active.disabled || active.readOnly) return { ok: false, error: "target_not_editable" };
        active.focus();
        return { ok: true };
      `, { selector: options.selector ?? null });
      if (!focusResult?.ok) {
        throw new Error(focusResult?.error ?? "browser_focus_failed");
      }
      await send("Input.insertText", { text });
      const verified = await evaluate(send, `
        ${browserDomScript()}
        const selector = __arrobaArgs.selector;
        const active = selector
          ? resolveTarget(selector, ["field"])
          : document.activeElement?.matches?.("input, textarea, select, [contenteditable=true]")
            ? document.activeElement
            : null;
        if (!active) return { ok: false, error: selector ? "target_not_found" : "no_focused_fillable_element" };
        const value = "value" in active ? active.value : active.textContent;
        return { ok: String(value || "").length >= __arrobaArgs.minLength };
      `, { selector: options.selector ?? null, minLength: text.length });
      if (!verified?.ok) {
        throw new Error(verified?.error ?? "browser_type_not_applied");
      }
    }
  });
}

function browserDomScript() {
  return `
    window.__arrobaCdpElementIds = window.__arrobaCdpElementIds || new WeakMap();
    window.__arrobaCdpNextElementId = window.__arrobaCdpNextElementId || 1;
    const visible = (element) => {
      const style = window.getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      return style.visibility !== "hidden" && style.display !== "none" && rect.width > 0 && rect.height > 0;
    };
    const cssString = (value) => CSS.escape(String(value));
    const selectorFor = (element) => {
      if (element.id) return "#" + cssString(element.id);
      if (element.name) {
        const named = element.tagName.toLowerCase() + "[name='" + cssString(element.name) + "']";
        if (document.querySelectorAll(named).length === 1) return named;
      }
      const parts = [];
      let current = element;
      while (current && current.nodeType === Node.ELEMENT_NODE && current !== document.body) {
        const tag = current.tagName.toLowerCase();
        const siblings = Array.from(current.parentElement?.children ?? []).filter((item) => item.tagName === current.tagName);
        const index = siblings.indexOf(current) + 1;
        parts.unshift(siblings.length > 1 ? tag + ":nth-of-type(" + index + ")" : tag);
        current = current.parentElement;
      }
      return parts.length ? "body > " + parts.join(" > ") : element.tagName.toLowerCase();
    };
    const fieldIdFor = (element, kind) => {
      const existing = window.__arrobaCdpElementIds.get(element);
      if (existing) return existing;
      const next = String(window.__arrobaCdpNextElementId++);
      const id = kind + ":" + next;
      window.__arrobaCdpElementIds.set(element, id);
      return id;
    };
    const labelFor = (element) => {
      if (element.id) {
        const label = document.querySelector("label[for='" + cssString(element.id) + "']");
        if (label?.innerText?.trim()) return label.innerText.trim();
      }
      const wrapper = element.closest("label");
      if (wrapper?.innerText?.trim()) return wrapper.innerText.trim();
      return "";
    };
    const roleFor = (element) => element.getAttribute("role") || "";
    const summaryTextFor = (element) => {
      const type = String(element.getAttribute("type") || "").toLowerCase();
      if (type === "password") {
        return (element.getAttribute("placeholder") || element.getAttribute("aria-label") || "Password").trim().slice(0, 200);
      }
      return (element.innerText || element.value || element.getAttribute("aria-label") || "").trim().slice(0, 200);
    };
    const summarize = (element, kind) => ({
      kind,
      selector: selectorFor(element),
      field_id: fieldIdFor(element, kind),
      tag: element.tagName.toLowerCase(),
      type: element.getAttribute("type") || "",
      name: element.getAttribute("name") || "",
      id: element.id || "",
      role: roleFor(element),
      label: labelFor(element),
      placeholder: element.getAttribute("placeholder") || "",
      text: summaryTextFor(element),
      disabled: Boolean(element.disabled),
      readOnly: Boolean(element.readOnly),
    });
    const fields = Array.from(document.querySelectorAll("input, textarea, select, [contenteditable=true]")).filter(visible).map((element) => summarize(element, "field"));
    const buttons = Array.from(document.querySelectorAll("button, input[type=button], input[type=submit], [role=button]")).filter(visible).map((element) => summarize(element, "button"));
    const links = Array.from(document.querySelectorAll("a[href], [role=link]")).filter(visible).map((element) => summarize(element, "link"));
    const resolveTarget = (target, kinds = ["field", "button", "link"]) => {
      if (!target) return null;
      try {
        const bySelector = document.querySelector(target);
        if (bySelector) return bySelector;
      } catch (_) {
        // Opaque field_id values are not necessarily valid CSS selectors.
      }
      const selectorByKind = {
        field: "input, textarea, select, [contenteditable=true]",
        button: "button, input[type=button], input[type=submit], [role=button]",
        link: "a[href], [role=link]",
      };
      const selectors = kinds.map((kind) => selectorByKind[kind]).filter(Boolean).join(",");
      for (const element of Array.from(document.querySelectorAll(selectors))) {
        for (const kind of kinds) {
          if (fieldIdFor(element, kind) === target) return element;
        }
      }
      return null;
    };
  `;
}

async function browserStatus() {
  return await withSocket(async (send) => evaluate(send, `
    ${browserDomScript()}
    const active = document.activeElement && document.activeElement !== document.body
      ? summarize(document.activeElement, document.activeElement.matches("input, textarea, select, [contenteditable=true]") ? "field" : "element")
      : null;
    return {
      url: location.href,
      host: location.host,
      title: document.title,
      readyState: document.readyState,
      focusedElement: active,
      fields,
      buttons,
      links,
    };
  `));
}

async function browserFind(query, kind = "any") {
  return await withSocket(async (send) => evaluate(send, `
    ${browserDomScript()}
    const query = (__arrobaArgs.query || "").toLowerCase();
    const kind = __arrobaArgs.kind || "any";
    const pool = [
      ...(kind === "field" || kind === "any" ? fields : []),
      ...(kind === "button" || kind === "any" ? buttons : []),
      ...(kind === "link" || kind === "any" ? links : []),
    ];
    const matches = pool.filter((entry) => [
      entry.selector,
      entry.id,
      entry.name,
      entry.role,
      entry.label,
      entry.placeholder,
      entry.text,
      entry.type,
    ].some((value) => String(value || "").toLowerCase().includes(query)));
    return { query: __arrobaArgs.query, kind, matches };
  `, { query, kind }));
}

async function clickSelector(selector) {
  await withSocket(async (send) => {
    const result = await evaluate(send, `
      ${browserDomScript()}
      const element = resolveTarget(__arrobaArgs.selector, ["button", "link", "field"]);
      if (!element) return { ok: false, error: "target_not_found" };
      element.scrollIntoView({ block: "center", inline: "center" });
      const rect = element.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return { ok: false, error: "target_not_visible" };
      return {
        ok: true,
        x: rect.left + rect.width / 2,
        y: rect.top + rect.height / 2,
      };
    `, { selector });
    if (!result?.ok) throw new Error(result?.error ?? "browser_click_failed");
    await showAgentCursor(send, result.x, result.y, true);
    await send("Input.dispatchMouseEvent", {
      type: "mouseMoved",
      x: result.x,
      y: result.y,
    });
    await send("Input.dispatchMouseEvent", {
      type: "mousePressed",
      x: result.x,
      y: result.y,
      button: "left",
      clickCount: 1,
    });
    await send("Input.dispatchMouseEvent", {
      type: "mouseReleased",
      x: result.x,
      y: result.y,
      button: "left",
      clickCount: 1,
    });
  });
}

async function submitSelector(selector) {
  await withSocket(async (send) => {
    const result = await evaluate(send, `
      ${browserDomScript()}
      const target = __arrobaArgs.selector ? resolveTarget(__arrobaArgs.selector, ["field", "button"]) : document.activeElement;
      if (!target) return { ok: false, error: "target_not_found" };
      const form = target.closest?.("form");
      if (!form) return { ok: false, error: "form_not_found" };
      if (form.requestSubmit) form.requestSubmit();
      else form.submit();
      return { ok: true };
    `, { selector: selector ?? null });
    if (!result?.ok) throw new Error(result?.error ?? "browser_submit_failed");
  });
}

async function handleDialog(action, promptText) {
  const accept = action === "accept";
  if (!accept && action !== "dismiss") {
    throw new Error("dialog_action_must_be_accept_or_dismiss");
  }
  await withSocket(async (send) => {
    await send("Page.handleJavaScriptDialog", {
      accept,
      ...(accept && promptText ? { promptText } : {}),
    });
  });
  return { ok: true, action };
}

async function waitForPredicate(label, timeoutMs, predicateSource, args = {}) {
  const timeout = Math.max(100, Math.min(Number(timeoutMs) || 10000, 60000));
  const started = Date.now();
  let lastResult = null;
  while (Date.now() - started <= timeout) {
    lastResult = await withSocket(async (send) => evaluate(send, predicateSource, args));
    if (lastResult?.ok) {
      return {
        ok: true,
        waited_ms: Date.now() - started,
        ...lastResult,
      };
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  return {
    ok: false,
    error: "timeout",
    wait: label,
    timeout_ms: timeout,
    waited_ms: Date.now() - started,
    last: lastResult,
  };
}

async function waitForText(text, timeoutMs) {
  return await waitForPredicate("text", timeoutMs, `
    const text = String(__arrobaArgs.text || "");
    const body = document.body?.innerText || "";
    return { ok: body.includes(text), text, readyState: document.readyState };
  `, { text });
}

async function waitForSelector(selector, timeoutMs) {
  return await waitForPredicate("selector", timeoutMs, `
    const selector = String(__arrobaArgs.selector || "");
    const element = selector ? document.querySelector(selector) : null;
    if (!element) return { ok: false, selector, reason: "missing" };
    const style = window.getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    return {
      ok: style.visibility !== "hidden" && style.display !== "none" && rect.width > 0 && rect.height > 0,
      selector,
      reason: "not_visible",
    };
  `, { selector });
}

async function waitForIdle(timeoutMs) {
  return await waitForPredicate("idle", timeoutMs, `
    return {
      ok: document.readyState === "complete",
      readyState: document.readyState,
      url: location.href,
      title: document.title,
    };
  `);
}

if (command === "click") {
  const x = Number(args[0]);
  const y = Number(args[1]);
  await withSocket(async (send) => {
    await showAgentCursor(send, x, y, true);
    await send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
    await send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
    await send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
  });
} else if (command === "type") {
  const text = args.join(" ");
  await typeIntoBrowser(text, { allowFallback: true });
} else if (command === "type-stdin") {
  await typeIntoBrowser(await readStdin(), { allowFallback: true });
} else if (command === "secret-paste-stdin") {
  await typeIntoBrowser(await readStdin(), { selector: args[0] || null, useNativeInput: true, append: false, allowFallback: false });
} else if (command === "secret-paste-submit-stdin") {
  await typeIntoBrowser(await readStdin(), { selector: args[0] || null, useNativeInput: true, append: false, submit: true, allowFallback: false });
} else if (command === "key") {
  const key = args[0];
  await withSocket(async (send) => {
    await send("Input.dispatchKeyEvent", { type: "keyDown", key });
    await send("Input.dispatchKeyEvent", { type: "keyUp", key });
  });
} else if (command === "text") {
  const result = await withSocket(async (send) => {
    return await send("Runtime.evaluate", {
      expression: "document.body.innerText",
      returnByValue: true,
    });
  });
  process.stdout.write(result.result.value ?? "");
} else if (command === "status") {
  process.stdout.write(JSON.stringify(await browserStatus()));
} else if (command === "find") {
  process.stdout.write(JSON.stringify(await browserFind(args[0] || "", args[1] || "any")));
} else if (command === "fill") {
  const selector = args[0];
  const text = args.slice(1).join(" ");
  await typeIntoBrowser(text, { selector, append: false, allowFallback: false });
  process.stdout.write(JSON.stringify({ ok: true, selector }));
} else if (command === "fill-stdin") {
  const selector = args[0];
  await typeIntoBrowser(await readStdin(), { selector, append: false, allowFallback: false });
  process.stdout.write(JSON.stringify({ ok: true, selector }));
} else if (command === "click-selector") {
  const selector = args[0];
  await clickSelector(selector);
  process.stdout.write(JSON.stringify({ ok: true, selector }));
} else if (command === "cursor-status") {
  const result = await withSocket(async (send) => evaluate(send, `
    const cursor = document.getElementById("__arroba-agent-cursor");
    return { visible: Boolean(cursor && cursor.style.opacity === "1") };
  `));
  process.stdout.write(JSON.stringify(result));
} else if (command === "submit") {
  const selector = args[0] || null;
  await submitSelector(selector);
  process.stdout.write(JSON.stringify({ ok: true, selector }));
} else if (command === "dialog") {
  process.stdout.write(JSON.stringify(await handleDialog(args[0] || "", args[1] || "")));
} else if (command === "wait-text") {
  const result = await waitForText(args[0] || "", args[1]);
  if (!result.ok) process.exitCode = 1;
  process.stdout.write(JSON.stringify(result));
} else if (command === "wait-selector") {
  const result = await waitForSelector(args[0] || "", args[1]);
  if (!result.ok) process.exitCode = 1;
  process.stdout.write(JSON.stringify(result));
} else if (command === "wait-idle") {
  const result = await waitForIdle(args[0]);
  if (!result.ok) process.exitCode = 1;
  process.stdout.write(JSON.stringify(result));
} else if (command === "close-browser") {
  await closeBrowser();
} else if (command === "navigate") {
  await navigate(args[0]);
} else {
  console.error("Usage: browser-cdp.mjs click <x> <y> | type <text> | type-stdin | secret-paste-stdin [selector] | secret-paste-submit-stdin [selector] | key <key> | text | status | find <query> [kind] | fill <selector> <text> | fill-stdin <selector> | click-selector <selector> | cursor-status | submit [selector] | dialog <accept|dismiss> [prompt-text] | close-browser | navigate <url>");
  process.exit(2);
}
