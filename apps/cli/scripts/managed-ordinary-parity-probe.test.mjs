import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { execFile } from "node:child_process"
import { chmod, chown, mkdtemp, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises"
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
const kernelEndpointSource = fileURLToPath(new URL("./lib/managed-ordinary-kernel-endpoint.mjs", import.meta.url))
const PROBE_FIXTURE_PATHS = Object.freeze([
  "apps/cli/scripts/managed-ordinary-parity-probe.mjs",
  "apps/cli/scripts/managed-ordinary-parity-matrix.mjs",
  "apps/cli/scripts/lib/managed-ordinary-provider-turn-binding.mjs",
  "apps/cli/scripts/lib/managed-ordinary-project-setup-observer.mjs",
  "apps/cli/scripts/lib/managed-ordinary-kernel-endpoint.mjs",
  "apps/cli/scripts/lib/managed-ordinary-ancestry-observer.mjs",
  "apps/cli/scripts/lib/managed-ordinary-proc-metadata.mjs",
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
  const observerPath = join(root, PROBE_FIXTURE_PATHS[3])
  await writeFile(observerPath, await readFile(projectSetupObserverSource))
  await writeFile(join(root, PROBE_FIXTURE_PATHS[4]), await readFile(kernelEndpointSource))
  for (const path of PROBE_FIXTURE_PATHS.slice(5)) {
    await writeFile(join(root, path), await readFile(new URL(path.replace("apps/cli/scripts/", "./"), import.meta.url)))
  }
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

test("control-file protection requires its parent to remain writable as a workspace", async (context) => {
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
  await writeFile(join(root, ".git/info/exclude"), "workspace/\n")
  await git(root, ["add", ...PROBE_FIXTURE_PATHS])
  await git(root, ["commit", "--quiet", "-m", "probe fixture"])
  const reviewedCommit = await git(root, ["rev-parse", "HEAD"])
  // MP-03: root bypasses directory permissions, so restrict this fixture's child.
  const restrictedIdentity = process.getuid?.() === 0 ? { uid: 65534, gid: 65534 } : {}
  if (restrictedIdentity.uid !== undefined) {
    async function transferFixtureOwnership(path) {
      for (const entry of await readdir(path, { withFileTypes: true })) {
        const child = join(path, entry.name)
        if (entry.isDirectory()) await transferFixtureOwnership(child)
        else await chown(child, restrictedIdentity.uid, restrictedIdentity.gid)
      }
      await chown(path, restrictedIdentity.uid, restrictedIdentity.gid)
    }
    await transferFixtureOwnership(root)
  }
  await chmod(controlFile, 0o000)
  await chmod(controlParent, 0o555)
  context.after(async () => {
    await chmod(controlParent, 0o755)
    await rm(root, { recursive: true, force: true })
  })
  const probeArguments = [
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
  ]
  const result = await execFileAsync(process.execPath, probeArguments, {
    ...restrictedIdentity,
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
  await chmod(controlParent, 0o755)
  await chmod(controlFile, 0o400)
  const readable = await execFileAsync(process.execPath, probeArguments, {
    ...restrictedIdentity, cwd: root, encoding: "utf8", env: {
      ...process.env, CHARIOX_PARITY_CONTROL_FILE: controlFile, CHARIOX_PARITY_CONTROL_SIBLING: sibling,
    },
  }).catch((error) => error)
  assert.equal(JSON.parse(readable.stdout).ok, false, "MP-03 a readable control file is not protected")
  await chmod(controlFile, 0o000)
  const actual = await execFileAsync(process.execPath, probeArguments, {
    ...restrictedIdentity, cwd: root, encoding: "utf8", env: {
      ...process.env,
      CHARIOX_PARITY_CONTROL_FILE: controlFile,
      CHARIOX_PARITY_CONTROL_SIBLING: sibling,
      CHARIOX_PARITY_CONTROL_PROTECTION_EVIDENCE_JSON: "",
    },
  })
  const protectedResult = JSON.parse(actual.stdout)
  assert.equal(protectedResult.ok, true)
  assert.equal(protectedResult.result.control_file_denied, true)
  assert.equal(protectedResult.result.parent_workspace_accessible, true)
  assert.equal(protectedResult.result.sibling_accessible, true)
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
  const destination = await copyProbeRuntimeSources(root)
  const observerDestination = join(root, "apps/cli/scripts/lib/managed-ordinary-project-setup-observer.mjs")
  await mkdir(dirname(observerDestination), { recursive: true })
  await writeFile(observerDestination, await readFile(projectSetupObserverSource))
  await git(root, ["init", "--quiet"])
  await git(root, ["config", "user.name", "parity-probe-test"])
  await git(root, ["config", "user.email", "parity-probe-test@example.invalid"])
  await git(root, [
    "add",
    ...PROBE_FIXTURE_PATHS,
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

// MP-03: a caller assertion must not override the observed Unix boundary.
test("MP-03 rejects a forged denial for an accessible exact control file", async (t) => {
  const root = await mkdtemp(join(os.tmpdir(), "chariox-parity-control-forged-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const probe = await copyProbeRuntimeSources(root)
  const control = join(root, "control.json")
  const sibling = join(root, "sibling")
  await writeFile(control, "fixture")
  await writeFile(sibling, "fixture")
  await git(root, ["init", "--quiet"])
  await git(root, ["config", "user.name", "parity-probe-test"])
  await git(root, ["config", "user.email", "parity-probe-test@example.invalid"])
  await git(root, ["add", "."])
  await git(root, ["commit", "--quiet", "-m", "MP-03 fixture"])
  const commit = await git(root, ["rev-parse", "HEAD"])
  const output = await execFileAsync(process.execPath, [probe,
    "--source-root", root, "--reviewed-commit", commit,
    "--parity-row", "MP-03", "--parity-check", "control_file_protection",
    "--topology", "ordinary", "--json", "--home-path", "/home", "--tmp-path", "/tmp",
    "--nested-path", "/tmp/chariox-parity-control-nested", "--new-directory", "/tmp/chariox-parity-control-new",
  ], { cwd: root, encoding: "utf8", env: {
    ...process.env, CHARIOX_PARITY_CONTROL_FILE: control, CHARIOX_PARITY_CONTROL_SIBLING: sibling,
    CHARIOX_PARITY_CONTROL_PROTECTION_EVIDENCE_JSON: JSON.stringify({ observed: true, control_file_denied: true }),
  } }).then(result => ({ code: 0, ...result })).catch(error => ({ code: error.code, stdout: error.stdout, stderr: error.stderr }))
  assert.equal(output.code, 1, output.stdout)
  assert.match(JSON.parse(output.stdout).error, /exact control file protection was not observed/)
})

async function probeFixture(t) {
  const root = await mkdtemp(join(os.tmpdir(), "chariox-parity-local-proof-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  await chmod(root, 0o755)
  const destination = await copyProbeRuntimeSources(root)
  await git(root, ["init", "--quiet"])
  await git(root, ["config", "user.name", "parity-probe-test"])
  await git(root, ["config", "user.email", "parity-probe-test@example.invalid"])
  await git(root, ["add", ...PROBE_FIXTURE_PATHS])
  await git(root, ["commit", "--quiet", "-m", "MP-02/MP-04/MP-05 synthetic source fixture"])
  const reviewedCommit = await git(root, ["rev-parse", "HEAD"])
  return async (row, check, values = {}, options = {}) => {
    const argumentsByName = {
      "source-root": root, "reviewed-commit": reviewedCommit, "parity-row": row,
      "parity-check": check, topology: "ordinary", "home-path": "/home", "tmp-path": "/tmp",
      "nested-path": join(root, "nested"), "new-directory": join(root, "new"), ...values,
    }
    const args = [destination, "--json", ...Object.entries(argumentsByName).flatMap(([name, value]) => [`--${name}`, value])]
    const result = await execFileAsync(process.execPath, args, {cwd: root, encoding: "utf8", ...options, env: {...(options.env ?? process.env), GIT_CONFIG_COUNT: "1", GIT_CONFIG_KEY_0: "safe.directory", GIT_CONFIG_VALUE_0: root}}).catch(error => error)
    return JSON.parse(result.stdout)
  }
}

test("MP-02 exact discovery and cwd accept a searchable directory with denied enumeration", async (t) => {
  const probe = await probeFixture(t)
  const root = await mkdtemp(join(os.tmpdir(), "chariox-parity-unlistable-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  await chmod(root, 0o755)
  const workspace = join(root, "workspace")
  await mkdir(workspace, { mode: 0o311 })
  const identity = process.getuid() === 0 ? { uid: 65534, gid: 65534 } : {}
  const discovery = await probe("MP-02", "directory_discovery", {"home-path": workspace}, identity)
  assert.equal(discovery.ok, true, discovery.error)
  assert.equal(discovery.result.child_enumeration_denied, true)
  const entry = await probe("MP-02", "exact_path_entry", {"expected-cwd": workspace}, {...identity, cwd: workspace})
  assert.equal(entry.ok, true, entry.error)
  assert.equal(entry.result.cwd_matches_requested, true)
})

for (const [row, check, selector] of [
  ["MP-02", "directory_creation", "new-directory"],
  ["MP-05", "empty_workspace", "nested-path"],
  ["MP-05", "basename_collision", "new-directory"],
]) {
  test(`${row}/${check} refuses an existing scratch directory without removing its contents`, async (t) => {
    const probe = await probeFixture(t)
    const root = await mkdtemp(join(os.tmpdir(), "chariox-parity-existing-"))
    t.after(() => rm(root, { recursive: true, force: true }))
    const marker = join(root, "existing-user-work")
    await writeFile(marker, "must survive")
    const result = await probe(row, check, {[selector]: root})
    assert.equal(result.ok, false, "existing fixture must not be adopted")
    assert.equal(await readFile(marker, "utf8"), "must survive")
  })
}

test("MP-04 selected provider cwd can differ from the reviewed source checkout", async (t) => {
  const probe = await probeFixture(t)
  const root = await mkdtemp(join(os.tmpdir(), "chariox-parity-home-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const state = join(root, ".chariox")
  await mkdir(state)
  const hash = value => `sha256:${createHash('sha256').update(value).digest('hex')}`
  const result = await probe("MP-04", "provider_environment", {"expected-cwd": root}, {
    cwd: root, env: {...process.env, HOME: root, CHARIOX_HOME: state,
      CHARIOX_PARITY_ORDINARY_ENVIRONMENT_JSON: JSON.stringify({
        home_fingerprint: hash(root), chariox_home_fingerprint: hash(state), cwd_fingerprint: hash(root),
        uid: process.getuid(), gid: process.getgid(),
      }),
    },
  })
  assert.equal(result.ok, true, result.error)
  assert.equal(result.result.cwd_matches_requested, true)
})

test("MP-01 caller ancestry booleans cannot manufacture an official provider descendant", async (t) => {
  const probe = await probeFixture(t)
  const result = await probe("MP-01", "provider_ancestry", {provider: "codex"}, {env: {
    ...process.env, CHARIOX_PARITY_PROVIDER_PROCESS_OBSERVED: "true",
    CHARIOX_PARITY_WORKER_EVIDENCE_JSON: JSON.stringify({observed: true, fresh_worker: true}),
    CHARIOX_PARITY_ANCESTRY_EVIDENCE_JSON: JSON.stringify({
      observed: true, provider_observed: true, bwrap_ancestor: false, fresh_worker: true, ancestry_complete: true,
    }),
  }})
  assert.equal(result.ok, false, "MP-01 forged context must not qualify a shell-only launch")
})

test("MP-04 rejects a HOME or mutable state owned by another user", async (t) => {
  const probe = await probeFixture(t)
  const root = await mkdtemp(join(os.tmpdir(), "chariox-parity-ownership-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  await chmod(root, 0o755)
  const state = join(root, ".chariox")
  await mkdir(state, {mode: 0o755})
  // Root remains the fixture custodian; only the child drops privilege.
  if (process.getuid() !== 0) { t.skip("MP-04 requires a separate file owner"); return }
  const hash = value => `sha256:${createHash('sha256').update(value).digest('hex')}`
  const result = await probe("MP-04", "provider_environment", {"expected-cwd": root}, {
    uid: 65534, gid: 65534, cwd: root, env: {...process.env, HOME: root, CHARIOX_HOME: state,
      CHARIOX_PARITY_ORDINARY_ENVIRONMENT_JSON: JSON.stringify({
        home_fingerprint: hash(root), chariox_home_fingerprint: hash(state), cwd_fingerprint: hash(root), uid: 65534, gid: 65534,
      }),
    },
  })
  assert.equal(result.ok, false)
  assert.match(result.error, /ownership/)
})
