#!/usr/bin/env node

const command = process.argv[2];
const args = process.argv.slice(3);

async function getPage() {
  const response = await fetch("http://127.0.0.1:9222/json/list");
  const targets = await response.json();
  const pages = targets.filter((target) => target.type === "page" && target.webSocketDebuggerUrl);
  const page =
    pages.find((target) => target.url?.includes("arroba-slice-screen-test")) ??
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

if (command === "click") {
  const x = Number(args[0]);
  const y = Number(args[1]);
  await withSocket(async (send) => {
    await send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
    await send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
    await send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
  });
} else if (command === "type") {
  const text = args.join(" ");
  await withSocket(async (send) => {
    await send("Runtime.evaluate", {
      expression: `
        (() => {
          const text = ${JSON.stringify(text)};
          const active = document.activeElement?.matches?.("input, textarea, [contenteditable=true]")
            ? document.activeElement
            : document.querySelector("input, textarea, [contenteditable=true]");
          if (active && "value" in active) {
            active.value += text;
            active.dispatchEvent(new Event("input", { bubbles: true }));
            return true;
          }
          return false;
        })()
      `,
      awaitPromise: false,
    });
  });
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
} else {
  console.error("Usage: browser-cdp.mjs click <x> <y> | type <text> | key <key> | text");
  process.exit(2);
}
