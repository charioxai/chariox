import assert from "node:assert/strict"
import { createHash, generateKeyPairSync, sign } from "node:crypto"
import { chmod, lstat, mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join, relative, sep } from "node:path"
import { spawnSync } from "node:child_process"
import test from "node:test"
import {
  PATH1_DATA_VOLUME_ARTIFACTS,
  stageReleaseFixtureSourceAssets,
} from "./verify-image-release-fixture-helper.mjs"

const verifier = new URL("./verify-image-release.mjs", import.meta.url)
const SOURCE_COMMIT = "a".repeat(40)
const SOURCE_TREE = "b".repeat(40)
const TARGET = "x86_64-unknown-linux-gnu"
const PATH1_HOME_EXEC_START = "ExecStart=/usr/local/bin/chariox-managed-bootstrap"
const PATH1_BOOTSTRAP_PATH = "Environment=PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
const PATH1_SERVICE = await readFile(new URL("./chariox-path1-managed-bootstrap.service", import.meta.url), "utf8")
const ARTIFACTS = [
  ["chariox-kernel", "/usr/local/bin/chariox-kernel", "file"],
  ["chariox-managed-bootstrap", "/usr/local/bin/chariox-managed-bootstrap", "file"],
  ["chariox-managed-bootstrap.service", "/etc/systemd/system/chariox-managed-bootstrap.service", "file"],
  ["chariox-path1-managed-bootstrap.service", "/etc/systemd/system/chariox-path1-managed-bootstrap.service", "file"],
  ["chariox-rootless-docker.service", "/etc/systemd/system/chariox-rootless-docker.service", "file"],
  ...PATH1_DATA_VOLUME_ARTIFACTS.map(({ name, path }) => [name, path, "file"]),
  ["chariox-slice-broker.service", "/etc/systemd/system/chariox-slice-broker.service", "file"],
  ["chariox-slice-image", "/usr/lib/chariox/slice-image", "tree"],
  ["chariox-slice-build-context", "/usr/lib/chariox/slice-build-context", "tree"],
  ["chariox-build-attestation", "/usr/lib/chariox/build-attestation.json", "file"],
  ["chariox-build-attestation-signature", "/usr/lib/chariox/build-attestation.sig", "file"],
  ["chariox-builder-public-key", "/usr/lib/chariox/builder-public-key", "file"],
  ["chariox-app-package", "/usr/local/bin/chariox-app-package", "file"],
  ["chariox-app-wasm-worker", "/usr/local/bin/chariox-app-wasm-worker", "file"],
]
const KERNEL = Buffer.from("kernel artifact")
const BOOTSTRAP = Buffer.from("bootstrap artifact")
const RELAY = Buffer.from("relay artifact")
const APP_PACKAGE = Buffer.from("app package artifact")
const APP_STORAGE = Buffer.from("app storage artifact")
const APP_WASM_WORKER = Buffer.from("app wasm worker artifact")
// Releases built before Apps carry none of these.
const APP_ARTIFACT_NAMES = new Set(["chariox-app-package", "chariox-app-wasm-worker"])
const DATA_VOLUME_ARTIFACT_NAMES = new Set(PATH1_DATA_VOLUME_ARTIFACTS.map(({ name }) => name))
const PATH1_SERVICE_ARTIFACT_NAMES = new Set(["chariox-path1-managed-bootstrap.service"])
const WORKER_SERVICE_NAME = "chariox-disposable-worker-bootstrap.service"
const WORKER_SERVICE_PATH = "/etc/systemd/system/chariox-disposable-worker-bootstrap.service"
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
  mutateManifest = () => {},
  malformedAttestation = false,
  invalidBuilderSignature = false,
  wrongTrustedBuilderKey = false,
  includePath1Services = true,
  dataVolumeArtifactNames = [...DATA_VOLUME_ARTIFACT_NAMES],
  appArtifactNames = [...APP_ARTIFACT_NAMES],
  path1Service = PATH1_SERVICE,
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

  const contentByName = await stageReleaseFixtureSourceAssets(rootfs)
  contentByName.set(
    "chariox-path1-managed-bootstrap.service",
    Buffer.from(path1Service),
  )

  await put(rootfs, "/usr/lib/chariox/slice-image/chariox-slice-linux.tar.gz", "image fixture")
  const treeRoot = join(rootfs, "usr/lib/chariox/slice-build-context")
  const relayPath = join(treeRoot, "apps/kernel/slice-linux-docker/prebuilt/chariox-relay")
  const kernelPath = join(treeRoot, "apps/kernel/slice-linux-docker/prebuilt/chariox-kernel")
  const releaseMarkerPath = join(treeRoot, "apps/kernel/slice-linux-docker/prebuilt/.managed-release")
  await mkdir(dirname(relayPath), { recursive: true, mode: 0o755 })
  await writeFile(relayPath, RELAY, { mode: 0o755 })
  await chmod(relayPath, 0o755)
  await writeFile(kernelPath, KERNEL, { mode: 0o755 })
  await chmod(kernelPath, 0o755)
  await writeFile(releaseMarkerPath, "builder-attested\n", { mode: 0o644 })
  await chmod(releaseMarkerPath, 0o644)
  for (const directory of [
    treeRoot,
    join(treeRoot, "apps"),
    join(treeRoot, "apps/kernel"),
    join(treeRoot, "apps/kernel/slice-linux-docker"),
    join(treeRoot, "apps/kernel/slice-linux-docker/prebuilt"),
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
      ...(appArtifactNames.length > 0 ? [
        { name: "chariox-app-package", sha256: sha256(APP_PACKAGE) },
        { name: "chariox-app-wasm-worker", sha256: sha256(APP_WASM_WORKER) },
      ] : []),
    ],
  }
  mutateAttestation(attestation)
  const attestationBytes = malformedAttestation ? Buffer.from("{") : Buffer.from(JSON.stringify(attestation))
  const attestationSigner = invalidBuilderSignature ? unrelatedBuilderKeys.privateKey : builderKeys.privateKey
  const attestationSignature = Buffer.from(sign(null, attestationBytes, attestationSigner).toString("base64"))
  const embeddedBuilderKey = Buffer.from(rawPublicKey(builderKeys.publicKey))
  contentByName.set("chariox-kernel", KERNEL)
  contentByName.set("chariox-managed-bootstrap", BOOTSTRAP)
  contentByName.set("chariox-build-attestation", attestationBytes)
  contentByName.set("chariox-build-attestation-signature", attestationSignature)
  contentByName.set("chariox-builder-public-key", embeddedBuilderKey)
  contentByName.set("chariox-app-package", APP_PACKAGE)
  contentByName.set("chariox-app-wasm-worker", APP_WASM_WORKER)
  const requestedDataVolumeArtifacts = new Set(dataVolumeArtifactNames)
  const requestedAppArtifacts = new Set(appArtifactNames)
  const fixtureArtifacts = ARTIFACTS.filter(([name]) => {
    if (!includePath1Services && PATH1_SERVICE_ARTIFACT_NAMES.has(name)) return false
    if (APP_ARTIFACT_NAMES.has(name)) return requestedAppArtifacts.has(name)
    return !DATA_VOLUME_ARTIFACT_NAMES.has(name) || requestedDataVolumeArtifacts.has(name)
  })
  const artifacts = []
  for (const [name, path, type] of fixtureArtifacts) {
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

  const manifest = {
    schemaVersion: 2,
    sourceCommit: SOURCE_COMMIT,
    sourceTree: SOURCE_TREE,
    artifacts,
  }
  mutateManifest(manifest)
  const manifestBytes = Buffer.from(JSON.stringify(manifest))
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

test("signed release manifests retain legacy schema 2 and require schema 3 capability 1", async (context) => {
  for (const schemaVersion of [2, 3]) {
    const fixture = await createReleaseFixture(context, { mutateManifest(manifest) {
      manifest.schemaVersion = schemaVersion
      if (schemaVersion === 3) manifest.managedUpdateEvidenceVersion = 1
    } })
    const result = runVerifier(fixture)
    assert.equal(result.status, 0, result.stderr)
  }
  for (const [schemaVersion, capability] of [[3, undefined], [3, 2], [3, null], [2, 1]]) {
    const fixture = await createReleaseFixture(context, { mutateManifest(manifest) {
      manifest.schemaVersion = schemaVersion
      if (capability !== undefined) manifest.managedUpdateEvidenceVersion = capability
    } })
    const result = runVerifier(fixture)
    assert.equal(result.status, 1, result.stderr)
    assert.match(result.stderr, /release manifest schema is unsupported|release manifest contains unsupported fields/)
  }
})


for (const path of [
  "/extra",
  "/usr/local/bin/extra",
  "/usr/lib/chariox/extra",
  "/etc/systemd/system/chariox-path1-managed-bootstrap.service.d/99-extra.conf",
  "/etc/systemd/system/chariox-rootless-docker.service.d/99-extra.conf",
]) {
  test(`Path-1 closed inventory rejects an undeclared file at ${path}`, async (context) => {
    const fixture = await createReleaseFixture(context)
    await put(fixture.rootfs, path, "undeclared\n")
    const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
    assert.equal(result.status, 1, result.stderr)
    assert.match(result.stderr, /release root contains an undeclared entry/)
  })
}

for (const path of ["empty", "usr/local/bin/empty", "usr/lib/chariox/empty"]) {
  test(`Path-1 closed inventory rejects an undeclared empty directory at ${path}`, async (context) => {
    const fixture = await createReleaseFixture(context)
    await mkdir(join(fixture.rootfs, path))
    const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
    assert.equal(result.status, 1, result.stderr)
    assert.match(result.stderr, /release root contains an undeclared entry/)
  })
}

for (const path of ["extra", "usr/local/bin/extra", "usr/lib/chariox/extra"]) {
  test(`Path-1 closed inventory rejects a symbolic link at ${path}`, async (context) => {
    const fixture = await createReleaseFixture(context)
    await symlink("/usr/local/bin/chariox-kernel", join(fixture.rootfs, path))
    const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
    assert.equal(result.status, 1, result.stderr)
    assert.match(result.stderr, /contains a symbolic link/)
  })
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

test("Path-1 verification rejects a signed release with no data-volume admission artifacts", async (context) => {
  const fixture = await createReleaseFixture(context, { dataVolumeArtifactNames: [] })
  const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
  assert.notEqual(result.status, 0)
  assert.match(
    result.stderr,
    /^verify-image-release\.mjs: Path-1 releases must include data-volume admission and both ordering drop-ins/,
  )
})

test("Path-1 verification rejects a signed release with a partial data-volume artifact set", async (context) => {
  const fixture = await createReleaseFixture(context, {
    dataVolumeArtifactNames: [PATH1_DATA_VOLUME_ARTIFACTS[0].name, PATH1_DATA_VOLUME_ARTIFACTS[1].name],
  })
  const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
  assert.notEqual(result.status, 0)
  assert.match(
    result.stderr,
    /^verify-image-release\.mjs: release contains an incomplete Path-1 data-volume admission artifact set/,
  )
})

test("Path-1 verification accepts signed direct ExecStart commands and static bootstrap PATHs", async (context) => {
  const fixture = await createReleaseFixture(context)
  const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
  assert.equal(result.status, 0, result.stderr)
})

for (const [description, fixtureOptions] of [
  ["a login shell that could read a provider-writable profile", {
    path1Service: PATH1_SERVICE.replace(
      PATH1_HOME_EXEC_START,
      "ExecStart=/bin/bash --login -c 'exec /usr/local/bin/chariox-managed-bootstrap'",
    ),
  }],
  ["an extra Path-1 command", {
    path1Service: PATH1_SERVICE.replace(
      PATH1_HOME_EXEC_START,
      `${PATH1_HOME_EXEC_START}; /tmp/untrusted`,
    ),
  }],
  ["an untrusted Path-1 executable", {
    path1Service: PATH1_SERVICE.replace(PATH1_HOME_EXEC_START, PATH1_HOME_EXEC_START.replace(
      "/usr/local/bin/chariox-managed-bootstrap",
      "/tmp/chariox-managed-bootstrap",
    )),
  }],
  ["multiple Path-1 ExecStart directives", {
    path1Service: `${PATH1_SERVICE}ExecStart=/tmp/untrusted\n`,
  }],
  ["a home command with the removed disposable-worker flag", {
    path1Service: PATH1_SERVICE.replace(PATH1_HOME_EXEC_START, `${PATH1_HOME_EXEC_START} --disposable-worker`),
  }],
  ["a user-writable home bootstrap PATH", {
    path1Service: PATH1_SERVICE.replace(
      PATH1_BOOTSTRAP_PATH,
      "Environment=PATH=/home/chariox/.local/bin:/usr/local/bin:/usr/bin:/bin",
    ),
  }],
  ["multiple home bootstrap PATH declarations", {
    path1Service: `${PATH1_SERVICE}${PATH1_BOOTSTRAP_PATH}\n`,
  }],
]) {
  test(`Path-1 verification rejects ${description}`, async (context) => {
    const fixture = await createReleaseFixture(context, fixtureOptions)
    const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /incompatible ExecStart|incompatible bootstrap PATH|overrides Environment=PATH/)
  })
}

test("Path-1 verification rejects a signed service without the independent runtime builder key", async (context) => {
  const fixture = await createReleaseFixture(context, {
    path1Service: PATH1_SERVICE.replace(
      "Environment=CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY=/etc/chariox/trusted-builder-public-key\n",
      "",
    ),
  })
  const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY/)
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
  ["App package hash", (attestation) => { attestation.artifacts[3].sha256 = `sha256:${"2".repeat(64)}` }],
  ["App WASM worker hash", (attestation) => { attestation.artifacts[4].sha256 = `sha256:${"4".repeat(64)}` }],
  ["App binary set", (attestation) => { attestation.artifacts.splice(3) }],
  ["App WASM worker missing from the attested list", (attestation) => { attestation.artifacts.splice(4) }],
]) {
  test(`Path-1 verification rejects an attestation with a mismatched ${field}`, async (context) => {
    const fixture = await createReleaseFixture(context, { mutateAttestation })
    const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /builder attestation|attestation artifacts/)
  })
}

