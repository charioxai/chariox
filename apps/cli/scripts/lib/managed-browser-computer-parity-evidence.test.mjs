import assert from "node:assert/strict"
import { execFile as execFileCallback } from "node:child_process"
import { mkdtemp, mkdir, readFile, rm, symlink, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { promisify } from "node:util"
import { test } from "node:test"

import {
  assertManagedParityEvidenceRoot,
  enumerateManagedParityEvidenceRoot,
  ManagedParityEvidenceError,
  writeManagedParityEvidenceManifest,
} from "./managed-browser-computer-parity-evidence.mjs"

const execFile = promisify(execFileCallback)

async function temporaryEvidenceRoot(context) {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-managed-parity-evidence-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  return root
}

function isInventoryFailure(error) {
  return error instanceof ManagedParityEvidenceError && error.code === "evidence_inventory_incomplete"
}

test("evidence scanner returns an explicit bounded manifest for nested regular files", async (context) => {
  const root = await temporaryEvidenceRoot(context)
  await mkdir(path.join(root, "nested"))
  await writeFile(path.join(root, "managed-parity.json"), '{"status":"safe"}\n', { mode: 0o600 })
  await writeFile(path.join(root, "nested", "operations.jsonl"), '{"operation":"complete"}\n', { mode: 0o600 })

  const manifest = await enumerateManagedParityEvidenceRoot(root)
  assert.equal(manifest.enumerationComplete, true)
  assert.equal(manifest.source, "physical-evidence-root")
  assert.equal(manifest.enumeratedFileCount, 2)
  assert.equal(manifest.totalBytes, manifest.files.reduce((total, file) => total + file.sizeBytes, 0))
  assert.deepEqual(manifest.files.map((file) => file.relativePath), ["managed-parity.json", "nested/operations.jsonl"])
  assert.ok(manifest.files.every((file) => file.scan.completed && file.scan.forbiddenMatches === 0))
})

test("final evidence manifest is a private sibling and includes the written report and artifact index", async (context) => {
  const evidenceRoot = await temporaryEvidenceRoot(context)
  const runDir = path.join(evidenceRoot, "managed-parity-run-001")
  await mkdir(runDir)
  await writeFile(path.join(runDir, "managed-browser-computer-parity.json"), '{"status":"passed"}\n', { mode: 0o600 })
  await writeFile(path.join(runDir, "chariox-drill-artifacts.json"), '{"schema":"chariox.drill.artifact_index.v1"}\n', { mode: 0o600 })

  const { manifest, manifestPath } = await writeManagedParityEvidenceManifest(runDir)

  assert.equal(path.dirname(manifestPath), evidenceRoot)
  assert.match(path.basename(manifestPath), /^\.managed-parity-run-001\.evidence-manifest\.json$/)
  assert.deepEqual(manifest.files.map(({ relativePath }) => relativePath), [
    "chariox-drill-artifacts.json",
    "managed-browser-computer-parity.json",
  ])
  const persisted = JSON.parse(await readFile(manifestPath, "utf8"))
  assert.deepEqual(persisted.files.map(({ relativePath }) => relativePath), manifest.files.map(({ relativePath }) => relativePath))
})

test("evidence scanner fails closed on secret-looking names and values without returning payloads", async (context) => {
  const root = await temporaryEvidenceRoot(context)
  const secretName = "credential.json"
  await writeFile(path.join(root, secretName), "safe metadata\n", { mode: 0o600 })
  await assert.rejects(
    () => enumerateManagedParityEvidenceRoot(root),
    (error) => isInventoryFailure(error) && !error.message.includes(secretName),
  )

  await rm(path.join(root, secretName), { force: true })
  const secretValue = `Bearer ${"x".repeat(32)}`
  await writeFile(path.join(root, "safe.log"), secretValue, { mode: 0o600 })
  await assert.rejects(
    () => enumerateManagedParityEvidenceRoot(root),
    (error) => isInventoryFailure(error) && !error.message.includes(secretValue),
  )
})

test("evidence scanner rejects symlinks and special files", { skip: process.platform === "win32" }, async (context) => {
  const root = await temporaryEvidenceRoot(context)
  const target = path.join(root, "target.log")
  await writeFile(target, "safe\n", { mode: 0o600 })
  await symlink(target, path.join(root, "alias.log"))
  await assert.rejects(() => enumerateManagedParityEvidenceRoot(root), isInventoryFailure)

  await rm(path.join(root, "alias.log"), { force: true })
  const fifo = path.join(root, "events.fifo")
  await execFile("mkfifo", [fifo])
  await assert.rejects(() => enumerateManagedParityEvidenceRoot(root), isInventoryFailure)
})

test("evidence scanner enforces file-count and byte ceilings", async (context) => {
  const root = await temporaryEvidenceRoot(context)
  await writeFile(path.join(root, "one.log"), "12345678", { mode: 0o600 })
  await writeFile(path.join(root, "two.log"), "ok", { mode: 0o600 })
  await assert.rejects(
    () => enumerateManagedParityEvidenceRoot(root, { maxFiles: 1 }),
    isInventoryFailure,
  )
  await assert.rejects(
    () => enumerateManagedParityEvidenceRoot(root, { maxFileBytes: 4 }),
    isInventoryFailure,
  )
  await assert.rejects(
    () => enumerateManagedParityEvidenceRoot(root, { maxTotalBytes: 4 }),
    isInventoryFailure,
  )
})

test("evidence root admission rejects relative and symlinked roots", async (context) => {
  const root = await temporaryEvidenceRoot(context)
  const alias = `${root}-alias`
  await symlink(root, alias)
  context.after(() => rm(alias, { force: true }))
  await assert.rejects(() => assertManagedParityEvidenceRoot("relative/evidence"), isInventoryFailure)
  await assert.rejects(() => assertManagedParityEvidenceRoot(alias), isInventoryFailure)
})
