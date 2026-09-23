import assert from "node:assert/strict"
import { createHash, generateKeyPairSync, sign, verify } from "node:crypto"
import { chmod, lstat, mkdir, mkdtemp, readFile, readdir, readlink, rename, rm, stat, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, relative, sep } from "node:path"
import { fileURLToPath } from "node:url"
import { spawn, spawnSync } from "node:child_process"
import { once } from "node:events"
import { test } from "node:test"

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const packager = join(repositoryRoot, "scripts/package-managed-kernel-release.mjs")
const builder = join(repositoryRoot, "scripts/build-managed-kernel-release.mjs")
const installer = join(repositoryRoot, "deploy/managed-kernel/install-image.sh")
const verifier = join(repositoryRoot, "deploy/managed-kernel/verify-image-release.mjs")
const service = join(repositoryRoot, "deploy/managed-kernel/chariox-managed-bootstrap.service")
const path1Service = join(repositoryRoot, "deploy/managed-kernel/chariox-path1-managed-bootstrap.service")
const workerService = join(repositoryRoot, "deploy/managed-kernel/chariox-disposable-worker-bootstrap.service")
const rootlessDockerService = join(repositoryRoot, "deploy/managed-kernel/chariox-rootless-docker.service")
const sliceBrokerService = join(repositoryRoot, "deploy/managed-kernel/chariox-slice-broker.service")
const sourceDateEpoch = "946684800"

test("managed prebuilt slice runtime materializes its runtime output directory", async () => {
  const dockerfile = await readFile(
    join(repositoryRoot, "apps/kernel/slice-linux-docker/docker/Dockerfile"),
    "utf8",
  )
  const start = dockerfile.indexOf("RUN mkdir -p /opt/chariox-runtime-bin")
  const end = dockerfile.indexOf("\n\nFROM ", start)
  assert.notEqual(start, -1, "prebuilt runtime branch is missing")
  assert.notEqual(end, -1, "prebuilt runtime branch has no stage boundary")

  const fixture = await mkdtemp(join(tmpdir(), "chariox-prebuilt-slice-"))
  const prebuilt = join(fixture, "prebuilt")
  const runtimeBin = join(fixture, "runtime-bin")
  try {
    await mkdir(prebuilt, { recursive: true })
    for (const name of ["chariox-kernel", "chariox-relay"]) {
      const path = join(prebuilt, name)
      await writeFile(path, `${name}\n`)
      await chmod(path, 0o755)
    }
    const command = dockerfile
      .slice(start + "RUN ".length, end)
      .replaceAll("\\\n", " ")
      .replaceAll("/opt/chariox-prebuilt", prebuilt)
      .replaceAll("/opt/chariox-runtime-bin", runtimeBin)
    const result = spawnSync("/bin/sh", ["-c", command], {
      cwd: fixture,
      encoding: "utf8",
      env: { ...process.env, CHARIOX_PREBUILT_RUNTIME: "1" },
    })
    assert.equal(result.status, 0, result.stderr)
    assert.equal(await readFile(join(runtimeBin, "chariox-kernel"), "utf8"), "chariox-kernel\n")
    assert.equal(await readFile(join(runtimeBin, "chariox-relay"), "utf8"), "chariox-relay\n")
  } finally {
    await rm(fixture, { recursive: true, force: true })
  }
})

async function waitForPath(path, timeoutMs = 5000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (await lstat(path).then(() => true, () => false)) return
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20))
  }
  throw new Error(`timed out waiting for ${path}`)
}

async function withTimeout(promise, message, timeoutMs = 10_000) {
  let timer
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(message)), timeoutMs)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

function processGroupId(pid) {
  const result = spawnSync("ps", ["-o", "pgid=", "-p", String(pid)], { encoding: "utf8" })
  assert.equal(result.status, 0, result.stderr)
  const pgid = Number(result.stdout.trim())
  assert.ok(Number.isSafeInteger(pgid) && pgid > 0, `invalid installer PGID for ${pid}`)
  return pgid
}

function killProcessGroup(pgid) {
  if (!pgid) return
  try { process.kill(-pgid, "SIGKILL") } catch {}
}

function processGroupSnapshot(pgid) {
  const result = spawnSync("ps", ["-eo", "pid=,ppid=,pgid=,sid=,stat=,args="], { encoding: "utf8" })
  return result.stdout.split("\n").filter((line) => line.trim().split(/\s+/)[2] === String(pgid))
}

async function waitForProcessGroupExit(pgid, label, timeoutMs = 5000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (processGroupSnapshot(pgid).length === 0) return
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20))
  }
  throw new Error(`${label} PGID ${pgid} did not exit: ${processGroupSnapshot(pgid).join(" | ")}`)
}

function rawPublicKey(publicKey) {
  const der = publicKey.export({ format: "der", type: "spki" })
  return der.subarray(der.length - 32)
}

function packagerArguments({
  kernel,
  supervisor,
  relay,
  builderAttestation,
  builderAttestationSignature,
  trustedBuilderPublicKey,
  signingKey,
  sourceRepository,
  sourceCommit,
  output,
}) {
  return [
    packager,
    "--kernel", kernel,
    "--supervisor", supervisor,
    "--relay", relay,
    "--builder-attestation", builderAttestation,
    "--builder-attestation-signature", builderAttestationSignature,
    "--trusted-builder-public-key", trustedBuilderPublicKey,
    "--signing-key", signingKey,
    "--source-repository", sourceRepository,
    "--source-commit", sourceCommit,
    "--output", output,
  ]
}

function runPackager(options, umask = "022", extraEnvironment = {}) {
  return spawnSync(
    "/bin/sh",
    ["-c", 'umask "$1"; shift; exec "$@"', "chariox-release-packager", umask, process.execPath, ...packagerArguments(options)],
    { encoding: "utf8", env: { ...process.env, ...extraEnvironment, SOURCE_DATE_EPOCH: sourceDateEpoch } },
  )
}

function runVerifier(rootfs, digest, trustedPublicKey, topology) {
  return spawnSync(
    process.execPath,
    [verifier, rootfs, digest, trustedPublicKey, ...(topology ? [topology] : [])],
    { encoding: "utf8" },
  )
}

async function snapshotTree(root, current = root) {
  const metadata = await lstat(current)
  const record = {
    path: relative(root, current) || ".",
    mode: metadata.mode & 0o777,
    mtimeMs: metadata.mtimeMs,
    type: metadata.isDirectory() ? "directory" : metadata.isFile() ? "file" : "unsupported",
  }
  if (metadata.isFile()) record.sha256 = createHash("sha256").update(await readFile(current)).digest("hex")
  const records = [record]
  if (metadata.isDirectory()) {
    for (const name of (await readdir(current)).sort()) records.push(...(await snapshotTree(root, join(current, name))))
  }
  return records
}

async function updateTreeHash(root, current, hash) {
  for (const name of (await readdir(current)).sort()) {
    const path = join(current, name)
    const metadata = await lstat(path)
    const pathFromRoot = relative(root, path).split(sep).join("/")
    const mode = metadata.mode & 0o7777
    if (metadata.isDirectory()) {
      hash.update(`directory:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:`)
      await updateTreeHash(root, path, hash)
      continue
    }
    hash.update(`file:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:${metadata.size}:`)
    hash.update(await readFile(path))
  }
}

async function treeDigest(root) {
  const hash = createHash("sha256")
  const metadata = await lstat(root)
  hash.update(`directory:1:.:${metadata.mode & 0o7777}:`)
  await updateTreeHash(root, root, hash)
  return `sha256:${hash.digest("hex")}`
}

async function makeFixture(root, variant = "") {
  const kernel = join(root, "chariox-kernel")
  const supervisor = join(root, "chariox-managed-bootstrap")
  const relay = join(root, "chariox-relay")
  const signingKey = join(root, "release-key.pem")
  const trustedPublicKey = join(root, "trusted-release-public-key")
  const kernelContents = variant ? `kernel fixture ${variant}\n` : "kernel fixture\n"
  const relayContents = variant ? `relay fixture ${variant}\n` : "relay fixture\n"
  await writeFile(kernel, kernelContents, { mode: 0o755 })
  await writeFile(supervisor, "supervisor fixture\n", { mode: 0o755 })
  await writeFile(relay, relayContents, { mode: 0o755 })
  const { privateKey, publicKey } = generateKeyPairSync("ed25519")
  await writeFile(signingKey, privateKey.export({ format: "pem", type: "pkcs8" }), { mode: 0o600 })
  await writeFile(trustedPublicKey, rawPublicKey(publicKey).toString("base64"), { mode: 0o600 })
  const sourceRepository = join(root, "source")
  await mkdir(sourceRepository, { recursive: true })
  const sourceFiles = new Map([
    ["Cargo.toml", "[workspace]\nmembers = []\n"],
    ["Cargo.lock", "version = 4\n"],
    ["adapters/rust/Cargo.toml", "[package]\nname = \"adapter-fixture\"\n"],
    ["apps/aegs-dummy/Cargo.toml", "[package]\nname = \"aegs-fixture\"\n"],
    ["apps/kernel/Cargo.toml", "[package]\nname = \"kernel-fixture\"\n"],
    ["apps/kernel/slice-linux-docker/docker/Dockerfile", "FROM fixture@sha256:0000000000000000000000000000000000000000000000000000000000000000\n"],
    [
      "apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
      await readFile(join(repositoryRoot, "apps/kernel/slice-linux-docker/managed-docker-broker.mjs")),
    ],
    ["apps/kernel/slice-linux-docker/enter-rootless-docker-namespace.sh", "#!/bin/sh\nexec \"$@\"\n"],
    ...await Promise.all(["managed-rootless-service.sh", "chariox-rootless-engine.service", "chariox-rootless-user-manager.conf"].map(async (name) => {
      const path = `apps/kernel/slice-linux-docker/${name}`
      return [path, await readFile(join(repositoryRoot, path))]
    })),
    ["apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh", "#!/bin/sh\nSLICE_BUILD_IMAGE=fixture\n"],
    ["apps/kernel/slice-linux-docker/managed-publication-access.sh", "#!/bin/sh\nexit 0\n"],
    [
      "apps/kernel/slice-linux-docker/managed-publication-acl.awk",
      await readFile(join(repositoryRoot, "apps/kernel/slice-linux-docker/managed-publication-acl.awk")),
    ],
    ["apps/kernel/slice-linux-docker/toolchain/package-lock.json", "{\"lockfileVersion\":3}\n"],
    ["apps/kernel/src/transport/relay_peer.rs", "pub const RELAY_PEER_PROTOCOL_VERSION: u32 = 1;\n"],
    ["apps/relay/Cargo.toml", "[package]\nname = \"relay-fixture\"\n"],
    ["deploy/managed-kernel/chariox-managed-bootstrap.service", await readFile(service)],
    ["deploy/managed-kernel/chariox-path1-managed-bootstrap.service", await readFile(path1Service)],
    ["deploy/managed-kernel/chariox-disposable-worker-bootstrap.service", await readFile(workerService)],
    ["deploy/managed-kernel/chariox-rootless-docker.service", await readFile(rootlessDockerService)],
    ["deploy/managed-kernel/chariox-slice-broker.service", await readFile(sliceBrokerService)],
    ["examples/workflow-code/example.md", "workflow fixture\n"],
    ["packages/aegs-sdk/Cargo.toml", "[package]\nname = \"sdk-fixture\"\n"],
    ["packages/event-protocol/Cargo.toml", "[package]\nname = \"event-fixture\"\n"],
  ])
  if (variant) sourceFiles.set("release-variant.txt", `${variant}\n`)
  for (const [path, contents] of sourceFiles) {
    const destination = join(sourceRepository, path)
    await mkdir(join(destination, ".."), { recursive: true })
    await writeFile(destination, contents, {
      mode: path.endsWith(".sh") ? 0o755 : 0o644,
    })
  }
  const git = (args) => spawnSync("git", args, {
    cwd: sourceRepository,
    encoding: "utf8",
    env: {
      ...process.env,
      GIT_AUTHOR_NAME: "Release Fixture",
      GIT_AUTHOR_EMAIL: "release@example.invalid",
      GIT_AUTHOR_DATE: "2000-01-01T00:00:00Z",
      GIT_COMMITTER_NAME: "Release Fixture",
      GIT_COMMITTER_EMAIL: "release@example.invalid",
      GIT_COMMITTER_DATE: "2000-01-01T00:00:00Z",
    },
  })
  assert.equal(git(["init", "-q"]).status, 0)
  assert.equal(git(["add", "."]).status, 0)
  const committed = git(["commit", "-q", "-m", "fixture"])
  assert.equal(committed.status, 0, committed.stderr)
  const sourceCommit = git(["rev-parse", "HEAD"]).stdout.trim()
  const sourceTree = git(["rev-parse", "HEAD^{tree}"]).stdout.trim()
  const builderAttestation = join(root, "build-attestation.json")
  const builderAttestationSignature = join(root, "build-attestation.sig")
  const trustedBuilderPublicKey = join(root, "trusted-builder-public-key")
  const builderKeys = generateKeyPairSync("ed25519")
  const binaryDigest = (path) => `sha256:${createHash("sha256").update(path).digest("hex")}`
  const attestationBytes = Buffer.from(JSON.stringify({
    schemaVersion: 1,
    sourceCommit,
    sourceTree,
    target: "x86_64-unknown-linux-gnu",
    artifacts: [
      { name: "chariox-kernel", sha256: binaryDigest(kernelContents) },
      { name: "chariox-managed-bootstrap", sha256: binaryDigest("supervisor fixture\n") },
      { name: "chariox-relay", sha256: binaryDigest(relayContents) },
    ],
  }))
  await writeFile(builderAttestation, attestationBytes, { mode: 0o644 })
  await writeFile(
    builderAttestationSignature,
    sign(null, attestationBytes, builderKeys.privateKey).toString("base64"),
    { mode: 0o644 },
  )
  await writeFile(trustedBuilderPublicKey, rawPublicKey(builderKeys.publicKey).toString("base64"), {
    mode: 0o600,
  })
  return {
    kernel,
    kernelContents,
    supervisor,
    relay,
    relayContents,
    signingKey,
    trustedPublicKey,
    builderAttestation,
    builderAttestationSignature,
    trustedBuilderPublicKey,
    builderPublicKey: builderKeys.publicKey,
    builderPrivateKey: builderKeys.privateKey,
    publicKey,
    releasePrivateKey: privateKey,
    sourceRepository,
    sourceCommit,
    sourceTree,
    serviceBytes: sourceFiles.get("deploy/managed-kernel/chariox-managed-bootstrap.service"),
    path1ServiceBytes: sourceFiles.get("deploy/managed-kernel/chariox-path1-managed-bootstrap.service"),
    workerServiceBytes: sourceFiles.get("deploy/managed-kernel/chariox-disposable-worker-bootstrap.service"),
    rootlessDockerServiceBytes: sourceFiles.get("deploy/managed-kernel/chariox-rootless-docker.service"),
    sliceBrokerServiceBytes: sourceFiles.get("deploy/managed-kernel/chariox-slice-broker.service"),
  }
}

