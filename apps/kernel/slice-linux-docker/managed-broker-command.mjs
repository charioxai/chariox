import { spawn } from "node:child_process"
import { closeSync, lstatSync, mkdirSync, mkdtempSync, openSync, writeSync } from "node:fs"
import { join } from "node:path"
import { fileURLToPath } from "node:url"

const guard = fileURLToPath(new URL("./slice-command-guard.py", import.meta.url))

// MP-08/MP-10/MP-11: raw noninteractive Docker uses the provisioner's
// 30-second control policy. Auth inspection needs only an exit status.
export function dockerControlPolicy(args) {
  // Image snapshots and container-to-host copies are archive producers.
  // Like provisioner builds and home capture, they retain owned cancellation.
  if (args[0] === "commit" || args[0] === "cp") return {}
  return {
    timeout: 30_000,
    statusOnly: args[0] === "exec" && args[1] === "-u" && args[2] === "slice"
      && (args[4] === "gh" || args[4] === "test"),
  }
}

// MP-08/MP-10/MP-11: the same process-group owner as ordinary provisioning.
// A live or silent build has no total deadline. Lease loss settles its producer.
export async function runBrokerCommand(command, args, { env, maxBuffer, signal, logRoot, timeout, statusOnly = false }) {
  if (signal?.aborted) throw new Error("slice broker operation cancelled")
  if (!Number.isSafeInteger(maxBuffer) || maxBuffer < 1 || maxBuffer > 4 * 1024 * 1024) throw new Error("invalid broker summary limit")
  if (timeout !== undefined && (!Number.isSafeInteger(timeout) || timeout < 1 || timeout > 2147483647)) throw new Error("invalid broker control timeout")
  let logDirectory
  const descriptors = []
  if (!statusOnly) {
    mkdirSync(logRoot, { recursive: true, mode: 0o700 })
    const metadata = lstatSync(logRoot)
    if (!metadata.isDirectory() || metadata.isSymbolicLink() || (metadata.mode & 0o777) !== 0o700 || metadata.uid !== process.getuid()) throw new Error("broker command log root is not private")
    logDirectory = mkdtempSync(join(logRoot, "command-"))
    for (const name of ["stdout.log", "stderr.log"]) descriptors.push(openSync(join(logDirectory, name), "wx", 0o600))
  }
  const lifetime = timeout === undefined ? ["unbounded"] : ["run", String(timeout / 1000)]
  const child = spawn("/usr/bin/python3", [guard, ...lifetime, "--", command, ...args], {
    // Drop auth probe streams before any broker capture or log allocation.
    env, stdio: ["ignore", statusOnly ? "ignore" : "pipe", statusOnly ? "ignore" : "pipe"],
  })
  const output = [Buffer.alloc(0), Buffer.alloc(0)], truncated = [false, false]
  let error
  const stop = () => child.kill("SIGTERM")
  signal?.addEventListener("abort", stop, { once: true })
  try {
    return await new Promise(resolve => {
      child.on("error", failure => { error = failure })
      for (const [index, stream] of [child.stdout, child.stderr].entries()) {
        if (!stream) continue
        stream.on("data", chunk => {
          if (!error) {
            try {
              // Complete diagnostics go to private files; only a bounded tail
              // crosses the broker frame. Output volume never fails a producer.
              let offset = 0
              while (offset < chunk.length) offset += writeSync(descriptors[index], chunk, offset, chunk.length - offset)
              truncated[index] ||= output[index].length + chunk.length > maxBuffer
              const tail = Buffer.concat([output[index], chunk])
              output[index] = Buffer.from(tail.subarray(Math.max(0, tail.length - maxBuffer)))
            } catch (failure) { error = failure; stop() }
          }
        })
      }
      child.on("close", status => resolve({
        status: error ? 125 : status ?? 125,
        stdout: output[0],
        stderr: Buffer.concat([error ? Buffer.from("slice broker log write failed") : output[1],
          truncated.some(Boolean) ? Buffer.from(`\n[slice-linux] complete diagnostics: ${logDirectory}\n`) : Buffer.alloc(0)]),
        error,
      }))
    })
  } finally {
    signal?.removeEventListener("abort", stop)
    for (const descriptor of descriptors) closeSync(descriptor)
  }
}
