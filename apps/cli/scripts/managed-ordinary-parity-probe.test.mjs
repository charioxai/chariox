import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { chmod, mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"
import { test } from "node:test"
import { normalizeMountInfo } from "./managed-ordinary-parity-probe.mjs"

const execFileAsync = promisify(execFile)
const probeSource = fileURLToPath(new URL("./managed-ordinary-parity-probe.mjs", import.meta.url))
const matrixSource = fileURLToPath(new URL("./managed-ordinary-parity-matrix.mjs", import.meta.url))
const projectSetupObserverSource = fileURLToPath(new URL("./lib/managed-ordinary-project-setup-observer.mjs", import.meta.url))
const providerTurnBindingSource = fileURLToPath(new URL("./lib/managed-ordinary-provider-turn-binding.mjs", import.meta.url))
const PROBE_FIXTURE_PATHS = Object.freeze([
  "apps/cli/scripts/managed-ordinary-parity-probe.mjs",
  "apps/cli/scripts/managed-ordinary-parity-matrix.mjs",
  "apps/cli/scripts/lib/managed-ordinary-provider-turn-binding.mjs",
])

async function copyProbeRuntimeSources(root) {
  const [probePath, matrixPath, bindingPath] = PROBE_FIXTURE_PATHS.map((path) => join(root, ...path.split("/")))
  await Promise.all([
    mkdir(dirname(probePath), { recursive: true }),
    mkdir(dirname(bindingPath), { recursive: true }),
  ])
  await Promise.all([
    writeFile(probePath, await readFile(probeSource)),
    writeFile(matrixPath, await readFile(matrixSource)),
    writeFile(bindingPath, await readFile(providerTurnBindingSource)),
  ])
  return probePath
}

async function git(root, args) {
  const result = await execFileAsync("git", args, {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null", LC_ALL: "C" },
  })
  return String(result.stdout).trim()
}

test("repo-owned probe executes its real command path and binds its file to the reviewed commit", async (context) => {
  const root = await mkdtemp(join(os.tmpdir(), "chariox-managed-ordinary-parity-probe-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const destination = await copyProbeRuntimeSources(root)
  await git(root, ["init", "--quiet"])
  await git(root, ["config", "user.name", "parity-probe-test"])
  await git(root, ["config", "user.email", "parity-probe-test@example.invalid"])
  await git(root, ["add", ...PROBE_FIXTURE_PATHS])
  await git(root, ["commit", "--quiet", "-m", "probe fixture"])
  const reviewedCommit = await git(root, ["rev-parse", "HEAD"])
  const output = await execFileAsync(process.execPath, [
    destination,
    "--source-root", root,
    "--reviewed-commit", reviewedCommit,
    "--parity-row", "MP-02",
    "--parity-check", "home_access",
    "--topology", "ordinary",
    "--json",
    "--home-path", root,
    "--tmp-path", os.tmpdir(),
    "--nested-path", join(os.tmpdir(), `chariox-parity-probe-nested-${process.pid}`),
    "--new-directory", join(os.tmpdir(), `chariox-parity-probe-created-${process.pid}`),
  ], { cwd: root, encoding: "utf8" })
  const payload = JSON.parse(output.stdout)
  assert.equal(payload.ok, true)
  assert.equal(payload.result.observed, true)
  assert.equal(payload.result.accessible, true)
  assert.equal(payload.result.probe_identity_verified, true)
  assert.equal(payload.result.probe_source_commit, reviewedCommit)
  assert.equal(payload.result.probe_file, "apps/cli/scripts/managed-ordinary-parity-probe.mjs")
  assert.match(payload.result.probe_file_git_blob, /^[0-9a-f]{40}$/)
  assert.match(payload.result.probe_file_sha256, /^sha256:[0-9a-f]{64}$/)
})

test("exact-path probe honors a workspace cwd distinct from the reviewed source checkout", async (context) => {
  const root = await mkdtemp(join(os.tmpdir(), "chariox-managed-ordinary-parity-exact-cwd-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const workspace = join(root, "workspace-created-in-fixture")
  const destination = await copyProbeRuntimeSources(root)
  await mkdir(workspace, { recursive: true })
  await git(root, ["init", "--quiet"])
  await git(root, ["config", "user.name", "parity-probe-test"])
  await git(root, ["config", "user.email", "parity-probe-test@example.invalid"])
  await git(root, ["add", ...PROBE_FIXTURE_PATHS])
  await git(root, ["commit", "--quiet", "-m", "probe fixture"])
  const reviewedCommit = await git(root, ["rev-parse", "HEAD"])
  const output = await execFileAsync(process.execPath, [
    destination,
    "--source-root", root,
    "--reviewed-commit", reviewedCommit,
    "--parity-row", "MP-02",
    "--parity-check", "exact_path_entry",
    "--topology", "ordinary",
    "--expected-cwd", workspace,
    "--json",
    "--home-path", "/home",
    "--tmp-path", "/tmp",
    "--nested-path", join(os.tmpdir(), `chariox-parity-nested-${process.pid}`),
    "--new-directory", join(os.tmpdir(), `chariox-parity-created-${process.pid}`),
  ], { cwd: workspace, encoding: "utf8" })
  const payload = JSON.parse(output.stdout)
  assert.equal(payload.ok, true)
  assert.equal(payload.result.exact_path_accessible, true)
  assert.equal(payload.result.cwd_matches_requested, true)
  assert.equal(payload.result.cwd_fingerprint, payload.result.requested_cwd_fingerprint)
})

test("control-file protection requires its parent to remain writable as a workspace", {
  skip: process.getuid?.() === 0,
}, async (context) => {
  const root = await mkdtemp(join(os.tmpdir(), "chariox-managed-ordinary-parity-control-parent-"))
  const controlParent = join(root, "workspace")
  const destination = await copyProbeRuntimeSources(root)
  await mkdir(controlParent, { recursive: true })
  const controlFile = join(controlParent, "managed-control.json")
  const sibling = join(controlParent, "sibling-workspace-file")
  await writeFile(controlFile, "control fixture\n", { mode: 0o600 })
  await writeFile(sibling, "sibling fixture\n", { mode: 0o600 })
  await git(root, ["init", "--quiet"])
  await git(root, ["config", "user.name", "parity-probe-test"])
  await git(root, ["config", "user.email", "parity-probe-test@example.invalid"])
  await git(root, [
    "add",
    ...PROBE_FIXTURE_PATHS,
    "workspace/managed-control.json",
    "workspace/sibling-workspace-file",
  ])
  await git(root, ["commit", "--quiet", "-m", "probe fixture"])
  const reviewedCommit = await git(root, ["rev-parse", "HEAD"])
  await chmod(controlParent, 0o555)
  context.after(async () => {
    await chmod(controlParent, 0o755)
    await rm(root, { recursive: true, force: true })
  })
  const result = await execFileAsync(process.execPath, [
    destination,
    "--source-root", root,
    "--reviewed-commit", reviewedCommit,
    "--parity-row", "MP-03",
    "--parity-check", "control_file_protection",
    "--topology", "ordinary",
    "--json",
    "--home-path", "/home",
    "--tmp-path", "/tmp",
    "--nested-path", join(os.tmpdir(), `chariox-parity-nested-${process.pid}`),
    "--new-directory", join(os.tmpdir(), `chariox-parity-created-${process.pid}`),
  ], {
    cwd: root,
    encoding: "utf8",
    env: {
      ...process.env,
      CHARIOX_PARITY_CONTROL_FILE: controlFile,
      CHARIOX_PARITY_CONTROL_SIBLING: sibling,
      CHARIOX_PARITY_CONTROL_PROTECTION_EVIDENCE_JSON: JSON.stringify({
        observed: true,
        control_file_denied: true,
        evidence_id: "fixture-control-protection",
      }),
    },
  }).then(({ stdout, stderr }) => ({ code: 0, stdout, stderr })).catch((error) => ({
    code: error.code,
    stdout: String(error.stdout ?? ""),
    stderr: String(error.stderr ?? error.message ?? ""),
  }))
  assert.equal(result.code, 1, result.stdout || result.stderr)
  const payload = JSON.parse(result.stdout)
  assert.equal(payload.ok, false)
  assert.match(payload.error, /parent workspace/)
})

test("control-file protection rejects an accessible file outside the control workspace", async (context) => {
  const root = await mkdtemp(join(os.tmpdir(), "chariox-managed-ordinary-parity-control-sibling-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const destination = await copyProbeRuntimeSources(root)
  const controlFile = join(root, "workspace/managed-control.json")
  const unrelatedFile = join(root, "elsewhere/unrelated-file")
  await mkdir(dirname(controlFile), { recursive: true })
  await mkdir(dirname(unrelatedFile), { recursive: true })
  await writeFile(controlFile, "control fixture\n")
  await writeFile(unrelatedFile, "unrelated fixture\n")
  await git(root, ["init", "--quiet"])
  await git(root, ["config", "user.name", "parity-probe-test"])
  await git(root, ["config", "user.email", "parity-probe-test@example.invalid"])
  await git(root, ["add", ...PROBE_FIXTURE_PATHS, "workspace/managed-control.json", "elsewhere/unrelated-file"])
  await git(root, ["commit", "--quiet", "-m", "probe fixture"])
  const reviewedCommit = await git(root, ["rev-parse", "HEAD"])
  const result = await execFileAsync(process.execPath, [
    destination,
    "--source-root", root,
    "--reviewed-commit", reviewedCommit,
    "--parity-row", "MP-03",
    "--parity-check", "control_file_protection",
    "--topology", "ordinary",
    "--json",
    "--home-path", "/home",
    "--tmp-path", "/tmp",
    "--nested-path", join(os.tmpdir(), `chariox-parity-nested-${process.pid}`),
    "--new-directory", join(os.tmpdir(), `chariox-parity-created-${process.pid}`),
  ], {
    cwd: root,
    encoding: "utf8",
    env: {
      ...process.env,
      CHARIOX_PARITY_CONTROL_FILE: controlFile,
      CHARIOX_PARITY_CONTROL_SIBLING: unrelatedFile,
      CHARIOX_PARITY_CONTROL_PROTECTION_EVIDENCE_JSON: JSON.stringify({
        observed: true,
        control_file_denied: true,
        evidence_id: "fixture-control-protection",
      }),
    },
  }).then(({ stdout, stderr }) => ({ code: 0, stdout, stderr })).catch((error) => ({
    code: error.code,
    stdout: String(error.stdout ?? ""),
    stderr: String(error.stderr ?? error.message ?? ""),
  }))
  assert.equal(result.code, 1, result.stdout || result.stderr)
  const payload = JSON.parse(result.stdout)
  assert.equal(payload.ok, false)
  assert.match(payload.error, /same parent/)
})

test("probe fails closed when a required product observation is absent", async (context) => {
  const root = await mkdtemp(join(os.tmpdir(), "chariox-managed-ordinary-parity-missing-observation-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const destination = await copyProbeRuntimeSources(root)
  await git(root, ["init", "--quiet"])
  await git(root, ["config", "user.name", "parity-probe-test"])
  await git(root, ["config", "user.email", "parity-probe-test@example.invalid"])
  await git(root, ["add", ...PROBE_FIXTURE_PATHS])
  await git(root, ["commit", "--quiet", "-m", "probe fixture"])
  const reviewedCommit = await git(root, ["rev-parse", "HEAD"])
  const result = await execFileAsync(process.execPath, [
    destination,
    "--source-root", root,
    "--reviewed-commit", reviewedCommit,
    "--parity-row", "MP-08",
    "--parity-check", "session_agent_launch",
    "--topology", "ordinary",
    "--json",
    "--home-path", root,
    "--tmp-path", os.tmpdir(),
    "--nested-path", join(os.tmpdir(), `chariox-parity-probe-missing-nested-${process.pid}`),
    "--new-directory", join(os.tmpdir(), `chariox-parity-probe-missing-created-${process.pid}`),
  ], { cwd: root, encoding: "utf8", env: { ...process.env, CHARIOX_PARITY_SESSION_AGENT_EVIDENCE_JSON: "" } }).then(({ stdout, stderr }) => ({ code: 0, stdout, stderr })).catch((error) => ({
    code: error.code,
    stdout: String(error.stdout ?? ""),
    stderr: String(error.stderr ?? error.message ?? ""),
  }))
  assert.equal(result.code, 1, result.stderr)
  const payload = JSON.parse(result.stdout)
  assert.equal(payload.ok, false)
  assert.match(payload.error, /missing observation context|session[ /]agent product observation is missing|session agent/i)
})

test("MP-08 Project setup external assertions cannot satisfy the product observer", async (context) => {
  const root = await mkdtemp(join(os.tmpdir(), "chariox-managed-ordinary-project-setup-assertion-only-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const destination = join(root, "apps/cli/scripts/managed-ordinary-parity-probe.mjs")
  const matrixDestination = join(root, "apps/cli/scripts/managed-ordinary-parity-matrix.mjs")
  const observerDestination = join(root, "apps/cli/scripts/lib/managed-ordinary-project-setup-observer.mjs")
  await mkdir(dirname(destination), { recursive: true })
  await mkdir(dirname(observerDestination), { recursive: true })
  await writeFile(destination, await readFile(probeSource))
  await writeFile(matrixDestination, await readFile(matrixSource))
  await writeFile(observerDestination, await readFile(projectSetupObserverSource))
  await git(root, ["init", "--quiet"])
  await git(root, ["config", "user.name", "parity-probe-test"])
  await git(root, ["config", "user.email", "parity-probe-test@example.invalid"])
  await git(root, [
    "add",
    "apps/cli/scripts/managed-ordinary-parity-probe.mjs",
    "apps/cli/scripts/managed-ordinary-parity-matrix.mjs",
    "apps/cli/scripts/lib/managed-ordinary-project-setup-observer.mjs",
  ])
  await git(root, ["commit", "--quiet", "-m", "probe fixture"])
  const reviewedCommit = await git(root, ["rev-parse", "HEAD"])
  const result = await execFileAsync(process.execPath, [
    destination,
    "--source-root", root,
    "--reviewed-commit", reviewedCommit,
    "--parity-row", "MP-08",
    "--parity-check", "project_setup",
    "--topology", "ordinary",
    "--json",
    "--home-path", root,
    "--tmp-path", os.tmpdir(),
    "--nested-path", join(os.tmpdir(), `chariox-parity-project-setup-nested-${process.pid}`),
    "--new-directory", join(os.tmpdir(), `chariox-parity-project-setup-created-${process.pid}`),
  ], {
    cwd: root,
    encoding: "utf8",
    env: {
      ...process.env,
      CHARIOX_PARITY_PROJECT_SETUP_EVIDENCE_JSON: JSON.stringify({
        observed: true,
        project_setup_ok: true,
        project_identity: "externally-asserted-project",
      }),
    },
  }).then(({ stdout, stderr }) => ({ code: 0, stdout, stderr })).catch((error) => ({
    code: error.code,
    stdout: String(error.stdout ?? ""),
    stderr: String(error.stderr ?? error.message ?? ""),
  }))
  assert.equal(result.code, 1, result.stdout || result.stderr)
  const payload = JSON.parse(result.stdout)
  assert.equal(payload.ok, false)
  assert.match(payload.error, /selection/)
})

test("mount normalization preserves comparable topology facts", () => {
  const first = normalizeMountInfo("36 25 0:32 / /rw rw,relatime - overlay overlay rw\n")
  const second = normalizeMountInfo("37 25 0:32 / /rw rw,relatime - overlay overlay rw,lowerdir=/different\n")
  assert.equal(first.length, 1)
  assert.equal(first[0].mount_point, "/rw")
  assert.notDeepEqual(first, second)
})