test("managed kernel release packages one reproducible signed rootfs", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const firstOutput = join(root, "release-one")
  const secondOutput = join(root, "release-two")
  const first = runPackager({ ...fixture, output: firstOutput }, "022")
  await writeFile(
    join(fixture.sourceRepository, "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh"),
    "working tree drift must not be packaged\n",
  )
  const second = runPackager({ ...fixture, output: secondOutput }, "077")
  assert.equal(first.status, 0, first.stderr)
  assert.equal(second.status, 0, second.stderr)
  assert.match(first.stdout, /^sha256:[a-f0-9]{64}\n$/)
  assert.equal(second.stdout, first.stdout)
  assert.equal(first.stderr, "")

  const releaseRoot = join(firstOutput, "rootfs")
  const secondReleaseRoot = join(secondOutput, "rootfs")
  assert.deepEqual(await snapshotTree(releaseRoot), await snapshotTree(secondReleaseRoot))
  const snapshot = await snapshotTree(releaseRoot)
  assert.equal(snapshot.every((entry) => entry.mtimeMs === Number(sourceDateEpoch) * 1000), true)
  assert.equal(snapshot.every((entry) => entry.type !== "unsupported"), true)
  const packagedPaths = snapshot.filter((entry) => entry.type === "file").map((entry) => entry.path)
  for (const requiredPath of [
    "etc/systemd/system/chariox-managed-bootstrap.service",
    "etc/systemd/system/chariox-path1-managed-bootstrap.service",
    "etc/systemd/system/chariox-disposable-worker-bootstrap.service",
    "etc/systemd/system/chariox-rootless-docker.service",
    "etc/systemd/system/chariox-slice-broker.service",
    "usr/lib/chariox/release-manifest.json",
    "usr/lib/chariox/release-manifest.sig",
    "usr/lib/chariox/release-public-key",
    "usr/lib/chariox/build-attestation.json",
    "usr/lib/chariox/build-attestation.sig",
    "usr/lib/chariox/builder-public-key",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/enter-rootless-docker-namespace.sh",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-rootless-service.sh",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/chariox-rootless-engine.service",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/chariox-rootless-user-manager.conf",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-publication-access.sh",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-publication-acl.awk",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/.managed-release",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-kernel",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-relay",
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/toolchain/package-lock.json",
    "usr/lib/chariox/slice-build-context/Cargo.lock",
    "usr/lib/chariox/slice-build-context/apps/kernel/src/transport/relay_peer.rs",
    "usr/lib/chariox/slice-build-context/apps/relay/Cargo.toml",
    "usr/lib/chariox/slice-build-context/packages/event-protocol/Cargo.toml",
    "usr/local/bin/chariox-kernel",
    "usr/local/bin/chariox-managed-bootstrap",
  ]) {
    assert.ok(packagedPaths.includes(requiredPath), `missing packaged path ${requiredPath}`)
  }

  const manifestBytes = await readFile(join(releaseRoot, "usr/lib/chariox/release-manifest.json"))
  const manifest = JSON.parse(manifestBytes)
  const digest = (contents) => `sha256:${createHash("sha256").update(contents).digest("hex")}`
  assert.deepEqual(manifest, {
    schemaVersion: 2,
    sourceCommit: fixture.sourceCommit,
    sourceTree: fixture.sourceTree,
    artifacts: [
      { name: "chariox-kernel", path: "/usr/local/bin/chariox-kernel", sha256: digest("kernel fixture\n") },
      { name: "chariox-managed-bootstrap", path: "/usr/local/bin/chariox-managed-bootstrap", sha256: digest("supervisor fixture\n") },
      {
        name: "chariox-managed-bootstrap.service",
        path: "/etc/systemd/system/chariox-managed-bootstrap.service",
        sha256: digest(fixture.serviceBytes),
      },
      {
        name: "chariox-path1-managed-bootstrap.service",
        path: "/etc/systemd/system/chariox-path1-managed-bootstrap.service",
        sha256: digest(fixture.path1ServiceBytes),
      },
      {
        name: "chariox-disposable-worker-bootstrap.service",
        path: "/etc/systemd/system/chariox-disposable-worker-bootstrap.service",
        sha256: digest(fixture.workerServiceBytes),
      },
      {
        name: "chariox-rootless-docker.service",
        path: "/etc/systemd/system/chariox-rootless-docker.service",
        sha256: digest(fixture.rootlessDockerServiceBytes),
      },
      {
        name: "chariox-slice-broker.service",
        path: "/etc/systemd/system/chariox-slice-broker.service",
        sha256: digest(fixture.sliceBrokerServiceBytes),
      },
      {
        name: "chariox-slice-build-context",
        path: "/usr/lib/chariox/slice-build-context",
        sha256: await treeDigest(join(releaseRoot, "usr/lib/chariox/slice-build-context")),
      },
      {
        name: "chariox-build-attestation",
        path: "/usr/lib/chariox/build-attestation.json",
        sha256: digest(await readFile(fixture.builderAttestation)),
      },
      {
        name: "chariox-build-attestation-signature",
        path: "/usr/lib/chariox/build-attestation.sig",
        sha256: digest((await readFile(fixture.builderAttestationSignature, "utf8")).trim()),
      },
      {
        name: "chariox-builder-public-key",
        path: "/usr/lib/chariox/builder-public-key",
        sha256: digest((await readFile(fixture.trustedBuilderPublicKey, "utf8")).trim()),
      },
    ],
  })
  assert.equal(first.stdout.trim(), digest(manifestBytes))
  const signature = Buffer.from((await readFile(join(releaseRoot, "usr/lib/chariox/release-manifest.sig"), "utf8")).trim(), "base64")
  assert.equal(signature.length, 64)
  assert.equal(verify(null, manifestBytes, fixture.publicKey, signature), true)
  assert.deepEqual(
    Buffer.from((await readFile(join(releaseRoot, "usr/lib/chariox/release-public-key"), "utf8")).trim(), "base64"),
    rawPublicKey(fixture.publicKey),
  )
  assert.deepEqual(
    await readFile(join(releaseRoot, "etc/systemd/system/chariox-managed-bootstrap.service")),
    fixture.serviceBytes,
  )
  assert.deepEqual(
    await readFile(join(releaseRoot, "etc/systemd/system/chariox-path1-managed-bootstrap.service")),
    fixture.path1ServiceBytes,
  )
  assert.deepEqual(
    await readFile(join(releaseRoot, "etc/systemd/system/chariox-disposable-worker-bootstrap.service")),
    fixture.workerServiceBytes,
  )
  assert.match(
    await readFile(
      join(releaseRoot, "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh"),
      "utf8",
    ),
    /SLICE_BUILD_IMAGE=fixture/,
  )
  assert.equal(
    await readFile(
      join(releaseRoot, "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-kernel"),
      "utf8",
    ),
    fixture.kernelContents,
  )
  assert.equal(
    await readFile(
      join(releaseRoot, "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-relay"),
      "utf8",
    ),
    fixture.relayContents,
  )
  assert.equal(snapshot.find((entry) => entry.path === "usr/local/bin/chariox-kernel").mode, 0o755)
  assert.equal(snapshot.find((entry) => entry.path === "usr/local/bin/chariox-managed-bootstrap").mode, 0o755)
  assert.equal(
    snapshot.find((entry) => entry.path.endsWith("/enter-rootless-docker-namespace.sh")).mode,
    0o755,
  )
  assert.equal(
    snapshot.find((entry) => entry.path.endsWith("/provision-linux-docker-slice.sh")).mode,
    0o755,
  )
  assert.equal(
    snapshot.find((entry) => entry.path.endsWith("/managed-publication-access.sh")).mode,
    0o755,
  )
  assert.equal(snapshot.find((entry) => entry.path.endsWith("/prebuilt/chariox-kernel")).mode, 0o755)
  assert.equal(snapshot.find((entry) => entry.path.endsWith("/managed-rootless-service.sh")).mode, 0o755)
  assert.equal(snapshot.find((entry) => entry.path.endsWith("/prebuilt/chariox-relay")).mode, 0o755)
  assert.equal(
    snapshot
      .filter((entry) =>
        entry.type === "file" &&
        !entry.path.startsWith("usr/local/bin/") &&
        !entry.path.endsWith("/enter-rootless-docker-namespace.sh") &&
        !entry.path.endsWith("/managed-rootless-service.sh") &&
        !entry.path.endsWith("/provision-linux-docker-slice.sh") &&
        !entry.path.endsWith("/managed-publication-access.sh") &&
        !entry.path.endsWith("/prebuilt/chariox-kernel") &&
        !entry.path.endsWith("/prebuilt/chariox-relay"),
      )
      .every((entry) => entry.mode === 0o644),
    true,
  )
  assert.equal(snapshot.some((entry) => entry.path.includes("release-key")), false)
  assert.equal(runVerifier(releaseRoot, first.stdout.trim(), fixture.trustedPublicKey).status, 0)

  const rerun = runPackager({ ...fixture, output: firstOutput })
  assert.equal(rerun.status, 1)
  assert.match(rerun.stderr, /output directory must be empty/)
})

