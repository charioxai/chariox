import { connect } from "./cdp.mjs";

async function browserTask(mode) {
  const expected = "chariox-fixture-value-v1";
  const database = await new Promise((resolve, reject) => {
    const request = indexedDB.open("chariox-profile-fixture", 1);
    request.onupgradeneeded = () => request.result.createObjectStore("values");
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(new Error("IndexedDB open failed"));
  });
  try {
    if (mode === "seed") {
      if (!(await fetch("/login", { method: "POST" })).ok) throw new Error("fixture login failed");
      document.cookie = `chariox_fixture_visible=${expected}; Path=/; SameSite=Strict; Max-Age=86400`;
      localStorage.setItem("fixture", expected);
      await new Promise((resolve, reject) => {
        const transaction = database.transaction("values", "readwrite");
        transaction.objectStore("values").put(expected, "fixture");
        transaction.oncomplete = resolve;
        transaction.onabort = transaction.onerror = () => reject(new Error("IndexedDB write failed"));
      });
    }
    if (mode === "revoked") await fetch("/revoke", { method: "POST" });
    const stored = await new Promise((resolve, reject) => {
      const request = database.transaction("values", "readonly").objectStore("values").get("fixture");
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(new Error("IndexedDB read failed"));
    });
    return {
      authenticated: (await (await fetch("/auth")).json()).authenticated === true,
      cookie: document.cookie.split(/;\s*/).includes(`chariox_fixture_visible=${expected}`),
      localStorage: localStorage.getItem("fixture") === expected,
      indexedDB: stored === expected,
    };
  } finally { database.close(); }
}

const mode = process.argv[2];
if (!["seed", "verify", "revoked", "empty"].includes(mode)) throw new Error("invalid profile drill mode");
const socket = await connect();
try {
  const url = "http://127.0.0.1:8765/app.html";
  const findTarget = async () => (await socket.send("Target.getTargets")).targetInfos.find(target => target.type === "page" && target.url === url);
  let target = await findTarget();
  // Session restoration is asynchronous after the production launcher starts.
  // A verification turn must observe the saved tab instead of creating one
  // that could hide a restore failure or race a delayed restored target.
  const restoreDeadline = Date.now() + 5000;
  while (mode === "verify" && !target && Date.now() < restoreDeadline) {
    await new Promise(resolve => setTimeout(resolve, 100));
    target = await findTarget();
  }
  if (mode === "verify" && !target) throw new Error("saved fixture tab did not become available");
  const sessionTabRestored = !!target;
  if (!target) target = await socket.send("Target.createTarget", { url });
  const { sessionId } = await socket.send("Target.attachToTarget", { targetId: target.targetId, flatten: true });
  for (let attempt = 0; ; attempt++) {
    const ready = await socket.send("Runtime.evaluate", { expression: `location.href === ${JSON.stringify(url)} && document.readyState === 'complete'`, returnByValue: true }, sessionId);
    if (ready.result?.value === true) break;
    if (attempt === 39) throw new Error("fixture page readiness deadline");
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  const result = await socket.send("Runtime.evaluate", { expression: `(${browserTask.toString()})(${JSON.stringify(mode)})`, returnByValue: true, awaitPromise: true, timeout: 4000 }, sessionId);
  if (result.exceptionDetails || typeof result.result?.value !== "object") throw new Error("fixture storage operation failed");
  const observed = result.result.value;
  const expected = mode === "empty" ? [false, false, false, false] : [mode !== "revoked", true, true, true];
  if (JSON.stringify([observed.authenticated, observed.cookie, observed.localStorage, observed.indexedDB]) !== JSON.stringify(expected)) {
    throw new Error(`unexpected fixture state: ${JSON.stringify(observed)}`);
  }
  console.log(JSON.stringify({ mode, ...observed, sessionTabRestored }));
} finally { socket.close(); }
