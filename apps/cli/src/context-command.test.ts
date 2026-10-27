import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"
import { promisify } from "node:util"

const execute = promisify(execFile)

test("MP-05/MP-08/MP-10/MP-11 context copy rejects a FIFO without a writer promptly", {
  skip: process.platform === "win32",
}, async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-context-fifo-"))
  const fifo = join(root, "selection.fifo")
  try {
    await execute("mkfifo", [fifo])
    await assert.rejects(execute("bun", [
      fileURLToPath(new URL("./index.js", import.meta.url)),
      "context", "copy", fifo, "--socket", join(root, "absent-kernel.sock"),
    ], {
      env: { ...process.env, CHARIOX_HOME: root },
      timeout: 5_000,
      killSignal: "SIGKILL",
    }), (error: unknown) => {
      const result = error as { code: number; killed: boolean; stderr: string }
      assert.equal(result.killed, false, "context copy must fail before the subprocess timeout")
      assert.equal(result.code, 1)
      assert.match(result.stderr, /Context selection must be a regular file/)
      return true
    })
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