test("Path-1 managed-home bootstrap is signed and selected by image install", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-path1-managed-home-install-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const output = join(root, "release")
  const packaged = runPackager({ ...fixture, output })
  assert.equal(packaged.status, 0, packaged.stderr)

  const path1UnitPath = join(output, "rootfs/etc/systemd/system/chariox-path1-managed-bootstrap.service")
  const path1UnitExists = await lstat(path1UnitPath).then(() => true, () => false)
  assert.equal(path1UnitExists, true, "signed release must contain the dedicated Path-1 unit")
  const path1Unit = await readFile(path1UnitPath, "utf8")
  assert.doesNotMatch(path1Unit, /^UMask=/m, "Path-1 must inherit systemd's ordinary system-unit umask")
  for (const required of [
    "Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1",
    "Environment=CHARIOX_MANAGED_BOOTSTRAP_PATH=/var/lib/chariox/managed-bootstrap.json",
    "Environment=HOME=/home/chariox",
    "Environment=CHARIOX_HOME=/home/chariox/.chariox",
    "Environment=CHARIOX_SLICE_DOCKER_BROKER_SOCKET=/var/lib/chariox-slice-share/.broker-private/control/control.sock",
    "Wants=network-online.target chariox-rootless-docker.service",
    "After=network-online.target chariox-rootless-docker.service",
    "ExecStartPre=-+/usr/bin/systemctl restart chariox-slice-broker.service",
    "ExecStart=/usr/local/bin/chariox-managed-bootstrap",
  ]) {
    assert.ok(path1Unit.includes(required), `Path-1 unit is missing ${required}`)
  }
  for (const forbidden of [
    "CHARIOX_MANAGED_PROVIDER_ISOLATION",
    "CHARIOX_CAPABILITY_ISOLATION_ROOT",
    "CHARIOX_MANAGED_PROVIDER_BWRAP",
    "CHARIOX_MANAGED_PROVIDER_HOME",
    "CHARIOX_MANAGED_SLICE_SERVICE_ROOT",
    "CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT",
    "CHARIOX_SLICE_ROOT",
    "bwrap",
    "--disposable-worker",
    "NoNewPrivileges=",
    "PrivateTmp=",
    "PrivateUsers=",
    "PrivateDevices=",
    "PrivateNetwork=",
    "ProtectSystem=",
    "ProtectHome=",
    "ProtectKernel",
    "RestrictNamespaces=",
    "RestrictAddressFamilies=",
    "RestrictSUIDSGID=",
    "ReadWritePaths=",
    "ReadOnlyPaths=",
    "InaccessiblePaths=",
    "BindPaths=",
    "BindReadOnlyPaths=",
    "RootDirectory=",
    "RootImage=",
    "SystemCallFilter=",
    "CapabilityBoundingSet=",
    "StateDirectory=",
    "SupplementaryGroups=",
  ]) {
    assert.ok(!path1Unit.includes(forbidden), `Path-1 unit must not contain ${forbidden}`)
  }
  const workerUnit = await readFile(
    join(output, "rootfs/etc/systemd/system/chariox-disposable-worker-bootstrap.service"), "utf8",
  )
  for (const required of [
    "Environment=CHARIOX_SLICE_DOCKER_BROKER_SOCKET=/var/lib/chariox-slice-share/.broker-private/control/control.sock",
    "Wants=network-online.target chariox-rootless-docker.service",
    "After=network-online.target chariox-rootless-docker.service",
    "ExecStartPre=-+/usr/bin/systemctl restart chariox-slice-broker.service",
  ]) {
    assert.ok(workerUnit.includes(required), `Path-1 worker unit is missing ${required}`)
  }

  const releaseRoot = join(output, "rootfs")
  assert.equal(runVerifier(releaseRoot, packaged.stdout.trim(), fixture.trustedPublicKey, "path1").status, 0)
  const manifestPath = join(releaseRoot, "usr/lib/chariox/release-manifest.json")
  const signaturePath = join(releaseRoot, "usr/lib/chariox/release-manifest.sig")
  const originalManifestBytes = await readFile(manifestPath)
  const originalSignature = await readFile(signaturePath)
  const mismatchedManifestObject = JSON.parse(originalManifestBytes)
  mismatchedManifestObject.artifacts = mismatchedManifestObject.artifacts.filter(
    (artifact) => artifact.name !== "chariox-path1-managed-bootstrap.service",
  )
  await rm(path1UnitPath)
  const mismatchedManifestBytes = Buffer.from(JSON.stringify(mismatchedManifestObject))
  await writeFile(manifestPath, mismatchedManifestBytes)
  await writeFile(
    signaturePath,
    sign(null, mismatchedManifestBytes, fixture.releasePrivateKey).toString("base64"),
  )
  const mismatchedDigest = `sha256:${createHash("sha256").update(mismatchedManifestBytes).digest("hex")}`
  const mismatched = runVerifier(releaseRoot, mismatchedDigest, fixture.trustedPublicKey, "path1")
  assert.equal(mismatched.status, 1)
  assert.match(mismatched.stderr, /does not declare the selected path1 managed bootstrap service/)
  await writeFile(path1UnitPath, path1Unit)
  await writeFile(manifestPath, originalManifestBytes)
  await writeFile(signaturePath, originalSignature)

  const disconnectedBrokerService = path1Unit.replace(
    "ExecStartPre=-+/usr/bin/systemctl restart chariox-slice-broker.service\n",
    "",
  )
  await writeFile(path1UnitPath, disconnectedBrokerService)
  const disconnectedBrokerManifestObject = JSON.parse(originalManifestBytes)
  const disconnectedBrokerArtifact = disconnectedBrokerManifestObject.artifacts.find(
    (artifact) => artifact.name === "chariox-path1-managed-bootstrap.service",
  )
  disconnectedBrokerArtifact.sha256 = `sha256:${createHash("sha256").update(disconnectedBrokerService).digest("hex")}`
  const disconnectedBrokerManifestBytes = Buffer.from(JSON.stringify(disconnectedBrokerManifestObject))
  await writeFile(manifestPath, disconnectedBrokerManifestBytes)
  await writeFile(
    signaturePath,
    sign(null, disconnectedBrokerManifestBytes, fixture.releasePrivateKey).toString("base64"),
  )
  const disconnectedBrokerDigest = `sha256:${createHash("sha256").update(disconnectedBrokerManifestBytes).digest("hex")}`
  const disconnectedBroker = runVerifier(releaseRoot, disconnectedBrokerDigest, fixture.trustedPublicKey, "path1")
  assert.equal(disconnectedBroker.status, 1)
  assert.match(disconnectedBroker.stderr, /must restart the one-shot broker before launch/)
  await writeFile(path1UnitPath, path1Unit)
  await writeFile(manifestPath, originalManifestBytes)
  await writeFile(signaturePath, originalSignature)

  const wrongTopologyService = path1Unit.replace(
    "Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1",
    "Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=shared_host",
  )
  await writeFile(path1UnitPath, wrongTopologyService)
  const wrongTopologyManifestObject = JSON.parse(originalManifestBytes)
  const path1Artifact = wrongTopologyManifestObject.artifacts.find(
    (artifact) => artifact.name === "chariox-path1-managed-bootstrap.service",
  )
  path1Artifact.sha256 = `sha256:${createHash("sha256").update(wrongTopologyService).digest("hex")}`
  const wrongTopologyManifestBytes = Buffer.from(JSON.stringify(wrongTopologyManifestObject))
  await writeFile(manifestPath, wrongTopologyManifestBytes)
  await writeFile(
    signaturePath,
    sign(null, wrongTopologyManifestBytes, fixture.releasePrivateKey).toString("base64"),
  )
  const wrongTopologyDigest = `sha256:${createHash("sha256").update(wrongTopologyManifestBytes).digest("hex")}`
  const wrongTopology = runVerifier(releaseRoot, wrongTopologyDigest, fixture.trustedPublicKey, "path1")
  assert.equal(wrongTopology.status, 1)
  assert.match(wrongTopology.stderr, /missing or overrides Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1/)
  await writeFile(path1UnitPath, path1Unit)
  await writeFile(manifestPath, originalManifestBytes)
  await writeFile(signaturePath, originalSignature)

  const restrictedUmaskService = path1Unit.replace(
    "KillMode=control-group",
    "KillMode=control-group\nUMask=0007",
  )
  await writeFile(path1UnitPath, restrictedUmaskService)
  const restrictedUmaskManifestObject = JSON.parse(originalManifestBytes)
  const restrictedUmaskArtifact = restrictedUmaskManifestObject.artifacts.find(
    (artifact) => artifact.name === "chariox-path1-managed-bootstrap.service",
  )
  restrictedUmaskArtifact.sha256 = `sha256:${createHash("sha256").update(restrictedUmaskService).digest("hex")}`
  const restrictedUmaskManifestBytes = Buffer.from(JSON.stringify(restrictedUmaskManifestObject))
  await writeFile(manifestPath, restrictedUmaskManifestBytes)
  await writeFile(
    signaturePath,
    sign(null, restrictedUmaskManifestBytes, fixture.releasePrivateKey).toString("base64"),
  )
  const restrictedUmaskDigest = `sha256:${createHash("sha256").update(restrictedUmaskManifestBytes).digest("hex")}`
  const restrictedUmask = runVerifier(releaseRoot, restrictedUmaskDigest, fixture.trustedPublicKey, "path1")
  assert.equal(restrictedUmask.status, 1)
  assert.match(restrictedUmask.stderr, /contains UMask=/)
  await writeFile(path1UnitPath, path1Unit)
  await writeFile(manifestPath, originalManifestBytes)
  await writeFile(signaturePath, originalSignature)

  if (process.platform !== "linux" || process.getuid?.() !== 0) {
    context.diagnostic("signed release and topology checks passed; installer execution requires Linux root ownership")
    return
  }

  const harness = await createInstallerHarness(root)
  const env = {
    ...process.env,
    PATH: `${harness.bin}:${process.env.PATH}`,
    HARNESS_STATE: harness.state,
    CHARIOX_IMAGE_INSTALL_ROOT: harness.installRoot,
    CHARIOX_IMAGE_INSTALL_LOCK: join(harness.state, "install.lock"),
  }
  const installed = spawnSync(installer, [
    join(output, "rootfs"),
    packaged.stdout.trim(),
    fixture.trustedPublicKey,
    "path1",
  ], { encoding: "utf8", env })
  assert.equal(installed.status, 0, installed.stderr)
  assert.equal(
    await readFile(join(harness.installRoot, "etc/systemd/system/chariox-path1-managed-bootstrap.service"), "utf8"),
    path1Unit,
  )
  const systemctlCalls = await readFile(join(harness.state, "systemctl"), "utf8")
  assert.match(systemctlCalls, /enable chariox-path1-managed-bootstrap\.service/)
  assert.doesNotMatch(systemctlCalls, /enable chariox-managed-bootstrap\.service/)
})

