import { spawn } from "node:child_process"
import { fileURLToPath } from "node:url"

const guard = fileURLToPath(new URL("./slice-command-guard.py", import.meta.url))

// MP-08/MP-10/MP-11: the same process-group owner as ordinary provisioning.
// A live or silent build has no total deadline. Lease loss settles its producer.
export async function runBrokerCommand(command, args, { env, maxBuffer, signal }) {
  if (signal?.aborted) throw new Error("slice broker operation cancelled")
  const child = spawn("/usr/bin/python3", [guard, "unbounded", "--", command, ...args], {
    env, stdio: ["ignore", "pipe", "pipe"],
  })
  const output = [[], []], sizes = [0, 0]
  let error
  const stop = () => child.kill("SIGTERM")
  signal?.addEventListener("abort", stop, { once: true })
  try {
    return await new Promise(resolve => {
      child.on("error", failure => { error = failure })
      for (const [index, stream] of [child.stdout, child.stderr].entries()) {
        stream.on("data", chunk => {
          sizes[index] += chunk.length
          if (sizes[index] > maxBuffer) {
            error ??= new Error("slice broker output exceeds capture limit")
            stop()
          } else output[index].push(chunk)
        })
      }
      child.on("close", status => resolve({
        status: error ? 125 : status ?? 125,
        stdout: Buffer.concat(output[0]),
        stderr: error ? Buffer.from(error.message) : Buffer.concat(output[1]),
        error,
      }))
    })
  } finally {
    signal?.removeEventListener("abort", stop)
  }
}
