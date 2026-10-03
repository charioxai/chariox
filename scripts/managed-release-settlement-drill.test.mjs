import assert from "node:assert/strict"
import { createHash, randomUUID } from "node:crypto"
import { spawn } from "node:child_process"
import { delimiter, dirname, join } from "node:path"
import { chmod, lstat, mkdtemp, mkdir, readFile, readdir, realpath, rm, rmdir, symlink, unlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { fileURLToPath } from "node:url"
import test from "node:test"
import { acquireManagedReleaseBuilderLease } from "./managed-release-settlement.mjs"
import {
  DockerBuildxEngine,
  OWNER_FILE,
  parseHistoryRows,
  removeOwnedDirectory,
  runSettlementScenario,
  trackChild,
  writeBuildContext,
} from "./managed-release-settlement-drill.mjs"
import { verifyOciBaseLayout } from "./managed-release-oci-layout.mjs"

const BUILDER = "chariox-settlement-test-00000000"
const NODE = "builder0"
const FINGERPRINT = {
  name: BUILDER,
  nodes: [{
    name: NODE,
    endpoint: "test-context",
    containerId: "a".repeat(64),
    startedAt: "2026-09-27T00:00:00.000Z",
    pidsLimit: 128,
    effectiveCpus: 1,
    memoryBytes: 1024 ** 3,
    memoryWithSwapBytes: 2 * 1024 ** 3,
    init: true,
    networkMode: "none",
  }],
}
const REPOSITORY = await realpath(dirname(dirname(fileURLToPath(import.meta.url))))
const OCI_INDEX_MEDIA_TYPE = "application/vnd.oci.image.index.v1+json"
const OCI_MANIFEST_MEDIA_TYPE = "application/vnd.oci.image.manifest.v1+json"
const OCI_CONFIG_MEDIA_TYPE = "application/vnd.oci.image.config.v1+json"
const OCI_LAYER_MEDIA_TYPE = "application/vnd.oci.image.layer.v1.tar"

function digest(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`
}

function ociDescriptor(mediaType, bytes, platform) {
  return { mediaType, digest: digest(bytes), size: bytes.length, ...(platform ? { platform } : {}) }
}

async function makeOciLayout(parent, {
  symlinkLayer = false,
  missingLayer = false,
  ambiguousPlatform = false,
  foreignDescriptor = false,
  auxiliaryArtifact = true,
} = {}) {
  const layout = join(parent, "layout")
  const blobs = join(layout, "blobs", "sha256")
  await mkdir(blobs, { recursive: true, mode: 0o700 })
  const layerBytes = Buffer.alloc(1024)
  const layer = ociDescriptor(OCI_LAYER_MEDIA_TYPE, layerBytes)
  const configBytes = Buffer.from(JSON.stringify({
    architecture: "amd64",
    os: "linux",
    rootfs: { type: "layers", diff_ids: [digest(layerBytes)] },
  }))
  const config = ociDescriptor(OCI_CONFIG_MEDIA_TYPE, configBytes)
  const manifestBytes = Buffer.from(JSON.stringify({
    schemaVersion: 2,
    mediaType: OCI_MANIFEST_MEDIA_TYPE,
    config,
    layers: [layer],
  }))
  const manifest = ociDescriptor(OCI_MANIFEST_MEDIA_TYPE, manifestBytes, { os: "linux", architecture: "amd64" })
  const nonselected = {
    mediaType: OCI_MANIFEST_MEDIA_TYPE,
    digest: `sha256:${"f".repeat(64)}`,
    size: 123,
    platform: { os: "linux", architecture: "arm64" },
  }
  const indexManifests = [nonselected, manifest]
  if (ambiguousPlatform) {
    indexManifests.push({ ...manifest, digest: `sha256:${"e".repeat(64)}` })
  }
  if (foreignDescriptor) {
    indexManifests.push({
      mediaType: "application/vnd.docker.distribution.manifest.v2+json",
      digest: `sha256:${"d".repeat(64)}`,
      size: 123,
      platform: { os: "linux", architecture: "s390x" },
      urls: ["https://example.invalid/foreign"],
    })
  }
  const indexBytes = Buffer.from(JSON.stringify({
    schemaVersion: 2,
    mediaType: OCI_INDEX_MEDIA_TYPE,
    manifests: indexManifests,
  }))
  const index = ociDescriptor(OCI_INDEX_MEDIA_TYPE, indexBytes)
  const artifact = {
    mediaType: OCI_MANIFEST_MEDIA_TYPE,
    digest: "sha256:f5dc0fe8b50909a5905558c86ce9983eca9542c513068ec82112869b0b96faf6",
    size: 880,
    annotations: { "io.containerd.manifest.subject": index.digest },
    artifactType: "application/vnd.dev.sigstore.bundle.v0.3+json",
  }
  const rootIndex = Buffer.from(JSON.stringify({
    schemaVersion: 2,
    mediaType: OCI_INDEX_MEDIA_TYPE,
    manifests: [index, ...(auxiliaryArtifact ? [artifact] : [])],
  }))
  await writeFile(join(layout, "oci-layout"), JSON.stringify({ imageLayoutVersion: "1.0.0" }))
  await writeFile(join(layout, "index.json"), rootIndex)
  for (const [descriptor, bytes] of [[index, indexBytes], [manifest, manifestBytes], [config, configBytes]]) {
    await writeFile(join(blobs, descriptor.digest.slice("sha256:".length)), bytes)
  }
  const externalLayer = join(parent, "external-layer.blob")
  if (symlinkLayer) {
    await writeFile(externalLayer, layerBytes)
    await symlink(externalLayer, join(blobs, layer.digest.slice("sha256:".length)))
  } else if (!missingLayer) {
    await writeFile(join(blobs, layer.digest.slice("sha256:".length)), layerBytes)
  }
  const freeze = async (path) => {
    for (const entry of await readdir(path, { withFileTypes: true })) {
      const child = join(path, entry.name)
      if (entry.isDirectory()) {
        await freeze(child)
        await chmod(child, 0o555)
      } else if (!entry.isSymbolicLink()) {
        await chmod(child, 0o444)
      }
    }
  }
  await freeze(layout)
  await chmod(layout, 0o555)
  if (symlinkLayer) await chmod(externalLayer, 0o444)
  const expected = {
    mode: "oci-layout",
    layoutPath: await realpath(layout),
    indexDigest: index.digest,
    manifestDigest: manifest.digest,
  }
  return { layout: expected.layoutPath, expected, index, manifest, layer, artifact, externalLayer }
}

async function makeWritableAndRemove(path) {
  let metadata
  try { metadata = await lstat(path) } catch { return }
  if (metadata.isSymbolicLink()) {
    await unlink(path)
    return
  }
  if (metadata.isDirectory()) {
    await chmod(path, 0o700)
    for (const entry of await readdir(path)) await makeWritableAndRemove(join(path, entry))
    await rmdir(path)
  } else {
    await chmod(path, 0o600)
    await rm(path, { force: true })
  }
}

async function stopFixtureProcess(pid) {
  try { process.kill(pid, "SIGKILL") } catch (error) {
    if (error?.code === "ESRCH") return
    throw error
  }
  for (let attempt = 0; attempt < 80; attempt++) {
    try { process.kill(pid, 0) } catch (error) {
      if (error?.code === "ESRCH") return
      throw error
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 25))
  }
  throw new Error("owned escaped-pipe fixture process did not exit after termination")
}

function historyRow(ref, status, completed = false) {
  return {
    ref,
    name: "settlement-fixture",
    status,
    created_at: "2026-09-27T00:00:00.000Z",
    total_steps: 3,
    completed_steps: status === "running" ? 1 : 3,
    cached_steps: 0,
    ...(completed ? { completed_at: "2026-09-27T00:01:00.000Z" } : {}),
  }
}

async function fixtureRoots() {
  const temporaryParent = await mkdtemp(join(tmpdir(), "managed-release-settlement-"))
  const parent = await realpath(temporaryParent)
  const scratchToken = randomUUID()
  const scratch = join(parent, `scratch-${scratchToken}`)
  const evidence = join(parent, "evidence-owned")
  const home = join(scratch, "home")
  await mkdir(scratch, { mode: 0o700 })
  await mkdir(evidence, { mode: 0o700 })
  await mkdir(home, { mode: 0o700 })
  await writeFile(join(scratch, OWNER_FILE), JSON.stringify({ kind: "scratch", token: scratchToken }), { mode: 0o600 })
  await writeFile(join(evidence, OWNER_FILE), JSON.stringify({ kind: "evidence", token: randomUUID() }), { mode: 0o600 })
  return {
    parent: temporaryParent,
    home,
    scratch: { directory: scratch, token: scratchToken, kind: "scratch" },
    evidence: { directory: evidence, token: "unused", kind: "evidence" },
  }
}

function fakeEngine(options = {}) {
  let row
  let terminalInspect
  let sourceDirectory
  let startTime
  let inspectCalls = 0
  const fingerprint = structuredClone(FINGERPRINT)
  return {
    async inspectBuilder() {
      inspectCalls += 1
      if (options.builderDrift && inspectCalls > 1) {
        return {
          ...fingerprint,
          nodes: [{ ...fingerprint.nodes[0], pidsLimit: 127 }],
        }
      }
      return structuredClone(fingerprint)
    },
    async historyList() {
      return row ? `${JSON.stringify(row)}\n` : ""
    },
    async historyInspect(id, cwd) {
      assert.equal(id, "solve-001", "production reconciliation must inspect with a bare Buildx ID")
      assert.equal(cwd, sourceDirectory, "inspect must run from the retained invocation source")
      return {
        Ref: options.foreignInspect ? "foreign-id" : id,
        Context: cwd,
        Target: "managed-release-artifacts",
        StartedAt: startTime,
        ...(terminalInspect ?? { Status: "running" }),
      }
    },
    async startBuild(context, { invocationId }) {
      sourceDirectory = context
      startTime = new Date().toISOString()
      row = historyRow(options.foreignHistory ? "another-builder/node0/foreign-id" : `${BUILDER}/${NODE}/solve-001`, "running")
      if (options.builderDrift) fingerprint.nodes[0].pidsLimit = 127
      let closed = false
      const completion = new Promise(() => {})
      return {
        completion,
        get closed() { return closed },
        async interrupt() {
          closed = true
          if (options.sourceDrift) await writeFile(join(sourceDirectory, "fixture.txt"), `mutated ${invocationId}\n`)
          row = historyRow(row.ref, "error", true)
          terminalInspect = { Status: "error", CompletedAt: new Date(Date.now() + 1_000).toISOString() }
          return { status: 130, signal: null, interrupted: true, reaped: true, streamsClosed: true }
        },
      }
    },
  }
}

async function runFake(options = {}) {
  const roots = await fixtureRoots()
  try {
    const result = await runSettlementScenario({
      ownedScratch: roots.scratch,
      evidence: roots.evidence,
      home: roots.home,
      builderName: BUILDER,
      baseImage: "busybox:stable@sha256:" + "b".repeat(64),
      repository: REPOSITORY,
      engine: fakeEngine(options),
      sleep: async () => {},
    })
    return { roots, result }
  } catch (error) {
    return { roots, error }
  }
}

test("parses actual newline-delimited Buildx history and its empty output", () => {
  assert.deepEqual(parseHistoryRows(""), [])
  const row = historyRow(`${BUILDER}/${NODE}/solve-001`, "running")
  assert.deepEqual(parseHistoryRows(`${JSON.stringify(row)}\n`), [{ ref: row.ref, status: "running" }])
  assert.throws(() => parseHistoryRows(`${JSON.stringify(historyRow(`${BUILDER}/${NODE}/solve-001`, "error"))}\n`), /missing a completion timestamp/)
})

test("verifies only the pinned linux/amd64 OCI index-to-manifest closure", async () => {
  const parent = await mkdtemp(join(tmpdir(), "managed-release-oci-"))
  try {
    const fixture = await makeOciLayout(await realpath(parent))
    const input = await verifyOciBaseLayout({ ...fixture.expected, repository: REPOSITORY })
    assert.equal(input.indexDigest, fixture.index.digest)
    assert.equal(input.manifestDigest, fixture.manifest.digest)
    assert.equal(input.platform, "linux/amd64")
    assert.equal(input.layers.length, 1)
    assert.equal(input.layers[0].digest, fixture.layer.digest)
    assert.match(input.sourceContext, /^oci-layout:\/\/\/.+@sha256:[a-f0-9]{64}$/)
  } finally {
    await makeWritableAndRemove(parent)
  }
})

test("accepts the source OCI index's nonselected containerd signature artifact sibling", async () => {
  const parent = await mkdtemp(join(tmpdir(), "managed-release-oci-artifact-sibling-"))
  try {
    const fixture = await makeOciLayout(await realpath(parent))
    const input = await verifyOciBaseLayout({ ...fixture.expected, repository: REPOSITORY })
    assert.equal(input.indexDigest, fixture.index.digest)
    assert.equal(input.manifestDigest, fixture.manifest.digest)
  } finally {
    await makeWritableAndRemove(parent)
  }
})

test("refuses to select the source OCI signature artifact as the pinned build index", async () => {
  const parent = await mkdtemp(join(tmpdir(), "managed-release-oci-artifact-selected-"))
  try {
    const fixture = await makeOciLayout(await realpath(parent))
    await assert.rejects(verifyOciBaseLayout({
      ...fixture.expected,
      indexDigest: fixture.artifact.digest,
      manifestDigest: fixture.artifact.digest,
      repository: REPOSITORY,
    }), /non-index descriptor/)
  } finally {
    await makeWritableAndRemove(parent)
  }
})

test("rejects an OCI base digest that does not match its verified index", async () => {
  const parent = await mkdtemp(join(tmpdir(), "managed-release-oci-wrong-digest-"))
  try {
    const fixture = await makeOciLayout(await realpath(parent))
    await assert.rejects(verifyOciBaseLayout({
      ...fixture.expected,
      manifestDigest: `sha256:${"e".repeat(64)}`,
      repository: REPOSITORY,
    }), /requested linux\/amd64 manifest/)
  } finally {
    await makeWritableAndRemove(parent)
  }
})

test("rejects symlink and absent blobs in the selected OCI closure", async () => {
  for (const mode of ["symlink", "missing"]) {
    const parent = await mkdtemp(join(tmpdir(), `managed-release-oci-${mode}-`))
    try {
      const fixture = await makeOciLayout(await realpath(parent), {
        symlinkLayer: mode === "symlink",
        missingLayer: mode === "missing",
      })
      await assert.rejects(verifyOciBaseLayout({ ...fixture.expected, repository: REPOSITORY }),
        mode === "symlink" ? /symlink/ : /ENOENT|layer/i)
    } finally {
      await makeWritableAndRemove(parent)
    }
  }
})

test("rejects ambiguous platform selection and foreign OCI descriptors", async () => {
  for (const mode of ["ambiguous", "foreign"]) {
    const parent = await mkdtemp(join(tmpdir(), `managed-release-oci-${mode}-`))
    try {
      const fixture = await makeOciLayout(await realpath(parent), {
        ambiguousPlatform: mode === "ambiguous",
        foreignDescriptor: mode === "foreign",
      })
      await assert.rejects(verifyOciBaseLayout({ ...fixture.expected, repository: REPOSITORY }),
        mode === "ambiguous" ? /requested linux\/amd64 manifest/ : /unsupported media type/)
    } finally {
      await makeWritableAndRemove(parent)
    }
  }
})

test("passes the fixed OCI base context and offline platform flags through the executable Buildx argv", async () => {
  const parent = await mkdtemp(join(tmpdir(), "managed-release-oci-argv-"))
  try {
    const canonicalParent = await realpath(parent)
    const fixture = await makeOciLayout(canonicalParent)
    const baseInput = await verifyOciBaseLayout({ ...fixture.expected, repository: REPOSITORY })
    const source = join(canonicalParent, "source")
    const bin = join(canonicalParent, "bin")
    const trace = join(canonicalParent, "argv.json")
    await mkdir(source, { mode: 0o700 })
    await mkdir(bin, { mode: 0o700 })
    await writeBuildContext(source, baseInput, randomUUID())
    const docker = join(bin, "docker")
    await writeFile(docker, "#!/usr/bin/env node\nrequire('node:fs').writeFileSync(process.env.DRILL_ARGV_TRACE, JSON.stringify(process.argv.slice(2)))\n", { mode: 0o755 })
    await chmod(docker, 0o755)
    const engine = new DockerBuildxEngine({
      builder: BUILDER,
      containerId: FINGERPRINT.nodes[0].containerId,
      dockerEnvironment: {
        ...process.env,
        PATH: `${bin}${delimiter}${process.env.PATH ?? ""}`,
        DRILL_ARGV_TRACE: trace,
      },
    })
    const client = engine.startBuild(source, { baseInput, invocationId: randomUUID() })
    const result = await client.completion
    assert.equal(result.status, 0)
    assert.equal(result.reaped, true)
    assert.equal(result.streamsClosed, true)
    const args = JSON.parse(await readFile(trace, "utf8"))
    const contextIndex = args.indexOf("--build-context")
    assert.ok(contextIndex >= 0)
    assert.equal(args[contextIndex + 1], `settlementbase=${baseInput.sourceContext}`)
    assert.ok(args.includes("--pull=false"))
    assert.ok(args.includes("--network=none"))
    assert.equal(args[args.indexOf("--platform") + 1], "linux/amd64")
    assert.match(await readFile(join(source, "Dockerfile"), "utf8"), /^FROM settlementbase AS managed-release-artifacts\n/)
  } finally {
    await makeWritableAndRemove(parent)
  }
})

test("tracked child deadlines and output limits wait for close and reap", async () => {
  const timed = await trackChild(spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], {
    stdio: ["ignore", "pipe", "pipe"],
  }), 75).completion
  assert.equal(timed.timedOut, true)
  assert.equal(timed.reaped, true)
  assert.equal(timed.streamsClosed, true)

  const limited = await trackChild(spawn(process.execPath, ["-e", "process.stdout.write(Buffer.alloc(3 * 1024 * 1024)); setInterval(() => {}, 1000)"], {
    stdio: ["ignore", "pipe", "pipe"],
  }), 5_000).completion
  assert.equal(limited.outputLimitExceeded, true)
  assert.equal(limited.reaped, true)
  assert.equal(limited.streamsClosed, true)
})

test("safe OCI openers reject deterministic regular-file-to-FIFO swaps without hanging", async () => {
  const parent = await mkdtemp(join(tmpdir(), "managed-release-oci-fifo-race-"))
  const moduleUrl = new URL("./managed-release-oci-layout.mjs", import.meta.url).href
  const raceChild = async ({ kind, path, layoutPath, descriptor }) => {
    const code = `
      (async () => {
        const { spawnSync } = await import("node:child_process")
        const fs = await import("node:fs/promises")
        const oci = await import(process.env.OCI_MODULE_URL)
        const beforeOpen = async (file) => {
          await fs.unlink(file)
          const result = spawnSync("mkfifo", [file], { timeout: 1000 })
          if (result.error || result.status !== 0) throw new Error("bounded mkfifo fixture failed")
        }
        try {
          if (process.env.OCI_RACE_KIND === "read") {
            await oci.readLayoutFile(process.env.OCI_RACE_PATH, "race fixture", 1024, { beforeOpen })
          } else {
            await oci.hashLayoutBlob(process.env.OCI_RACE_LAYOUT, JSON.parse(process.env.OCI_RACE_DESCRIPTOR), "race fixture", { beforeOpen })
          }
          process.exitCode = 11
        } catch (error) {
          if (/changed to a non-regular file/.test(error.message)) process.exitCode = 0
          else {
            process.stderr.write(String(error.stack || error))
            process.exitCode = 12
          }
        }
      })().catch((error) => {
        process.stderr.write(String(error.stack || error))
        process.exitCode = 13
      })
    `
    const result = await trackChild(spawn(process.execPath, ["-e", code], {
      env: {
        ...process.env,
        OCI_MODULE_URL: moduleUrl,
        OCI_RACE_KIND: kind,
        OCI_RACE_PATH: path,
        OCI_RACE_LAYOUT: layoutPath ?? "",
        OCI_RACE_DESCRIPTOR: descriptor ? JSON.stringify(descriptor) : "{}",
      },
      stdio: ["ignore", "pipe", "pipe"],
    }), 2_500).completion
    assert.equal(result.timedOut, false, `${kind} safe opener must not block on the raced FIFO`)
    assert.equal(result.reaped, true)
    assert.equal(result.streamsClosed, true)
    assert.equal(result.status, 0, result.stderr)
  }

  try {
    const readPath = join(parent, "read-race.json")
    await writeFile(readPath, "{}")
    await chmod(readPath, 0o444)
    await raceChild({ kind: "read", path: readPath })

    const layoutPath = join(parent, "blob-layout")
    const blobDirectory = join(layoutPath, "blobs", "sha256")
    const blobBytes = Buffer.from("bounded FIFO opener race")
    const blobDescriptor = ociDescriptor(OCI_LAYER_MEDIA_TYPE, blobBytes)
    const blobPath = join(blobDirectory, blobDescriptor.digest.slice("sha256:".length))
    await mkdir(blobDirectory, { recursive: true, mode: 0o700 })
    await writeFile(blobPath, blobBytes)
    await chmod(blobPath, 0o444)
    await raceChild({ kind: "hash", path: blobPath, layoutPath, descriptor: blobDescriptor })
  } finally {
    await makeWritableAndRemove(parent)
  }
})

test("tracked child closes an escaped inherited pipe without claiming a remote Solve settled", async () => {
  const parent = await mkdtemp(join(tmpdir(), "managed-release-child-pipe-"))
  const pidFile = join(parent, "escaped.pid")
  let escapedPid
  try {
    const source = [
      "const { spawn } = require('node:child_process')",
      "const escaped = spawn(process.execPath, ['-e', 'setInterval(() => {}, 1000)'], { detached: true, stdio: ['ignore', 1, 2] })",
      "escaped.unref()",
      "require('node:fs').writeFileSync(process.env.ESCAPED_PID_FILE, String(escaped.pid))",
    ].join(";")
    const startedAt = Date.now()
    const result = await trackChild(spawn(process.execPath, ["-e", source], {
      env: { ...process.env, ESCAPED_PID_FILE: pidFile },
      stdio: ["ignore", "pipe", "pipe"],
    }), 5_000).completion
    assert.equal(result.status, 0)
    assert.equal(result.timedOut, false)
    assert.equal(result.reaped, true)
    assert.equal(result.streamsClosed, true)
    assert.equal(result.forcedPipeClose, true)
    assert.ok(Date.now() - startedAt < 4_000)
    escapedPid = Number(await readFile(pidFile, "utf8"))
    assert.ok(Number.isSafeInteger(escapedPid) && escapedPid > 0)
  } finally {
    if (!escapedPid) {
      try { escapedPid = Number(await readFile(pidFile, "utf8")) } catch {}
    }
    if (escapedPid) await stopFixtureProcess(escapedPid)
    await makeWritableAndRemove(parent)
  }
})

test("persists a running full ref, rejects the successor, and settles from exact bare-ID terminal proof", async () => {
  const { roots, result, error } = await runFake()
  try {
    assert.ifError(error)
    assert.equal(result.outcome, "settled")
    assert.equal(result.terminalProof.fullRef, `${BUILDER}/${NODE}/solve-001`)
    assert.equal(result.terminalProof.status, "error")
    const report = JSON.parse(await readFile(result.evidencePath, "utf8"))
    assert.equal(report.scope, "BuildKit settlement seam only; no signed release acceptance")
    assert.deepEqual(report.events.map((event) => event.phase), [
      "base-input-verified",
      "builder-admitted",
      "barrier-persisted",
      "running-full-ref-persisted",
      "successor-rejected-unsettled",
      "client-interrupted",
      "terminal-proof",
    ])
    assert.equal(report.scratchRemoved, true)
  } finally {
    await rm(roots.parent, { recursive: true, force: true })
  }
})

test("preserves the barrier when new history belongs to a foreign builder", async () => {
  const { roots, error } = await runFake({ foreignHistory: true })
  try {
    assert.match(error.message, /foreign builder or node/)
    const lease = await acquireManagedReleaseBuilderLease({ home: roots.home, builderName: BUILDER })
    try {
      const barrier = await lease.readBarrier()
      assert.ok(barrier)
      assert.equal(barrier.buildRef, null)
    } finally {
      await lease.release()
    }
    assert.match(await readFile(join(roots.evidence.directory, "settlement-report.json"), "utf8"), /unproven/)
  } finally {
    await rm(roots.parent, { recursive: true, force: true })
  }
})

test("preserves the barrier when inspect returns a foreign bare reference", async () => {
  const { roots, error } = await runFake({ foreignInspect: true })
  try {
    assert.match(error.message, /reference does not match the listed build/)
    const lease = await acquireManagedReleaseBuilderLease({ home: roots.home, builderName: BUILDER })
    try {
      assert.equal((await lease.readBarrier()).buildRef, null)
    } finally {
      await lease.release()
    }
  } finally {
    await rm(roots.parent, { recursive: true, force: true })
  }
})

test("preserves the persisted barrier if the measured builder drifts", async () => {
  const { roots, error } = await runFake({ builderDrift: true })
  try {
    assert.match(error.message, /successor did not reject the unresolved invocation: builder fingerprint changed/)
    const lease = await acquireManagedReleaseBuilderLease({ home: roots.home, builderName: BUILDER })
    try {
      assert.equal((await lease.readBarrier()).buildRef, `${BUILDER}/${NODE}/solve-001`)
    } finally {
      await lease.release()
    }
  } finally {
    await rm(roots.parent, { recursive: true, force: true })
  }
})

test("preserves the persisted barrier if the retained source drifts", async () => {
  const { roots, error } = await runFake({ sourceDrift: true })
  try {
    assert.match(error.message, /retained source does not match its invocation digest/)
    const lease = await acquireManagedReleaseBuilderLease({ home: roots.home, builderName: BUILDER })
    try {
      assert.equal((await lease.readBarrier()).buildRef, `${BUILDER}/${NODE}/solve-001`)
    } finally {
      await lease.release()
    }
  } finally {
    await rm(roots.parent, { recursive: true, force: true })
  }
})

test("cleanup refuses to remove a scratch root with a foreign owner marker", async () => {
  const roots = await fixtureRoots()
  try {
    await writeFile(join(roots.scratch.directory, OWNER_FILE), JSON.stringify({ kind: "scratch", token: randomUUID() }))
    await assert.rejects(removeOwnedDirectory(roots.scratch), /foreign owner marker/)
    assert.equal((await readFile(join(roots.scratch.directory, OWNER_FILE), "utf8")) !== "", true)
  } finally {
    await rm(roots.parent, { recursive: true, force: true })
  }
})
