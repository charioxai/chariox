import { createHash } from "node:crypto"
import { createReadStream } from "node:fs"

// Reopen the already pinned inode, not its mutable filesystem name. The stream
// owns this second descriptor; the caller retains its pin through restoration.
export async function digestPinnedHomeArchive(fd, progressTimeoutMs) {
  if (!Number.isSafeInteger(fd) || fd < 0) throw new Error("invalid home archive descriptor")
  validateProgressTimeout(progressTimeoutMs)
  const stream = createReadStream(`/proc/${process.pid}/fd/${fd}`, { highWaterMark: 64 * 1024 })
  return digestHomeArchiveStream(stream, progressTimeoutMs)
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
