import assert from "node:assert/strict"
import { createHash, generateKeyPairSync, sign } from "node:crypto"
import { spawnSync } from "node:child_process"
import {
  chmod,
  chown,
  cp,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readlink,
  readdir,
  rename,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import { dirname, join, relative, sep } from "node:path"
import { fileURLToPath } from "node:url"
import { test } from "node:test"
import { allocationWorkerBindingDigest } from "../deploy/managed-kernel/allocation-worker-receipt.mjs"

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const upgrade = join(repositoryRoot, "deploy/managed-kernel/upgrade-image.sh")
const upgradeState = join(repositoryRoot, "deploy/managed-kernel/managed-kernel-upgrade-state.mjs")
const verifyRelease = join(repositoryRoot, "deploy/managed-kernel/verify-image-release.mjs")
const managedService = join(repositoryRoot, "deploy/managed-kernel/chariox-managed-bootstrap.service")
const serviceName = "chariox-managed-bootstrap.service"

test("managed kernel upgrade requires an explicit valid provider topology before reading the image", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-upgrade-topology-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const bin = join(root, "bin")
  await put(join(bin, "id"), "#!/bin/sh\nprintf '0\\n'\n", 0o755)
  const env = {
    ...process.env,
    PATH: `${bin}:${process.env.PATH}`,
    CHARIOX_MANAGED_UPGRADE_ROOT: join(root, "host"),
  }
  const args = ["missing-image", "wrong-current", "wrong-next", "missing-key"]
  for (const topology of [undefined, "", "unexpected"]) {
    const candidateEnv = { ...env }
    if (topology === undefined) delete candidateEnv.CHARIOX_MANAGED_PROVIDER_TOPOLOGY
    else candidateEnv.CHARIOX_MANAGED_PROVIDER_TOPOLOGY = topology
    const result = spawnSync(upgrade, args, { encoding: "utf8", env: candidateEnv })
    assert.equal(result.status, 1)
    assert.match(result.stderr, topology === "unexpected"
      ? /CHARIOX_MANAGED_PROVIDER_TOPOLOGY must be path1 or shared_host/
      : /CHARIOX_MANAGED_PROVIDER_TOPOLOGY must be explicitly set to path1 or shared_host/)
    assert.doesNotMatch(result.stderr, /release digest|image root/)
  }
  for (const topology of ["path1", "shared_host"]) {
    const result = spawnSync(upgrade, args, {
      encoding: "utf8",
      env: { ...env, CHARIOX_MANAGED_PROVIDER_TOPOLOGY: topology },
    })
    assert.equal(result.status, 1)
    assert.match(result.stderr, /managed release digest is invalid/)
  }
})

test("Path-1 upgrade drop-in guard checks reload freshness and both effective services", async (context) => {
  const source = await readFile(upgrade, "utf8")
  const guard = source.match(/assert_path1_units_have_no_dropins\(\) \{\n[\s\S]*?^\}/m)?.[0]
  assert.ok(guard)
  assert.match(guard, /systemctl show --property=NeedDaemonReload --value "\$unit"/)
  assert.match(guard, /systemctl show --property=DropInPaths --value "\$unit"/)
  assert.ok(guard.indexOf("--property=NeedDaemonReload") < guard.indexOf("--property=DropInPaths"))
  assert.match(source, /select_supervisor_service\nassert_path1_units_have_no_dropins\nrecover_transaction/)
  assert.match(source, /systemctl daemon-reload \|\| return 1\n  assert_path1_units_have_no_dropins \|\| return 1\n  health_not_before_ms=/)
  assert.match(source, /if ! systemctl daemon-reload \\\n  \|\| ! assert_path1_units_have_no_dropins \\\n/)

  const scratch = await mkdtemp(join(tmpdir(), "chariox-upgrade-dropin-"))
  context.after(() => rm(scratch, { recursive: true, force: true }))
  const bin = join(scratch, "bin")
  await put(join(bin, "systemctl"), `#!/bin/sh
case "$*" in
  *--property=NeedDaemonReload*chariox-path1-managed-bootstrap.service) printf '%s' "\${SYSTEMD_HOME_NEED_DAEMON_RELOAD:-no}" ;;
  *--property=NeedDaemonReload*chariox-disposable-worker-bootstrap.service) printf '%s' "\${SYSTEMD_WORKER_NEED_DAEMON_RELOAD:-no}" ;;
  *--property=DropInPaths*chariox-path1-managed-bootstrap.service) printf '%s' "\${SYSTEMD_HOME_DROP_IN_PATHS:-}" ;;
  *--property=DropInPaths*chariox-disposable-worker-bootstrap.service) printf '%s' "\${SYSTEMD_WORKER_DROP_IN_PATHS:-}" ;;
esac
`, 0o755)
  const command = `managed_provider_topology=path1\n${guard}\nassert_path1_units_have_no_dropins\n`
  const env = { ...process.env, PATH: `${bin}:${process.env.PATH}` }
  const clean = spawnSync("/bin/sh", ["-c", command], { encoding: "utf8", env })
  assert.equal(clean.status, 0, clean.stderr)
  for (const [name, unit] of [
    ["SYSTEMD_HOME_DROP_IN_PATHS", "chariox-path1-managed-bootstrap.service"],
    ["SYSTEMD_WORKER_DROP_IN_PATHS", "chariox-disposable-worker-bootstrap.service"],
  ]) {
    const inherited = spawnSync("/bin/sh", ["-c", command], {
      encoding: "utf8",
      env: { ...env, [name]: "/etc/systemd/system/50-hardening.conf" },
    })
    assert.equal(inherited.status, 1)
    assert.match(inherited.stderr, new RegExp(`Path-1 service ${unit.replaceAll(".", "\\.")} has systemd drop-ins`))
  }
  for (const [name, unit] of [
    ["SYSTEMD_HOME_NEED_DAEMON_RELOAD", "chariox-path1-managed-bootstrap.service"],
    ["SYSTEMD_WORKER_NEED_DAEMON_RELOAD", "chariox-disposable-worker-bootstrap.service"],
  ]) {
    const stale = spawnSync("/bin/sh", ["-c", command], {
      encoding: "utf8",
      env: { ...env, [name]: "yes" },
    })
    assert.equal(stale.status, 1)
    assert.match(stale.stderr, new RegExp(`Path-1 service ${unit.replaceAll(".", "\\.")} needs systemd daemon-reload`))
  }
})

test("repository release policy permits deployed protocol 325 and intermediates 326 through 332 to 333 upgrade and rollback", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-release-policy-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const current = join(root, "current")
  const target = join(root, "target")
  const policyPath = "usr/lib/chariox/slice-build-context/apps/kernel/managed-upgrade-protocol-transitions.json"
  await mkdir(current)
  await put(join(target, policyPath), await readFile(join(repositoryRoot, "apps/kernel/managed-upgrade-protocol-transitions.json")))
  for (const args of [
    [current, "325", target, "333"],
    [target, "333", current, "325"],
    [current, "326", target, "333"],
    [target, "333", current, "326"],
    [current, "327", target, "333"],
    [target, "333", current, "327"],
    [current, "328", target, "333"],
    [target, "333", current, "328"],
    [current, "329", target, "333"],
    [target, "333", current, "329"],
    [current, "330", target, "333"],
    [target, "333", current, "330"],
    [current, "331", target, "333"],
    [target, "333", current, "331"],
    [current, "332", target, "333"],
    [target, "333", current, "332"],
  ]) {
    const result = spawnSync(process.execPath, [upgradeState, "validate-protocol-transition", ...args], { encoding: "utf8" })
    assert.equal(result.status, 0, result.stderr)
  }
})

function rawPublicKey(publicKey) {
  const der = publicKey.export({ format: "der", type: "spki" })
  return der.subarray(der.length - 32)
}

async function sha256File(path) {
  return `sha256:${createHash("sha256").update(await readFile(path)).digest("hex")}`
}

async function updateTreeHash(root, current, hash) {
  for (const entry of await readdir(current, { withFileTypes: true }).then((entries) =>
    entries.sort((left, right) => left.name.localeCompare(right.name)),
  )) {
    const path = join(current, entry.name)
    const pathFromRoot = relative(root, path).split(sep).join("/")
    const metadata = await lstat(path)
    const mode = metadata.mode & 0o7777
    if (entry.isDirectory()) {
      hash.update(`directory:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:`)
      await updateTreeHash(root, path, hash)
    } else {
      hash.update(`file:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:${metadata.size}:`)
      hash.update(await readFile(path))
    }
  }
}

async function sha256Tree(root) {
  const hash = createHash("sha256")
  const metadata = await lstat(root)
  hash.update(`directory:1:.:${metadata.mode & 0o7777}:`)
  await updateTreeHash(root, root, hash)
  return `sha256:${hash.digest("hex")}`
}

async function put(path, contents, mode = 0o644) {
  await mkdir(dirname(path), { recursive: true, mode: 0o755 })
  await writeFile(path, contents, { mode })
  await chmod(path, mode)
}

async function createRootPrivateDirectory(path) {
  await mkdir(path, { recursive: true, mode: 0o700 })
  if (process.getuid?.() === 0) await chown(path, 0, 0)
  await chmod(path, 0o700)
}

function namespaceMapsId(contents, id) {
  return contents.trim().split("\n").some((line) => {
    const [inside, , length] = line.trim().split(/\s+/).map(Number)
    return Number.isSafeInteger(inside) && Number.isSafeInteger(length)
      && id >= inside && id < inside + length
  })
}

async function effectiveCharioxIdentity() {
  const uid = Number(spawnSync("id", ["-u", "chariox"], { encoding: "utf8" }).stdout.trim())
  const gid = Number(spawnSync("id", ["-g", "chariox"], { encoding: "utf8" }).stdout.trim())
  if (process.platform !== "linux") return { uid, gid }
  const [uidMap, gidMap] = await Promise.all([
    readFile("/proc/self/uid_map", "utf8").catch(() => ""),
    readFile("/proc/self/gid_map", "utf8").catch(() => ""),
  ])
  if (namespaceMapsId(uidMap, uid) && namespaceMapsId(gidMap, gid)) return { uid, gid }
  return { uid: 0, gid: 0 }
}

