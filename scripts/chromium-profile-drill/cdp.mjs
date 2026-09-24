import { boundedResponseJson } from "/opt/chariox-slice/chromium-sandbox-probe.mjs";

export async function connect() {
  const response = await fetch("http://127.0.0.1:9222/json/version", { signal: AbortSignal.timeout(3000) });
  if (!response.ok) throw new Error("debugger is unavailable");
  const endpoint = new URL((await boundedResponseJson(response)).webSocketDebuggerUrl);
  if (endpoint.protocol !== "ws:" || endpoint.hostname !== "127.0.0.1" || endpoint.port !== "9222") throw new Error("unexpected debugger endpoint");
  const socket = new WebSocket(endpoint);
  const pending = new Map();
  let next = 0;
  const close = () => {
    for (const call of pending.values()) call.reject(new Error("debugger closed"));
    socket.close();
  };
  socket.addEventListener("error", close);
  socket.addEventListener("close", () => { for (const call of pending.values()) call.reject(new Error("debugger closed")); });
  socket.addEventListener("message", ({ data }) => {
    if (typeof data !== "string" || Buffer.byteLength(data) > 65536) { close(); return; }
    try {
      const message = JSON.parse(data);
      const call = pending.get(message.id);
      if (call) message.error ? call.reject(new Error("debugger request failed")) : call.resolve(message.result);
    } catch { close(); }
  });
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => { close(); reject(new Error("debugger connection deadline")); }, 3000);
    socket.addEventListener("open", () => { clearTimeout(timer); resolve(); }, { once: true });
    socket.addEventListener("error", () => { clearTimeout(timer); reject(new Error("debugger connection failed")); }, { once: true });
  });
  return {
    close,
    send(method, params = {}, sessionId) {
      return new Promise((resolve, reject) => {
        if (pending.size >= 4) return reject(new Error("debugger request bound"));
        const id = ++next;
        const finish = (callback, value) => { clearTimeout(timer); pending.delete(id); callback(value); };
        const timer = setTimeout(() => finish(reject, new Error("debugger request deadline")), 5000);
        pending.set(id, { resolve: value => finish(resolve, value), reject: error => finish(reject, error) });
        try { socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) })); }
        catch (error) { finish(reject, error); }
      });
    },
  };
}
