// Hermetic kernel-browser harness for the RED drills: the real KernelBrowserHost + real Chrome (as the kernel runs them),
// pointed at the owned fixtures. No web client, no relay, no agent. Needs a host that is NOT inside Bubblewrap and has
// Xvfb, openbox, dbus-daemon, setxkbmap, bwrap and Chrome (a Boat sandbox qualifies; an agent shell does not).
// Env: CHARIOX_MDACCESS_DRILL_ROOT (disposable dir), CHARIOX_KERNEL_BROWSER_EXECUTABLE, D3_REPO (repo root, default: this checkout).
import { mkdtemp, rm } from "node:fs/promises"
import { inflateRawSync, constants as zlib } from "node:zlib"
import path from "node:path"
import { fileURLToPath } from "node:url"

const here = path.dirname(fileURLToPath(import.meta.url))
export const repo = process.env.D3_REPO ?? path.resolve(here, "../../../..")
const docker = path.join(repo, "apps/kernel/slice-linux-docker/docker")
const { KernelBrowserHost } = await import(path.join(docker, "kernel-browser-host.mjs"))
const { HostChromium } = await import(path.join(docker, "kernel-browser-process.mjs"))

export const delay = ms => new Promise(resolve => setTimeout(resolve, ms))

/** Runs fn({host, open, evaluate, subscribe, tabs}) against a fresh browser, then tears everything down. */
export async function withHost(fn) {
  const base = process.env.CHARIOX_MDACCESS_DRILL_ROOT
  if (!base) throw new Error("set CHARIOX_MDACCESS_DRILL_ROOT to a disposable directory")
  const root = await mkdtemp(path.join(base, "d3-")), apps = await mkdtemp(path.join(base, "d3apps-"))
  const host = new KernelBrowserHost(root, { appStorageRoot: apps, chromium: new HostChromium(root, { appStorageRoot: apps, environment: process.env }) })
  const evaluateIn = async (tab_id, expression) => {
    const { connection, sessionId } = await host.browser.resolvePageTarget(host.tabs.get(tab_id).target_id)
    const reply = await connection.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }, sessionId)
    if (reply.exceptionDetails) throw new Error("evaluate failed: " + (reply.exceptionDetails.exception?.description ?? "").split("\n")[0])
    return reply.result.value
  }
  const open = async url => {
    const opened = await host.request({ op: "open", url })
    for (let i = 0; i < 400 && (await evaluateIn(opened.tab_id, "document.readyState === 'complete' && location.protocol !== 'about:'").catch(() => false)) !== true; i++) await delay(25)
    return { ...opened, evaluate: expression => evaluateIn(opened.tab_id, expression) }
  }
  /** v2 mirror subscription for a tab. next() returns the decoded packet or throws the host's own error (code + message). */
  const subscribe = async (tab, extra = {}) => {
    const { subscription_id } = await host.request({ op: "mirror_subscribe", tab_id: tab.tab_id, generation: tab.generation, device_scale_factor: extra.dpr ?? 1, wire: 2 })
    let applied = 0, context = Buffer.alloc(0)
    const next = async (wait_ms = 500) => {
      let packet = await host.request({ op: "mirror_next", subscription_id, generation: tab.generation, after_sequence: applied, drift_nodes: [], wait_ms })
      applied = packet.sequence
      if (packet.reset) context = Buffer.alloc(0)
      if (packet.encoding === "deflate") {
        context = Buffer.concat([context, Buffer.from(packet.packet_base64, "base64")])
        const out = inflateRawSync(context, { finishFlush: zlib.Z_SYNC_FLUSH })
        packet = { ...JSON.parse(out.subarray(out.length - packet.packet_bytes)), resources: packet.resources ?? [], tiles: packet.tiles ?? [] }
      }
      return { ops: [], resources: [], tiles: [], ...packet }
    }
    return { subscription_id, next }
  }
  try { return await fn({ host, open, subscribe, evaluateIn }) }
  finally {
    await host.stop().catch(() => {})
    if (host.chromium.browserUser) await rm(host.chromium.browserUser.root, { recursive: true, force: true })
    await rm(root, { recursive: true, force: true }); await rm(apps, { recursive: true, force: true })
  }
}

/** Every string leaf of a packet, to ask "does the viewer's DOM hold this text" without knowing the packet shape. */
export function packetText(packet) {
  const out = []
  const walk = value => { if (typeof value === "string") out.push(value); else if (Array.isArray(value)) value.forEach(walk); else if (value && typeof value === "object") Object.values(value).forEach(walk) }
  walk({ nodes: packet.nodes, ops: packet.ops })
  return out.join("\n")
}

/** RED/GREEN reporting: a drill prints one line per check and exits 1 when any check fails (RED), 0 when all pass (GREEN). */
export function reporter(row) {
  const lines = []
  return {
    check(name, ok, detail = "") { lines.push({ name, ok: !!ok }); console.log(`${ok ? "PASS" : "FAIL"} row ${row}: ${name}${detail ? " | " + detail : ""}`) },
    done() { const red = lines.some(l => !l.ok); console.log(`${red ? "RED" : "GREEN"} row ${row} (${lines.filter(l => !l.ok).length} failing of ${lines.length})`); process.exit(red ? 1 : 0) },
  }
}