test("release identity rejects unattested binaries and verifier rejects tampering", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-binding-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const originalOutput = join(root, "original")
  const changedOutput = join(root, "changed")
  const original = runPackager({ ...fixture, output: originalOutput })
  assert.equal(original.status, 0, original.stderr)
  await writeFile(fixture.supervisor, "different supervisor\n", { mode: 0o755 })
  const changed = runPackager({ ...fixture, output: changedOutput })
  assert.equal(changed.status, 1)
  assert.match(changed.stderr, /builder attestation artifacts do not match/)
  assert.equal(await lstat(join(changedOutput, "rootfs/usr/lib/chariox/release-manifest.json")).then(() => true, () => false), false)
  await writeFile(fixture.supervisor, "supervisor fixture\n", { mode: 0o755 })

  const originalRoot = join(originalOutput, "rootfs")
  await writeFile(join(originalRoot, "usr/local/bin/chariox-managed-bootstrap"), "tampered\n")
  const supervisorTamper = runVerifier(originalRoot, original.stdout.trim(), fixture.trustedPublicKey)
  assert.equal(supervisorTamper.status, 1)
  assert.match(supervisorTamper.stderr, /chariox-managed-bootstrap is corrupted/)

  const serviceOutput = join(root, "service")
  const serviceRelease = runPackager({ ...fixture, output: serviceOutput })
  assert.equal(serviceRelease.status, 0, serviceRelease.stderr)
  const changedRoot = join(serviceOutput, "rootfs")
  await writeFile(join(changedRoot, "etc/systemd/system/chariox-managed-bootstrap.service"), "tampered service\n")
  const serviceTamper = runVerifier(changedRoot, serviceRelease.stdout.trim(), fixture.trustedPublicKey)
  assert.equal(serviceTamper.status, 1)
  assert.match(serviceTamper.stderr, /chariox-managed-bootstrap\.service is corrupted/)

  const contextOutput = join(root, "context")
  const contextRelease = runPackager({ ...fixture, output: contextOutput })
  assert.equal(contextRelease.status, 0, contextRelease.stderr)
  const contextRoot = join(contextOutput, "rootfs")
  await writeFile(
    join(contextRoot, "usr/lib/chariox/slice-build-context/apps/relay/Cargo.toml"),
    "tampered context\n",
  )
  const contextTamper = runVerifier(
    contextRoot,
    contextRelease.stdout.trim(),
    fixture.trustedPublicKey,
  )
  assert.equal(contextTamper.status, 1)
  assert.match(contextTamper.stderr, /chariox-slice-build-context is corrupted/)

  const emptyDirectoryOutput = join(root, "empty-directory")
  const emptyDirectoryRelease = runPackager({ ...fixture, output: emptyDirectoryOutput })
  assert.equal(emptyDirectoryRelease.status, 0, emptyDirectoryRelease.stderr)
  const emptyDirectoryRoot = join(emptyDirectoryOutput, "rootfs")
  await mkdir(join(emptyDirectoryRoot, "usr/lib/chariox/slice-build-context/unsigned-empty"))
  const emptyDirectoryTamper = runVerifier(
    emptyDirectoryRoot,
    emptyDirectoryRelease.stdout.trim(),
    fixture.trustedPublicKey,
  )
  assert.equal(emptyDirectoryTamper.status, 1)
  assert.match(emptyDirectoryTamper.stderr, /chariox-slice-build-context is corrupted/)

  const rootModeOutput = join(root, "root-mode")
  const rootModeRelease = runPackager({ ...fixture, output: rootModeOutput })
  assert.equal(rootModeRelease.status, 0, rootModeRelease.stderr)
  const rootModeRoot = join(rootModeOutput, "rootfs")
  await chmod(join(rootModeRoot, "usr/lib/chariox/slice-build-context"), 0o700)
  const rootModeTamper = runVerifier(rootModeRoot, rootModeRelease.stdout.trim(), fixture.trustedPublicKey)
  assert.equal(rootModeTamper.status, 1)
  assert.match(rootModeTamper.stderr, /chariox-slice-build-context is corrupted/)

  const nestedModeOutput = join(root, "nested-mode")
  const nestedModeRelease = runPackager({ ...fixture, output: nestedModeOutput })
  assert.equal(nestedModeRelease.status, 0, nestedModeRelease.stderr)
  const nestedModeRoot = join(nestedModeOutput, "rootfs")
  await chmod(join(nestedModeRoot, "usr/lib/chariox/slice-build-context/apps"), 0o750)
  const nestedModeTamper = runVerifier(
    nestedModeRoot,
    nestedModeRelease.stdout.trim(),
    fixture.trustedPublicKey,
  )
  assert.equal(nestedModeTamper.status, 1)
  assert.match(nestedModeTamper.stderr, /chariox-slice-build-context is corrupted/)
})

test("release verifier rejects root and key aliases into the packaged image", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-alias-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const output = join(root, "release")
  const packaged = runPackager({ ...fixture, output })
  assert.equal(packaged.status, 0, packaged.stderr)
  const rootfs = join(output, "rootfs")
  const rootfsAlias = join(root, "rootfs-alias")
  await symlink(rootfs, rootfsAlias)
  const rootAliasResult = runVerifier(rootfsAlias, packaged.stdout.trim(), fixture.trustedPublicKey)
  assert.equal(rootAliasResult.status, 1)
  assert.match(rootAliasResult.stderr, /image root must be a directory, not a symlink/)

  const releaseDirectoryAlias = join(root, "release-directory-alias")
  await symlink(join(rootfs, "usr/lib/chariox"), releaseDirectoryAlias)
  const keyAliasResult = runVerifier(
    rootfs,
    packaged.stdout.trim(),
    join(releaseDirectoryAlias, "release-public-key"),
  )
  assert.equal(keyAliasResult.status, 1)
  assert.match(keyAliasResult.stderr, /trusted release public key must be supplied outside/)

  const externalLocal = join(root, "external-local")
  await rename(join(rootfs, "usr/local"), externalLocal)
  await symlink(externalLocal, join(rootfs, "usr/local"))
  const ancestorAliasResult = runVerifier(rootfs, packaged.stdout.trim(), fixture.trustedPublicKey)
  assert.equal(ancestorAliasResult.status, 1)
  assert.match(ancestorAliasResult.stderr, /image root contains a symbolic link/)
})

test("managed kernel release packaging rejects symlink inputs", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-symlink-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const kernelLink = join(root, "kernel-link")
  await symlink(fixture.kernel, kernelLink)
  const result = runPackager({ ...fixture, kernel: kernelLink, output: join(root, "output") })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /kernel binary must be a bounded regular file/)
})

test("managed kernel release packaging rejects a broadly readable signing key", async (context) => {
  if (process.platform === "win32") return
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-key-mode-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  await chmod(fixture.signingKey, 0o644)
  const result = runPackager({ ...fixture, output: join(root, "output") })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /must not be readable by group or other users/)
})

test("managed kernel release requires an exact immutable source commit", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-source-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const abbreviated = runPackager({
    ...fixture,
    sourceCommit: fixture.sourceCommit.slice(0, 12),
    output: join(root, "abbreviated"),
  })
  assert.equal(abbreviated.status, 1)
  assert.match(abbreviated.stderr, /source commit must be a full lowercase Git commit ID/)

  const missing = runPackager({
    ...fixture,
    sourceCommit: "0".repeat(40),
    output: join(root, "missing"),
  })
  assert.equal(missing.status, 1)
  assert.match(missing.stderr, /source commit cannot be resolved/)
})

test("managed release materialization ignores ambient Git attributes and tar options", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-attributes-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  await writeFile(join(fixture.sourceRepository, ".git/info/attributes"), "* export-ignore\n")
  const output = join(root, "output")
  const result = runPackager(
    { ...fixture, output },
    "022",
    { TAR_OPTIONS: "--files-from=/etc/passwd", GIT_CONFIG_GLOBAL: join(root, "hostile-gitconfig") },
  )
  assert.equal(result.status, 0, result.stderr)
  assert.equal(
    await readFile(join(output, "rootfs/usr/lib/chariox/slice-build-context/Cargo.lock"), "utf8"),
    "version = 4\n",
  )
})

test("managed kernel builder archives the exact commit and emits a signed binary attestation", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-builder-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const bin = join(root, "builder-bin")
  const trace = join(root, "builder-trace")
  const extractionState = join(root, "builder-extraction-state")
  const output = join(root, "builder-output")
  await mkdir(bin)
  await writeHarnessCommand(join(bin, "docker"), `#!/bin/sh
set -eu
[ -z "\${RUSTC_WRAPPER:-}" ]
[ -z "\${CARGO:-}" ]
[ -z "\${RUSTFLAGS:-}" ]
case "$1" in
  build)
    case " $* " in *" --pull --platform linux/amd64 --target rust-builder "*) ;; *) exit 31 ;; esac
    case " $* " in *" --tag chariox-managed-builder:"*"-"*"-"*) ;; *) exit 31 ;; esac
    for source do :; done
    dockerfile=
    previous=
    for argument do
      if [ "$previous" = "--file" ]; then dockerfile=$argument; fi
      previous=$argument
    done
    [ -f "$dockerfile" ]
    [ "$dockerfile" = "$source/apps/kernel/slice-linux-docker/docker/Dockerfile" ]
    [ ! -e "$source/.git" ]
    [ ! -e "$source/working-tree-only" ]
    printf '%s\n' "$*" > '${trace}'
    ;;
  run)
    case " $* " in *" --rm --pull=never --platform linux/amd64 --entrypoint cat sha256:"*) ;; *) exit 32 ;; esac
    previous=
    image=
    for argument do
      if [ "$previous" = "cat" ]; then image=$argument; fi
      previous=$argument
      source_path=$argument
    done
    [ "$image" = "sha256:1111111111111111111111111111111111111111111111111111111111111111" ]
    case "$source_path" in
      *chariox-kernel)
        printf 'tag replaced after first immutable read\n' > '${extractionState}'
        printf 'kernel from archived commit\n'
        ;;
      *chariox-managed-bootstrap)
        [ -f '${extractionState}' ]
        printf 'supervisor from archived commit\n'
        ;;
      *chariox-relay)
        [ -f '${extractionState}' ]
        printf 'relay from archived commit\n'
        ;;
      *) exit 32 ;;
    esac
    ;;
  image)
    case "$2" in
      inspect) printf '%s\n' 'sha256:1111111111111111111111111111111111111111111111111111111111111111' ;;
      rm) ;;
      *) exit 33 ;;
    esac
    ;;
  rm) ;;
  *) exit 33 ;;
esac
`)
  await writeFile(join(fixture.sourceRepository, "working-tree-only"), "must not enter the build\n")
  await writeFile(join(fixture.sourceRepository, ".git/info/attributes"), "* export-ignore\n")
  const result = spawnSync(
    process.execPath,
    [
      builder,
      "--source-repository", fixture.sourceRepository,
      "--source-commit", fixture.sourceCommit,
      "--builder-signing-key", fixture.signingKey,
      "--output", output,
    ],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        PATH: `${bin}:${process.env.PATH}`,
        RUSTC_WRAPPER: "/tmp/hostile-rustc-wrapper",
        CARGO: "/tmp/hostile-cargo",
        RUSTFLAGS: "-C link-arg=/tmp/hostile",
        TAR_OPTIONS: "--files-from=/etc/passwd",
      },
    },
  )
  assert.equal(result.status, 0, result.stderr)
  assert.match(await readFile(trace, "utf8"), /--target rust-builder/)
  assert.match(
    await readFile(trace, "utf8"),
    /--file .*\/apps\/kernel\/slice-linux-docker\/docker\/Dockerfile/,
  )
  const attestationBytes = await readFile(join(output, "build-attestation.json"))
  const attestation = JSON.parse(attestationBytes)
  assert.equal(attestation.sourceCommit, fixture.sourceCommit)
  assert.equal(attestation.sourceTree, fixture.sourceTree)
  assert.equal(attestation.target, "x86_64-unknown-linux-gnu")
  assert.deepEqual(
    attestation.artifacts.map((artifact) => artifact.name),
    ["chariox-kernel", "chariox-managed-bootstrap", "chariox-relay"],
  )
  const signature = Buffer.from(await readFile(join(output, "build-attestation.sig"), "utf8"), "base64")
  assert.equal(verify(null, attestationBytes, fixture.publicKey, signature), true)
  assert.equal(
    await readFile(join(output, "builder-public-key"), "utf8"),
    rawPublicKey(fixture.publicKey).toString("base64"),
  )
})

