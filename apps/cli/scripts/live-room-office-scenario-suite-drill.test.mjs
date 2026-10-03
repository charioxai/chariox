import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtemp, mkdir, realpath, rm, symlink } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { resolveEvidenceDirectory } from "./live-room-office-scenario-suite-drill.mjs"

const script = fileURLToPath(new URL("./live-room-office-scenario-suite-drill.mjs", import.meta.url))
const repoRoot = path.resolve(path.dirname(script), "../../..")

test("CLI catalog lists the canonical five scenarios without loading the built kernel client", () => {
  const child = spawnSync(process.execPath, [script, "--list"], { encoding: "utf8" })
  assert.equal(child.status, 0, child.stderr)
  const report = JSON.parse(child.stdout)
  assert.equal(report.status, "catalog")
  assert.deepEqual(report.scenarios.map((item) => item.id), [
    "email-gated-onboarding", "vendor-research-crm", "document-intake-follow-up",
    "public-api-extension", "support-ticket-lifecycle",
  ])
})

test("CLI identifies missing operator choices without connecting to a kernel", () => {
  const child = spawnSync(process.execPath, [script, "--scenario", "email-gated-onboarding"], { encoding: "utf8" })
  assert.equal(child.status, 2)
  const report = JSON.parse(child.stdout)
  assert.equal(report.status, "required_user_action")
  assert.ok(report.requiredUserActions.includes("explicit_execute_flag"))
})

test("CLI rejects repository evidence output before loading or contacting the kernel", () => {
  const child = spawnSync(process.execPath, [
    script, "--scenario", "email-gated-onboarding", "--task", "email-gated-onboarding=Use the already approved test service.",
    "--confirm-external-actions", "email-gated-onboarding", "--kernel-url", "ws://127.0.0.1:43118",
    "--session", "session-1", "--agent", "agent-1", "--output-dir", repoRoot, "--execute",
  ], { encoding: "utf8" })
  assert.equal(child.status, 2)
  assert.equal(JSON.parse(child.stdout).reason, "new_external_evidence_directory_required")
  assert.equal(child.stderr, "")
})

test("external evidence output requires a new directory and rejects existing and symlink targets", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-office-suite-cli-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const repo = path.join(root, "repo")
  const evidenceParent = path.join(root, "evidence")
  await mkdir(repo)
  await mkdir(evidenceParent)
  const newPath = path.join(evidenceParent, "new-capture")
  const canonicalEvidenceParent = await realpath(evidenceParent)
  assert.equal(await resolveEvidenceDirectory(newPath, repo), path.join(canonicalEvidenceParent, "new-capture"))
  await mkdir(path.join(evidenceParent, "existing"))
  await assert.rejects(resolveEvidenceDirectory(path.join(evidenceParent, "existing"), repo))
  await symlink(path.join(evidenceParent, "target"), path.join(evidenceParent, "alias"), "dir")
  await assert.rejects(resolveEvidenceDirectory(path.join(evidenceParent, "alias"), repo))
})

test("unknown CLI flags fail with fixed text and do not echo supplied data", () => {
  const secretMarker = "NEVER_ECHO_THIS_TASK_TEXT"
  const child = spawnSync(process.execPath, [script, "--unknown", secretMarker], { encoding: "utf8" })
  assert.equal(child.status, 2)
  assert.equal(child.stdout, "")
  assert.equal(child.stderr, "error: invalid options; use --help\n")
  assert.equal(child.stderr.includes(secretMarker), false)
})
