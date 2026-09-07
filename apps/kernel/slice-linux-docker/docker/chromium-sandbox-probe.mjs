#!/usr/bin/env node
// Kernel-owned migration verification. Report booleans and bounded counts, never page data.
import { opendir, readlink } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Only this module's fixed diagnostic text is safe to publish. Filesystem,
// JSON and network errors can contain paths or response bytes.
class ProbeCheckError extends Error {}
export function formatProbeFailure(error) {
  const prefix = "Chromium sandbox/profile verification failed";
  return error instanceof ProbeCheckError ? `${prefix}: ${error.message}` : prefix;
}
function requireCheck(value, message) {
  if (!value) throw new ProbeCheckError(message);
}

export function validateSandboxReport(text, browser, renderers) {
  requireCheck(typeof text === "string" && Buffer.byteLength(text) <= 65536, "invalid sandbox diagnostic text");
  requireCheck(/PID namespaces\s+Yes/.test(text) && /Network namespaces\s+Yes/.test(text), "Chromium namespace sandbox is unavailable");
  requireCheck(/Seccomp-BPF sandbox\s+Yes/.test(text), "Chromium Seccomp-BPF sandbox is unavailable");
  const validUid = process => Number.isSafeInteger(process.uid) && process.uid > 0
    && Array.isArray(process.uids) && process.uids.length === 4 && process.uids.every(uid => uid === process.uid);
  const validNamespaces = process => /^pid:\[\d+\]$/.test(process.pidNamespace)
    && /^net:\[\d+\]$/.test(process.netNamespace);
  const validPidHierarchy = process => Number.isSafeInteger(process.pid) && process.pid > 0
    && Array.isArray(process.namespacePids) && process.namespacePids.length > 0 && process.namespacePids.length <= 32
    && process.namespacePids[0] === process.pid
    && process.namespacePids.every(pid => Number.isSafeInteger(pid) && pid > 0);
  requireCheck(validUid(browser) && validNamespaces(browser) && validPidHierarchy(browser),
    "non-root browser with observed namespaces is required");
  requireCheck(Number.isSafeInteger(browser.seccompFilters) && browser.seccompFilters >= 0,
    "browser seccomp baseline is unavailable");
  requireCheck(Array.isArray(renderers) && renderers.length > 0 && renderers.length <= 8192,
    "no observable browser renderer processes");
  for (const renderer of renderers) {
    requireCheck(validUid(renderer) && renderer.uid === browser.uid, "unexpected renderer UID");
    requireCheck(renderer.capabilities === "0000000000000000", "renderer retains capabilities");
    requireCheck(renderer.noNewPrivileges === 1 && renderer.seccomp === 2, "renderer lockdown is incomplete");
    requireCheck(validPidHierarchy(renderer) && renderer.namespacePids.length > browser.namespacePids.length,
      "renderer lacks an observed nested PID namespace");
    // Namespace symlinks are ptrace-gated. Chromium's non-dumpable sandbox
    // processes can intentionally deny them, even to this same-UID observer.
    // NSpid remains independent kernel evidence for PID nesting. Chromium's
    // own diagnostic above attests network isolation; if namespace inodes are
    // readable, they must independently agree as well. Missing data without an
    // explicit access denial is never accepted as evidence.
    for (const kind of ["pid", "net"]) {
      const field = `${kind}Namespace`;
      const observed = renderer[field];
      requireCheck(observed === null && renderer[`${field}Restricted`] === true
        || typeof observed === "string" && new RegExp(`^${kind}:\\[\\d+\\]$`).test(observed) && observed !== browser[field],
      "renderer namespace evidence is missing or contradicts isolation");
    }
    requireCheck(Number.isSafeInteger(renderer.seccompFilters) && renderer.seccompFilters > browser.seccompFilters,
      "renderer did not install an additional seccomp filter");
  }
  return {
    rendererCount: renderers.length,
    restrictedPidNamespaceLinks: renderers.filter(renderer => renderer.pidNamespaceRestricted === true).length,
    restrictedNetworkNamespaceLinks: renderers.filter(renderer => renderer.netNamespaceRestricted === true).length,
  };
}

