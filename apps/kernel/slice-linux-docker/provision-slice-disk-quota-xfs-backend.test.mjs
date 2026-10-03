import assert from "node:assert/strict"
import { chmod, mkdtemp, mkdir, readFile, rm, symlink } from "node:fs/promises"
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

test("trusted-path production helper can walk the owner-only Docker tree without DAC bypass", async () => {
  const uid = process.getuid()
  assert.notEqual(uid, 0, "run this filesystem regression as an unprivileged test user")
  const status = await readFile("/proc/self/status", "utf8")
  const effectiveCaps = status.match(/^CapEff:\s+([0-9a-f]+)$/m)?.[1]
  assert.ok(effectiveCaps, "Linux must expose effective capabilities for this regression")
  const dacBypass = (1n << 1n) | (1n << 2n)
  assert.equal(BigInt(`0x${effectiveCaps}`) & dacBypass, 0n, "test must not have CAP_DAC_OVERRIDE or CAP_DAC_READ_SEARCH")

  const unit = await readFile(new URL("./chariox-slice-disk-quota-allocator.service", import.meta.url), "utf8")
  assert.match(unit, /^User=chariox-docker$/m)
  assert.match(unit, /^Group=chariox-docker$/m)
  assert.match(unit, /^CapabilityBoundingSet=CAP_SYS_ADMIN$/m)
  assert.match(unit, /^AmbientCapabilities=CAP_SYS_ADMIN$/m)
  assert.doesNotMatch(unit, /CAP_DAC_(?:OVERRIDE|READ_SEARCH)/)

  const root = await mkdtemp(join(tmpdir(), "slice-quota-private-path-"))
  try {
    const pathComponents = ["var", "lib", "chariox-docker", "data", "volumes", "home", "_data"]
    const target = join(root, ...pathComponents)
    const prefix = join(root, ...pathComponents.slice(0, -1))
    await mkdir(target, { recursive: true })
    let current = root
    for (const component of pathComponents) {
      await chmod(current, 0o700)
      current = join(current, component)
    }
    await chmod(target, 0o700)

    assert.equal(assertTrustedPath(target, prefix, uid), target)
  } finally {
    await chmod(root, 0o700)
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
