import { spawn } from "node:child_process"
import { readFile } from "node:fs/promises"

const PROBE = new URL("./managed-browser-computer-parity-host-probe.py", import.meta.url)
const MAX_OUTPUT = 2 * 1024 * 1024

// The remote timeout owns the probe's lifetime even if SSH loses its connection.
// The local subprocess is always reaped before observation returns or rejects.
export async function observeManagedParityHost({ host, engine, resources, retained, paths, signal }) {
  if (!/^[a-zA-Z0-9_.@:-]+$/.test(host ?? "") || host.startsWith("-")) {
    throw new Error("managed parity observer requires an explicit SSH host")
  }
  if (!/^unix:\/\/[\/a-zA-Z0-9_.-]+\.sock$/.test(engine?.endpoint ?? "")
    || !/^[a-zA-Z0-9:_-]{8,128}$/.test(engine?.id ?? "")) {
    throw new Error("managed parity observer requires a pinned Unix Docker endpoint and engine identity")
  }
  const source = await readFile(PROBE, "utf8")
  const input = JSON.stringify({ engine, resources, retained, paths })
  if (Buffer.byteLength(input) > 256 * 1024) throw new Error("managed parity observer input limit exceeded")
  const quote = (value) => `'${value.replaceAll("'", "'\\''")}'`
  const command = `/usr/bin/timeout --signal=TERM --kill-after=1s 12s /usr/bin/python3 -c ${quote(source)}`
  const output = await runBoundedObserverCommand("ssh", [
    "-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "--", host, command,
  ], { input, signal, timeoutMs: 18_000 })
  const result = JSON.parse(output)
  if (result?.schema !== "chariox.managed_parity.host_observation.v1") {
    throw new Error("managed parity observer returned an unsupported observation")
  }
  return result
}

export function runBoundedObserverCommand(command, args, { input = "", signal, timeoutMs = 18_000 } = {}) {
  if (signal?.aborted) return Promise.reject(new Error("managed parity host observation aborted"))
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ["pipe", "pipe", "pipe"], detached: true })
    const chunks = []
    let bytes = 0
    let failure = null
    let killTimer
    const killGroup = (kind) => {
      if (!child.pid) return
      try { process.kill(-child.pid, kind) } catch (error) { if (error.code !== "ESRCH") failure ??= error }
    }
    const stop = (reason) => {
      if (failure) return
      failure = new Error(reason)
      killGroup("SIGTERM")
      killTimer = setTimeout(() => killGroup("SIGKILL"), 100)
    }
    const abort = () => stop("managed parity host observation aborted")
    const timer = setTimeout(() => stop("managed parity host observation deadline exceeded"), timeoutMs)
    signal?.addEventListener("abort", abort, { once: true })
    child.stdout.on("data", (chunk) => {
      bytes += chunk.length
      if (bytes > MAX_OUTPUT) stop("managed parity host observation output limit exceeded")
      else chunks.push(chunk)
    })
    // Never echo diagnostics: SSH or Docker errors may contain environment data.
    child.stderr.on("data", (chunk) => {
      bytes += chunk.length
      if (bytes > MAX_OUTPUT) stop("managed parity host observation output limit exceeded")
    })
    child.stdin.on("error", () => {})
    child.once("error", () => { failure ??= new Error("managed parity observer process could not start") })
    child.once("close", (code) => {
      clearTimeout(timer)
      clearTimeout(killTimer)
      signal?.removeEventListener("abort", abort)
      // A descendant must not outlive an exited group leader.
      killGroup("SIGKILL")
      if (failure) reject(failure)
      else if (code === 77) reject(new Error("managed parity physical census requires a root SSH identity"))
      else if (code !== 0) reject(new Error("managed parity physical host inspection failed"))
      else resolve(Buffer.concat(chunks).toString("utf8"))
    })
    child.stdin.end(input)
    if (signal?.aborted) abort()
  })
}
