import assert from "node:assert/strict"
import { createHash, generateKeyPairSync, sign } from "node:crypto"
import { chmod, lstat, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join, relative, sep } from "node:path"
import { spawnSync } from "node:child_process"
import test from "node:test"

const verifier = new URL("./verify-image-release.mjs", import.meta.url)
const SOURCE_COMMIT = "a".repeat(40)
const SOURCE_TREE = "b".repeat(40)
const TARGET = "x86_64-unknown-linux-gnu"
const ARTIFACTS = [
  ["chariox-kernel", "/usr/local/bin/chariox-kernel", "file"],
  ["chariox-managed-bootstrap", "/usr/local/bin/chariox-managed-bootstrap", "file"],
  ["chariox-managed-bootstrap.service", "/etc/systemd/system/chariox-managed-bootstrap.service", "file"],
  ["chariox-path1-managed-bootstrap.service", "/etc/systemd/system/chariox-path1-managed-bootstrap.service", "file"],
  ["chariox-disposable-worker-bootstrap.service", "/etc/systemd/system/chariox-disposable-worker-bootstrap.service", "file"],
  ["chariox-rootless-docker.service", "/etc/systemd/system/chariox-rootless-docker.service", "file"],
  ["chariox-slice-broker.service", "/etc/systemd/system/chariox-slice-broker.service", "file"],
  ["chariox-slice-build-context", "/usr/lib/chariox/slice-build-context", "tree"],
  ["chariox-build-attestation", "/usr/lib/chariox/build-attestation.json", "file"],
  ["chariox-build-attestation-signature", "/usr/lib/chariox/build-attestation.sig", "file"],
  ["chariox-builder-public-key", "/usr/lib/chariox/builder-public-key", "file"],
]
const KERNEL = Buffer.from("kernel artifact")
const BOOTSTRAP = Buffer.from("bootstrap artifact")
const RELAY = Buffer.from("relay artifact")
const PATH1_SERVICE = [
  "[Unit]",
  "After=network-online.target chariox-rootless-docker.service",
  "Wants=network-online.target chariox-rootless-docker.service",
  "[Service]",
  "Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1",
  "Environment=CHARIOX_MANAGED_BOOTSTRAP_PATH=/var/lib/chariox/managed-bootstrap.json",
  "Environment=HOME=/home/chariox",
  "Environment=CHARIOX_HOME=/home/chariox/.chariox",
  "Environment=CHARIOX_SLICE_DOCKER_BROKER_SOCKET=/var/lib/chariox-slice-share/.broker-private/control/control.sock",
  "ExecStartPre=-+/usr/bin/systemctl restart chariox-slice-broker.service",
  "ExecStart=/usr/local/bin/chariox-managed-bootstrap",
  "",
].join("\n")