async function makeRelease(root, label, protocol, privateKey, publicKey, transitionPolicy = null, includeWorkerService = false, builderKeys = null) {
  const rootfs = join(root, `image-${label}`)
  const kernel = join(rootfs, "usr/local/bin/chariox-kernel")
  const supervisor = join(rootfs, "usr/local/bin/chariox-managed-bootstrap")
  const managedService = join(rootfs, `etc/systemd/system/${serviceName}`)
  const rootlessService = join(rootfs, "etc/systemd/system/chariox-rootless-docker.service")
  const brokerService = join(rootfs, "etc/systemd/system/chariox-slice-broker.service")
  const context = join(rootfs, "usr/lib/chariox/slice-build-context")
  const attestation = join(rootfs, "usr/lib/chariox/build-attestation.json")
  const attestationSignature = join(rootfs, "usr/lib/chariox/build-attestation.sig")
  const builderKey = join(rootfs, "usr/lib/chariox/builder-public-key")
  await put(kernel, `#!/bin/sh\nif [ "\$1" = "--print-local-daemon-protocol-version" ]; then echo ${protocol}; exit 0; fi\nexit 1\n`, 0o755)
  await put(supervisor, `#!/bin/sh\necho supervisor-${label}\n`, 0o755)
  await put(managedService, `[Service]\nExecStart=/usr/local/bin/chariox-managed-bootstrap\n# ${label}\n`)
  await put(rootlessService, `[Service]\n# rootless ${label}\n`)
  await put(brokerService, `[Service]\n# broker ${label}\n`)
  await put(join(context, "apps/kernel/slice-linux-docker/runtime.txt"), `${label}\n`)
  if (transitionPolicy) {
    await put(
      join(context, "apps/kernel/managed-upgrade-protocol-transitions.json"),
      `${JSON.stringify(transitionPolicy)}\n`,
    )
  }
  const sourceCommit = createHash("sha1").update(`commit-${label}`).digest("hex")
  const sourceTree = createHash("sha1").update(`tree-${label}`).digest("hex")
  if (builderKeys) {
    const relay = join(context, "apps/kernel/slice-linux-docker/prebuilt/chariox-relay")
    await put(relay, `#!/bin/sh\necho relay-${label}\n`, 0o755)
    const attestationBytes = Buffer.from(JSON.stringify({
      schemaVersion: 1,
      sourceCommit,
      sourceTree,
      target: "x86_64-unknown-linux-gnu",
      artifacts: [
        { name: "chariox-kernel", sha256: await sha256File(kernel) },
        { name: "chariox-managed-bootstrap", sha256: await sha256File(supervisor) },
        { name: "chariox-relay", sha256: await sha256File(relay) },
      ],
    }))
    await put(attestation, attestationBytes)
    await put(attestationSignature, sign(null, attestationBytes, builderKeys.privateKey).toString("base64"))
    await put(builderKey, rawPublicKey(builderKeys.publicKey).toString("base64"))
  } else {
    await put(attestation, JSON.stringify({ schemaVersion: 1, label }))
    await put(attestationSignature, Buffer.alloc(64, label.charCodeAt(0)).toString("base64"))
    await put(builderKey, Buffer.alloc(32, protocol % 255).toString("base64"))
  }

  const artifactSpecs = [
    ["chariox-kernel", "/usr/local/bin/chariox-kernel", kernel, "file"],
    ["chariox-managed-bootstrap", "/usr/local/bin/chariox-managed-bootstrap", supervisor, "file"],
    ["chariox-managed-bootstrap.service", `/etc/systemd/system/${serviceName}`, managedService, "file"],
    ["chariox-rootless-docker.service", "/etc/systemd/system/chariox-rootless-docker.service", rootlessService, "file"],
    ["chariox-slice-broker.service", "/etc/systemd/system/chariox-slice-broker.service", brokerService, "file"],
    ["chariox-slice-build-context", "/usr/lib/chariox/slice-build-context", context, "tree"],
    ["chariox-build-attestation", "/usr/lib/chariox/build-attestation.json", attestation, "file"],
    ["chariox-build-attestation-signature", "/usr/lib/chariox/build-attestation.sig", attestationSignature, "file"],
    ["chariox-builder-public-key", "/usr/lib/chariox/builder-public-key", builderKey, "file"],
  ]
  const artifacts = []
  if (includeWorkerService) {
    const path = "/etc/systemd/system/chariox-disposable-worker-bootstrap.service"
    const source = join(rootfs, path)
    await put(source, await readFile(join(repositoryRoot, "deploy/managed-kernel/chariox-disposable-worker-bootstrap.service")))
    artifactSpecs.push(["chariox-disposable-worker-bootstrap.service", path, source, "file"])
  }
  if (builderKeys) {
    const path = "/etc/systemd/system/chariox-path1-managed-bootstrap.service"
    const source = join(rootfs, path)
    await put(source, await readFile(join(repositoryRoot, "deploy/managed-kernel/chariox-path1-managed-bootstrap.service")))
    artifactSpecs.push(["chariox-path1-managed-bootstrap.service", path, source, "file"])
  }
  for (const [name, path, source, type] of artifactSpecs) {
    artifacts.push({ name, path, sha256: type === "tree" ? await sha256Tree(source) : await sha256File(source) })
  }
  const manifestBytes = Buffer.from(JSON.stringify({
    schemaVersion: 2,
    sourceCommit,
    sourceTree,
    artifacts,
  }))
  await put(join(rootfs, "usr/lib/chariox/release-manifest.json"), manifestBytes)
  await put(
    join(rootfs, "usr/lib/chariox/release-manifest.sig"),
    sign(null, manifestBytes, privateKey).toString("base64"),
  )
  await put(
    join(rootfs, "usr/lib/chariox/release-public-key"),
    rawPublicKey(publicKey).toString("base64"),
  )
  return {
    rootfs,
    digest: `sha256:${createHash("sha256").update(manifestBytes).digest("hex")}`,
  }
}

test("Path-1 upgrade fixture has a verified builder attestation and both supervisor units", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-path1-upgrade-fixture-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const releaseKeys = generateKeyPairSync("ed25519")
  const builderKeys = generateKeyPairSync("ed25519")
  const releaseKey = join(root, "release-public-key")
  const builderKey = join(root, "builder-public-key")
  await put(releaseKey, rawPublicKey(releaseKeys.publicKey).toString("base64"))
  await put(builderKey, rawPublicKey(builderKeys.publicKey).toString("base64"))
  const release = await makeRelease(
    root, "path1", 325, releaseKeys.privateKey, releaseKeys.publicKey, null, true, builderKeys,
  )
  const result = spawnSync(process.execPath, [
    verifyRelease, release.rootfs, release.digest, releaseKey, "path1", builderKey,
  ], { encoding: "utf8" })
  assert.equal(result.status, 0, result.stderr)
})

