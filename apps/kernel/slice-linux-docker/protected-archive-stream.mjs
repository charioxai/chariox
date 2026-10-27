import { spawn } from "node:child_process"
import { constants, openSync, closeSync, fsyncSync, lstatSync, statfsSync, unlinkSync, createReadStream } from "node:fs"
import { createHash } from "node:crypto"
import { dirname } from "node:path"
import { verifyPrivateHostDirectory } from "./protected-host-root.mjs"

// Streaming keeps intentional browser-session backup out of a container layer,
// output buffer, task scratch and evidence. The caller verifies the source layout
// before constructing the fixed Docker command; this function verifies its sink.
export async function streamArchiveToProtectedSink({ command, args, environment, path,
  maxBytes = 32 * 1024 ** 3, reserveBytes = 2 * 1024 ** 3, timeoutMs = 10 * 60_000 }) {
  verifyPrivateHostDirectory(dirname(path), process.getuid())
  const fd = openSync(path, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600)
  let success = false
  try {
    const child = spawn(command, args, { env: environment, stdio: ["ignore", fd, "ignore"] })
    let refusal = null
    let killTimer
    const terminate = reason => {
      if (refusal) return
      refusal = reason
      child.kill("SIGTERM")
      killTimer = setTimeout(() => child.kill("SIGKILL"), 1000)
    }
    const watchdog = setInterval(() => {
      try {
        const metadata = lstatSync(path)
        const capacity = statfsSync(dirname(path), { bigint: true })
        if (metadata.size > maxBytes || capacity.bavail * capacity.bsize < BigInt(reserveBytes)) terminate("archive storage reserve exceeded")
      } catch { terminate("archive storage check failed") }
    }, 100)
    const deadline = setTimeout(() => terminate("archive capture deadline exceeded"), timeoutMs)
    try {
      await new Promise((resolve, reject) => {
        child.once("error", reject)
        child.once("close", code => code === 0 && !refusal ? resolve() : reject(new Error(refusal ?? "archive capture failed")))
      })
    } finally {
      clearInterval(watchdog); clearTimeout(deadline); clearTimeout(killTimer)
    }
    const metadata = lstatSync(path)
    if (!metadata.isFile() || metadata.nlink !== 1 || metadata.uid !== process.getuid() || (metadata.mode & 0o077) !== 0 || metadata.size === 0 || metadata.size > maxBytes) throw new Error("archive capture metadata is invalid")
    const filesystem = statfsSync(dirname(path), { bigint: true })
    if (filesystem.bavail * filesystem.bsize < BigInt(reserveBytes)) throw new Error("archive storage reserve exceeded")
    fsyncSync(fd)
    const digest = createHash("sha256")
    for await (const chunk of createReadStream(path)) digest.update(chunk)
    success = true
    return { sizeBytes: metadata.size, sha256: digest.digest("hex") }
  } finally {
    closeSync(fd)
    if (!success) unlinkSync(path)
  }
}