test("managed kernel release requires a matching trusted builder attestation", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-release-attestation-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const original = JSON.parse(await readFile(fixture.builderAttestation, "utf8"))
  const writeAttestation = async (value) => {
    const bytes = Buffer.from(JSON.stringify(value))
    await writeFile(fixture.builderAttestation, bytes)
    await writeFile(
      fixture.builderAttestationSignature,
      sign(null, bytes, fixture.builderPrivateKey).toString("base64"),
    )
  }

  await writeAttestation({ ...original, sourceCommit: "0".repeat(40) })
  const stale = runPackager({ ...fixture, output: join(root, "stale") })
  assert.equal(stale.status, 1)
  assert.match(stale.stderr, /identity or target does not match/)

  await writeAttestation({ ...original, target: "aarch64-unknown-linux-gnu" })
  const wrongTarget = runPackager({ ...fixture, output: join(root, "wrong-target") })
  assert.equal(wrongTarget.status, 1)
  assert.match(wrongTarget.stderr, /identity or target does not match/)

  await writeAttestation(original)
  await writeFile(fixture.builderAttestationSignature, Buffer.alloc(64).toString("base64"))
  const badSignature = runPackager({ ...fixture, output: join(root, "bad-signature") })
  assert.equal(badSignature.status, 1)
  assert.match(badSignature.stderr, /builder attestation signature is invalid/)

  await writeAttestation({
    ...original,
    artifacts: [original.artifacts[1], original.artifacts[0], original.artifacts[2]],
  })
  const wrongArtifacts = runPackager({ ...fixture, output: join(root, "wrong-artifacts") })
  assert.equal(wrongArtifacts.status, 1)
  assert.match(wrongArtifacts.stderr, /artifacts do not match/)
})

async function writeHarnessCommand(path, contents) {
  await writeFile(path, contents, { mode: 0o755 })
  await chmod(path, 0o755)
}

async function createInstallerHarness(root) {
  const bin = join(root, "bin")
  const state = join(root, "command-state")
  const installRoot = join(root, "installed")
  const chownLog = join(root, "chown.log")
  const chownPreload = join(root, "chown-preload.mjs")
  const migrationFaultMarker = join(root, "migration-fault.marker")
  await mkdir(bin, { recursive: true })
  await mkdir(state, { recursive: true })
  await writeFile(
    chownPreload,
    `import fs from "node:fs"
import { syncBuiltinESMExports } from "node:module"
import { dirname } from "node:path"

const logPath = ${JSON.stringify(chownLog)}
const originalChown = fs.promises.chown.bind(fs.promises)
const originalRename = fs.promises.rename.bind(fs.promises)
fs.promises.chown = async (path, uid, gid) => {
  await fs.promises.appendFile(logPath, String(path) + "\\t" + String(uid) + "\\t" + String(gid) + "\\n")
  if (Number(uid) === 0 && Number(gid) === 0) return originalChown(path, uid, gid)
}
async function publishFaultMarker(value) {
  const marker = process.env.HARNESS_MIGRATION_FAULT_MARKER
  if (!marker) return
  const temporary = marker + ".tmp." + process.pid
  const file = await fs.promises.open(temporary, "wx", 0o600)
  try {
    await file.writeFile(value + "\\n")
    await file.sync()
  } finally {
    await file.close()
  }
  await originalRename(temporary, marker)
  const directory = await fs.promises.open(dirname(marker), "r")
  try {
    await directory.sync()
  } finally {
    await directory.close()
  }
}
let faultInjected = false
fs.promises.rename = async (source, destination) => {
  const result = await originalRename(source, destination)
  const rootRename = process.env.HARNESS_MIGRATION_KILL_AFTER_ROOT_RENAME === "1"
    && source === process.env.HARNESS_MIGRATION_LEGACY_HOME
    && destination === process.env.HARNESS_MIGRATION_MANAGED_HOME
  const controlMove = process.env.HARNESS_MIGRATION_KILL_AFTER_CONTROL_PATH
    && destination === process.env.HARNESS_MIGRATION_KILL_AFTER_CONTROL_PATH
  if (!faultInjected && (rootRename || controlMove)) {
    faultInjected = true
    await publishFaultMarker(rootRename ? "root-renamed" : "control-moved")
    process.kill(process.pid, "SIGSTOP")
  }
  return result
}
syncBuiltinESMExports()
`,
  )
  await writeHarnessCommand(join(bin, "id"), `#!/bin/sh
if [ "\${1:-}" = "-u" ]; then
  if [ "\${2:-}" = chariox-docker ]; then echo 997; elif [ "\${2:-}" = chariox ]; then echo 998; else echo 0; fi
  exit 0
fi
if [ "\${1:-}" = "-g" ]; then
  if [ "\${2:-}" = chariox-docker ]; then echo 997; elif [ "\${2:-}" = chariox ]; then echo 998; else echo 0; fi
  exit 0
fi
if [ "\${1:-}" = "-gn" ]; then
  [ "\${2:-}" = "chariox" ] && [ -f "$HARNESS_STATE/user-chariox" ] && echo chariox && exit 0
  [ "\${2:-}" = "chariox-docker" ] && [ -f "$HARNESS_STATE/user-chariox-docker" ] && echo chariox-docker && exit 0
  exit 1
fi
if [ "\${1:-}" = "chariox" ]; then [ -f "$HARNESS_STATE/user-chariox" ]; exit $?; fi
if [ "\${1:-}" = "chariox-docker" ]; then [ -f "$HARNESS_STATE/user-chariox-docker" ]; exit $?; fi
exit 1
`)
  await writeHarnessCommand(join(bin, "getent"), `#!/bin/sh
if [ "\${1:-}" = "group" ] && [ -f "$HARNESS_STATE/group-\${2:-}" ]; then echo "\${2}:x:998:"; exit 0; fi
if [ "\${1:-}" = "passwd" ] && [ "\${2:-}" = "chariox" ] && [ -f "$HARNESS_STATE/user-chariox" ]; then
  home=/home/chariox
  [ -f "$HARNESS_STATE/user-chariox-home" ] && home=$(cat "$HARNESS_STATE/user-chariox-home")
  echo "chariox:x:998:998::\${home}:/usr/sbin/nologin"
  exit 0
fi
if [ "\${1:-}" = "passwd" ] && [ "\${2:-}" = "chariox-docker" ] && [ -f "$HARNESS_STATE/user-chariox-docker" ]; then echo 'chariox-docker:x:997:997::/var/lib/chariox-docker/home:/usr/sbin/nologin'; exit 0; fi
exit 2
`)
  await writeHarnessCommand(join(bin, "groupadd"), "#!/bin/sh\nfor value in \"$@\"; do name=$value; done\ntouch \"$HARNESS_STATE/group-$name\"\n")
  await writeHarnessCommand(join(bin, "useradd"), "#!/bin/sh\nprevious=\nfor value in \"$@\"; do if [ \"$previous\" = --home-dir ]; then printf '%s' \"$value\" > \"$HARNESS_STATE/user-$name-home\"; fi; previous=$value; name=$value; done\ntouch \"$HARNESS_STATE/user-$name\"\n")
  await writeHarnessCommand(join(bin, "usermod"), "#!/bin/sh\nif [ \"\${1:-}\" = --home ] && [ \"\${3:-}\" = chariox ]; then printf '%s' \"$2\" > \"$HARNESS_STATE/user-chariox-home\"; fi\nexit 0\n")
  await writeHarnessCommand(join(bin, "loginctl"), `#!/bin/sh
[ "$*" = "enable-linger chariox-docker" ] || exit 1
[ "\${HARNESS_LOGINCTL_FAIL:-0}" = 0 ] || exit 1
printf '%s\\n' "$*" >> "$HARNESS_STATE/loginctl"
`)
  await writeHarnessCommand(join(bin, "setfacl"), "#!/bin/sh\nexit 0\n")
  await writeHarnessCommand(join(bin, "systemctl"), `#!/bin/sh
printf '%s\\n' "$*" >> "$HARNESS_STATE/systemctl"
if [ -n "\${HARNESS_SYSTEMCTL_FAIL:-}" ]; then
  case "$*" in *"$HARNESS_SYSTEMCTL_FAIL"*) exit 1 ;; esac
fi
`)
  await writeHarnessCommand(join(bin, "flock"), `#!/bin/sh
if [ -n "\${HARNESS_FLOCK_ID:-}" ]; then
  touch "$HARNESS_STATE/flock-entered-$HARNESS_FLOCK_ID"
  while ! mkdir "$HARNESS_STATE/flock-held" 2>/dev/null; do sleep 0.05; done
  touch "$HARNESS_STATE/flock-acquired-$HARNESS_FLOCK_ID"
  parent=$PPID
  (while kill -0 "$parent" 2>/dev/null; do sleep 0.05; done; rmdir "$HARNESS_STATE/flock-held" 2>/dev/null || true) >/dev/null 2>&1 &
  while [ ! -f "$HARNESS_STATE/flock-release-$HARNESS_FLOCK_ID" ]; do sleep 0.05; done
fi
exit 0
`)
  await writeHarnessCommand(join(bin, "find"), `#!/bin/sh
if [ "\${HARNESS_FIND_FAIL:-0}" = "1" ]; then exit 1; fi
exec /usr/bin/find "$@"
`)
  await writeHarnessCommand(join(bin, "node"), `#!/bin/sh
if [ -n "\${HARNESS_MUTATE_SOURCE:-}" ]; then
  printf '%s\n' 'mutated after staging' > "$HARNESS_MUTATE_SOURCE"
fi
export NODE_OPTIONS="--import=${chownPreload} \${NODE_OPTIONS:-}"
case "\${1:-}:\${2:-}" in
  *managed-kernel-home-migration.mjs:apply)
    if [ "\${HARNESS_MIGRATION_REPLACE_SOURCE:-0}" = 1 ]; then
      /bin/mv -- "\${HARNESS_MIGRATION_LEGACY_HOME:?}" "\${HARNESS_MIGRATION_LEGACY_HOME:?}.original"
      /bin/mkdir "\${HARNESS_MIGRATION_LEGACY_HOME:?}"
    fi
    ;;
esac
exec "${process.execPath}" "$@"
`)
  await writeHarnessCommand(join(bin, "install"), `#!/usr/bin/env node
const { spawnSync } = require("node:child_process")
const args = process.argv.slice(2)
const filtered = []
for (let index = 0; index < args.length; index += 1) {
  if (args[index] === "-o" || args[index] === "-g") { index += 1; continue }
  if (args[index] === "-m" && args[index + 1] === "2710") {
    filtered.push("-m", "0710")
    index += 1
    continue
  }
  filtered.push(args[index])
}
const result = spawnSync("/usr/bin/install", filtered, { stdio: "inherit" })
process.exit(result.status ?? 1)
`)
  await writeHarnessCommand(join(bin, "mv"), `#!/bin/sh
if [ "$1" = -Tf ]; then echo 'mv -T is not portable' >&2; exit 64; fi
exec /bin/mv "$@"
`)
  return { bin, state, installRoot, chownLog, migrationFaultMarker }
}

