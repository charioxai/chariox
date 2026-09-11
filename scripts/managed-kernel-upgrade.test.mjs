import assert from "node:assert/strict"
import { createHash, generateKeyPairSync, sign } from "node:crypto"
import { spawnSync } from "node:child_process"
import {
  chmod,
  cp,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readlink,
  readdir,
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

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const upgrade = join(repositoryRoot, "deploy/managed-kernel/upgrade-image.sh")
const serviceName = "chariox-managed-bootstrap.service"

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

async function makeRelease(root, label, protocol, privateKey, publicKey) {
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
  await put(attestation, JSON.stringify({ schemaVersion: 1, label }))
  await put(attestationSignature, Buffer.alloc(64, label.charCodeAt(0)).toString("base64"))
  await put(builderKey, Buffer.alloc(32, protocol % 255).toString("base64"))

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
  for (const [name, path, source, type] of artifactSpecs) {
    artifacts.push({ name, path, sha256: type === "tree" ? await sha256Tree(source) : await sha256File(source) })
  }
  const manifestBytes = Buffer.from(JSON.stringify({
    schemaVersion: 2,
    sourceCommit: createHash("sha1").update(`commit-${label}`).digest("hex"),
    sourceTree: createHash("sha1").update(`tree-${label}`).digest("hex"),
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

async function makeHarness(context, { targetProtocol = 323 } = {}) {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-upgrade-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const { privateKey, publicKey } = generateKeyPairSync("ed25519")
  const trustedKey = join(root, "trusted-release-public-key")
  await put(trustedKey, rawPublicKey(publicKey).toString("base64"), 0o600)
  const current = await makeRelease(root, "current", 323, privateKey, publicKey)
  const target = await makeRelease(root, "target", targetProtocol, privateKey, publicKey)
  const installRoot = join(root, "host")
  const releases = join(installRoot, "usr/lib/chariox/releases")
  const currentRelease = join(releases, current.digest.slice("sha256:".length))
  await mkdir(releases, { recursive: true })
  await cp(current.rootfs, currentRelease, { recursive: true, preserveTimestamps: true })
  await symlink(`releases/${current.digest.slice("sha256:".length)}`, join(installRoot, "usr/lib/chariox/current"))
  const receiptPath = join(installRoot, "var/lib/chariox/home/managed/bootstrap-receipt.json")
  const receipt = {
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
  await put(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`, 0o600)
  const persistent = {
    credential: join(installRoot, "var/lib/chariox/home/.chariox/vault/vault.json"),
    provider: join(installRoot, "var/lib/chariox/provider-home/provider-state.json"),
    repository: join(installRoot, "var/lib/chariox/home/repositories/repo-1/HEAD"),
    workspace: join(installRoot, "var/lib/chariox/home/workspaces/workspace-1/state"),
    session: join(installRoot, "var/lib/chariox/home/sessions/session-1.json"),
  }
  for (const [name, path] of Object.entries(persistent)) await put(path, `${name}-sentinel\n`, 0o600)

  const state = join(root, "harness-state")
  const bin = join(root, "bin")
  await mkdir(state)
  await mkdir(bin)
  await put(join(bin, "systemctl"), `#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$HARNESS_STATE/systemctl.log"
if [ "$1" = "is-active" ] && [ -f "$HARNESS_STATE/fail-health-once" ]; then
  rm -f "$HARNESS_STATE/fail-health-once"
  exit 1
fi
exit 0
`, 0o755)
  await put(join(bin, "node"), `#!/bin/sh
set -eu
if [ "\${1##*/}" = "managed-kernel-upgrade-state.mjs" ] \
  && [ "\${2:-}" = "atomic-symlink" ] \
  && [ -f "$HARNESS_STATE/crash-before-symlink" ]; then
  rm -f "$HARNESS_STATE/crash-before-symlink"
  kill -KILL "$PPID"
  exit 1
fi
exec "${process.execPath}" "$@"
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
    HARNESS_STATE: state,
    CHARIOX_MANAGED_UPGRADE_ROOT: installRoot,
    CHARIOX_MANAGED_UPGRADE_LOCK: join(state, "upgrade.lock"),
    CHARIOX_MANAGED_UPGRADE_HEALTH_HOST: "127.0.0.1",
    CHARIOX_MANAGED_UPGRADE_HEALTH_PORT: String(port),
    CHARIOX_MANAGED_UPGRADE_HEALTH_TIMEOUT_MS: "1000",
  }
  const run = (extraEnv = {}, args = [target.rootfs, current.digest, target.digest, trustedKey]) =>
    spawnSync(upgrade, args, { encoding: "utf8", env: { ...env, ...extraEnv } })
  return { root, installRoot, receiptPath, receipt, persistent, current, target, trustedKey, state, run }
}

async function persistentSnapshot(paths) {
  return Promise.all(Object.entries(paths).map(async ([name, path]) => ({
    name,
    contents: await readFile(path, "utf8"),
    mode: (await stat(path)).mode & 0o777,
  })))
}

test("managed kernel upgrade atomically advances the release and receipt without changing persistent state", async (context) => {
  const harness = await makeHarness(context)
  const before = await persistentSnapshot(harness.persistent)
  const result = harness.run()
  assert.equal(result.status, 0, result.stderr)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.target.digest.slice("sha256:".length)}`,
  )
  const receipt = JSON.parse(await readFile(harness.receiptPath, "utf8"))
  assert.deepEqual(receipt, { ...harness.receipt, runtimeReleaseDigest: harness.target.digest })
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

test("managed kernel upgrade rejects a release with an incompatible local daemon protocol", async (context) => {
  const harness = await makeHarness(context, { targetProtocol: 324 })
  const result = harness.run()
  assert.equal(result.status, 1)
  assert.match(result.stderr, /target local daemon protocol 324 is incompatible with installed protocol 323/)
  assert.equal(
    await readlink(join(harness.installRoot, "usr/lib/chariox/current")),
    `releases/${harness.current.digest.slice("sha256:".length)}`,
  )
  assert.equal(await lstat(join(harness.state, "systemctl.log")).then(() => true, () => false), false)
})

test("managed kernel upgrade remains a dedicated offline release operation", async () => {
  const contents = await readFile(upgrade, "utf8")
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
})
