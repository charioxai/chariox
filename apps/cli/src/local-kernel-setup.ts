// MP-07 / MP-08 / MP-11: CLI projects a setup offer; the generic installer owns installation.
import { spawn } from "node:child_process"
import { access, realpath } from "node:fs/promises"
import { dirname, join } from "node:path"
import { createInterface } from "node:readline/promises"
import { loadLocalKernelPresences } from "./local-kernel-presence.js"
import { isKernelEndpointReachable } from "./kernel-endpoint.js"
import type { RelayCloudProfile } from "./preferences.js"

export async function hasLocalKernel(): Promise<boolean> {
  if (loadLocalKernelPresences().length) return true
  const port = Number(process.env.CHARIOX_KERNEL_PORT ?? "43118")
  return Number.isInteger(port) && port > 0 && port <= 65535
    ? isKernelEndpointReachable(`ws://127.0.0.1:${port}/kernel`) : false
}
async function setupExecutable(): Promise<string> {
  const executable = await realpath(process.execPath)
  const candidates = [join(dirname(executable), "chariox-setup"), join(process.env.HOME ?? "", ".local/bin/chariox-setup")]
  for (const candidate of candidates) { try { await access(candidate); return candidate } catch { /* next install */ } }
  throw new Error("Chariox Setup is missing. Download the signed Chariox Setup installer from chariox.com.")
}
export async function startLocalKernelSetup(profile: RelayCloudProfile, notice?: (message: string) => void): Promise<void> {
  const executable = await setupExecutable()
  // Account/user/API are public display/binding inputs, never terminal session credentials.
  const args = ["--api-url", profile.apiUrl, "--user-id", profile.userId]
  const marker = join(process.env.HOME ?? "", ".local/share/chariox/ssh-machines/local/install.json")
  try { await access(marker); args.push("--repair") } catch { /* fresh install */ }
  const env = { ...process.env }
  // A terminal launched from a managed/slice kernel must not redirect this install's state.
  for (const key of Object.keys(env)) if (key.startsWith("CHARIOX_")) delete env[key]
  await new Promise<void>((resolve, reject) => {
    const child = spawn(executable, args, { env, stdio: notice ? ["ignore", "pipe", "ignore"] : "inherit" })
    let total = 0
    const stop = () => {
      if (child.exitCode !== null || child.signalCode !== null) return
      if (typeof child.pid !== "number" || !Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("refusing unsafe Setup process signal")
      child.kill("SIGKILL")
    }
    child.stdout?.on("data", (chunk: Buffer) => { total += chunk.length; if (total > 32768) stop(); else notice?.(chunk.toString("utf8")) })
    const timer = setTimeout(stop, 900_000)
    child.once("error", () => { clearTimeout(timer); reject(new Error("Chariox Setup could not start")) })
    child.once("close", code => { clearTimeout(timer); code === 0 ? resolve() : reject(new Error("Chariox Setup failed; check the release, user service and machine approval")) })
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
