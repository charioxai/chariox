import { spawn } from "node:child_process"
import { closeSync, lstatSync, mkdirSync, mkdtempSync, openSync, writeSync } from "node:fs"
import { join } from "node:path"
import { fileURLToPath } from "node:url"

const guard = fileURLToPath(new URL("./slice-command-guard.py", import.meta.url))

// MP-08/MP-10/MP-11: the same process-group owner as ordinary provisioning.
// A live or silent build has no total deadline. Lease loss settles its producer.
export async function runBrokerCommand(command, args, { env, maxBuffer, signal, logRoot }) {
  if (signal?.aborted) throw new Error("slice broker operation cancelled")
  if (!Number.isSafeInteger(maxBuffer) || maxBuffer < 1 || maxBuffer > 4 * 1024 * 1024) throw new Error("invalid broker summary limit")
  mkdirSync(logRoot, { recursive: true, mode: 0o700 })
  const metadata = lstatSync(logRoot)
  if (!metadata.isDirectory() || metadata.isSymbolicLink() || (metadata.mode & 0o777) !== 0o700 || metadata.uid !== process.getuid()) throw new Error("broker command log root is not private")
  const logDirectory = mkdtempSync(join(logRoot, "command-"))
  const descriptors = ["stdout.log", "stderr.log"].map(name => openSync(join(logDirectory, name), "wx", 0o600))
  const child = spawn("/usr/bin/python3", [guard, "unbounded", "--", command, ...args], {
    env, stdio: ["ignore", "pipe", "pipe"],
  })
  const output = [Buffer.alloc(0), Buffer.alloc(0)], truncated = [false, false]
  let error
  const stop = () => child.kill("SIGTERM")
  signal?.addEventListener("abort", stop, { once: true })
  try {
    return await new Promise(resolve => {
      child.on("error", failure => { error = failure })
      for (const [index, stream] of [child.stdout, child.stderr].entries()) {
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