async function boundedText(path, limit = 65536) {
  const { open } = await import("node:fs/promises");
  const file = await open(path, "r");
  try {
    const buffer = Buffer.alloc(limit + 1);
    let used = 0;
    while (used < buffer.length) {
      const { bytesRead } = await file.read(buffer, used, buffer.length - used, used);
      if (bytesRead === 0) break;
      used += bytesRead;
    }
    requireCheck(used <= limit, "oversized process metadata");
    // Linux permits arbitrary bytes in unrelated processes' argv; replacement
    // decoding must not make that unrelated process abort the whole scan.
    return buffer.subarray(0, used).toString("utf8");
  } finally { await file.close(); }
}

export async function namespaceLink(path, inspect = readlink) {
  try { return { value: await inspect(path), restricted: false }; }
  catch (error) {
    if (["EACCES", "EPERM"].includes(error.code)) return { value: null, restricted: true };
    throw error;
  }
}

async function processInfo(pid) {
  const [status, command, pidNamespace, netNamespace] = await Promise.all([
    boundedText(`/proc/${pid}/status`), boundedText(`/proc/${pid}/cmdline`),
    namespaceLink(`/proc/${pid}/ns/pid`), namespaceLink(`/proc/${pid}/ns/net`),
  ]);
  const number = name => Number(status.match(new RegExp(`^${name}:\\s+(\\d+)`, "m"))?.[1] ?? NaN);
  const list = name => status.match(new RegExp(`^${name}:\\s+([0-9 \\t]+)$`, "m"))?.[1].trim().split(/\s+/).map(Number) ?? [];
  const uids = list("Uid");
  return { pid, parent: number("PPid"), uid: uids[0], uids, command: command.split("\0"),
    capabilities: status.match(/^CapEff:\s+([a-f0-9]+)$/m)?.[1],
    noNewPrivileges: number("NoNewPrivs"), seccomp: number("Seccomp"), seccompFilters: number("Seccomp_filters"),
    namespacePids: list("NSpid"),
    pidNamespace: pidNamespace.value, pidNamespaceRestricted: pidNamespace.restricted,
    netNamespace: netNamespace.value, netNamespaceRestricted: netNamespace.restricted };
}

export async function boundedResponseJson(response, limit = 16384) {
  requireCheck(response.body && Number.isSafeInteger(limit) && limit > 0 && limit <= 65536,
    "invalid debugger response");
  const reader = response.body.getReader();
  const chunks = [];
  let used = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      used += value.byteLength;
      requireCheck(used <= limit, "oversized debugger response");
      chunks.push(value);
    }
    return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(Buffer.concat(chunks, used)));
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}

async function inspectProcesses(profile) {
  const processes = new Map();
  let entries = 0;
  for await (const entry of await opendir("/proc")) {
    if (!/^\d+$/.test(entry.name)) continue;
    requireCheck(++entries <= 8192, "process verification limit exceeded");
    try { processes.set(Number(entry.name), await processInfo(Number(entry.name))); }
    catch (error) { if (!["ENOENT", "ESRCH", "EACCES", "EPERM"].includes(error.code)) throw error; }
  }
  const browsers = [...processes.values()].filter(p => p.command.includes(`--user-data-dir=${profile}`)
    && p.command.includes("--remote-debugging-port=9222") && !p.command.some(arg => arg.startsWith("--type=")));
  requireCheck(browsers.length === 1, "expected one kernel profile browser");
  const browser = browsers[0];
  requireCheck(browser.command.includes("--password-store=basic") && !browser.command.includes("--no-sandbox"), "unexpected browser storage or sandbox options");
  const renderers = [...processes.values()].filter(p => {
    if (!p.command.includes("--type=renderer")) return false;
    let parent = p.parent;
    for (let hop = 0; hop < 32 && parent; hop++) {
      if (parent === browser.pid) return true;
      parent = processes.get(parent)?.parent;
    }
    return false;
  });
  return { browser, renderers };
}

