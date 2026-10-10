// MP-07 / MP-08 / MP-11: CLI projects a setup offer; the generic installer owns installation.
import { spawn } from "node:child_process"
import { readdirSync } from "node:fs"
import { access, lstat, readFile, realpath } from "node:fs/promises"
import { homedir } from "node:os"
import { dirname, join, relative } from "node:path"
import { createInterface } from "node:readline/promises"
import { loadLocalKernelPresences } from "./local-kernel-presence.js"
import { isKernelEndpointReachable } from "./kernel-endpoint.js"
import type { RelayCloudProfile } from "./preferences.js"

export async function hasLocalKernel(): Promise<boolean> {
  if (loadLocalKernelPresences().length) return true
  // Setup uses isolated kernel homes; inspect the same ordinary public heartbeat records.
  const registries: string[] = []
  if (process.env.CHARIOX_HOME) registries.push(join(process.env.CHARIOX_HOME, "kernels/active"))
  const installs = join(homedir(), ".chariox/dev/ssh-machines")
  try {
    for (const entry of readdirSync(installs, { withFileTypes: true }).filter(e => e.isDirectory() && /^[a-z][a-z0-9-]{0,47}$/.test(e.name)).slice(0, 128)) {
      registries.push(join(installs, entry.name, "kernels/active"))
    }
  } catch { /* no isolated installs */ }
  if (registries.some(directory => loadLocalKernelPresences(directory).length)) return true
  const port = Number(process.env.CHARIOX_KERNEL_PORT ?? "43118")
  return Number.isInteger(port) && port > 0 && port <= 65535
    ? isKernelEndpointReachable(`ws://127.0.0.1:${port}/kernel`) : false
}
// MP-07 / MP-08 / MP-11: resolve the marked CLI release before choosing Setup defaults.
async function setupCommand(profile: RelayCloudProfile): Promise<{ executable: string; args: string[] }> {
  const invoking = await realpath(process.execPath), home = homedir()
  const installs = join(home, ".local/share/chariox/ssh-machines")
  const path = relative(installs, invoking).split("/")
  const marked = path.length === 5 && path[1] === "releases" && path[3] === "bin" && path[4] === "chariox"
  const installId = marked ? path[0]! : "local"
  if (!/^[a-z][a-z0-9-]{0,47}$/.test(installId)) throw new Error("Invoking CLI has an invalid Chariox install ID")
  const root = join(installs, installId), markerPath = join(root, "install.json")
  let selection: { installId: string; port: number } | undefined
  const metadata = await lstat(markerPath).catch(error => {
    if (error.code === "ENOENT" && !marked) return null
    throw new Error("Invoking Chariox install marker is missing")
  })
  if (metadata) {
    if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.uid !== process.getuid?.() || (metadata.mode & 0o022) || metadata.size > 65536) throw new Error("Chariox install marker must be a bounded user-owned regular file")
    const marker = JSON.parse(await readFile(markerPath, "utf8"))
    const service = process.platform === "darwin" ? `com.chariox.kernel.${installId}.plist` : `chariox-ssh-${installId}.service`
    if (marker.format !== "chariox.ssh-machine-install.v1" || marker.installId !== installId || marker.service !== service || !Number.isInteger(marker.port) || marker.port < 1024 || marker.port > 65534 || [43117, 43118, 43119, 43120].includes(marker.port) || !/^sha256:[a-f0-9]{64}$/.test(marker.releaseDigest ?? "")) throw new Error("Chariox install marker has an invalid identity or port")
    selection = { installId, port: marker.port }
  }
  const candidates = marked
    ? [join(root, "current/bin/chariox-setup")]
    : [join(dirname(invoking), "chariox-setup"), join(home, ".local/bin/chariox-setup")]
  for (const executable of candidates) {
    try { await access(executable) } catch { continue }
    // Account/user/API are public binding inputs, never terminal session credentials.
    const args = ["--api-url", profile.apiUrl, "--user-id", profile.userId]
    if (selection) args.push("--id", selection.installId, "--port", String(selection.port), "--repair")
    return { executable, args }
  }
  throw new Error("Chariox Setup is missing. Download the signed Chariox Setup installer from chariox.com.")
}
// MP-07/MP-08: Cloud status renders each notice line as one table row.
function setupNotice(text: string): string {
  return text.split(/\r?\n/).map(line => line.replace(/(.{1,76})\s+/g, "$1\n")).join("\n")
}
export async function startLocalKernelSetup(profile: RelayCloudProfile, notice?: (message: string) => void): Promise<void> {
  const { executable, args } = await setupCommand(profile)
  const env = { ...process.env }
  // A terminal launched from a managed/slice kernel must not redirect this install's state.
  for (const key of Object.keys(env)) if (key.startsWith("CHARIOX_")) delete env[key]
  await new Promise<void>((resolve, reject) => {
    const child = spawn(executable, args, { env, stdio: notice ? ["ignore", "pipe", "pipe"] : "inherit" })
    let total = 0, stderr = ""
    const stop = () => {
      if (child.exitCode !== null || child.signalCode !== null) return
      if (typeof child.pid !== "number" || !Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("refusing unsafe Setup process signal")
      child.kill("SIGKILL")
    }
    for (const output of [child.stdout, child.stderr]) {
      output?.on("data", (chunk: Buffer) => {
        total += chunk.length
        if (total > 32768) stop()
        else { if (output === child.stderr) stderr += chunk.toString("utf8"); notice?.(output === child.stderr ? setupNotice(chunk.toString("utf8")) : chunk.toString("utf8")) }
      })
    }
    const timer = setTimeout(stop, 900_000)
    child.once("error", () => { clearTimeout(timer); reject(new Error("Chariox Setup could not start")) })
    child.once("close", code => {
      clearTimeout(timer)
      const detail = stderr.trim().split(/\r?\n/).at(-1)?.slice(0, 2048)
      code === 0 ? resolve() : reject(new Error(detail ? `Chariox Setup failed: ${detail}` : "Chariox Setup failed; check the release, user service and machine approval"))
    })
  })
}
export async function offerLocalKernelSetup(profile: RelayCloudProfile): Promise<void> {
  if (await hasLocalKernel()) return
  if (!process.stdin.isTTY || !process.stdout.isTTY) {
    process.stdout.write("Set up a Chariox kernel on this machine? Run chariox setup to continue.\n")
    return
  }
  const rl = createInterface({ input: process.stdin, output: process.stdout })
  let answer: string
  try { answer = await rl.question("Set up a Chariox kernel on this machine? [y/N] ") }
  finally { rl.close() }
  if (/^y(es)?$/i.test(answer.trim())) await startLocalKernelSetup(profile)
}