test("shared-host rollback retains direct ExecStart and release-signature verification without a builder pin", async (context) => {
  // Model a legacy schema-2 rollback before Path-1 home, storage and App assets existed.
  const fixture = await createReleaseFixture(context, {
    malformedAttestation: true,
    includePath1Services: false,
    dataVolumeArtifactNames: [],
    appArtifactNames: [],
  })
  const result = runVerifier(fixture, "shared_host")
  assert.equal(result.status, 0, result.stderr)
})

test("a Path-1 release built before Apps stays verifiable with its three attested binaries", async (context) => {
  const fixture = await createReleaseFixture(context, { appArtifactNames: [] })
  const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
  assert.equal(result.status, 0, result.stderr)
})

test("release verification rejects a partial or undeclared App artifact set", async (context) => {
  const partial = await createReleaseFixture(context, { appArtifactNames: ["chariox-app-package"] })
  const partialResult = runVerifier(partial, "path1", partial.trustedBuilderKey)
  assert.notEqual(partialResult.status, 0)
  assert.match(partialResult.stderr, /release contains an incomplete App artifact set/)

  // The WASM worker is what lets a managed kernel run portable Apps: a release without it must not install.
  const withoutWorker = await createReleaseFixture(context, {
    appArtifactNames: ["chariox-app-package"],
  })
  const withoutWorkerResult = runVerifier(withoutWorker, "path1", withoutWorker.trustedBuilderKey)
  assert.notEqual(withoutWorkerResult.status, 0)
  assert.match(withoutWorkerResult.stderr, /release contains an incomplete App artifact set/)

  const undeclared = await createReleaseFixture(context, { appArtifactNames: [] })
  await put(undeclared.rootfs, "/usr/local/bin/chariox-app-wasm-worker", APP_WASM_WORKER, 0o755)
  const undeclaredResult = runVerifier(undeclared, "path1", undeclared.trustedBuilderKey)
  assert.notEqual(undeclaredResult.status, 0)
  assert.match(undeclaredResult.stderr, /release contains an undeclared App artifact: chariox-app-wasm-worker/)
})

