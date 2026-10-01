import { createHash } from "node:crypto"
import { spawn } from "node:child_process"
import { fileURLToPath } from "node:url"

// MP-08/MP-10/MP-11: inherit the retained inode as stdin into the common
// process supervisor. The caller keeps its pin; no mutable name is reopened.
export async function digestPinnedHomeArchive(fd, progressTimeoutMs, signal) {
  if (!Number.isSafeInteger(fd) || fd < 0) throw new Error("invalid home archive descriptor")
  validateProgressTimeout(progressTimeoutMs)
  if (signal?.aborted) throw new Error("home archive verification cancelled")
  const guard = fileURLToPath(new URL("./slice-command-guard.py", import.meta.url))
  const child = spawn("/usr/bin/python3", [guard, "digest-stdin", String(progressTimeoutMs)], {
    stdio: [fd, "pipe", "pipe"],
  })
  let output = Buffer.alloc(0), failure
  const stop = () => child.kill("SIGTERM")
  signal?.addEventListener("abort", stop, { once: true })
  try {
    return await new Promise((resolve, reject) => {
      child.on("error", error => { failure = error })
      child.stdout.on("data", chunk => {
        if (output.length + chunk.length > 65) { failure = new Error("invalid archive digest output"); stop() }
        else output = Buffer.concat([output, chunk])
      })
      child.stderr.resume()
      child.on("close", status => {
        const digest = output.toString().trim()
        if (failure || status !== 0 || !/^[a-f0-9]{64}$/.test(digest)) reject(failure ?? new Error("home archive verification failed, cancelled, or made no progress"))
        else resolve(digest)
      })
    })
  } finally { signal?.removeEventListener("abort", stop) }
}

export async function digestHomeArchiveStream(stream, progressTimeoutMs) {
  validateProgressTimeout(progressTimeoutMs)
  const digest = createHash("sha256")
  let timer
  const arm = () => {
    clearTimeout(timer)
    timer = setTimeout(() => stream.destroy(new Error("home archive verification made no progress")), progressTimeoutMs)
  }
  arm()
  try {
    for await (const chunk of stream) {
      if (chunk.length === 0) continue
      digest.update(chunk)
      arm()
    }
    return digest.digest("hex")
  } finally {
    clearTimeout(timer)
    stream.destroy()
  }
}

function validateProgressTimeout(value) {
  if (!Number.isSafeInteger(value) || value <= 0 || value > 2_147_483_647) {
    throw new Error("invalid home archive progress timeout")
  }
}
