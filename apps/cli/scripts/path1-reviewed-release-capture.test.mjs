import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { createHash, generateKeyPairSync, sign } from "node:crypto"
import { chmod, lstat, mkdir, readFile, readdir, realpath, rename, rm, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { basename, dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"

import { capturePath1ReviewedRelease, verifyPath1ReviewedRelease } from "./path1-reviewed-release-capture.mjs"

const SOURCE_COMMIT = "a".repeat(40)
const SOURCE_TREE = "b".repeat(40)
const TARGET = "x86_64-unknown-linux-gnu"
const RELEASE_ARTIFACTS = [
  ["chariox-kernel", "/usr/local/bin/chariox-kernel", "file", 0o755],
  ["chariox-managed-bootstrap", "/usr/local/bin/chariox-managed-bootstrap", "file", 0o755],
  ["chariox-managed-bootstrap.service", "/etc/systemd/system/chariox-managed-bootstrap.service", "file", 0o644],
  ["chariox-path1-managed-bootstrap.service", "/etc/systemd/system/chariox-path1-managed-bootstrap.service", "file", 0o644],
  ["chariox-disposable-worker-bootstrap.service", "/etc/systemd/system/chariox-disposable-worker-bootstrap.service", "file", 0o644],
  ["chariox-rootless-docker.service", "/etc/systemd/system/chariox-rootless-docker.service", "file", 0o644],
  ["chariox-slice-broker.service", "/etc/systemd/system/chariox-slice-broker.service", "file", 0o644],
  ["chariox-slice-build-context", "/usr/lib/chariox/slice-build-context", "tree", 0o755],
  ["chariox-build-attestation", "/usr/lib/chariox/build-attestation.json", "file", 0o644],
  ["chariox-build-attestation-signature", "/usr/lib/chariox/build-attestation.sig", "file", 0o644],
  ["chariox-builder-public-key", "/usr/lib/chariox/builder-public-key", "file", 0o644],
]
const CONTEXT_EXECUTABLES = [
  "/slice-linux-docker/prebuilt/chariox-kernel",
  "/slice-linux-docker/prebuilt/chariox-relay",
  "/enter-rootless-docker-namespace.sh",
  "/managed-rootless-service.sh",
  "/provision-linux-docker-slice.sh",
  "/managed-publication-access.sh",
]

function sha256(bytes) {
  return "sha256:" + createHash("sha256").update(bytes).digest("hex")
}

function keyPair() {
  const pair = generateKeyPairSync("ed25519")
  const der = pair.publicKey.export({ format: "der", type: "spki" })
  return { privateKey: pair.privateKey, raw: der.subarray(der.length - 32) }
}

async function makeDirectory(path) {
  await mkdir(path, { recursive: true, mode: 0o755 })
  await chmod(path, 0o755)
}

async function writeInstalledFile(root, absolutePath, bytes, mode) {
  const path = join(root, absolutePath.slice(1))
  await makeDirectory(dirname(path))
  await writeFile(path, bytes, { mode })
  await chmod(path, mode)
  return path
}

async function writeContextFile(contextRoot, relativePath, bytes, mode) {
  const path = join(contextRoot, relativePath)
  await mkdir(dirname(path), { recursive: true, mode: 0o755 })
  await chmod(contextRoot, 0o755)
  const directories = relativePath.split("/").slice(0, -1)
  let current = contextRoot
  for (const directory of directories) {
    current = join(current, directory)
    await chmod(current, 0o755)
  }
  await writeFile(path, bytes, { mode })
  await chmod(path, mode)
}

async function contextDigest(root) {
  const hash = createHash("sha256")
  hash.update("directory:1:.:493:")
  const walk = async (directory, prefix) => {
    const names = await readdir(directory)
    names.sort((left, right) => (left < right ? -1 : left > right ? 1 : 0))
    for (const name of names) {
      const path = join(directory, name)
      const relativePath = prefix ? prefix + "/" + name : name
      const metadata = await lstat(path)
      const mode = metadata.mode & 0o7777
      const byteLength = Buffer.byteLength(relativePath)
      if (metadata.isDirectory()) {
        hash.update("directory:" + byteLength + ":" + relativePath + ":" + mode + ":")
        await walk(path, relativePath)
      } else {
        hash.update("file:" + byteLength + ":" + relativePath + ":" + mode + ":" + metadata.size + ":")
        hash.update(await readFile(path))
      }
    }
  }
  await walk(root, "")
  return "sha256:" + hash.digest("hex")
}

async function writeTrustedKey(path, raw) {
  await writeFile(path, raw.toString("base64"), { mode: 0o644 })
}

async function createFixture(options = {}) {
  const { mkdtemp } = await import("node:fs/promises")
  const temporaryRoot = await realpath(await mkdtemp(join(tmpdir(), "path1-reviewed-release-")))
  const draftRoot = join(temporaryRoot, "draft")
  await makeDirectory(draftRoot)
  const releases = join(temporaryRoot, "releases")
  await makeDirectory(releases)
  const contextRoot = join(draftRoot, "usr/lib/chariox/slice-build-context")

  const releasePair = keyPair()
  const builderPair = keyPair()
  const otherReleasePair = keyPair()
  const otherBuilderPair = keyPair()
  const sourceCommit = options.sourceCommit ?? SOURCE_COMMIT
  const sourceTree = options.sourceTree ?? SOURCE_TREE

  const kernel = Buffer.from("#!/bin/sh\nkernel fixture\n")
  const bootstrap = Buffer.from("#!/bin/sh\nbootstrap fixture\n")
  const relay = Buffer.from("#!/bin/sh\nrelay fixture\n")
  await writeInstalledFile(draftRoot, "/usr/local/bin/chariox-kernel", kernel, 0o755)
  await writeInstalledFile(draftRoot, "/usr/local/bin/chariox-managed-bootstrap", bootstrap, 0o755)

  const services = [
    ["/etc/systemd/system/chariox-managed-bootstrap.service", "service managed"],
    ["/etc/systemd/system/chariox-path1-managed-bootstrap.service", "service path1"],
    ["/etc/systemd/system/chariox-disposable-worker-bootstrap.service", "service worker"],
    ["/etc/systemd/system/chariox-rootless-docker.service", "service rootless"],
    ["/etc/systemd/system/chariox-slice-broker.service", "service slice"],
  ]
  for (const [path, body] of services) {
    await writeInstalledFile(draftRoot, path, Buffer.from(body + "\n"), 0o644)
  }

  await writeContextFile(
    contextRoot,
    "apps/kernel/slice-linux-docker/prebuilt/chariox-kernel",
    kernel,
    0o755,
  )
  await writeContextFile(
    contextRoot,
    "apps/kernel/slice-linux-docker/prebuilt/chariox-relay",
    relay,
    0o755,
  )
  await writeContextFile(contextRoot, "apps/kernel/managed-rootless-service.sh", Buffer.from("#!/bin/sh\n"), 0o755)
  await writeContextFile(contextRoot, "apps/kernel/README.md", Buffer.from("fixture source\n"), 0o644)

  const attestedSourceCommit = options.attestedSourceCommit ?? sourceCommit
  const attestedSourceTree = options.attestedSourceTree ?? sourceTree
  const attestationBytes = Buffer.from(
    JSON.stringify({
      schemaVersion: 1,
      sourceCommit: attestedSourceCommit,
      sourceTree: attestedSourceTree,
      target: TARGET,
      artifacts: [
        { name: "chariox-kernel", sha256: sha256(kernel) },
        { name: "chariox-managed-bootstrap", sha256: sha256(bootstrap) },
        { name: "chariox-relay", sha256: sha256(relay) },
      ],
    }),
  )
  const attestationSignature = sign(null, attestationBytes, builderPair.privateKey)
  await writeInstalledFile(draftRoot, "/usr/lib/chariox/build-attestation.json", attestationBytes, 0o644)
  await writeInstalledFile(
    draftRoot,
    "/usr/lib/chariox/build-attestation.sig",
    Buffer.from(attestationSignature.toString("base64")),
    0o644,
  )
  await writeInstalledFile(
    draftRoot,
    "/usr/lib/chariox/builder-public-key",
    Buffer.from(builderPair.raw.toString("base64")),
    0o644,
  )

  const artifactDigests = new Map()
  for (const [name, path, kind] of RELEASE_ARTIFACTS) {
    if (kind === "tree") {
      artifactDigests.set(name, await contextDigest(contextRoot))
    } else {
      artifactDigests.set(name, sha256(await readFile(join(draftRoot, path.slice(1)))))
    }
  }
  const manifestArtifacts = RELEASE_ARTIFACTS.map(([name, path]) => ({
    name,
    path,
    sha256: artifactDigests.get(name),
  }))
  if (options.duplicateArtifact) manifestArtifacts.push({ ...manifestArtifacts[0] })

  let manifestText = JSON.stringify({
      schemaVersion: 2,
      sourceCommit,
      sourceTree,
      artifacts: manifestArtifacts,
    })
  if (options.duplicateField) manifestText = manifestText.replace('"schemaVersion":2,', '"schemaVersion":2,"schemaVersion":2,')
  const manifestBytes = Buffer.from(manifestText)
  const releaseSignature = sign(null, manifestBytes, releasePair.privateKey)
  await writeInstalledFile(draftRoot, "/usr/lib/chariox/release-manifest.json", manifestBytes, 0o644)
  await writeInstalledFile(
    draftRoot,
    "/usr/lib/chariox/release-manifest.sig",
    Buffer.from(releaseSignature.toString("base64")),
    0o644,
  )
  await writeInstalledFile(
    draftRoot,
    "/usr/lib/chariox/release-public-key",
    Buffer.from(releasePair.raw.toString("base64")),
    0o644,
  )

  const digestName = createHash("sha256").update(manifestBytes).digest("hex")
  const releaseRoot = join(releases, digestName)
  await rename(draftRoot, releaseRoot)
  const trustedReleasePublicKeyPath = join(temporaryRoot, "trusted-release.pub")
  const trustedBuilderPublicKeyPath = join(temporaryRoot, "trusted-builder.pub")
  const otherReleasePublicKeyPath = join(temporaryRoot, "other-release.pub")
  const otherBuilderPublicKeyPath = join(temporaryRoot, "other-builder.pub")
  await writeTrustedKey(trustedReleasePublicKeyPath, releasePair.raw)
  await writeTrustedKey(trustedBuilderPublicKeyPath, builderPair.raw)
  await writeTrustedKey(otherReleasePublicKeyPath, otherReleasePair.raw)
  await writeTrustedKey(otherBuilderPublicKeyPath, otherBuilderPair.raw)

  return {
    temporaryRoot,
    releaseRoot,
    options: {
      releaseRoot,
      trustedReleasePublicKeyPath,
      trustedBuilderPublicKeyPath,
      expectedSourceCommit: SOURCE_COMMIT,
      expectedSourceTree: SOURCE_TREE,
      executable: "chariox-kernel",
      evidenceOutputPath: join(temporaryRoot, "captured-evidence.json"),
    },
    otherReleasePublicKeyPath,
    otherBuilderPublicKeyPath,
  }
}

async function withFixture(options, run) {
  const fixture = await createFixture(options)
  try {
    await run(fixture)
  } finally {
    await rm(fixture.temporaryRoot, { recursive: true, force: true })
  }
}

test("captures the exact installed schema-v2 release format with independently trusted fixture keys", async () => {
  await withFixture({}, async (fixture) => {
    const evidence = await capturePath1ReviewedRelease(fixture.options)
    assert.equal(evidence.releaseManifestDigest, "sha256:" + basename(fixture.releaseRoot))
    assert.equal(evidence.sourceCommit, SOURCE_COMMIT)
    assert.equal(evidence.sourceTree, SOURCE_TREE)
    assert.equal(evidence.target, TARGET)
    assert.deepEqual(evidence.selectedExecutable, {
      name: "chariox-kernel",
      path: "/usr/local/bin/chariox-kernel",
      sha256: evidence.artifactBindings[0].actualSha256,
    })
    assert.equal(evidence.artifactBindings.length, 11)
    assert.equal(evidence.artifactBindings[3].name, "chariox-path1-managed-bootstrap.service")
    assert.equal(evidence.artifactBindings[3].sha256, evidence.artifactBindings[3].actualSha256)
    assert.equal(evidence.artifactBindings[7].name, "chariox-slice-build-context")
    assert.ok(evidence.releaseManifestBase64)
    assert.ok(evidence.releaseManifestSignatureBase64)
    assert.ok(evidence.builderAttestationBase64)
    assert.ok(evidence.builderAttestationSignatureBase64)
    assert.equal(Object.keys(evidence).some((key) => /verified|accepted|verdict/i.test(key)), false)
    const output = await readFile(fixture.options.evidenceOutputPath, "utf8")
    assert.deepEqual(JSON.parse(output), evidence)
    const outputMetadata = await lstat(fixture.options.evidenceOutputPath)
    assert.equal(outputMetadata.mode & 0o777, 0o600)
    await assert.rejects(capturePath1ReviewedRelease(fixture.options), /new writable regular file/)
    await assert.rejects(
      capturePath1ReviewedRelease({
        ...fixture.options,
        evidenceOutputPath: join(fixture.releaseRoot, "capture.json"),
      }),
      /outside the release/,
    )
  })
})

test("verifies a known non-kernel executable selection from the signed attestation", async () => {
  await withFixture({}, async (fixture) => {
    fixture.options.executable = "chariox-relay"
    const evidence = await verifyPath1ReviewedRelease(fixture.options)
    assert.equal(evidence.selectedExecutable.name, "chariox-relay")
    assert.equal(
      evidence.selectedExecutable.path,
      "/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-relay",
    )
  })
})

test("rejects symlinked output parents without writing evidence", async () => {
  await withFixture({}, async (fixture) => {
    const alias = join(fixture.temporaryRoot, "evidence-alias")
    await symlink(fixture.temporaryRoot, alias)
    await assert.rejects(capturePath1ReviewedRelease({
      ...fixture.options,
      evidenceOutputPath: join(alias, "symlink-output.json"),
    }), /symlink/)
    await assert.rejects(lstat(join(fixture.temporaryRoot, "symlink-output.json")), { code: "ENOENT" })
  })
})

test("rejects repository evidence destinations without creating a file", async () => {
  await withFixture({}, async (fixture) => {
    const output = fileURLToPath(new URL("./release-evidence-must-not-exist.json", import.meta.url))
    await assert.rejects(lstat(output), { code: "ENOENT" })
    await assert.rejects(capturePath1ReviewedRelease({
      ...fixture.options,
      evidenceOutputPath: output,
    }), /outside the repository/)
    await assert.rejects(lstat(output), { code: "ENOENT" })
  })
})

test("CLI accepts only explicit trust and source inputs and writes evidence", async () => {
  await withFixture({}, async (fixture) => {
    const outputPath = join(fixture.temporaryRoot, "cli-evidence.json")
    const scriptPath = fileURLToPath(new URL("./path1-reviewed-release-capture.mjs", import.meta.url))
    const result = spawnSync(
      process.execPath,
      [
        scriptPath,
        "--release-root", fixture.options.releaseRoot,
        "--trusted-release-public-key", fixture.options.trustedReleasePublicKeyPath,
        "--trusted-builder-public-key", fixture.options.trustedBuilderPublicKeyPath,
        "--expected-source-commit", fixture.options.expectedSourceCommit,
        "--expected-source-tree", fixture.options.expectedSourceTree,
        "--executable", "chariox-kernel",
        "--evidence-output", outputPath,
      ],
      { encoding: "utf8", timeout: 5000 },
    )
    assert.equal(result.status, 0, result.stderr)
    assert.match(result.stdout, /release manifest digest: sha256:[0-9a-f]{64}/)
    assert.match(result.stdout, /raw verification evidence written:/)
    assert.equal(JSON.parse(await readFile(outputPath, "utf8")).sourceCommit, SOURCE_COMMIT)
    assert.equal((await lstat(outputPath)).mode & 0o777, 0o600)
  })
})

test("rejects a wrong independently trusted release key", async () => {
  await withFixture({}, async (fixture) => {
    fixture.options.trustedReleasePublicKeyPath = fixture.otherReleasePublicKeyPath
    await assert.rejects(verifyPath1ReviewedRelease(fixture.options), /included release key does not match/)
  })
})

test("rejects a wrong independently trusted builder key", async () => {
  await withFixture({}, async (fixture) => {
    fixture.options.trustedBuilderPublicKeyPath = fixture.otherBuilderPublicKeyPath
    await assert.rejects(verifyPath1ReviewedRelease(fixture.options), /included builder key does not match/)
  })
})

test("rejects signed source identities that do not match the exact expected commit and tree", async () => {
  await withFixture({ sourceCommit: "c".repeat(40), attestedSourceCommit: "c".repeat(40) }, async (fixture) => {
    await assert.rejects(verifyPath1ReviewedRelease(fixture.options), /expected commit and tree/)
  })
  await withFixture({ sourceTree: "d".repeat(40), attestedSourceTree: "d".repeat(40) }, async (fixture) => {
    await assert.rejects(verifyPath1ReviewedRelease(fixture.options), /expected commit and tree/)
  })
  await withFixture({ attestedSourceCommit: "c".repeat(40) }, async (fixture) => {
    await assert.rejects(verifyPath1ReviewedRelease(fixture.options), /builder attestation source identity/)
  })
  await withFixture({ attestedSourceTree: "d".repeat(40) }, async (fixture) => {
    await assert.rejects(verifyPath1ReviewedRelease(fixture.options), /builder attestation source identity/)
  })
})

test("rejects tampered direct and slice executable artifacts", async () => {
  await withFixture({}, async (fixture) => {
    await writeFile(join(fixture.releaseRoot, "usr/local/bin/chariox-kernel"), Buffer.from("tampered"))
    await assert.rejects(verifyPath1ReviewedRelease(fixture.options), /chariox-kernel artifact digest/)
  })
  await withFixture({}, async (fixture) => {
    await writeFile(
      join(
        fixture.releaseRoot,
        "usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-relay",
      ),
      Buffer.from("tampered"),
    )
    await assert.rejects(verifyPath1ReviewedRelease(fixture.options), /slice build context digest/)
  })
})

test("rejects symlinked artifacts, duplicate manifest entries, and unknown executable selections", async () => {
  await withFixture({}, async (fixture) => {
    const kernel = join(fixture.releaseRoot, "usr/local/bin/chariox-kernel")
    const target = join(fixture.temporaryRoot, "external-kernel")
    await writeFile(target, Buffer.from("external"))
    await rm(kernel)
    await symlink(target, kernel)
    await assert.rejects(verifyPath1ReviewedRelease(fixture.options), /symlinked/)
  })
  await withFixture({ duplicateArtifact: true }, async (fixture) => {
    await assert.rejects(verifyPath1ReviewedRelease(fixture.options), /installed format/)
  })
  await withFixture({ duplicateField: true }, async (fixture) => {
    await assert.rejects(verifyPath1ReviewedRelease(fixture.options), /duplicate field/)
  })
  await withFixture({}, async (fixture) => {
    fixture.options.executable = "/usr/local/bin/unknown"
    await assert.rejects(verifyPath1ReviewedRelease(fixture.options), /unknown executable selection/)
  })
})