// MP-01/MP-07/MP-11: valid signatures must not admit unreviewed service restrictions.
for (const directive of ["NoExecPaths=/home /tmp", "ProtectProc=invisible", "MemoryDenyWriteExecute=yes"]) {
  test(`MP-01/MP-07/MP-11 signed home rejects ${directive}`, async (context) => {
    const fixture = await createReleaseFixture(context, {
      path1Service: PATH1_SERVICE.replace("[Service]", `[Service]\n${directive}`),
    })
    const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
    assert.notEqual(result.status, 0, "signed provider restriction must fail before installation")
    assert.match(result.stderr, /unsupported Service directive/)
  })
}

// MP-01/MP-04/MP-07/MP-11: independently signed home policy mutations.
for (const directive of [
  "PrivateTmp=true", "ProtectHome=true", "NoNewPrivileges=true", "UMask=0077",
  "Environment=CHARIOX_MANAGED_PROVIDER_ISOLATION=1",
  "Environment=HOME=/tmp/wrong-home",
  "Environment=CHARIOX_HOME=/tmp/wrong-state",
  "Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=shared_host",
  "Environment=CHARIOX_SLICE_DOCKER_BROKER_SOCKET=/tmp/wrong.sock",
  "ExecStartPre=/bin/true", "StateDirectory=extra-state",
]) {
  test(`MP-11 signed home rejects ${directive}`, async (context) => {
    const fixture = await createReleaseFixture(context, {
      path1Service: PATH1_SERVICE.replace("[Service]", `[Service]\n${directive}`),
    })
    const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
    assert.notEqual(result.status, 0, result.stderr)
    assert.match(result.stderr, /Path-1 managed bootstrap service/)
  })
}