async function makeHarness(context, {
  currentProtocol = 325,
  targetProtocol = 325,
  currentTransitionPolicy = null,
  targetTransitionPolicy = null,
  receiptKind = "managed_environment",
  workerCapableCurrent = false,
  path1Release = false,
} = {}) {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-upgrade-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const { privateKey, publicKey } = generateKeyPairSync("ed25519")
  const trustedKey = join(root, "trusted-release-public-key")
  await put(trustedKey, rawPublicKey(publicKey).toString("base64"), 0o600)
  const builderKeys = path1Release ? generateKeyPairSync("ed25519") : null
  const trustedBuilderKey = join(root, "trusted-builder-public-key")
  if (builderKeys) await put(trustedBuilderKey, rawPublicKey(builderKeys.publicKey).toString("base64"), 0o600)
  const current = await makeRelease(
    root, "current", currentProtocol, privateKey, publicKey, currentTransitionPolicy,
    path1Release || workerCapableCurrent || receiptKind === "allocation_worker", builderKeys,
  )
  const target = await makeRelease(
    root, "target", targetProtocol, privateKey, publicKey, targetTransitionPolicy, true, builderKeys,
  )
  const installRoot = join(root, "host")
  if (path1Release) await mkdir(join(installRoot, "etc"), { recursive: true, mode: 0o755 })
  const releases = join(installRoot, "usr/lib/chariox/releases")
  const currentRelease = join(releases, current.digest.slice("sha256:".length))
  await mkdir(releases, { recursive: true })
  await chmod(installRoot, 0o755)
  await chmod(join(installRoot, "usr"), 0o755)
  await chmod(join(installRoot, "usr/lib"), 0o755)
  await chmod(dirname(releases), 0o755)
  await chmod(releases, 0o755)
  await cp(current.rootfs, currentRelease, { recursive: true, preserveTimestamps: true })
  await symlink(`releases/${current.digest.slice("sha256:".length)}`, join(installRoot, "usr/lib/chariox/current"))
  await symlink("current/usr/lib/chariox/slice-build-context", join(installRoot, "usr/lib/chariox/slice-build-context"))
  const receiptPath = join(installRoot, receiptKind === "allocation_worker"
    ? "var/lib/chariox/disposable-worker/bootstrap-receipt.json"
    : "var/lib/chariox/managed/bootstrap-receipt.json")
  const binding = {
    allocationId: "allocation-1",
    expectedHomeKernelId: "home-kernel-1",
    userId: "owner-1",
    realmId: "realm-1",
    workerMachineId: "machine-1",
    workerKernelId: "kernel-1",
    imageDigest: `sha256:${"a".repeat(64)}`,
    runtimeReleaseDigest: current.digest,
    managerOperationId: "operation-1",
    managerOperationFence: 7,
    managerRequestDigest: `sha256:${"b".repeat(64)}`,
    senderKeyThumbprint: `sha256:${"c".repeat(64)}`,
  }
  let bindingDigest = `sha256:${createHash("sha256").update(JSON.stringify(binding)).digest("hex")}`
  const receipt = receiptKind === "allocation_worker" ? {
    schemaVersion: 1, status: "confirmed", allocationId: "allocation-1",
    machineId: "machine-1", kernelId: "kernel-1", relayPublicKey: "relay-public-key",
    runtimeReleaseDigest: current.digest, confirmedAt: "2026-09-11T00:00:00Z",
    homeCaller: { accountId: "account-1", userId: "owner-1", realmId: "realm-1",
      machineId: "home-machine-1", kernelId: "home-kernel-1", relayPublicKey: "home-key" },
  } : receiptKind === "disposable_worker" ? {
    schemaVersion: 1,
    kind: "disposable_worker",
    status: "exchanged",
    cloudApiUrl: "https://cloud.example.test",
    relayPublicKey: "relay-public-key",
    bindingDigest,
    binding,
    enrollmentReceipt: {
      grantId: "grant-1",
      allocationId: binding.allocationId,
      workerMachineId: binding.workerMachineId,
      workerKernelId: binding.workerKernelId,
      imageDigest: binding.imageDigest,
      runtimeReleaseDigest: binding.runtimeReleaseDigest,
      exchangedAt: "2026-09-11T00:00:00Z",
    },
    cloudRelay: {
      apiUrl: "https://cloud.example.test",
      email: "worker@example.test",
      accountId: "account-1",
      userId: binding.userId,
      accountSlug: "account-one",
      realmId: binding.realmId,
      relayUrl: "wss://relay.example.test",
      issuerId: "issuer-1",
      machineId: binding.workerMachineId,
      machineAlias: "Disposable worker",
      machineCredential: `mcred_${"d".repeat(43)}`,
    },
  } : {
    schemaVersion: 1,
    status: "confirmed",
    environmentId: "environment-1",
    machineId: "machine-1",
    kernelId: "kernel-1",
    relayPublicKey: "relay-public-key",
    runtimeReleaseDigest: current.digest,
    confirmedAt: "2026-09-11T00:00:00Z",
    contextPlan: { schemaVersion: 1, repositories: [{ id: "repo-1", url: "ssh://example.invalid/repo" }] },
  }
  if (receiptKind === "allocation_worker") bindingDigest = allocationWorkerBindingDigest(receipt)
  await put(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, 0o640)
  const persistent = {
    credential: join(installRoot, "home/chariox/.chariox/vault/vault.json"),
    provider: join(installRoot, "var/lib/chariox/provider-home/provider-state.json"),
    repository: join(installRoot, "home/chariox/repositories/repo-1/HEAD"),
    workspace: join(installRoot, "home/chariox/workspaces/workspace-1/state"),
    session: join(installRoot, "home/chariox/sessions/session-1.json"),
  }
  for (const [name, path] of Object.entries(persistent)) await put(path, `${name}-sentinel\n`, 0o600)

  const state = join(root, "harness-state")
  const bin = join(root, "bin")
  const charioxIdentity = await effectiveCharioxIdentity()
  // Keep root-owned install fixtures runnable when Node tests execute rootless.
  const rootlessHarness = process.getuid?.() !== 0
  await mkdir(state)
  await mkdir(bin)
  await put(join(bin, "id"), `#!/bin/sh
set -eu
if [ "\${1:-}" = "-u" ] && [ "\${2:-}" = "chariox" ]; then
  printf '%s\n' "${charioxIdentity.uid}"
  exit 0
fi
if [ "\${1:-}" = "-g" ] && [ "\${2:-}" = "chariox" ]; then
  printf '%s\n' "${charioxIdentity.gid}"
  exit 0
fi
if [ "\${MANAGED_UPGRADE_TEST_ROOTLESS:-0}" = 1 ] \
  && [ "\${1:-}" = "-u" ] && [ "$#" -eq 1 ]; then
  printf '0\n'
  exit 0
fi
exec /usr/bin/id "$@"
`, 0o755)
  await put(join(bin, "install"), `#!/bin/bash
set -eu
if [ "\${MANAGED_UPGRADE_TEST_ROOTLESS:-0}" != 1 ]; then
  exec /usr/bin/install "$@"
fi
arguments=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o|-g) shift 2 ;;
    *) arguments+=("$1"); shift ;;
  esac
done
exec /usr/bin/install "\${arguments[@]}"
`, 0o755)
  await put(join(bin, "systemctl"), `#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$HARNESS_STATE/systemctl.log"
if [ "$1" = "show" ]; then
  case "$*" in
    *--property=NeedDaemonReload*chariox-path1-managed-bootstrap.service)
      if [ -f "$HARNESS_STATE/disk-home-drop-in" ] && [ ! -f "$HARNESS_STATE/systemd-reloaded" ]; then
        printf 'yes\n'
      else
        printf 'no\n'
      fi
      ;;
    *--property=NeedDaemonReload*chariox-disposable-worker-bootstrap.service)
      if [ -f "$HARNESS_STATE/disk-worker-drop-in" ] && [ ! -f "$HARNESS_STATE/systemd-reloaded" ]; then
        printf 'yes\n'
      else
        printf 'no\n'
      fi
      ;;
    *--property=DropInPaths*chariox-path1-managed-bootstrap.service)
      if [ -f "$HARNESS_STATE/home-drop-in" ] \
        || { [ -f "$HARNESS_STATE/disk-home-drop-in" ] && [ -f "$HARNESS_STATE/systemd-reloaded" ]; }; then
        printf '%s\n' /etc/systemd/system/chariox-path1-managed-bootstrap.service.d/50-hardening.conf
      fi
      ;;
    *--property=DropInPaths*chariox-disposable-worker-bootstrap.service)
      if [ -f "$HARNESS_STATE/worker-drop-in" ] \
        || { [ -f "$HARNESS_STATE/disk-worker-drop-in" ] && [ -f "$HARNESS_STATE/systemd-reloaded" ]; }; then
        printf '%s\n' /etc/systemd/system/chariox-disposable-worker-bootstrap.service.d/50-hardening.conf
      fi
      ;;
  esac
  exit 0
fi
if [ "$1" = "daemon-reload" ]; then
  if [ -f "$HARNESS_STATE/disk-home-drop-in" ] || [ -f "$HARNESS_STATE/disk-worker-drop-in" ]; then
    printf 'present\n' > "$HARNESS_STATE/systemd-reloaded"
  fi
  if [ -f "$HARNESS_STATE/worker-drop-in-after-reload" ]; then
    rm -f -- "$HARNESS_STATE/worker-drop-in-after-reload"
    printf 'present\n' > "$HARNESS_STATE/worker-drop-in"
  fi
fi
presence="$CHARIOX_MANAGED_UPGRADE_ROOT/var/lib/chariox/kernels/active/kernel-1.json"
if [ "$1" = "stop" ]; then
  if [ -f "$HARNESS_STATE/write-legacy-home-on-stop" ]; then
    legacy_home="$CHARIOX_MANAGED_UPGRADE_ROOT/var/lib/chariox/home"
    mkdir -p "$legacy_home"
    printf 'late-service-write\n' > "$legacy_home/late-service-write"
    rm -f -- "$HARNESS_STATE/write-legacy-home-on-stop"
  fi
  rm -f -- "$presence"
fi
if [ "$1" = "start" ]; then
  if [ -f "$HARNESS_STATE/skip-presence-once" ]; then
    rm -f -- "$HARNESS_STATE/skip-presence-once"
  else
    mkdir -p "\${presence%/*}"
    protocol=$("$CHARIOX_MANAGED_UPGRADE_ROOT/usr/lib/chariox/current/usr/local/bin/chariox-kernel" --print-local-daemon-protocol-version)
    now=$(node -e 'process.stdout.write(String(Date.now()))')
    machine_id=machine-1
    if [ -f "$HARNESS_STATE/wrong-machine-once" ]; then
      rm -f -- "$HARNESS_STATE/wrong-machine-once"
      machine_id=machine-wrong
    fi
    if [ -f "$HARNESS_STATE/stale-presence-once" ]; then
      rm -f -- "$HARNESS_STATE/stale-presence-once"
      now=1
    fi
    if [ -f "$HARNESS_STATE/oversized-presence-once" ]; then
      rm -f -- "$HARNESS_STATE/oversized-presence-once"
      node -e 'process.stdout.write("x".repeat(32769))' > "$presence"
    else
      printf '{"schema_version":1,"kernel_id":"kernel-1","machine_id":"%s","host":"127.0.0.1","port":%s,"relay_connected":true,"process_id":%s,"started_at_ms":%s,"heartbeat_at_ms":%s,"local_daemon_protocol_version":%s}\n' \
        "$machine_id" "$CHARIOX_MANAGED_UPGRADE_HEALTH_PORT" "$PPID" "$now" "$now" "$protocol" > "$presence"
    fi
    if [ -f "$HARNESS_STATE/wrong-release-once" ]; then
      rm -f -- "$HARNESS_STATE/wrong-release-once"
      receipt="$CHARIOX_MANAGED_UPGRADE_ROOT/var/lib/chariox/managed/bootstrap-receipt.json"
      sed 's/sha256:[a-f0-9]\\{64\\}/sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff/' \
        "$receipt" > "$receipt.wrong-release"
      chmod 0640 "$receipt.wrong-release"
      mv "$receipt.wrong-release" "$receipt"
    fi
    if [ -f "$HARNESS_STATE/wrong-environment-once" ]; then
      rm -f -- "$HARNESS_STATE/wrong-environment-once"
      receipt="$CHARIOX_MANAGED_UPGRADE_ROOT/var/lib/chariox/managed/bootstrap-receipt.json"
      sed 's/"environmentId": "environment-1"/"environmentId": "environment-wrong"/' \
        "$receipt" > "$receipt.wrong-environment"
      chmod 0640 "$receipt.wrong-environment"
      mv "$receipt.wrong-environment" "$receipt"
    fi
  fi
fi
if [ "$1" = "is-active" ] && [ -f "$HARNESS_STATE/fail-health-once" ]; then
  rm -f "$HARNESS_STATE/fail-health-once"
  exit 1
fi
if [ "$1" = "is-active" ] && [ -f "$HARNESS_STATE/fail-health-always" ]; then
  exit 1
fi
exit 0
`, 0o755)
  await put(join(bin, "node"), `#!/bin/sh
set -eu
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \
  && [ "\${2:-}" = "validate-receipt-match" ] \
  && [ -f "$HARNESS_STATE/fail-final-receipt-validation" ]; then
  rm -f -- "$HARNESS_STATE/fail-final-receipt-validation"
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-home-migration.mjs" ] \
  && [ "\${2:-}" = "apply" ] \
  && [ -f "$HARNESS_STATE/crash-after-home-root-rename" ]; then
  /bin/mv -- "\${5}" "\${6}"
  rm -f -- "$HARNESS_STATE/crash-after-home-root-rename"
  kill -KILL "$PPID"
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-home-migration.mjs" ] \
  && [ "\${2:-}" = "apply" ] \
  && [ -f "$HARNESS_STATE/crash-before-home-completion-marker" ]; then
  "${process.execPath}" "$@"
  rm -f -- "\${3%/*}/home-migration-complete" \
    "$HARNESS_STATE/crash-before-home-completion-marker"
  kill -KILL "$PPID"
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \
  && [ "\${2:-}" = "atomic-symlink" ] \
  && [ -f "$HARNESS_STATE/crash-before-symlink" ]; then
  rm -f "$HARNESS_STATE/crash-before-symlink"
  kill -KILL "$PPID"
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \
  && [ "\${2:-}" = "atomic-symlink" ] \
  && [ "\${4##*/}" = "slice-build-context" ] \
  && [ -f "$HARNESS_STATE/crash-after-facade-symlink" ]; then
  rm -f "$HARNESS_STATE/crash-after-facade-symlink"
  "${process.execPath}" "$@"
  kill -KILL "$PPID"
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \
  && [ "\${2:-}" = "atomic-text" ] \
  && [ -f "$HARNESS_STATE/crash-after-phase-\${3:-}" ]; then
  "${process.execPath}" "$@"
  rm -f "$HARNESS_STATE/crash-after-phase-\${3:-}"
  kill -KILL "$PPID"
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \
  && [ "\${2:-}" = "publish-transaction" ] \
  && [ -f "$HARNESS_STATE/crash-after-phase-prepared" ]; then
  "${process.execPath}" "$@"
  rm -f "$HARNESS_STATE/crash-after-phase-prepared"
  kill -KILL "$PPID"
  exit 1
fi
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \
  && [ "\${2:-}" = "tombstone-transaction" ]; then
  terminal_phase=$(sed -n '1p' "\${3:-}/phase")
  marker="$HARNESS_STATE/crash-after-\${terminal_phase}-tombstone"
  if [ -f "$marker" ]; then
    "${process.execPath}" "$@"
    rm -f "$marker"
    kill -KILL "$PPID"
    exit 1
  fi
fi
exec "${process.execPath}" "$@"
`, 0o755)
  await put(join(bin, "stat"), `#!/bin/sh
set -eu
if [ "\${1:-}" = "-c" ] && [ "\${2:-}" = "%u" ]; then
  if [ -f "$HARNESS_STATE/non-root-key-owner" ] && [ "\${3:-}" = "$MANAGED_TRUSTED_KEY" ]; then
    printf '1\n'
    exit 0
  fi
  if [ -f "$HARNESS_STATE/foreign-receipt-ancestor" ] && [ "\${3:-}" = "$MANAGED_RECEIPT_DIRECTORY" ]; then
    printf '1\n'
    exit 0
  fi
  if [ "\${MANAGED_UPGRADE_TEST_ROOTLESS:-0}" = 1 ]; then
    printf '0\n'
    exit 0
  fi
fi
exec /usr/bin/stat "$@"
`, 0o755)
  const server = createServer()
  await new Promise((resolvePromise, reject) => {
    server.once("error", reject)
    server.listen(0, "127.0.0.1", resolvePromise)
  })
  context.after(() => new Promise((resolvePromise) => server.close(resolvePromise)))
  const port = server.address().port
  const env = {
    ...process.env,
    PATH: `${bin}:${process.env.PATH}`,
    MANAGED_UPGRADE_TEST_ROOTLESS: rootlessHarness ? "1" : "0",
    HARNESS_STATE: state,
    MANAGED_TRUSTED_KEY: trustedKey,
    MANAGED_RECEIPT_DIRECTORY: dirname(receiptPath),
    CHARIOX_MANAGED_UPGRADE_ROOT: installRoot,
    CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "shared_host",
    CHARIOX_MANAGED_UPGRADE_LOCK: join(state, "upgrade.lock"),
    CHARIOX_MANAGED_UPGRADE_HEALTH_HOST: "127.0.0.1",
    CHARIOX_MANAGED_UPGRADE_HEALTH_PORT: String(port),
    CHARIOX_MANAGED_UPGRADE_HEALTH_TIMEOUT_MS: "1000",
  }
  const run = (extraEnv = {}, args = [target.rootfs, current.digest, target.digest, trustedKey]) =>
    spawnSync(upgrade, args, { encoding: "utf8", env: { ...env, ...extraEnv } })
  return {
    root, installRoot, receiptPath, receipt, bindingDigest, persistent, charioxIdentity,
    current, target, trustedKey, trustedBuilderKey, state, run,
  }
}

