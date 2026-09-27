import assert from "node:assert/strict"
import { mkdtemp, mkdir, rm, symlink } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { assertTrustedPath } from "./slice-disk-quota-xfs-backend.mjs"

test("trusted Docker paths walk normal nested filesystem components", async () => {
  const root = await mkdtemp(join(tmpdir(), "slice-quota-path-"))
  try {
    const prefix = join(root, "var", "lib", "chariox-docker", "data", "volumes", "home")
    const target = join(prefix, "_data")
    await mkdir(target, { recursive: true })

    assert.equal(assertTrustedPath(target, prefix, process.getuid()), target)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("trusted Docker paths reject noncanonical targets and the wrong owner", async () => {
  const root = await mkdtemp(join(tmpdir(), "slice-quota-path-"))
  try {
    const prefix = join(root, "docker", "data", "volumes", "home")
    const target = join(prefix, "_data")
    const elsewhere = join(root, "elsewhere")
    await mkdir(target, { recursive: true })
    await mkdir(elsewhere)
    await symlink(elsewhere, join(prefix, "linked"))

    assert.throws(() => assertTrustedPath(join(prefix, "linked"), prefix, process.getuid()), /canonical/)
    assert.throws(() => assertTrustedPath(target, prefix, process.getuid() + 1), /ownership/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
