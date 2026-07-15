import assert from "node:assert/strict"
import { mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises"
import { chmod } from "node:fs/promises"
import { join } from "node:path"
import { tmpdir } from "node:os"
import test from "node:test"

import { readPrivateAgentAppAuditUrlFile } from "../publication-agent-app.js"

test("agent app audit URL file is consumed after descriptor validation", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-agent-app-audit-url-"))
  const path = join(root, "audit-url")
  try {
    await writeFile(path, " http://127.0.0.1:43119/audit?capability=test\n", { mode: 0o600 })

    assert.equal(
      readPrivateAgentAppAuditUrlFile(path),
      "http://127.0.0.1:43119/audit?capability=test",
    )
    await assert.rejects(readFile(path, "utf8"), /ENOENT/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app audit URL file rejects symlinks without consuming the target", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-agent-app-audit-symlink-"))
  const target = join(root, "target")
  const path = join(root, "audit-url")
  try {
    await writeFile(target, "http://127.0.0.1:43119/audit", { mode: 0o600 })
    await chmod(target, 0o600)
    await symlink(target, path)

    assert.throws(() => readPrivateAgentAppAuditUrlFile(path), /could not be opened safely/)
    assert.equal(await readFile(target, "utf8"), "http://127.0.0.1:43119/audit")
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
