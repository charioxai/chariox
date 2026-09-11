import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { chmod, mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { pathToFileURL } from "node:url"
import { test } from "node:test"

import {
  parseManagedBrowserComputerParityArgs,
  readPrivateManagedParityConfig,
  loadReviewedManagedParityAdapterModule,
  validateManagedParityEvidenceDirectory,
  verifyManagedParityAdapterModule,
} from "./managed-browser-computer-parity-cli.mjs"

test("managed parity CLI accepts only metadata paths and external evidence", () => {
  const options = parseManagedBrowserComputerParityArgs([
    "--config", "/private/config.json",
    "--transport-module", "/reviewed/product-transport.mjs",
    "--inspector-module", "/reviewed/cleanup-inspector.mjs",
    "--evidence-root", "/evidence/cha-16",
  ], { repoRoot: "/source/chariox" })
  assert.equal(options.configPath, "/private/config.json")
  assert.equal(options.transportModulePath, "/reviewed/product-transport.mjs")
  assert.equal(options.inspectorModulePath, "/reviewed/cleanup-inspector.mjs")
  assert.equal(options.evidenceRoot, "/evidence/cha-16")
  assert.throws(
    () => parseManagedBrowserComputerParityArgs([
      "--config", "/private/config.json",
      "--transport-module", "/reviewed/product-transport.mjs",
      "--inspector-module", "/reviewed/cleanup-inspector.mjs",
      "--evidence-root", "/source/chariox/evidence",
    ], { repoRoot: "/source/chariox" }),
    /outside the repository/,
  )
  assert.throws(
    () => parseManagedBrowserComputerParityArgs(["--token", "must-not-be-an-argument"], { repoRoot: "/source/chariox" }),
    /unknown managed parity argument/,
  )
})

test("managed parity CLI bounds config input and rejects symlinked evidence paths", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-managed-parity-paths-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const repoRoot = path.join(root, "repo")
  const evidenceRoot = path.join(root, "evidence")
  await mkdir(repoRoot)
  await mkdir(evidenceRoot)
  const alias = path.join(root, "evidence-alias")
  await symlink(evidenceRoot, alias)
  await assert.rejects(() => validateManagedParityEvidenceDirectory(alias, repoRoot), /symbolic links/)

  const oversized = path.join(root, "oversized.json")
  await writeFile(oversized, Buffer.alloc(1024 * 1024 + 1, 0x20), { mode: 0o600 })
  await assert.rejects(() => readPrivateManagedParityConfig(oversized), /bounded size limit/)
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
  await assert.rejects(() => readPrivateManagedParityConfig(alias), /regular file|symbolic links/)
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
    () => verifyManagedParityAdapterModule(modulePath, { MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY: expected.identity }, { ...expected, sha256: `sha256:${"0".repeat(64)}` }),
    /hash does not match/,
  )
  await assert.rejects(
    () => verifyManagedParityAdapterModule(modulePath, { MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY: "self-asserted" }, expected),
    /identity does not match/,
  )
  await assert.rejects(
    () => loadReviewedManagedParityAdapterModule(modulePath, { ...expected, sha256: `sha256:${"0".repeat(64)}` }),
    /hash does not match/,
  )
})

test("managed parity adapter import cannot swap reviewed bytes and restore them after import", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-managed-parity-adapter-swap-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const modulePath = path.join(root, "adapter.mjs")
  const reviewed = 'export const MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY = "reviewed-adapter-v1"\nexport const payload = "reviewed"\n'
  const substituted = 'export const MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY = "reviewed-adapter-v1"\nexport const payload = "substituted"\n'
  await writeFile(modulePath, reviewed, { mode: 0o600 })
  const expected = {
    identity: "reviewed-adapter-v1",
    sha256: `sha256:${createHash("sha256").update(reviewed).digest("hex")}`,
  }
  await assert.rejects(
    () => loadReviewedManagedParityAdapterModule(modulePath, expected, async (candidate) => {
      await writeFile(candidate, substituted, { mode: 0o600 })
      const imported = await import(`${pathToFileURL(candidate).href}?swap=${Date.now()}`)
      await writeFile(candidate, reviewed, { mode: 0o600 })
      assert.equal(imported.payload, "substituted")
      return imported
    }),
    /reviewed bytes|loader injection|hash does not match/,
  )
})
