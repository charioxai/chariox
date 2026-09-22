import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { chmod, mkdtemp, rename, rm, symlink, unlink, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { pathToFileURL } from "node:url"
import { test } from "node:test"

import {
  parseManagedBrowserComputerParityArgs,
  readPrivateManagedParityConfig,
  loadReviewedManagedParityAdapterModule,
  verifyManagedParityAdapterModule,
} from "./managed-browser-computer-parity-cli.mjs"

test("managed parity CLI accepts only metadata paths and external evidence", () => {
  const options = parseManagedBrowserComputerParityArgs([
    "--config", "/private/config.json",
    "--transport-module", "/reviewed/product-transport.mjs",
    "--evidence-root", "/evidence/cha-16",
  ], { repoRoot: "/source/chariox" })
  assert.equal(options.configPath, "/private/config.json")
  assert.equal(options.transportModulePath, "/reviewed/product-transport.mjs")
  assert.equal(options.evidenceRoot, "/evidence/cha-16")
  assert.throws(
    () => parseManagedBrowserComputerParityArgs([
      "--config", "/private/config.json",
      "--transport-module", "/reviewed/product-transport.mjs",
      "--evidence-root", "/source/chariox/evidence",
    ], { repoRoot: "/source/chariox" }),
    /outside the repository/,
  )
  assert.throws(
    () => parseManagedBrowserComputerParityArgs(["--token", "must-not-be-an-argument"], { repoRoot: "/source/chariox" }),
    /unknown managed parity argument/,
  )
})

test("managed parity CLI reads only private regular config files", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-managed-parity-cli-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const configPath = path.join(root, "config.json")
  await writeFile(configPath, '{"runId":"run-1"}\n', { mode: 0o600 })
  assert.deepEqual(await readPrivateManagedParityConfig(configPath), { runId: "run-1" })

  await chmod(configPath, 0o644)
  await assert.rejects(() => readPrivateManagedParityConfig(configPath), /must not be accessible/)
  await chmod(configPath, 0o600)
  const alias = path.join(root, "config-alias.json")
  await symlink(configPath, alias)
  await assert.rejects(() => readPrivateManagedParityConfig(alias), /regular file/)
})

test("managed parity CLI independently pins reviewed adapter identity and file hash", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-managed-parity-adapter-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const modulePath = path.join(root, "adapter.mjs")
  const source = 'export const MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY = "reviewed-adapter-v1"\n'
  await writeFile(modulePath, source, { mode: 0o600 })
  const expected = {
    identity: "reviewed-adapter-v1",
    sha256: `sha256:${createHash("sha256").update(source).digest("hex")}`,
  }
  assert.deepEqual(await verifyManagedParityAdapterModule(modulePath, {
    MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY: expected.identity,
  }, expected), { ...expected, verifiedBy: "chariox-harness-loader" })
  await assert.rejects(
    () => verifyManagedParityAdapterModule(
      modulePath,
      { MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY: expected.identity },
      { ...expected, sha256: `sha256:${"0".repeat(64)}` },
    ),
    /hash does not match/,
  )
  await assert.rejects(
    () => verifyManagedParityAdapterModule(
      modulePath,
      { MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY: "self-asserted" },
      expected,
    ),
    /identity does not match/,
  )
  let loaded = false
  await assert.rejects(
    () => loadReviewedManagedParityAdapterModule(
      modulePath,
      { ...expected, sha256: `sha256:${"0".repeat(64)}` },
      async () => {
        loaded = true
        return {}
      },
    ),
    /hash does not match/,
  )
  assert.equal(loaded, false)
})

test("managed parity adapter executes verified bytes during deterministic replacement races", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-managed-parity-adapter-race-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const modulePath = path.join(root, "adapter.mjs")
  const dependencyPath = path.join(root, "adapter-dependency.mjs")
  const replacementPath = path.join(root, "replacement.mjs")
  const source = [
    'import { payload as dependencyPayload } from "./adapter-dependency.mjs"',
    'export const MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY = "reviewed-adapter-v1"',
    'export const payload = `reviewed:${dependencyPayload}`',
    "",
  ].join("\n")
  const substituted = [
    'export const MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY = "reviewed-adapter-v1"',
    'export const payload = "substituted"',
    "",
  ].join("\n")
  await writeFile(modulePath, source, { mode: 0o600 })
  await writeFile(dependencyPath, 'export const payload = "dependency"\n', { mode: 0o600 })
  await writeFile(replacementPath, substituted, { mode: 0o600 })
  const expected = {
    identity: "reviewed-adapter-v1",
    sha256: `sha256:${createHash("sha256").update(source).digest("hex")}`,
  }
  const result = await loadReviewedManagedParityAdapterModule(modulePath, expected, async (snapshotPath) => {
    await rename(replacementPath, modulePath)
    try {
      return await import(pathToFileURL(snapshotPath).href)
    } finally {
      await unlink(modulePath, { force: true })
      await writeFile(modulePath, source, { mode: 0o600 })
    }
  })
  assert.equal(result.imported.payload, "reviewed:dependency")
  assert.deepEqual(result.verification, { ...expected, verifiedBy: "chariox-harness-loader" })

  const overwriteResult = await loadReviewedManagedParityAdapterModule(modulePath, expected, async (snapshotPath) => {
    await writeFile(modulePath, substituted, { mode: 0o600 })
    try {
      return await import(`${pathToFileURL(snapshotPath).href}?overwrite-race`)
    } finally {
      await writeFile(modulePath, source, { mode: 0o600 })
    }
  })
  assert.equal(overwriteResult.imported.payload, "reviewed:dependency")
})