test("MP-11 home HOME in another unit section cannot satisfy service policy", async (context) => {
  const fixture = await createReleaseFixture(context, {
    path1Service: PATH1_SERVICE.replace("Environment=HOME=/home/chariox\n", "")
      .replace("[Install]", "[Install]\nEnvironment=HOME=/home/chariox"),
  })
  const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
  assert.notEqual(result.status, 0, result.stderr)
  assert.match(result.stderr, /Path-1 managed bootstrap service/)
})

// The disposable-worker supervisor is gone: a release that still carries its unit is unsupported.
test("release verification refuses a release that carries the removed disposable-worker service", async (context) => {
  for (const topology of [undefined, "path1"]) {
    const undeclared = await createReleaseFixture(context)
    await put(undeclared.rootfs, WORKER_SERVICE_PATH, "[Service]\nExecStart=/usr/local/bin/chariox-managed-bootstrap --disposable-worker\n")
    const undeclaredResult = runVerifier(undeclared, topology, topology && undeclared.trustedBuilderKey)
    assert.equal(undeclaredResult.status, 1, undeclaredResult.stderr)
    assert.match(undeclaredResult.stderr, /release carries the removed disposable-worker service/)

    const declared = await createReleaseFixture(context, { mutateManifest(manifest) {
      manifest.artifacts.push({ name: WORKER_SERVICE_NAME, path: WORKER_SERVICE_PATH, sha256: sha256(Buffer.from("unit")) })
    } })
    const declaredResult = runVerifier(declared, topology, topology && declared.trustedBuilderKey)
    assert.equal(declaredResult.status, 1, declaredResult.stderr)
    assert.match(declaredResult.stderr, /release carries the removed disposable-worker service/)
  }
})

// MP-01/MP-07/MP-11: equivalent filesystem/network restrictions in the home role.
for (const directive of ["TemporaryFileSystem=/home:ro", "IPAddressDeny=any", "PrivateMounts=yes"]) {
  test(`MP-11 signed home rejects ${directive}`, async (context) => {
    const fixture = await createReleaseFixture(context, {
      path1Service: PATH1_SERVICE.replace("[Service]", `[Service]\n${directive}`),
    })
    const result = runVerifier(fixture, "path1", fixture.trustedBuilderKey)
    assert.notEqual(result.status, 0, result.stderr)
    assert.match(result.stderr, /Path-1 managed bootstrap service/)
  })
}