async function installManagedHotpatchFacade(harness) {
  const facade = join(harness.installRoot, "usr/lib/chariox/slice-build-context")
  const hotpatch = join(harness.root, "usr/local/lib/chariox/managed-hotpatch/slice-build-context-6694e438d1")
  await mkdir(hotpatch, { recursive: true })
  await rm(facade)
  await symlink(hotpatch, facade)
  return { facade, hotpatch }
}

async function persistentSnapshot(paths) {
  return Promise.all(Object.entries(paths).map(async ([name, path]) => ({
    name,
    contents: await readFile(path, "utf8"),
    mode: (await stat(path)).mode & 0o777,
  })))
}

async function treeSnapshot(root) {
  const entries = []
  async function visit(directory, prefix = "") {
    const names = (await readdir(directory)).sort()
    for (const name of names) {
      const path = join(directory, name)
      const relativePath = prefix ? `${prefix}/${name}` : name
      const metadata = await lstat(path)
      const entry = {
        path: relativePath,
        mode: metadata.mode & 0o777,
        mtimeMs: metadata.mtimeMs,
      }
      if (metadata.isSymbolicLink()) {
        entries.push({ ...entry, type: "symlink", target: await readlink(path) })
      } else if (metadata.isDirectory()) {
        entries.push({ ...entry, type: "directory" })
        await visit(path, relativePath)
      } else {
        entries.push({ ...entry, type: "file", contents: await readFile(path) })
      }
    }
  }
  await visit(root)
  return entries
}

test("managed kernel upgrade atomically advances the release and receipt without changing persistent state", async (context) => {
  const harness = await makeHarness(context)
  const before = await persistentSnapshot(harness.persistent)
  const receiptOwner = await stat(harness.receiptPath)
  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`,
  )
  const receipt = JSON.parse(await readFile(harness.receiptPath, "utf8"))
  assert.deepEqual(receipt, { ...harness.receipt, runtimeReleaseDigest: harness.target.digest })
  const upgradedReceiptOwner = await stat(harness.receiptPath)
  assert.equal(upgradedReceiptOwner.uid, receiptOwner.uid)
  assert.equal(upgradedReceiptOwner.gid, receiptOwner.gid)
  assert.equal(upgradedReceiptOwner.mode & 0o777, receiptOwner.mode & 0o777)
  assert.deepEqual(await persistentSnapshot(harness.persistent), before)
  assert.equal(
    await readFile(join(harness.installRoot, "usr/lib/chariox/current/usr/local/bin/chariox-managed-bootstrap"), "utf8"),
    "#!/bin/sh\necho supervisor-target\n",
  )
  assert.deepEqual((await readFile(join(harness.state, "systemctl.log"), "utf8")).trim().split("\n"), [
    `stop ${serviceName}`,
    "daemon-reload",
    `start ${serviceName}`,
    `is-active --quiet ${serviceName}`,
  ])
})

test("Path-1 upgrade rejects effective home or worker drop-ins before recovery or service mutation", async (context) => {
  for (const [marker, unit] of [
    ["home-drop-in", "chariox-path1-managed-bootstrap.service"],
    ["worker-drop-in", "chariox-disposable-worker-bootstrap.service"],
  ]) {
    const harness = await makeHarness(context)
    await put(join(harness.state, marker), "present\n")
    const priorReceipt = await readFile(harness.receiptPath, "utf8")
    const priorRelease = await readlink(join(harness.installRoot, "usr/lib/chariox/current"))
    const result = harness.run({
      CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
      CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedKey,
    })
    assert.equal(result.status, 1)
    assert.match(result.stderr, new RegExp(`Path-1 service ${unit.replaceAll(".", "\\.")} has systemd drop-ins`))
    const calls = (await readFile(join(harness.state, "systemctl.log"), "utf8")).trim().split("\n")
    assert.ok(calls.every((call) => call.startsWith("show --property=")), calls.join("\n"))
    assert.equal(await readFile(harness.receiptPath, "utf8"), priorReceipt)
    assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")), priorRelease)
  }
})

test("Path-1 upgrade refuses an unreloaded on-disk drop-in before pending transaction recovery or service mutation", async (context) => {
  const services = [
    ["disk-home-drop-in", "chariox-path1-managed-bootstrap.service"],
    ["disk-worker-drop-in", "chariox-disposable-worker-bootstrap.service"],
  ]
  for (const phase of ["prepared", "stopped"]) {
    for (const [diskDropIn, blockedUnit] of services) {
      const harness = await makeHarness(context, { path1Release: true })
      await put(join(harness.state, `crash-after-phase-${phase}`), "crash\n")
      const seeded = harness.run({
        CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
        CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey,
      })
      assert.equal(seeded.signal, "SIGKILL", `${phase}: ${seeded.stderr}`)

      const transactionRoot = join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade")
      assert.equal(await readFile(join(transactionRoot, "phase"), "utf8"), `${phase}\n`)
      const beforeInstall = await treeSnapshot(harness.installRoot)
      const beforeTransaction = await treeSnapshot(transactionRoot)
      const beforeSystemctl = (await readFile(join(harness.state, "systemctl.log"), "utf8"))
        .trim().split("\n").length
      await put(join(harness.state, diskDropIn), "unreloaded systemd drop-in\n")

      const result = harness.run({
        CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
        CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey,
      })
      assert.equal(result.status, 1, `${phase}/${blockedUnit}: ${result.stderr}`)
      assert.match(result.stderr, new RegExp(
        `Path-1 service ${blockedUnit.replaceAll(".", "\\.")} needs systemd daemon-reload; refusing upgrade before service mutation`,
      ))

      const calls = (await readFile(join(harness.state, "systemctl.log"), "utf8"))
        .trim().split("\n").slice(beforeSystemctl)
      assert.ok(calls.every((call) => call.startsWith("show --property=")), calls.join("\n"))
      assert.ok(calls.includes("show --property=NeedDaemonReload --value chariox-path1-managed-bootstrap.service"), calls.join("\n"))
      assert.ok(calls.includes("show --property=NeedDaemonReload --value chariox-disposable-worker-bootstrap.service"), calls.join("\n"))
      assert.ok(calls.includes("show --property=DropInPaths --value chariox-path1-managed-bootstrap.service"), calls.join("\n"))
      assert.ok(calls.includes("show --property=DropInPaths --value chariox-disposable-worker-bootstrap.service"), calls.join("\n"))
      assert.deepEqual(await treeSnapshot(transactionRoot), beforeTransaction)
      assert.deepEqual(await treeSnapshot(harness.installRoot), beforeInstall)
    }
  }
})

test("Path-1 upgrade starts the selected supervisor after both effective units pass the drop-in check", async (context) => {
  const harness = await makeHarness(context, { path1Release: true })
  const result = harness.run({
    CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
    CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey,
  })
  assert.equal(result.status, 0, result.stderr)
  const calls = (await readFile(join(harness.state, "systemctl.log"), "utf8")).trim().split("\n")
  assert.ok(calls.includes("show --property=DropInPaths --value chariox-path1-managed-bootstrap.service"), calls.join("\n"))
  assert.ok(calls.includes("show --property=DropInPaths --value chariox-disposable-worker-bootstrap.service"), calls.join("\n"))
  assert.ok(calls.includes("daemon-reload"), calls.join("\n"))
  assert.ok(calls.includes("start chariox-path1-managed-bootstrap.service"), calls.join("\n"))
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`)
})

