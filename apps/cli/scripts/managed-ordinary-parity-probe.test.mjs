import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { mkdtemp, mkdir, readFile, writeFile } from "node:fs/promises"
import os from "node:os"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"
import { test } from "node:test"

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

test("repo-owned probe executes its real command path and binds its file to the reviewed commit", async () => {
  const root = await mkdtemp(join(os.tmpdir(), "chariox-managed-ordinary-parity-probe-"))
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