function sha256(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`
}

function rawPublicKey(key) {
  return key.export({ format: "der", type: "spki" }).subarray(-32).toString("base64")
}

async function put(root, absolutePath, bytes, mode = 0o644) {
  const path = join(root, absolutePath.slice(1))
  await mkdir(dirname(path), { recursive: true, mode: 0o755 })
  await writeFile(path, bytes, { mode })
  await chmod(path, mode)
  return path
}

async function updateTreeHash(root, current, hash) {
  const entries = await readdir(current, { withFileTypes: true })
  entries.sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0)
  for (const entry of entries) {
    const path = join(current, entry.name)
    const pathFromRoot = relative(root, path).split(sep).join("/")
    const metadata = await lstat(path)
    const mode = metadata.mode & 0o7777
    if (metadata.isDirectory()) {
      hash.update(`directory:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:`)
      await updateTreeHash(root, path, hash)
    } else {
      hash.update(`file:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:${metadata.size}:`)
      hash.update(await readFile(path))
    }
  }
}

async function sha256Tree(root) {
  const metadata = await lstat(root)
  const hash = createHash("sha256")
  hash.update(`directory:1:.:${metadata.mode & 0o7777}:`)
  await updateTreeHash(root, root, hash)
  return `sha256:${hash.digest("hex")}`
}

async function createReleaseFixture(context, {
  mutateAttestation = () => {},
  malformedAttestation = false,
  invalidBuilderSignature = false,
  wrongTrustedBuilderKey = false,
} = {}) {
  const scratch = await mkdtemp(join(tmpdir(), "chariox-release-provenance-test-"))
  context.after(() => rm(scratch, { recursive: true, force: true }))
  const rootfs = join(scratch, "rootfs")
  await mkdir(rootfs, { mode: 0o755 })

  const releaseKeys = generateKeyPairSync("ed25519")
  const builderKeys = generateKeyPairSync("ed25519")
  const unrelatedBuilderKeys = generateKeyPairSync("ed25519")
  const trustedReleaseKey = join(scratch, "trusted-release-public-key")
  const trustedBuilderKey = join(scratch, "trusted-builder-public-key")
  await writeFile(trustedReleaseKey, rawPublicKey(releaseKeys.publicKey))
  await writeFile(
    trustedBuilderKey,
    wrongTrustedBuilderKey ? rawPublicKey(unrelatedBuilderKeys.publicKey) : rawPublicKey(builderKeys.publicKey),
  )

  const treeRoot = join(rootfs, "usr/lib/chariox/slice-build-context")
  const relayPath = join(treeRoot, "apps/kernel/slice-linux-docker/prebuilt/chariox-relay")
  await mkdir(dirname(relayPath), { recursive: true, mode: 0o755 })
  await writeFile(relayPath, RELAY, { mode: 0o755 })
  await chmod(relayPath, 0o755)
  for (const directory of [
    treeRoot,
    join(treeRoot, "apps"),
    join(treeRoot, "apps/kernel"),
    join(treeRoot, "apps/kernel/slice-linux-docker"),
    dirname(relayPath),
  ]) {
    await chmod(directory, 0o755)
  }

  const attestation = {
    schemaVersion: 1,
    sourceCommit: SOURCE_COMMIT,
    sourceTree: SOURCE_TREE,
    target: TARGET,
    artifacts: [
      { name: "chariox-kernel", sha256: sha256(KERNEL) },
      { name: "chariox-managed-bootstrap", sha256: sha256(BOOTSTRAP) },
      { name: "chariox-relay", sha256: sha256(RELAY) },
    ],
  }
  mutateAttestation(attestation)
  const attestationBytes = malformedAttestation ? Buffer.from("{") : Buffer.from(JSON.stringify(attestation))
  const attestationSigner = invalidBuilderSignature ? unrelatedBuilderKeys.privateKey : builderKeys.privateKey
  const attestationSignature = Buffer.from(sign(null, attestationBytes, attestationSigner).toString("base64"))
  const embeddedBuilderKey = Buffer.from(rawPublicKey(builderKeys.publicKey))
  const contentByName = new Map([
    ["chariox-kernel", KERNEL],
    ["chariox-managed-bootstrap", BOOTSTRAP],
    ["chariox-managed-bootstrap.service", Buffer.from("[Service]\nExecStart=/usr/local/bin/chariox-managed-bootstrap\n")],
    ["chariox-path1-managed-bootstrap.service", Buffer.from(PATH1_SERVICE)],
    ["chariox-disposable-worker-bootstrap.service", Buffer.from("[Service]\nExecStart=/usr/local/bin/chariox-managed-bootstrap\n")],
    ["chariox-rootless-docker.service", Buffer.from("[Service]\n")],
    ["chariox-slice-broker.service", Buffer.from("[Service]\n")],
    ["chariox-build-attestation", attestationBytes],
    ["chariox-build-attestation-signature", attestationSignature],
    ["chariox-builder-public-key", embeddedBuilderKey],
  ])
  const artifacts = []
  for (const [name, path, type] of ARTIFACTS) {
    let digest
    if (type === "tree") {
      digest = await sha256Tree(join(rootfs, path.slice(1)))
    } else {
      const contents = contentByName.get(name)
      await put(rootfs, path, contents)
      digest = sha256(contents)
    }
    artifacts.push({ name, path, sha256: digest })
  }

  const manifestBytes = Buffer.from(JSON.stringify({
    schemaVersion: 2,
    sourceCommit: SOURCE_COMMIT,
    sourceTree: SOURCE_TREE,
    artifacts,
  }))
  await put(rootfs, "/usr/lib/chariox/release-public-key", Buffer.from(rawPublicKey(releaseKeys.publicKey)))
  await put(rootfs, "/usr/lib/chariox/release-manifest.json", manifestBytes)
  await put(
    rootfs,
    "/usr/lib/chariox/release-manifest.sig",
    Buffer.from(sign(null, manifestBytes, releaseKeys.privateKey).toString("base64")),
  )
  return {
    rootfs,
    digest: sha256(manifestBytes),
    trustedReleaseKey,
    trustedBuilderKey,
  }
}

function runVerifier(fixture, topology, trustedBuilderKeyPath) {
  const args = [verifier.pathname, fixture.rootfs, fixture.digest, fixture.trustedReleaseKey]
  if (topology) args.push(topology)
  if (trustedBuilderKeyPath) args.push(trustedBuilderKeyPath)
  return spawnSync(process.execPath, args, { encoding: "utf8", timeout: 10_000 })
}

test("Path-1 verification requires an independently supplied builder trust root", async (context) => {
  const fixture = await createReleaseFixture(context)
  const result = runVerifier(fixture, "path1")
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /independently supplied trusted builder public key/)
})

test("Path-1 verification refuses to use the image's builder key as its trust root", async (context) => {
  const fixture = await createReleaseFixture(context)
  const embeddedKey = join(fixture.rootfs, "usr/lib/chariox/builder-public-key")
  const result = runVerifier(fixture, "path1", embeddedKey)
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /must be supplied outside the image root/)
})

test("Path-1 verification accepts a valid externally trusted builder attestation", async (context) => {
  const fixture = await createReleaseFixture(context)
  const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
  assert.equal(result.status, 0, result.stderr)
})

test("Path-1 verification rejects an embedded key that differs from its independent trust root", async (context) => {
  const fixture = await createReleaseFixture(context, { wrongTrustedBuilderKey: true })
  const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /not independently trusted/)
})

test("Path-1 verification rejects malformed builder attestations", async (context) => {
  const fixture = await createReleaseFixture(context, { malformedAttestation: true })
  const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /builder attestation is invalid JSON/)
})

test("Path-1 verification rejects invalid builder signatures", async (context) => {
  const fixture = await createReleaseFixture(context, { invalidBuilderSignature: true })
  const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /builder attestation signature is invalid/)
})

for (const [field, mutateAttestation] of [
  ["source commit", (attestation) => { attestation.sourceCommit = "c".repeat(40) }],
  ["source tree", (attestation) => { attestation.sourceTree = "d".repeat(40) }],
  ["target", (attestation) => { attestation.target = "aarch64-unknown-linux-gnu" }],
  ["kernel hash", (attestation) => { attestation.artifacts[0].sha256 = `sha256:${"e".repeat(64)}` }],
  ["bootstrap hash", (attestation) => { attestation.artifacts[1].sha256 = `sha256:${"f".repeat(64)}` }],
  ["relay hash", (attestation) => { attestation.artifacts[2].sha256 = `sha256:${"1".repeat(64)}` }],
]) {
  test(`Path-1 verification rejects an attestation with a mismatched ${field}`, async (context) => {
    const fixture = await createReleaseFixture(context, { mutateAttestation })
    const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /builder attestation|attestation artifacts/)
  })
}

test("shared-host rollback retains release-signature verification without a new builder pin", async (context) => {
  const fixture = await createReleaseFixture(context, { malformedAttestation: true })
  const result = runVerifier(fixture, "shared_host")
  assert.equal(result.status, 0, result.stderr)
})