test("Path-1 upgrade leaves the kernel stopped and rollback pending when a worker drop-in appears after reload", async (context) => {
  const harness = await makeHarness(context, { path1Release: true })
  const priorReceipt = await readFile(harness.receiptPath, "utf8")
  const priorRelease = await readlink(join(harness.installRoot, "usr/lib/chariox/current"))
  await put(join(harness.state, "worker-drop-in-after-reload"), "present\n")
  const result = harness.run({
    CHARIOX_MANAGED_PROVIDER_TOPOLOGY: "path1",
    CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY: harness.trustedBuilderKey,
  })
  assert.equal(result.status, 1, result.stderr)
  assert.match(result.stderr, /Path-1 service chariox-disposable-worker-bootstrap\.service has systemd drop-ins/)
  assert.match(result.stderr, /Path-1 systemd drop-ins blocked activation; rollback remains pending; verify the managed kernel service state before retry/)
  const calls = (await readFile(join(harness.state, "systemctl.log"), "utf8")).trim().split("\n")
  assert.ok(calls.includes("daemon-reload"), calls.join("\n"))
  assert.ok(calls.includes("stop chariox-path1-managed-bootstrap.service"), calls.join("\n"))
  assert.ok(calls.every((call) => !call.startsWith("start ")), calls.join("\n"))
  assert.equal(await readFile(harness.receiptPath, "utf8"), priorReceipt)
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/current")), priorRelease)
  assert.equal((await stat(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade"))).isDirectory(), true)
})

test("managed kernel upgrade migrates legacy home state and rejects a home collision", async (context) => {
  const harness = await makeHarness(context)
  const canonicalHome = join(harness.installRoot, "home/chariox")
  const legacyHome = join(harness.installRoot, "var/lib/chariox/home")
  const legacyReceipt = join(legacyHome, "managed/bootstrap-receipt.json")
  await rename(canonicalHome, legacyHome)
  await createRootPrivateDirectory(dirname(legacyReceipt))
  await rename(harness.receiptPath, legacyReceipt)

  const migrated = harness.run()
  assert.equal(migrated.status, 0, migrated.stderr)
  assert.equal(await lstat(legacyHome).then(() => true, () => false), false)
  assert.equal(
    await readFile(join(harness.installRoot, "home/chariox/repositories/repo-1/HEAD"), "utf8"),
    "repository-sentinel\n",
  )
  assert.equal(
    await readFile(join(harness.installRoot, "home/chariox/.chariox/vault/vault.json"), "utf8"),
    "credential-sentinel\n",
  )
  assert.equal(
    JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest,
    harness.target.digest,
  )
  assert.equal((await stat(join(harness.installRoot, "home/chariox"))).mode & 0o777, 0o700)
  assert.equal((await stat(join(harness.installRoot, "home/chariox/.chariox"))).mode & 0o777, 0o700)
  const { uid: charioxUid, gid: charioxGid } = harness.charioxIdentity
  assert.equal((await stat(join(harness.installRoot, "home/chariox"))).uid, charioxUid)
  assert.equal((await stat(join(harness.installRoot, "home/chariox"))).gid, charioxGid)
  assert.equal((await stat(join(harness.installRoot, "home/chariox/.chariox"))).uid, charioxUid)
  assert.equal((await stat(join(harness.installRoot, "home/chariox/.chariox"))).gid, charioxGid)

  const collisionHarness = await makeHarness(context)
  const collisionLegacy = join(collisionHarness.installRoot, "var/lib/chariox/home")
  await createRootPrivateDirectory(collisionLegacy)
  await writeFile(join(collisionLegacy, "must-remain-untouched"), "collision\n")
  const collision = collisionHarness.run()
  assert.equal(collision.status, 1)
  assert.match(collision.stderr, /both exist; refusing to overwrite either/)
  assert.equal(
    await readFile(join(collisionLegacy, "must-remain-untouched"), "utf8"),
    "collision\n",
  )
  assert.equal(
    await lstat(join(collisionHarness.state, "systemctl.log")).then(() => true, () => false),
    false,
  )
})

test("managed kernel upgrade stops the live service before migrating legacy home", async (context) => {
  const harness = await makeHarness(context)
  const canonicalHome = join(harness.installRoot, "home/chariox")
  const legacyHome = join(harness.installRoot, "var/lib/chariox/home")
  const legacyReceipt = join(legacyHome, "managed/bootstrap-receipt.json")
  await rename(canonicalHome, legacyHome)
  await createRootPrivateDirectory(dirname(legacyReceipt))
  await rename(harness.receiptPath, legacyReceipt)
  await put(join(harness.state, "write-legacy-home-on-stop"), "write\n")

  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(
    await lstat(legacyHome).then(() => true, () => false),
    false,
    "the stopped service must not recreate a legacy state tree after migration",
  )
  assert.equal(
    await readFile(join(canonicalHome, "late-service-write"), "utf8"),
    "late-service-write\n",
    "the migration must retain the service's final write",
  )
  assert.deepEqual((await readFile(join(harness.state, "systemctl.log"), "utf8")).trim().split("\n"), [
    `stop ${serviceName}`,
    "daemon-reload",
    `start ${serviceName}`,
    `is-active --quiet ${serviceName}`,
  ])
})

test("managed kernel upgrade resumes its identity-backed home migration after crash windows", async (context) => {
  const crashWindows = ["crash-after-home-root-rename", "crash-before-home-completion-marker"]
  const observedCrashWindows = []
  for (const marker of crashWindows) {
    const harness = await makeHarness(context)
    const canonicalHome = join(harness.installRoot, "home/chariox")
    const legacyHome = join(harness.installRoot, "var/lib/chariox/home")
    const legacyReceipt = join(legacyHome, "managed/bootstrap-receipt.json")
    await rename(canonicalHome, legacyHome)
    await createRootPrivateDirectory(dirname(legacyReceipt))
    await rename(harness.receiptPath, legacyReceipt)
    await put(join(harness.state, marker), "crash\n")

    const interrupted = harness.run()
    assert.equal(interrupted.signal, "SIGKILL", `${marker}: ${interrupted.stderr}`)
    observedCrashWindows.push(marker)
    const recovered = harness.run()
    assert.equal(recovered.status, 0, `${marker}: ${recovered.stderr}`)
    assert.equal(await lstat(legacyHome).then(() => true, () => false), false)
    assert.equal(
      await readFile(join(canonicalHome, "repositories/repo-1/HEAD"), "utf8"),
      "repository-sentinel\n",
    )
    assert.equal(
      JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest,
      harness.target.digest,
    )
  }
  assert.deepEqual(observedCrashWindows, crashWindows, "both injected home-migration crash windows must be observed")
})

test("managed kernel upgrade validates the signed candidate before planning legacy migration", async (context) => {
  const harness = await makeHarness(context)
  const canonicalHome = join(harness.installRoot, "home/chariox")
  const legacyHome = join(harness.installRoot, "var/lib/chariox/home")
  const legacyReceipt = join(legacyHome, "managed/bootstrap-receipt.json")
  await rename(canonicalHome, legacyHome)
  await createRootPrivateDirectory(dirname(legacyReceipt))
  await rename(harness.receiptPath, legacyReceipt)
  await put(
    join(harness.target.rootfs, "etc/systemd/system/chariox-disposable-worker-bootstrap.service"),
    "[Service]\nExecStart=/unexpected\n",
  )

  const result = harness.run()
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /chariox-disposable-worker-bootstrap.service is corrupted/)
  assert.equal(await lstat(legacyHome).then(() => true, () => false), true)
  assert.equal(await lstat(canonicalHome).then(() => true, () => false), false)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade rejects a symlinked legacy state entry before stopping the service", async (context) => {
  const harness = await makeHarness(context)
  const canonicalHome = join(harness.installRoot, "home/chariox")
  const legacyHome = join(harness.installRoot, "var/lib/chariox/home")
  const legacyReceipt = join(legacyHome, "managed/bootstrap-receipt.json")
  const outside = join(harness.root, "outside-managed-context")
  await rename(canonicalHome, legacyHome)
  await createRootPrivateDirectory(dirname(legacyReceipt))
  await rename(harness.receiptPath, legacyReceipt)
  await mkdir(outside)
  await writeFile(join(outside, "must-remain"), "outside\n")
  await symlink(outside, join(legacyHome, "managed-context"))

  const result = harness.run()
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /managed kernel managed-context is not a real directory/)
  assert.equal(await readFile(join(outside, "must-remain"), "utf8"), "outside\n")
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed upgrade rejects a corrupted declared worker service before stopping the kernel", async (context) => {
  const harness = await makeHarness(context)
  await put(
    join(harness.target.rootfs, "etc/systemd/system/chariox-disposable-worker-bootstrap.service"),
    "[Service]\nExecStart=/unexpected\n",
  )
  const result = harness.run()
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /chariox-disposable-worker-bootstrap.service is corrupted/)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("legacy managed release rejects an unsigned extra worker service before stopping the kernel", async (context) => {
  const harness = await makeHarness(context)
  await put(
    join(harness.installRoot, "usr/lib/chariox/current/etc/systemd/system/chariox-disposable-worker-bootstrap.service"),
    "[Service]\nExecStart=/unexpected\n",
  )
  const result = harness.run()
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /undeclared worker service/)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade rotates the release trust key explicitly", async (context) => {
  const harness = await makeHarness(context)
  const { privateKey, publicKey } = generateKeyPairSync("ed25519")
  const targetManifest = await readFile(join(harness.target.rootfs, "usr/lib/chariox/release-manifest.json"))
  await put(
    join(harness.target.rootfs, "usr/lib/chariox/release-manifest.sig"),
    sign(null, targetManifest, privateKey).toString("base64"),
  )
  const nextTrustedKey = join(harness.root, "next-trusted-release-public-key")
  const nextRawKey = rawPublicKey(publicKey).toString("base64")
  await put(join(harness.target.rootfs, "usr/lib/chariox/release-public-key"), nextRawKey)
  await put(nextTrustedKey, nextRawKey, 0o600)

  const result = harness.run({}, [
    harness.target.rootfs,
    harness.current.digest,
    harness.target.digest,
    harness.trustedKey,
    nextTrustedKey,
  ])
  assert.equal(result.status, 0, result.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`,
  )
})

test("managed kernel upgrade supersedes a managed-hotpatch slice context facade", async (context) => {
  const harness = await makeHarness(context)
  const { facade } = await installManagedHotpatchFacade(harness)
  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(await readlink(facade), "current/usr/lib/chariox/slice-build-context")
  assert.equal(
    await readFile(join(facade, "apps/kernel/slice-linux-docker/runtime.txt"), "utf8"),
    "target\n",
  )
})

test("managed kernel failed health restores the exact prior slice context facade", async (context) => {
  const harness = await makeHarness(context)
  const { facade, hotpatch } = await installManagedHotpatchFacade(harness)
  await put(join(harness.state, "fail-health-once"), "fail\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /health check failed; restored previous managed kernel release/)
  assert.equal(await readlink(facade), hotpatch)
})

test("managed kernel recovery restores a hotpatch facade after interrupted activation", async (context) => {
  const harness = await makeHarness(context)
  const { facade, hotpatch } = await installManagedHotpatchFacade(harness)
  await put(join(harness.state, "crash-after-facade-symlink"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  assert.equal(await readlink(facade), "current/usr/lib/chariox/slice-build-context")

  await put(join(harness.target.rootfs, "usr/local/bin/chariox-kernel"), "#!/bin/sh\nexit 1\n", 0o755)
  const recovered = harness.run()
  assert.equal(recovered.status, 1)
  assert.match(recovered.stderr, /release artifact chariox-kernel is corrupted/)
  assert.equal(await readlink(facade), hotpatch)
})

test("allocation worker upgrade preserves the receipt written by the active bootstrap", async (context) => {
  const harness = await makeHarness(context, { workerCapableCurrent: true })
  // Keep this shape aligned with WorkerReceipt in managed_bootstrap/worker.rs,
  // not the disabled pre-allocation bootstrap's binding/enrollmentReceipt shape.
  const receipt = {
    schemaVersion: 1,
    status: "confirmed",
    allocationId: "allocation-1",
    machineId: "machine-1",
    kernelId: "kernel-1",
    relayPublicKey: "relay-public-key",
    runtimeReleaseDigest: harness.current.digest,
    homeCaller: {
      accountId: "account-1", userId: "owner-1", realmId: "realm-1",
      machineId: "home-machine-1", kernelId: "home-kernel-1", relayPublicKey: "home-relay-key",
    },
    confirmedAt: "2026-09-11T00:00:00Z",
  }
  const receiptBytes = `${JSON.stringify(receipt, null, 2)}\n`
  const activeReceiptPath = harness.receiptPath
  await chmod(dirname(activeReceiptPath), 0o700)
  await put(activeReceiptPath, receiptBytes, 0o640)
  const before = await persistentSnapshot(harness.persistent)
  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(await readFile(activeReceiptPath, "utf8"), receiptBytes)
  assert.deepEqual(await persistentSnapshot(harness.persistent), before)
  const serviceLog = await readFile(join(harness.state, "systemctl.log"), "utf8")
  assert.match(serviceLog, /start chariox-disposable-worker-bootstrap.service/)
  assert.doesNotMatch(serviceLog, /(?:start|stop) chariox-managed-bootstrap.service/)
  const rollback = harness.run({}, [harness.current.rootfs, harness.target.digest, harness.current.digest, harness.trustedKey])
  assert.equal(rollback.status, 0, rollback.stderr)
  assert.equal(await readFile(activeReceiptPath, "utf8"), receiptBytes)
  assert.deepEqual(await persistentSnapshot(harness.persistent), before)
})

test("legacy worker upgrade is rejected before service mutation", async (context) => {
  const harness = await makeHarness(context, { receiptKind: "disposable_worker" })
  const receipt = await readFile(harness.receiptPath, "utf8")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /legacy disposable worker bootstrap is unsupported/)
  assert.equal(await readFile(harness.receiptPath, "utf8"), receipt)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade preserves an exact disposable worker receipt and persistent state", async (context) => {
  const harness = await makeHarness(context, { receiptKind: "allocation_worker" })
  const before = await persistentSnapshot(harness.persistent)
  const receiptBefore = await readFile(harness.receiptPath, "utf8")
  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(await readFile(harness.receiptPath, "utf8"), receiptBefore)
  assert.deepEqual(await persistentSnapshot(harness.persistent), before)
  assert.deepEqual(
    JSON.parse(await readFile(join(dirname(harness.receiptPath), "release-override.json"), "utf8")),
    {
      schemaVersion: 1,
      kind: "disposable_worker_release",
      bindingDigest: harness.bindingDigest,
      runtimeReleaseDigest: harness.target.digest,
    },
  )
})

test("ordinary managed upgrade rejects a worker release override before service mutation", async (context) => {
  const harness = await makeHarness(context)
  await put(
    join(dirname(harness.receiptPath), "release-override.json"),
    `${JSON.stringify({
      schemaVersion: 1,
      kind: "disposable_worker_release",
      bindingDigest: `sha256:${"a".repeat(64)}`,
      runtimeReleaseDigest: harness.current.digest,
    })}\n`,
    0o640,
  )
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /ordinary managed bootstrap receipt cannot use/)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("disposable worker upgrade rejects altered Cloud authority before service mutation", async (context) => {
  for (const mutate of [
    (receipt) => { receipt.homeCaller.kernelId = "other-home" },
    (receipt) => { receipt.status = "exchanged" },
    (receipt) => { receipt.homeCaller.accountId = "other-account" },
    (receipt) => { receipt.unexpected = true },
  ]) {
    const harness = await makeHarness(context, { receiptKind: "allocation_worker" })
    const receipt = JSON.parse(await readFile(harness.receiptPath, "utf8"))
    await put(join(dirname(harness.receiptPath), "release-override.json"), JSON.stringify({
      schemaVersion: 1, kind: "disposable_worker_release", bindingDigest: harness.bindingDigest,
      runtimeReleaseDigest: harness.current.digest,
    }), 0o640)
    mutate(receipt)
    await put(harness.receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, 0o640)
    const result = harness.run()
    assert.equal(result.status, 1)
    assert.match(result.stderr, /allocation worker bootstrap receipt is invalid|disposable worker release override is invalid/)
    assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
  }
})

test("disposable worker rollback restores its prior signed-release override", async (context) => {
  const harness = await makeHarness(context, { receiptKind: "allocation_worker" })
  const overridePath = join(dirname(harness.receiptPath), "release-override.json")
  await put(overridePath, `${JSON.stringify({
    schemaVersion: 1,
    kind: "disposable_worker_release",
    bindingDigest: harness.bindingDigest,
    runtimeReleaseDigest: harness.current.digest,
  }, null, 2)}\n`, 0o640)
  const before = await readFile(overridePath, "utf8")
  await put(join(harness.state, "fail-health-once"), "fail\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /health check failed; restored previous managed kernel release/)
  assert.equal(await readFile(overridePath, "utf8"), before)
})

test("disposable worker rollback removes a newly staged release override", async (context) => {
  const harness = await makeHarness(context, { receiptKind: "allocation_worker" })
  const receiptBefore = await readFile(harness.receiptPath, "utf8")
  const overridePath = join(dirname(harness.receiptPath), "release-override.json")
  await put(join(harness.state, "fail-health-once"), "fail\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /health check failed; restored previous managed kernel release/)
  assert.equal(await lstat(overridePath).then(() => true, () => false), false)
  assert.equal(await readFile(harness.receiptPath, "utf8"), receiptBefore)
})

test("disposable worker upgrade recovers an interruption without rewriting Cloud authority", async (context) => {
  const harness = await makeHarness(context, { receiptKind: "allocation_worker" })
  const receiptBefore = await readFile(harness.receiptPath, "utf8")
  await put(join(harness.state, "crash-before-symlink"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const retried = harness.run()
  assert.equal(retried.status, 0, retried.stderr)
  assert.equal(await readFile(harness.receiptPath, "utf8"), receiptBefore)
  assert.equal(
    JSON.parse(await readFile(join(dirname(harness.receiptPath), "release-override.json"), "utf8"))
      .runtimeReleaseDigest,
    harness.target.digest,
  )
})

test("managed kernel health rejects an unrelated listener without a fresh matching kernel presence", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "skip-presence-once"), "skip\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /health check failed; restored previous managed kernel release/)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
})

test("managed kernel health uses the installed home presence directory for upgrade and rollback", async (context) => {
  const harness = await makeHarness(context)
  const presenceRoot = join(harness.installRoot, "var/lib/chariox/kernels/active")
  await put(join(harness.state, "skip-presence-once"), "skip\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.ok(result.stderr.includes(`presence directory ${presenceRoot}`), result.stderr)
  assert.doesNotMatch(result.stderr, /home\/\.chariox\/kernels\/active/)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
  assert.equal(
    await lstat(join(presenceRoot, "kernel-1.json")).then(() => true, () => false),
    true,
  )
})

test("managed kernel health rejects stale, wrong-machine, or oversized presence", async (context) => {
  for (const marker of ["stale-presence-once", "wrong-machine-once", "oversized-presence-once"]) {
    const harness = await makeHarness(context)
    await put(join(harness.state, marker), "reject\n")
    const result = harness.run()
    assert.equal(result.status, 1)
    assert.match(result.stderr, /health check failed; restored previous managed kernel release/)
  }
})

test("managed kernel health is bound to the expected release digest", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "wrong-release-once"), "reject\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /health check failed; restored previous managed kernel release/)
  assert.doesNotMatch(result.stderr, /final receipt validation failed/)
})

test("managed kernel readiness preserves the registered environment identity", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "wrong-environment-once"), "reject\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /final receipt validation failed; restored previous managed kernel release/)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
  assert.deepEqual(JSON.parse(await readFile(harness.receiptPath, "utf8")), harness.receipt)
})

test("managed kernel upgrade rejects writable release authority and receipt state before stopping service", async (context) => {
  const writableReleaseRoot = await makeHarness(context)
  await chmod(join(writableReleaseRoot.installRoot, "usr/lib/chariox/releases"), 0o777)
  let result = writableReleaseRoot.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed kernel upgrade authority permissions are unsafe/)
  assert.equal(await lstat(join(writableReleaseRoot.state, "systemctl.log")).then(() => true, () => false), false)

  const writableCurrentRelease = await makeHarness(context)
  await chmod(
    join(
      writableCurrentRelease.installRoot,
      "usr/lib/chariox/releases",
      writableCurrentRelease.current.digest.slice("sha256:".length),
    ),
    0o777,
  )
  result = writableCurrentRelease.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed kernel upgrade authority permissions are unsafe/)
  assert.equal(await lstat(join(writableCurrentRelease.state, "systemctl.log")).then(() => true, () => false), false)

  const writableReceipt = await makeHarness(context)
  await chmod(writableReceipt.receiptPath, 0o660)
  result = writableReceipt.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed bootstrap receipt permissions are unsafe/)
  assert.equal(await lstat(join(writableReceipt.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade rolls the binary and receipt back together when health fails", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "fail-health-once"), "fail\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /health check failed; restored previous managed kernel release/)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
  assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, harness.current.digest)
  const calls = (await readFile(join(harness.state, "systemctl.log"), "utf8")).trim().split("\n")
  assert.deepEqual(calls.slice(-4), [
    `stop ${serviceName}`,
    "daemon-reload",
    `start ${serviceName}`,
    `is-active --quiet ${serviceName}`,
  ])
})

test("managed kernel rollback never reports success when restored health is unverified", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "fail-health-always"), "fail\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /rollback remains pending/)
  assert.doesNotMatch(result.stderr, /restored previous managed kernel release/)
  assert.equal(
    await lstat(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade"))
      .then((metadata) => metadata.isDirectory(), () => false),
    true,
  )
})

test("managed kernel upgrade immediately rolls back a post-health receipt validation failure", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "fail-final-receipt-validation"), "fail\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /final receipt validation failed; restored previous managed kernel release/)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
  assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, harness.current.digest)
  assert.equal(
    await lstat(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade"))
      .then(() => true, () => false),
    false,
  )
})

test("managed kernel upgrade recovers an interruption between activation stages before retrying", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "crash-before-symlink"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const transaction = join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade")
  assert.equal((await stat(transaction)).isDirectory(), true)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
  assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, harness.target.digest)

  const retried = harness.run()
  assert.equal(retried.status, 0, retried.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`,
  )
  assert.equal(JSON.parse(await readFile(harness.receiptPath, "utf8")).runtimeReleaseDigest, harness.target.digest)
  assert.equal(await lstat(transaction).then(() => true, () => false), false)
})

test("managed kernel upgrade rejects digest mismatch, a stale precondition, and a partial or unsigned release", async (context) => {
  const badDigest = await makeHarness(context)
  let result = badDigest.run({}, [
    badDigest.target.rootfs,
    badDigest.current.digest,
    `sha256:${"0".repeat(64)}`,
    badDigest.trustedKey,
  ])
  assert.equal(result.status, 1)
  assert.match(result.stderr, /release manifest digest does not match/)

  const staleRequest = await makeHarness(context)
  result = staleRequest.run({}, [
    staleRequest.target.rootfs,
    `sha256:${"f".repeat(64)}`,
    staleRequest.target.digest,
    staleRequest.trustedKey,
  ])
  assert.equal(result.status, 1)
  assert.match(result.stderr, /installed release does not match the expected current release/)

  const alreadyCurrent = await makeHarness(context)
  result = alreadyCurrent.run({}, [
    alreadyCurrent.current.rootfs,
    alreadyCurrent.current.digest,
    alreadyCurrent.current.digest,
    alreadyCurrent.trustedKey,
  ])
  assert.equal(result.status, 1)
  assert.match(result.stderr, /target release is not newer than current/)

  const partial = await makeHarness(context)
  await rm(join(partial.target.rootfs, "usr/local/bin/chariox-managed-bootstrap"))
  result = partial.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /cannot be read|invalid file|corrupted/)

  const unsigned = await makeHarness(context)
  await put(join(unsigned.target.rootfs, "usr/lib/chariox/release-manifest.sig"), Buffer.alloc(64).toString("base64"))
  result = unsigned.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /release signature is invalid/)

  const tampered = await makeHarness(context)
  await put(join(tampered.target.rootfs, "usr/local/bin/chariox-kernel"), "#!/bin/sh\nexit 0\n", 0o755)
  result = tampered.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /release artifact chariox-kernel is corrupted/)
})

