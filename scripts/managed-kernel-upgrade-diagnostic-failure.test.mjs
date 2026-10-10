import assert from "node:assert/strict"
import { mkdtemp, readFile, writeFile, chmod, rm } from "node:fs/promises"
import { join } from "node:path"
import { tmpdir } from "node:os"
import { spawnSync } from "node:child_process"
import test from "node:test"

test("MP-07 failed updater diagnostic producer is visible without altering authority or leaking stderr", async () => {
  const root = await mkdtemp(join(tmpdir(), "path1-diagnostic-failure-"))
  try {
    const source = await readFile(new URL("../deploy/managed-kernel/upgrade-image.sh", import.meta.url), "utf8")
    const fn = source.match(/record_diagnostic_phase\(\) \{\n[\s\S]*?\n\}/)?.[0]
    assert(fn)
    await writeFile(join(root, "python3"), '#!/bin/sh\nprintf "private diagnostic must not be printed\\n" >&2\nexit 71\n')
    await chmod(join(root, "python3"), 0o755)
    const result = spawnSync("sh", ["-eu", "-c", fn + '\nrecord_diagnostic_phase stopped\nprintf "authority-continued\\n"\n'], {
      env: { ...process.env, PATH: root + ":" + process.env.PATH, script_root: root, CHARIOX_RUNTIME_DIAGNOSTICS_DIR: root },
      encoding: "utf8",
    })
    assert.equal(result.status, 0)
    assert.equal(result.stdout, "authority-continued\n")
    assert.equal(result.stderr, "managed kernel upgrade diagnostic event publication failed\n")
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
