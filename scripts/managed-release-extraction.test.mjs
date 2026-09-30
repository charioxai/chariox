import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { fileURLToPath } from "node:url"
import test from "node:test"

test("release extraction rejects resource exhaustion and path attacks before publishing files", () => {
  const result = spawnSync("python3", [fileURLToPath(new URL("../deploy/managed-kernel/extract-release.test.py", import.meta.url)), "-v"], {
    encoding: "utf8",
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
    timeout: 30_000,
  })
  assert.equal(result.status, 0, result.stderr || result.error?.message)
})
