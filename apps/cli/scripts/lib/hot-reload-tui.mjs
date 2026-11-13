// Run the built native terminal in an isolated tmux server; sample its real
// screen as well as the existing automation view. Credentials never enter argv.
import { execFile } from "node:child_process"
import { promisify } from "node:util"
import { mkdir, writeFile, readFile, rm } from "node:fs/promises"
import net from "node:net"
import path from "node:path"
import { setTimeout as sleep } from "node:timers/promises"

const exec = promisify(execFile)
const quote = text => `'${text.replaceAll("'", "'\\''")}'`

export async function startHotReloadTui({ rig, ossRoot, sessionId, carrier, evidence, runtimeDir }) {
  await mkdir(runtimeDir, { recursive: true })
  const socket = path.join(runtimeDir, "tmux.sock")
  const automation = path.join(runtimeDir, "tui.sock")
  const bootstrap = path.join(runtimeDir, "bootstrap.mjs")
  const pidPath = path.join(runtimeDir, "cli.pid")
  const registry = path.join(runtimeDir, "remote-discovery")
  await mkdir(registry, { recursive: true })
  const tmux = (...args) => exec("tmux", ["-S", socket, ...args], { maxBuffer: 2 ** 22 })
  // Issue a normal key-bound CLIENT grant through the local kernel. The native
  // client then uses hosted WSS for every Room request and event.
  await writeFile(bootstrap, `
import { LocalIpcClient } from ${JSON.stringify(path.join(ossRoot, "packages/kernel-client/dist/ipc.js"))}
import { createCliRelayIdentityStore } from ${JSON.stringify(path.join(ossRoot, "apps/cli/dist/cli-relay-identity-store.js"))}
import { saveUiPreferences } from ${JSON.stringify(path.join(ossRoot, "apps/cli/dist/preferences.js"))}
import { writeFile } from "node:fs/promises"
await saveUiPreferences({ multiAgentResponseLayout: "split", maxAgentsPerScreen: 2 })
const rig = ${JSON.stringify(rig)}
const endpoint = "ws://127.0.0.1:" + rig.ports.kernel + "/kernel"
const args = ["bun", ${JSON.stringify(path.join(ossRoot, "apps/cli/dist/index.js"))}, "--session", ${JSON.stringify(sessionId)}, "--automation-socket", ${JSON.stringify(automation)}, "--workspace", rig.project, "--worktree", rig.project, "--client-id", "hot-reload-tui-${carrier}"]
const env = { ...process.env }
if (${JSON.stringify(carrier)} === "relay") {
  const identity = createCliRelayIdentityStore().getOrCreate()
  const client = new LocalIpcClient(endpoint)
  try {
    const reply = await client.send({ ResolveKernelClientConnection: { kernel_ref: rig.daemonId, machine_ref: rig.machineId, client_id: "hot-reload-tui-relay", session_id: ${JSON.stringify(sessionId)}, public_key_thumbprint: identity.publicKeyThumbprint } })
    const grant = reply.KernelClientConnectionResolved?.connection
    if (!grant) throw Error("Kernel did not issue a TUI relay grant")
    env.HOT_RELOAD_TUI_GRANT = grant.relay_token
    args.push("--relay-url", grant.relay_url, "--target-daemon-id", grant.target_daemon_id, "--relay-token-env", "HOT_RELOAD_TUI_GRANT", "--relay-token-issuer", endpoint, rig.daemonId)
  } finally { await client.close() }
} else args.push("--kernel-url", endpoint)
const child = Bun.spawn(args, { env, stdin: "inherit", stdout: "inherit", stderr: "inherit" })
await writeFile(${JSON.stringify(pidPath)}, String(child.pid))
process.on("SIGTERM", () => child.kill("SIGTERM"))
process.exit(await child.exited)
`, { mode: 0o600 })
  let connection
  try {
    await tmux("-f", "/dev/null", "new-session", "-d", "-s", "reload", "-x", "240", "-y", "80",
      `env CHARIOX_HOME=${quote(rig.charioxHome)} CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR=${quote(registry)} TERM=xterm-256color bun ${quote(bootstrap)} 2>${quote(path.join(evidence, "tui-stderr.log"))}`)
    await tmux("set-option", "-g", "status", "off")
    await tmux("set-option", "-g", "remain-on-exit", "on")
    const deadline = Date.now() + 60000
    while (Date.now() < deadline) {
      try {
        connection = net.createConnection(automation)
        await new Promise((ok, fail) => { connection.once("connect", ok); connection.once("error", fail) })
        break
      } catch { connection?.destroy(); connection = null; await sleep(100) }
    }
    if (!connection) throw Error("Native TUI automation socket did not open")
    connection.setEncoding("utf8")
    let buffer = "", id = 0
    const pending = new Map()
    connection.on("data", chunk => {
      buffer += chunk
      while (buffer.includes("\n")) {
        const end = buffer.indexOf("\n"), row = JSON.parse(buffer.slice(0, end)); buffer = buffer.slice(end + 1)
        const deferred = pending.get(row.id)
        if (deferred) { pending.delete(row.id); clearTimeout(deferred.timer); row.ok ? deferred.ok(row.data) : deferred.fail(Error(row.error)) }
      }
    })
    const send = action => new Promise((ok, fail) => {
      const key = ++id
      const timer = setTimeout(() => { pending.delete(key); fail(Error(`Native TUI ${action} timed out`)) }, 5000)
      pending.set(key, { ok, fail, timer }); connection.write(JSON.stringify({ id: key, action }) + "\n")
    })
    return {
      async transport() {
        const pid = Number(await readFile(pidPath, "utf8"))
        const sockets = (await exec("ss", ["-tpn"])).stdout.split("\n").filter(line => line.includes(`pid=${pid},`))
        return { pid, carrier, sockets }
      },
      async read() {
        const [snapshot, screen] = await Promise.all([send("snapshot"), tmux("capture-pane", "-p", "-t", "reload:0.0")])
        return { t: Date.now(), snapshot, screen: screen.stdout }
      },
      async close() {
        await send("exit").catch(() => {})
        connection.destroy()
        await tmux("kill-server").catch(() => {})
        await rm(bootstrap, { force: true })
      },
    }
  } catch (error) {
    const screen = await tmux("capture-pane", "-p", "-t", "reload:0.0").catch(() => ({ stdout: "No native screen" }))
    await writeFile(path.join(evidence, "tui-startup-failure.txt"), screen.stdout)
    connection?.destroy(); await tmux("kill-server").catch(() => {})
    throw error
  }
}