test("managed kernel upgrade rejects linked authority paths before service mutation", async (context) => {
  const linkedReceipt = await makeHarness(context)
  const receiptContents = await readFile(linkedReceipt.receiptPath)
  const externalReceipt = join(linkedReceipt.root, "external-receipt.json")
  await put(externalReceipt, receiptContents, 0o600)
  await rm(linkedReceipt.receiptPath)
  await symlink(externalReceipt, linkedReceipt.receiptPath)
  let result = linkedReceipt.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /invalid file/)
  assert.equal(await lstat(join(linkedReceipt.state, "systemctl.log")).then(() => true, () => false), false)

  const linkedTransaction = await makeHarness(context)
  const externalTransaction = join(linkedTransaction.root, "external-transaction")
  await mkdir(externalTransaction)
  await symlink(
    externalTransaction,
    join(linkedTransaction.installRoot, "usr/lib/chariox/.managed-kernel-upgrade"),
  )
  result = linkedTransaction.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /transaction path is obstructed/)
  assert.equal(await lstat(join(linkedTransaction.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade rejects writable or linked authority ancestors before service mutation", async (context) => {
  const writableReceiptAncestor = await makeHarness(context)
  await chmod(dirname(writableReceiptAncestor.receiptPath), 0o777)
  let result = writableReceiptAncestor.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed bootstrap receipt ancestor permissions are unsafe/)
  assert.equal(await lstat(join(writableReceiptAncestor.state, "systemctl.log")).then(() => true, () => false), false)

  const linkedReceiptAncestor = await makeHarness(context)
  const managedDirectory = dirname(linkedReceiptAncestor.receiptPath)
  const externalManagedDirectory = join(linkedReceiptAncestor.root, "external-managed")
  await rename(managedDirectory, externalManagedDirectory)
  await symlink(externalManagedDirectory, managedDirectory)
  result = linkedReceiptAncestor.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed bootstrap receipt ancestor is unsafe/)
  assert.equal(await lstat(join(linkedReceiptAncestor.state, "systemctl.log")).then(() => true, () => false), false)

  const writableKeyAncestor = await makeHarness(context)
  await chmod(dirname(writableKeyAncestor.trustedKey), 0o777)
  result = writableKeyAncestor.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /trusted release public key ancestor permissions are unsafe/)
  assert.equal(await lstat(join(writableKeyAncestor.state, "systemctl.log")).then(() => true, () => false), false)

  const foreignReceiptAncestor = await makeHarness(context)
  await put(join(foreignReceiptAncestor.state, "foreign-receipt-ancestor"), "simulate\n")
  result = foreignReceiptAncestor.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed bootstrap receipt ancestor owner is unsafe/)
  assert.equal(await lstat(join(foreignReceiptAncestor.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade rejects a non-root-owned trusted release key", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "non-root-key-owner"), "simulate\n")
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /trusted release public key owner is unsafe/)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade requires the exact confirmed registered-kernel receipt", async (context) => {
  const malformed = await makeHarness(context)
  const malformedReceipt = JSON.parse(await readFile(malformed.receiptPath, "utf8"))
  malformedReceipt.futureAuthority = { allowProvisioning: true }
  await put(malformed.receiptPath, `${JSON.stringify(malformedReceipt)}\n`, 0o600)
  let result = malformed.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /managed bootstrap receipt contains unsupported fields/)

  const unconfirmed = await makeHarness(context)
  const unconfirmedReceipt = JSON.parse(await readFile(unconfirmed.receiptPath, "utf8"))
  unconfirmedReceipt.status = "exchanged"
  unconfirmedReceipt.confirmedAt = null
  await put(unconfirmed.receiptPath, `${JSON.stringify(unconfirmedReceipt)}\n`, 0o600)
  result = unconfirmed.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /not a confirmed registered-kernel receipt/)
})

