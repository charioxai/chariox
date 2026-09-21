import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"
import { test } from "node:test"
import { normalizeMountInfo } from "./managed-ordinary-parity-probe.mjs"

const execFileAsync = promisify(execFile)
const probeSource = fileURLToPath(new URL("./managed-ordinary-parity-probe.mjs", import.meta.url))

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
  const destination = join(root, "apps/cli/scripts/managed-ordinary-parity-probe.mjs")
  await mkdir(dirname(destination), { recursive: true })
  await writeFile(destination, await readFile(probeSource))
  await git(root, ["init", "--quiet"])
  await git(root, ["config", "user.name", "parity-probe-test"])
  await git(root, ["config", "user.email", "parity-probe-test@example.invalid"])
  await git(root, ["add", "apps/cli/scripts/managed-ordinary-parity-probe.mjs"])
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

test("probe fails closed when a required product observation is absent", async (context) => {
  const root = await mkdtemp(join(os.tmpdir(), "chariox-managed-ordinary-parity-missing-observation-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const destination = join(root, "apps/cli/scripts/managed-ordinary-parity-probe.mjs")
  await mkdir(dirname(destination), { recursive: true })
  await writeFile(destination, await readFile(probeSource))
  await git(root, ["init", "--quiet"])
  await git(root, ["config", "user.name", "parity-probe-test"])
  await git(root, ["config", "user.email", "parity-probe-test@example.invalid"])
  await git(root, ["add", "apps/cli/scripts/managed-ordinary-parity-probe.mjs"])
  await git(root, ["commit", "--quiet", "-m", "probe fixture"])
  const reviewedCommit = await git(root, ["rev-parse", "HEAD"])
  const result = await execFileAsync(process.execPath, [
    destination,
    "--source-root", root,
    "--reviewed-commit", reviewedCommit,
    "--parity-row", "MP-05",
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

test("mount normalization preserves comparable topology facts", () => {
  const first = normalizeMountInfo("36 25 0:32 / /rw rw,relatime - overlay overlay rw\n")
  const second = normalizeMountInfo("37 25 0:32 / /rw rw,relatime - overlay overlay rw,lowerdir=/different\n")
  assert.equal(first.length, 1)
  assert.equal(first[0].mount_point, "/rw")
  assert.notDeepEqual(first, second)
})