export async function withSandboxText(inspect) {
  const response = await fetch("http://127.0.0.1:9222/json/version", { signal: AbortSignal.timeout(3000) });
  requireCheck(response.ok, "Chromium debugger is unavailable");
  const version = await boundedResponseJson(response);
  const endpoint = new URL(version.webSocketDebuggerUrl);
  requireCheck(endpoint.protocol === "ws:" && endpoint.hostname === "127.0.0.1" && endpoint.port === "9222", "unexpected debugger endpoint");
  const socket = new WebSocket(endpoint);
  let sequence = 0;
  let targetId;
  const pending = new Map();
  const rejectAll = () => { for (const call of pending.values()) call.reject(new ProbeCheckError("debugger connection closed")); };
  socket.addEventListener("close", rejectAll);
  socket.addEventListener("error", rejectAll);
  socket.addEventListener("message", event => {
    if (typeof event.data !== "string" || Buffer.byteLength(event.data) > 65536) { socket.close(); rejectAll(); return; }
    try {
      const message = JSON.parse(event.data);
      const call = pending.get(message.id);
      if (call) message.error ? call.reject(new ProbeCheckError("debugger request failed")) : call.resolve(message.result);
    } catch { socket.close(); rejectAll(); }
  });
  const send = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => finish(reject, new ProbeCheckError("debugger request timed out")), 3000);
    function finish(callback, value) { clearTimeout(timer); pending.delete(id); callback(value); }
    pending.set(id, { resolve: value => finish(resolve, value), reject: error => finish(reject, error) });
    try { socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) })); }
    catch (error) { finish(reject, error); }
  });
  try {
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new ProbeCheckError("debugger connection timed out")), 3000);
      socket.addEventListener("open", () => { clearTimeout(timer); resolve(); }, { once: true });
      socket.addEventListener("error", () => { clearTimeout(timer); reject(new ProbeCheckError("debugger connection failed")); }, { once: true });
    });
    ({ targetId } = await send("Target.createTarget", { url: "chrome://sandbox", background: true }));
    const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
    for (let attempt = 0; attempt < 20; attempt++) {
      const result = await send("Runtime.evaluate", { expression: "(document.body?.innerText ?? '').slice(0, 16384)", returnByValue: true }, sessionId);
      const text = result.result?.value;
      if (typeof text === "string" && text.includes("Seccomp")) {
        // Keep the owned diagnostic renderer alive while inspecting its process
        // sandbox. An otherwise empty browser may discard its last renderer as
        // soon as this target closes.
        return await inspect(text);
      }
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw new ProbeCheckError("sandbox diagnostic page did not load");
  } finally {
    // If createTarget's outcome is unknown after a lost connection, the kernel
    // stops/rolls back this candidate browser when verification fails. Only
    // our known diagnostic target may be closed here; never reuse a user tab.
    if (targetId && socket.readyState === WebSocket.OPEN) await send("Target.closeTarget", { targetId }).catch(() => {});
    socket.close();
    rejectAll();
  }
}

async function main() {
  requireCheck(process.getuid?.() > 0, "sandbox verification requires the slice user");
  const profile = process.env.CHARIOX_SLICE_CHROME_PROFILE || join(homedir(), ".config/chariox-slice-chromium");
  const observation = await withSandboxText(async text => {
    const { browser, renderers } = await inspectProcesses(profile);
    return validateSandboxReport(text, browser, renderers);
  });
  console.log(JSON.stringify({ chromiumSandboxVerified: true, profilePathVerified: true, basicPasswordStoreVerified: true, observation }));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(formatProbeFailure(error)); process.exitCode = 1; });
}