test("managed kernel upgrade accepts signed direct deployed protocol 325 to 333 transition and rollback", async (context) => {
  const harness = await makeHarness(context, {
    targetProtocol: 333,
    targetTransitionPolicy: {
      schemaVersion: 1,
      protocol: 333,
      upgradeFrom: [325, 326, 327, 328, 329, 330, 331, 332, 333],
      rollbackTo: [325, 326, 327, 328, 329, 330, 331, 332, 333],
    },
  })
  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`,
  )
  const rollback = harness.run({}, [
    harness.current.rootfs,
    harness.target.digest,
    harness.current.digest,
    harness.trustedKey,
  ])
  assert.equal(rollback.status, 0, rollback.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
})

test("managed kernel upgrade rejects a forward transition authorized on only one leg", async (context) => {
  const harness = await makeHarness(context, {
    currentProtocol: 322,
    targetProtocol: 323,
    targetTransitionPolicy: {
      schemaVersion: 1,
      protocol: 323,
      upgradeFrom: [322, 323],
      rollbackTo: [323],
    },
  })
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /not reciprocally authorized/)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade rejects a reverse transition authorized on only one leg", async (context) => {
  const harness = await makeHarness(context, {
    currentProtocol: 323,
    targetProtocol: 322,
    currentTransitionPolicy: {
      schemaVersion: 1,
      protocol: 323,
      upgradeFrom: [323],
      rollbackTo: [322, 323],
    },
  })
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /not reciprocally authorized/)
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade rejects an unsupported local daemon protocol transition", async (context) => {
  const harness = await makeHarness(context, { targetProtocol: 322 })
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /protocol transition policy is missing or invalid/)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("a signed transition policy permits post-success rollback to its declared protocol", async (context) => {
  const policy = {
    schemaVersion: 1,
    protocol: 323,
    upgradeFrom: [322, 323],
    rollbackTo: [322, 323],
  }
  const harness = await makeHarness(context, {
    currentProtocol: 322,
    targetProtocol: 323,
    targetTransitionPolicy: policy,
  })
  const upgraded = harness.run()
  assert.equal(upgraded.status, 0, upgraded.stderr)
  const rolledBack = harness.run({}, [
    harness.current.rootfs,
    harness.target.digest,
    harness.current.digest,
    harness.trustedKey,
  ])
  assert.equal(rolledBack.status, 0, rolledBack.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
})

test("managed kernel upgrade recovers interruption after every persisted nonterminal phase", async (context) => {
  for (const phase of ["prepared", "stopped", "activated"]) {
    const harness = await makeHarness(context)
    await put(join(harness.state, `crash-after-phase-${phase}`), "crash\n")
    const interrupted = harness.run()
    assert.equal(interrupted.signal, "SIGKILL", phase)
    const retried = harness.run()
    assert.equal(retried.status, 0, `${phase}: ${retried.stderr}`)
    assert.equal(
      await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
      `releases/${harness.target.digest.slice("sha256:".length)}`,
      phase,
    )
  }
})

test("managed kernel upgrade recovers interruption after the persisted rolled-back phase", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "fail-health-once"), "fail\n")
  await put(join(harness.state, "crash-after-phase-rolled_back"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const retried = harness.run()
  assert.equal(retried.status, 0, retried.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`,
  )
})