test("managed image installer verifies, installs twice, and rejects seeded runtime state", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-install-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const output = join(root, "release")
  const packaged = runPackager({ ...fixture, output })
  assert.equal(packaged.status, 0, packaged.stderr)
  const harness = await createInstallerHarness(root)
  const env = {
    ...process.env,
    PATH: `${harness.bin}:${process.env.PATH}`,
    HARNESS_STATE: harness.state,
    CHARIOX_IMAGE_INSTALL_ROOT: harness.installRoot,
    CHARIOX_IMAGE_INSTALL_LOCK: join(harness.state, "install.lock"),
  }
  await writeFile(join(output, "rootfs/usr/local/bin/unsigned-extra"), "must not publish\n")
  const args = [join(output, "rootfs"), packaged.stdout.trim(), fixture.trustedPublicKey]
  const badDigest = spawnSync(
    installer,
    [args[0], `sha256:${"0".repeat(64)}`, args[2]],
    { encoding: "utf8", env },
  )
  assert.equal(badDigest.status, 1)
  assert.match(badDigest.stderr, /release manifest digest does not match/)
  assert.equal(await lstat(harness.installRoot).then(() => true, () => false), false)
  const brokerWantsLink = join(
    harness.installRoot,
    "etc/systemd/system/multi-user.target.wants/chariox-slice-broker.service",
  )
  await mkdir(join(brokerWantsLink, ".."), { recursive: true })
  await symlink("../chariox-slice-broker.service", brokerWantsLink)
  const sourceKernel = join(args[0], "usr/local/bin/chariox-kernel")
  const first = spawnSync("/bin/sh", ["-c", 'umask 077; exec "$@"', "managed-installer", installer, ...args], {
    encoding: "utf8",
    env: { ...env, HARNESS_MUTATE_SOURCE: sourceKernel },
  })
  assert.equal(first.status, 0, first.stderr)
  const contextPath = "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker"
  for (const [link, source] of [
    ["etc/systemd/user/chariox-rootless-engine.service", "chariox-rootless-engine.service"],
    ["etc/systemd/system/user@997.service.d/50-chariox-docker.conf", "chariox-rootless-user-manager.conf"],
  ]) {
    assert.deepEqual(await readFile(join(harness.installRoot, link)), await readFile(join(args[0], contextPath, source)))
  }
  assert.equal(await readFile(join(harness.state, "loginctl"), "utf8"), "enable-linger chariox-docker\n")
  const deterministicRelease = join(
    harness.installRoot,
    "usr/lib/chariox/releases",
    packaged.stdout.trim().slice("sha256:".length),
  )
  for (const relativePath of ["usr", "usr/local", "usr/lib", "etc", "etc/systemd"]) {
    assert.equal(
      (await stat(join(deterministicRelease, relativePath))).mode & 0o777,
      0o755,
      `${relativePath} must be traversable by managed runtime users`,
    )
  }
  const firstReleaseInode = (await stat(deterministicRelease)).ino
  const currentLink = join(harness.installRoot, "usr/lib/chariox/current")
  const firstCurrentInode = (await lstat(currentLink)).ino
  const stalePending = join(
    harness.installRoot,
    "usr/lib/chariox/releases",
    `.new-${packaged.stdout.trim().slice("sha256:".length)}`,
  )
  await mkdir(stalePending)
  await writeFile(join(stalePending, "stale"), "interrupted install\n")
  await symlink("stale", `${currentLink}.new`)
  const unrelatedRelease = join(harness.installRoot, "usr/lib/chariox/releases/unrelated")
  await mkdir(unrelatedRelease)
  await writeFile(sourceKernel, "kernel fixture\n", { mode: 0o755 })
  const second = spawnSync(installer, args, { encoding: "utf8", env })
  assert.equal(second.status, 0, second.stderr)
  assert.equal((await stat(deterministicRelease)).ino, firstReleaseInode)
  assert.equal((await lstat(currentLink)).ino, firstCurrentInode)
  assert.equal(await lstat(stalePending).then(() => true, () => false), false)
  assert.equal(await lstat(`${currentLink}.new`).then(() => true, () => false), false)
  assert.equal((await stat(unrelatedRelease)).isDirectory(), true)

  await writeFile(`${currentLink}.new`, "must not be replaced\n")
  const obstructedLink = spawnSync(installer, args, { encoding: "utf8", env })
  assert.equal(obstructedLink.status, 1)
  assert.match(obstructedLink.stderr, /temporary link is obstructed/)
  assert.equal(await readFile(`${currentLink}.new`, "utf8"), "must not be replaced\n")
  await rm(`${currentLink}.new`)

  await writeFile(join(deterministicRelease, "usr/local/bin/chariox-kernel"), "corrupt release\n")
  const repairedRelease = spawnSync(installer, args, { encoding: "utf8", env })
  assert.equal(repairedRelease.status, 0, repairedRelease.stderr)
  assert.equal(
    await readFile(join(deterministicRelease, "usr/local/bin/chariox-kernel"), "utf8"),
    "kernel fixture\n",
  )
  assert.equal((await lstat(currentLink)).ino, firstCurrentInode)

  await rm(deterministicRelease, { recursive: true, force: true })
  await writeFile(deterministicRelease, "interrupted regular-file publication\n")
  const repairedObstruction = spawnSync(installer, args, { encoding: "utf8", env })
  assert.equal(repairedObstruction.status, 0, repairedObstruction.stderr)
  assert.equal((await stat(deterministicRelease)).isDirectory(), true)
  assert.equal((await lstat(currentLink)).ino, firstCurrentInode)
  assert.equal(await readFile(join(harness.installRoot, "usr/local/bin/chariox-kernel"), "utf8"), "kernel fixture\n")
  assert.equal((await stat(join(harness.installRoot, "usr/local/bin/chariox-kernel"))).mode & 0o777, 0o755)
  assert.equal((await stat(join(harness.installRoot, "usr/lib/chariox/release-manifest.json"))).mode & 0o777, 0o644)
  const installedProvisioner = join(
    harness.installRoot,
    "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
  )
  assert.equal((await stat(installedProvisioner)).mode & 0o777, 0o755)
  assert.match(await readFile(installedProvisioner, "utf8"), /SLICE_BUILD_IMAGE/)
  assert.equal(
    await readFile(join(harness.installRoot, "etc/systemd/system/chariox-managed-bootstrap.service"), "utf8"),
    fixture.serviceBytes.toString("utf8"),
  )
  const installedWorkerService = join(harness.installRoot, "etc/systemd/system/chariox-disposable-worker-bootstrap.service")
  assert.equal((await lstat(installedWorkerService)).isSymbolicLink(), true)
  assert.equal(await readFile(installedWorkerService, "utf8"), fixture.workerServiceBytes.toString("utf8"))
  const installedBrokerService = join(harness.installRoot, "etc/systemd/system/chariox-slice-broker.service")
  assert.equal((await lstat(installedBrokerService)).isSymbolicLink(), true)
  assert.equal(await readFile(installedBrokerService, "utf8"), fixture.sliceBrokerServiceBytes.toString("utf8"))
  assert.equal(await lstat(brokerWantsLink).then(() => true, () => false), false)
  assert.equal(await readlink(join(harness.installRoot, "usr/lib/chariox/slice-build-context")), "current/usr/lib/chariox/slice-build-context")
  assert.match(await readlink(join(harness.installRoot, "usr/lib/chariox/current")), /^releases\/[a-f0-9]{64}$/)
  assert.equal(
    await lstat(join(harness.installRoot, "usr/lib/chariox/current/usr/local/bin/unsigned-extra"))
      .then(() => true, () => false),
    false,
  )
  const systemctl = (await readFile(join(harness.state, "systemctl"), "utf8")).trim().split("\n")
  assert.deepEqual(systemctl, [
    "daemon-reload",
    "enable chariox-rootless-docker.service",
    "enable chariox-managed-bootstrap.service",
    "daemon-reload",
    "enable chariox-rootless-docker.service",
    "enable chariox-managed-bootstrap.service",
    "daemon-reload",
    "enable chariox-rootless-docker.service",
    "enable chariox-managed-bootstrap.service",
    "daemon-reload",
    "enable chariox-rootless-docker.service",
    "enable chariox-managed-bootstrap.service",
  ])

  const failedTraversal = spawnSync(installer, args, {
    encoding: "utf8",
    env: { ...env, HARNESS_FIND_FAIL: "1" },
  })
  assert.equal(failedTraversal.status, 1)
  assert.match(failedTraversal.stderr, /managed kernel state root could not be inspected/)

  const seededIdentity = join(harness.installRoot, "var/lib/chariox/daemon-machine-identity.json")
  await writeFile(seededIdentity, "should never enter an image")
  const rejected = spawnSync(installer, args, { encoding: "utf8", env })
  assert.equal(rejected.status, 1)
  assert.match(rejected.stderr, /managed kernel state root contains an unrecognized entry/)
  assert.equal(await readFile(seededIdentity, "utf8"), "should never enter an image")
})