test("cross-protocol recovery authorizes automatic rollback and re-upgrade before shutdown", async (context) => {
  const harness = await makeHarness(context, {
    currentProtocol: 322,
    targetProtocol: 323,
    targetTransitionPolicy: {
      schemaVersion: 1,
      protocol: 323,
      upgradeFrom: [322, 323],
      rollbackTo: [322, 323],
    },
  })
  await put(join(harness.state, "crash-after-phase-activated"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const retried = harness.run()
  assert.equal(retried.status, 0, retried.stderr)
  const calls = (await readFile(join(harness.state, "systemctl.log"), "utf8")).trim().split("\n")
  assert.equal(calls.filter((call) => call === `stop ${serviceName}`).length, 3)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`,
  )
})

test("managed kernel upgrade recovers interruption after the persisted committed phase", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "crash-after-phase-committed"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const recovered = harness.run({}, [
    harness.current.rootfs,
    harness.target.digest,
    harness.current.digest,
    harness.trustedKey,
  ])
  assert.equal(recovered.status, 0, recovered.stderr)
  assert.equal(
    await lstat(join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade"))
      .then(() => true, () => false),
    false,
  )
})

test("managed kernel upgrade discards a committed tombstone after cleanup interruption", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "crash-after-committed-tombstone"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const tombstone = join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade.terminal")
  assert.equal((await stat(tombstone)).isDirectory(), true)
  const recovered = harness.run({}, [
    harness.current.rootfs,
    harness.target.digest,
    harness.current.digest,
    harness.trustedKey,
  ])
  assert.equal(recovered.status, 0, recovered.stderr)
  assert.equal(await lstat(tombstone).then(() => true, () => false), false)
})

test("managed kernel upgrade discards a rolled-back tombstone after cleanup interruption", async (context) => {
  const harness = await makeHarness(context)
  await put(join(harness.state, "fail-health-once"), "fail\n")
  await put(join(harness.state, "crash-after-rolled_back-tombstone"), "crash\n")
  const interrupted = harness.run()
  assert.equal(interrupted.signal, "SIGKILL")
  const tombstone = join(harness.installRoot, "usr/lib/chariox/.managed-kernel-upgrade.terminal")
  assert.equal((await stat(tombstone)).isDirectory(), true)
  const retried = harness.run()
  assert.equal(retried.status, 0, retried.stderr)
  assert.equal(await lstat(tombstone).then(() => true, () => false), false)
})

test("managed kernel upgrade remains a dedicated offline release operation", async () => {
  const contents = await readFile(upgrade, "utf8")
  const stateContents = await readFile(upgradeState, "utf8")
  const serviceContents = await readFile(managedService, "utf8")
  assert.match(contents, /verify-image-release\.mjs/)
  assert.match(contents, /--print-local-daemon-protocol-version/)
  assert.match(contents, /systemctl stop "\$service_name"/)
  assert.match(contents, /systemctl start "\$service_name"/)
  assert.doesNotMatch(contents, /install-image\.sh/)
  assert.doesNotMatch(contents, /\b(?:groupadd|useradd|usermod|loginctl)\b/)
  assert.doesNotMatch(contents, /systemctl (?:enable|restart)/)
  assert.doesNotMatch(contents, /\b(?:curl|wget|ssh|scp)\b/)
  assert.doesNotMatch(contents, /installation[_-]origin|CHARIOX_INSTALLATION/)
  assert.doesNotMatch(contents, /\.arroba/)
  assert.match(contents, /CHARIOX_MANAGED_UPGRADE_HEALTH_TIMEOUT_MS:-120000/)
  assert.match(contents, /managed_provider_topology=\$\{CHARIOX_MANAGED_PROVIDER_TOPOLOGY-\}/)
  assert.match(contents, /service_name=chariox-path1-managed-bootstrap\.service/)
  assert.match(contents, /select_supervisor_service\(\)/)
  assert.match(contents, /verify_selected_release\(\)/)
  assert.match(contents, /path1_unit=chariox-path1-managed-bootstrap\.service/)
  assert.match(serviceContents, /ExecStartPre=-\+\/usr\/bin\/systemctl restart chariox-slice-broker\.service/)

  const publishRelease = contents.indexOf('mv "$pending_release" "$published_release"')
  const publishTransaction = contents.indexOf('publish-transaction \\', publishRelease)
  const stopService = contents.indexOf('systemctl stop "$service_name"', publishTransaction)
  assert.ok(publishRelease >= 0 && publishTransaction > publishRelease && stopService > publishTransaction)
  const verifyCandidate = contents.indexOf('verify_selected_release "$image_root"', 0)
  const planMigration = contents.indexOf('\nplan_home_migration\n', verifyCandidate)
  const applyMigration = contents.indexOf('resume_home_migration', stopService)
  assert.ok(verifyCandidate >= 0 && planMigration > verifyCandidate)
  assert.ok(planMigration < publishTransaction && applyMigration > stopService)
  assert.match(contents.slice(0, publishRelease), /sync-tree[^\n]*\$pending_release/)
  assert.match(contents.slice(publishRelease, publishTransaction), /sync-directory[^\n]*\$releases_root/)
  assert.match(contents.slice(publishRelease, publishTransaction), /sync-tree[^\n]*\$pending_transaction/)
  assert.match(contents.slice(publishTransaction, stopService), /publish-transaction/)
  assert.match(stateContents, /await rename\(source, destination\)\n  await fsyncDirectory\(dirname\(destination\)\)/)
})