test("managed image installer migrates legacy home state without clobbering canonical state", async (context) => {
  if (process.platform !== "linux" || process.getuid?.() !== 0) {
    context.skip("requires Linux root ownership semantics")
    return
  }
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-install-migration-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const output = join(root, "release")
  const packaged = runPackager({ ...fixture, output })
  assert.equal(packaged.status, 0, packaged.stderr)
  const harness = await createInstallerHarness(root)
  const legacyHome = join(harness.installRoot, "var/lib/chariox/home")
  const legacyState = join(legacyHome, ".chariox")
  await mkdir(join(legacyHome, "repositories/repo-1"), { recursive: true })
  await mkdir(join(legacyState, "vault"), { recursive: true })
  await mkdir(join(legacyHome, "managed"), { recursive: true })
  await mkdir(join(legacyHome, "kernels/active"), { recursive: true })
  await writeFile(join(legacyHome, "repositories/repo-1/HEAD"), "legacy-repository\n")
  await writeFile(join(legacyState, "vault/vault.json"), "legacy-vault\n")
  await writeFile(join(legacyHome, "managed/bootstrap-receipt.json"), "legacy-receipt\n")
  await writeFile(join(legacyHome, "kernels/active/kernel-1.json"), "legacy-presence\n")
  await chmod(legacyHome, 0o755)
  await chmod(legacyState, 0o755)
  const env = {
    ...process.env,
    PATH: harness.bin + ":" + process.env.PATH,
    HARNESS_STATE: harness.state,
    CHARIOX_IMAGE_INSTALL_ROOT: harness.installRoot,
    CHARIOX_IMAGE_INSTALL_LOCK: join(harness.state, "install.lock"),
  }
  const args = [join(output, "rootfs"), packaged.stdout.trim(), fixture.trustedPublicKey]
  const migrated = spawnSync(installer, args, { encoding: "utf8", env })
  assert.equal(migrated.status, 0, migrated.stderr)
  assert.equal(await lstat(legacyHome).then(() => true, () => false), false)
  assert.equal(
    await readFile(join(harness.installRoot, "home/chariox/repositories/repo-1/HEAD"), "utf8"),
    "legacy-repository\n",
  )
  assert.equal(
    await readFile(join(harness.installRoot, "home/chariox/.chariox/vault/vault.json"), "utf8"),
    "legacy-vault\n",
  )
  assert.equal(
    await readFile(join(harness.installRoot, "var/lib/chariox/managed/bootstrap-receipt.json"), "utf8"),
    "legacy-receipt\n",
  )
  assert.equal(
    await readFile(join(harness.installRoot, "var/lib/chariox/kernels/active/kernel-1.json"), "utf8"),
    "legacy-presence\n",
  )
  assert.equal((await stat(join(harness.installRoot, "home/chariox"))).mode & 0o777, 0o700)
  assert.equal((await stat(join(harness.installRoot, "home/chariox/.chariox"))).mode & 0o777, 0o700)
  assert.equal((await stat(join(harness.installRoot, "var/lib/chariox/managed"))).uid, 0)
  assert.equal((await stat(join(harness.installRoot, "var/lib/chariox/managed"))).gid, 0)
  assert.equal((await stat(join(harness.installRoot, "var/lib/chariox/managed"))).mode & 0o777, 0o700)
  assert.equal((await stat(join(harness.installRoot, "var/lib/chariox/managed/bootstrap-receipt.json"))).uid, 0)
  assert.equal((await stat(join(harness.installRoot, "var/lib/chariox/managed/bootstrap-receipt.json"))).gid, 0)
  assert.equal(
    (await stat(join(harness.installRoot, "var/lib/chariox/managed/bootstrap-receipt.json"))).mode & 0o777,
    0o600,
  )
  const migrationChowns = await readFile(harness.chownLog, "utf8")
  assert.match(migrationChowns, new RegExp(`${harness.installRoot}/home/chariox\\t998\\t998`))
  assert.match(migrationChowns, new RegExp(`${harness.installRoot}/home/chariox/.chariox\\t998\\t998`))
  assert.equal((await stat(join(harness.installRoot, "var/lib/chariox/kernels/active"))).uid, 0)
  assert.equal((await stat(join(harness.installRoot, "var/lib/chariox/kernels/active"))).gid, 0)
  assert.equal((await stat(join(harness.installRoot, "var/lib/chariox/kernels/active"))).mode & 0o777, 0o700)
  assert.equal((await stat(join(harness.installRoot, "var/lib/chariox/kernels/active/kernel-1.json"))).uid, 0)
  assert.equal((await stat(join(harness.installRoot, "var/lib/chariox/kernels/active/kernel-1.json"))).gid, 0)
  assert.equal(
    (await stat(join(harness.installRoot, "var/lib/chariox/kernels/active/kernel-1.json"))).mode & 0o777,
    0o600,
  )

  await mkdir(legacyHome, { recursive: true })
  await writeFile(join(legacyHome, "must-remain-untouched"), "collision\n")
  const collision = spawnSync(installer, args, { encoding: "utf8", env })
  assert.equal(collision.status, 1)
  assert.match(collision.stderr, /legacy managed kernel home identity changed during migration/)
  assert.equal(await readFile(join(legacyHome, "must-remain-untouched"), "utf8"), "collision\n")
  assert.equal(
    await readFile(join(harness.installRoot, "home/chariox/repositories/repo-1/HEAD"), "utf8"),
    "legacy-repository\n",
  )
})

test("managed image installer resumes interrupted home migration by identity", async (context) => {
  if (process.platform !== "linux" || process.getuid?.() !== 0) {
    context.skip("requires Linux root ownership semantics")
    return
  }
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-install-migration-recovery-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const output = join(root, "release")
  const packaged = runPackager({ ...fixture, output })
  assert.equal(packaged.status, 0, packaged.stderr)
  const rootfs = join(output, "rootfs")
  const digest = packaged.stdout.trim()

  const createLegacyCase = async (name) => {
    const caseRoot = join(root, name)
    await mkdir(caseRoot, { recursive: true })
    const harness = await createInstallerHarness(caseRoot)
    const stateRoot = join(harness.installRoot, "var/lib/chariox")
    const legacyHome = join(stateRoot, "home")
    const managedHome = join(harness.installRoot, "home/chariox")
    const completion = join(stateRoot, "home-migration-complete")
    await mkdir(join(legacyHome, "repositories/repo-1"), { recursive: true })
    await mkdir(join(legacyHome, "managed"), { recursive: true })
    await mkdir(join(legacyHome, "kernels/active"), { recursive: true })
    await writeFile(join(legacyHome, "repositories/repo-1/HEAD"), "legacy-repository\n")
    await writeFile(join(legacyHome, "managed/bootstrap-receipt.json"), "legacy-receipt\n")
    await writeFile(join(legacyHome, "kernels/active/kernel-1.json"), "legacy-presence\n")
    const env = {
      ...process.env,
      PATH: `${harness.bin}:${process.env.PATH}`,
      HARNESS_STATE: harness.state,
      CHARIOX_IMAGE_INSTALL_ROOT: harness.installRoot,
      CHARIOX_IMAGE_INSTALL_LOCK: join(harness.state, "install.lock"),
    }
    return {
      harness,
      stateRoot,
      legacyHome,
      managedHome,
      completion,
      faultMarker: harness.migrationFaultMarker,
      journal: join(stateRoot, "home-migration.json"),
      controlPath: join(stateRoot, "kernels/active"),
      args: [rootfs, digest, fixture.trustedPublicKey],
      env,
    }
  }

  const assertMigrated = async (migrationCase) => {
    assert.equal(await lstat(migrationCase.legacyHome).then(() => true, () => false), false)
    assert.equal(
      await readFile(join(migrationCase.managedHome, "repositories/repo-1/HEAD"), "utf8"),
      "legacy-repository\n",
    )
    assert.equal(
      await readFile(join(migrationCase.stateRoot, "managed/bootstrap-receipt.json"), "utf8"),
      "legacy-receipt\n",
    )
    assert.equal(
      await readFile(join(migrationCase.controlPath, "kernel-1.json"), "utf8"),
      "legacy-presence\n",
    )
    assert.equal(await readFile(migrationCase.completion, "utf8"), "complete\n")
    assert.equal((await stat(migrationCase.journal)).uid, 0)
    assert.equal((await stat(migrationCase.journal)).mode & 0o777, 0o600)
  }

  const rootRenameCase = await createLegacyCase("after-root-rename")
  const rootRenameProcess = spawn(installer, rootRenameCase.args, {
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
    env: {
      ...rootRenameCase.env,
      HARNESS_MIGRATION_KILL_AFTER_ROOT_RENAME: "1",
      HARNESS_MIGRATION_LEGACY_HOME: rootRenameCase.legacyHome,
      HARNESS_MIGRATION_MANAGED_HOME: rootRenameCase.managedHome,
      HARNESS_MIGRATION_FAULT_MARKER: rootRenameCase.faultMarker,
    },
  })
  let rootRenameStderr = ""
  rootRenameProcess.stderr.on("data", (chunk) => { rootRenameStderr += chunk.toString() })
  const rootRenameExit = once(rootRenameProcess, "exit")
  const rootRenamePgid = processGroupId(rootRenameProcess.pid)
  context.after(() => killProcessGroup(rootRenamePgid))
  try {
    await waitForPath(rootRenameCase.faultMarker)
  } catch (error) {
    killProcessGroup(rootRenamePgid)
    throw new Error(`${error.message}: ${rootRenameStderr}`)
  }
  assert.equal(await readFile(rootRenameCase.faultMarker, "utf8"), "root-renamed\n")
  killProcessGroup(rootRenamePgid)
  let rootRenameResult
  try {
    rootRenameResult = await withTimeout(
      rootRenameExit,
      "root-rename migration interruption timed out",
    )
  } catch (error) {
    killProcessGroup(rootRenamePgid)
    const state = await Promise.all([
      lstat(rootRenameCase.managedHome).then(() => "managed", () => "no-managed"),
      lstat(rootRenameCase.legacyHome).then(() => "legacy", () => "no-legacy"),
      lstat(rootRenameCase.journal).then(() => "journal", () => "no-journal"),
      lstat(rootRenameCase.completion).then(() => "complete", () => "no-complete"),
    ])
    throw new Error(`${error.message}: ${state.join(",")}; pid=${rootRenameProcess.pid}; stderr=${rootRenameStderr}`)
  }
  await waitForProcessGroupExit(rootRenamePgid, "root-rename migration")
  const [rootRenameCode] = rootRenameResult
  assert.notEqual(rootRenameCode, 0)
  assert.equal(await lstat(rootRenameCase.journal).then(() => true, () => false), true)
  assert.equal(await lstat(rootRenameCase.completion).then(() => true, () => false), false)
  const rootRenameRetry = spawnSync(installer, rootRenameCase.args, {
    encoding: "utf8",
    env: rootRenameCase.env,
    timeout: 20_000,
    killSignal: "SIGKILL",
  })
  assert.equal(rootRenameRetry.status, 0, rootRenameRetry.stderr)
  await assertMigrated(rootRenameCase)

  const controlMoveCase = await createLegacyCase("after-control-move")
  const controlMoveProcess = spawn(installer, controlMoveCase.args, {
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
    env: {
      ...controlMoveCase.env,
      HARNESS_MIGRATION_KILL_AFTER_CONTROL_PATH: controlMoveCase.controlPath,
      HARNESS_MIGRATION_FAULT_MARKER: controlMoveCase.faultMarker,
    },
  })
  let controlMoveStderr = ""
  controlMoveProcess.stderr.on("data", (chunk) => { controlMoveStderr += chunk.toString() })
  const controlMoveExit = once(controlMoveProcess, "exit")
  const controlMovePgid = processGroupId(controlMoveProcess.pid)
  context.after(() => killProcessGroup(controlMovePgid))
  try {
    await waitForPath(controlMoveCase.faultMarker)
  } catch (error) {
    killProcessGroup(controlMovePgid)
    throw new Error(`${error.message}: ${controlMoveStderr}`)
  }
  assert.equal(await readFile(controlMoveCase.faultMarker, "utf8"), "control-moved\n")
  killProcessGroup(controlMovePgid)
  let controlMoveResult
  try {
    controlMoveResult = await withTimeout(
      controlMoveExit,
      "control-state migration interruption timed out",
    )
  } catch (error) {
    killProcessGroup(controlMovePgid)
    const state = await Promise.all([
      lstat(controlMoveCase.managedHome).then(() => "managed", () => "no-managed"),
      lstat(controlMoveCase.legacyHome).then(() => "legacy", () => "no-legacy"),
      lstat(controlMoveCase.journal).then(() => "journal", () => "no-journal"),
      lstat(controlMoveCase.controlPath).then(() => "control", () => "no-control"),
      lstat(controlMoveCase.completion).then(() => "complete", () => "no-complete"),
    ])
    throw new Error(`${error.message}: ${state.join(",")}; pid=${controlMoveProcess.pid}; stderr=${controlMoveStderr}`)
  }
  await waitForProcessGroupExit(controlMovePgid, "control-state migration")
  const [controlMoveCode] = controlMoveResult
  assert.notEqual(controlMoveCode, 0)
  assert.equal(await lstat(controlMoveCase.journal).then(() => true, () => false), true)
  assert.equal(await lstat(controlMoveCase.completion).then(() => true, () => false), false)
  const controlMoveRetry = spawnSync(installer, controlMoveCase.args, {
    encoding: "utf8",
    env: controlMoveCase.env,
    timeout: 20_000,
    killSignal: "SIGKILL",
  })
  assert.equal(controlMoveRetry.status, 0, controlMoveRetry.stderr)
  await assertMigrated(controlMoveCase)

  const replacementCase = await createLegacyCase("source-replacement")
  const replacement = spawnSync(installer, replacementCase.args, {
    encoding: "utf8",
    env: {
      ...replacementCase.env,
      HARNESS_MIGRATION_REPLACE_SOURCE: "1",
      HARNESS_MIGRATION_LEGACY_HOME: replacementCase.legacyHome,
    },
    timeout: 20_000,
    killSignal: "SIGKILL",
  })
  assert.equal(replacement.status, 1)
  assert.match(replacement.stderr, /identity changed during migration/)
  assert.equal(await lstat(replacementCase.managedHome).then(() => true, () => false), false)
  assert.equal(await lstat(replacementCase.legacyHome).then(() => true, () => false), true)
  assert.equal(await lstat(`${replacementCase.legacyHome}.original`).then(() => true, () => false), true)
  assert.equal(
    await readFile(join(`${replacementCase.legacyHome}.original`, "repositories/repo-1/HEAD"), "utf8"),
    "legacy-repository\n",
  )

  const ownershipCaseRoot = join(root, "ownership-recovery")
  await mkdir(ownershipCaseRoot, { recursive: true })
  const ownershipHarness = await createInstallerHarness(ownershipCaseRoot)
  const ownershipStateRoot = join(ownershipHarness.installRoot, "var/lib/chariox")
  const ownershipHome = join(ownershipHarness.installRoot, "home/chariox")
  const ownershipState = join(ownershipHome, ".chariox")
  await mkdir(ownershipStateRoot, { recursive: true })
  await mkdir(ownershipState, { recursive: true })
  await writeFile(join(ownershipHome, "user-file"), "must-survive\n")
  await chmod(ownershipHome, 0o755)
  await chmod(ownershipState, 0o755)
  const ownershipEnv = {
    ...process.env,
    PATH: `${ownershipHarness.bin}:${process.env.PATH}`,
    HARNESS_STATE: ownershipHarness.state,
    CHARIOX_IMAGE_INSTALL_ROOT: ownershipHarness.installRoot,
    CHARIOX_IMAGE_INSTALL_LOCK: join(ownershipHarness.state, "install.lock"),
  }
  const ownershipResult = spawnSync(
    installer,
    [rootfs, digest, fixture.trustedPublicKey],
    { encoding: "utf8", env: ownershipEnv, timeout: 20_000, killSignal: "SIGKILL" },
  )
  assert.equal(ownershipResult.status, 0, ownershipResult.stderr)
  assert.equal(await readFile(join(ownershipHome, "user-file"), "utf8"), "must-survive\n")
  assert.equal((await stat(ownershipHome)).mode & 0o777, 0o700)
  assert.equal((await stat(ownershipState)).mode & 0o777, 0o700)
  const ownershipChowns = await readFile(ownershipHarness.chownLog, "utf8")
  assert.match(ownershipChowns, new RegExp(`${ownershipHome}\\t998\\t998`))
  assert.match(ownershipChowns, new RegExp(`${ownershipState}\\t998\\t998`))
})

test("managed image installer rejects a linked artifact ancestor before host mutation", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-install-ancestor-link-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const output = join(root, "release")
  const packaged = runPackager({ ...fixture, output })
  assert.equal(packaged.status, 0, packaged.stderr)

  const rootfs = join(output, "rootfs")
  const externalLocal = join(root, "external-local")
  await rename(join(rootfs, "usr/local"), externalLocal)
  await symlink(externalLocal, join(rootfs, "usr/local"))

  const harness = await createInstallerHarness(root)
  const result = spawnSync(
    installer,
    [rootfs, packaged.stdout.trim(), fixture.trustedPublicKey],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        PATH: `${harness.bin}:${process.env.PATH}`,
        HARNESS_STATE: harness.state,
        CHARIOX_IMAGE_INSTALL_ROOT: harness.installRoot,
        CHARIOX_IMAGE_INSTALL_LOCK: join(harness.state, "install.lock"),
      },
    },
  )
  assert.equal(result.status, 1)
  assert.match(result.stderr, /image root contains a symbolic link/)
  assert.equal(await lstat(harness.installRoot).then(() => true, () => false), false)
})

test("managed image installer atomically pivots current to a different release", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-install-pivot-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const firstRoot = join(root, "first-fixture")
  const secondRoot = join(root, "second-fixture")
  await mkdir(firstRoot)
  await mkdir(secondRoot)
  const firstFixture = await makeFixture(firstRoot, "one")
  const secondFixture = await makeFixture(secondRoot, "two")
  const firstOutput = join(root, "first-release")
  const secondOutput = join(root, "second-release")
  const firstPackage = runPackager({ ...firstFixture, output: firstOutput })
  const secondPackage = runPackager({ ...secondFixture, output: secondOutput })
  assert.equal(firstPackage.status, 0, firstPackage.stderr)
  assert.equal(secondPackage.status, 0, secondPackage.stderr)
  assert.notEqual(firstPackage.stdout, secondPackage.stdout)

  const harness = await createInstallerHarness(root)
  const env = {
    ...process.env,
    PATH: `${harness.bin}:${process.env.PATH}`,
    HARNESS_STATE: harness.state,
    CHARIOX_IMAGE_INSTALL_ROOT: harness.installRoot,
    CHARIOX_IMAGE_INSTALL_LOCK: join(harness.state, "install.lock"),
  }
  const installRelease = (output, packaged, fixture, environment = env) => spawnSync(installer, [
    join(output, "rootfs"), packaged.stdout.trim(), fixture.trustedPublicKey,
  ], { encoding: "utf8", env: environment })
  const first = installRelease(firstOutput, firstPackage, firstFixture)
  assert.equal(first.status, 0, first.stderr)
  const current = join(harness.installRoot, "usr/lib/chariox/current")
  assert.equal(await readlink(current), `releases/${firstPackage.stdout.trim().slice(7)}`)

  const failedLinger = installRelease(secondOutput, secondPackage, secondFixture, {
    ...env,
    HARNESS_LOGINCTL_FAIL: "1",
  })
  assert.equal(failedLinger.status, 1)
  assert.match(failedLinger.stderr, /restored previous current release/)
  assert.equal(await readlink(current), `releases/${firstPackage.stdout.trim().slice(7)}`)

  const failedSecond = installRelease(secondOutput, secondPackage, secondFixture, {
    ...env,
    HARNESS_SYSTEMCTL_FAIL: "enable chariox-managed-bootstrap.service",
  })
  assert.equal(failedSecond.status, 1)
  assert.match(failedSecond.stderr, /restored previous current release/)
  assert.equal(await readlink(current), `releases/${firstPackage.stdout.trim().slice(7)}`)

  const second = installRelease(secondOutput, secondPackage, secondFixture)
  assert.equal(second.status, 0, second.stderr)
  assert.equal(await readlink(current), `releases/${secondPackage.stdout.trim().slice(7)}`)
  assert.equal(
    await readFile(join(harness.installRoot, "usr/local/bin/chariox-kernel"), "utf8"),
    secondFixture.kernelContents,
  )
})

test("a terminated managed image install releases the lock for a concurrent install", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-install-lock-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeFixture(root)
  const output = join(root, "release")
  const packaged = runPackager({ ...fixture, output })
  assert.equal(packaged.status, 0, packaged.stderr)
  const harness = await createInstallerHarness(root)
  const env = {
    ...process.env,
    PATH: `${harness.bin}:${process.env.PATH}`,
    HARNESS_STATE: harness.state,
    CHARIOX_IMAGE_INSTALL_ROOT: harness.installRoot,
    CHARIOX_IMAGE_INSTALL_LOCK: join(harness.state, "install.lock"),
  }
  const args = [join(output, "rootfs"), packaged.stdout.trim(), fixture.trustedPublicKey]
  const first = spawn(installer, args, { env: { ...env, HARNESS_FLOCK_ID: "first" } })
  context.after(() => {
    first.kill("SIGKILL")
  })
  const firstExit = once(first, "exit")
  await waitForPath(join(harness.state, "flock-acquired-first"))
  const second = spawn(installer, args, { env: { ...env, HARNESS_FLOCK_ID: "second" } })
  context.after(() => {
    second.kill("SIGKILL")
  })
  const secondExit = once(second, "exit")
  await waitForPath(join(harness.state, "flock-entered-second"))
  assert.equal(await lstat(join(harness.state, "flock-acquired-second")).then(() => true, () => false), false)
  assert.equal(await lstat(harness.installRoot).then(() => true, () => false), false)
  for (const name of ["group-chariox", "group-chariox-slice", "group-chariox-docker", "user-chariox", "user-chariox-docker"]) {
    assert.equal(await lstat(join(harness.state, name)).then(() => true, () => false), false)
  }
  first.kill("SIGTERM")
  await writeFile(join(harness.state, "flock-release-first"), "release\n")
  const [firstCode, firstSignal] = await withTimeout(firstExit, "terminated installer timed out")
  assert.equal(firstCode, null)
  assert.equal(firstSignal, "SIGTERM")
  await waitForPath(join(harness.state, "flock-acquired-second"))
  await writeFile(join(harness.state, "flock-release-second"), "release\n")
  assert.equal((await withTimeout(secondExit, "second installer timed out"))[0], 0)
})

test("managed image installer has no runtime start or network path", async () => {
  const contents = await readFile(installer, "utf8")
  assert.match(contents, /if \[ "\$\(id -u\)" -ne 0 \]/)
  assert.match(contents, /node "\$script_root\/verify-image-release\.mjs"/)
  assert.match(contents, /managed kernel state root contains an unrecognized entry/)
  assert.match(contents, /groupadd --system chariox/)
  assert.match(contents, /groupadd --system chariox-docker/)
  assert.match(contents, /groupadd --system chariox-slice/)
  assert.match(contents, /useradd --system --gid chariox --home-dir \/home\/chariox/)
  assert.match(contents, /useradd --system --gid chariox-docker --home-dir \/var\/lib\/chariox-docker\/home/)
  assert.match(contents, /systemctl daemon-reload/)
  assert.match(contents, /path1\) selected_bootstrap_service=chariox-path1-managed-bootstrap\.service/)
  assert.match(contents, /shared_host\) selected_bootstrap_service=chariox-managed-bootstrap\.service/)
  assert.match(contents, /systemctl enable "\$selected_bootstrap_service"/)
  const lockIndex = contents.indexOf("flock 9")
  for (const mutation of ["state_entry=$(find", "groupadd --system", "useradd --system", "usermod --append", "\ninstall -d", "releases_root="]) {
    assert.ok(lockIndex >= 0 && lockIndex < contents.indexOf(mutation), `${mutation} must remain behind the install lock`)
  }
  assert.doesNotMatch(contents, /systemctl (?:start|restart|enable --now)/)
  assert.doesNotMatch(contents, /\b(?:curl|wget|ssh|scp)\b/)
  assert.doesNotMatch(contents, /\bmv\s+-T/)
  assert.match(contents, /renameSync\(source, destination\)/)
  assert.doesNotMatch(contents, /\.arroba/)
})
