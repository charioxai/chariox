import assert from "node:assert/strict"
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import test from "node:test"

import { createSlsaV02Statement } from "./publication-release-manifest.mjs"
import {
  APPLY_GUARD,
  CLOUD_PIN_WARNING,
  JOURNAL_SCHEMA,
  REQUIRED_PREFLIGHT_CHECKS,
  recoverInterruptedPublication,
  runPath1Publication,
} from "./path1-publication-executor.mjs"

const sourceRoot = resolve(".")
const sourceRevision = "a".repeat(40)
const repository = "ghcr.io/charioxai/path1-runtime"
const digest = `sha256:${"b".repeat(64)}`
let fixtureCounter = 0

function checksum(letter) {
  return letter.repeat(64)
}

function fixture(overrides = {}) {
  const scratch = mkdtempSync(join(tmpdir(), "chariox-path1-executor-test-"))
  const nonce = overrides.stagingNonce ?? `test-${String(++fixtureCounter).padStart(4, "0")}`
  const runRoot = overrides.runRoot ?? join(scratch, `chariox-publication-run-${nonce}`)
  const journalPath = overrides.journalPath ?? `${runRoot}.journal.json`
  const hostChecks = Object.fromEntries(REQUIRED_PREFLIGHT_CHECKS.map((check) => [check, true]))
  const hostPreflightReceipt = {
    schema: "chariox.path1.publication-host-preflight.v1",
    ready: true,
    checks: hostChecks,
    paths: {
      repository: sourceRoot,
      registryAuth: join(scratch, "docker-config.json"),
      signingKey: join(scratch, "cosign.key"),
      signingPasswordFile: join(scratch, "cosign.password"),
      profile: join(scratch, "path1-profile.json"),
      hostInventory: join(scratch, "publication-hosts.json"),
      builderRoot: join(scratch, "builder-root"),
      publicationVolume: join(scratch, "publication-volume"),
      quarantine: join(scratch, "quarantine"),
      ownedCleanup: join(scratch, "owned-cleanup"),
    },
    tools: {
      checksumsVerified: true,
      checksums: {
        docker: { sha256: checksum("c"), verified: true },
        buildx: { sha256: checksum("d"), verified: true },
        cosign: { sha256: checksum("e"), verified: true },
      },
    },
    cleanup: {
      broadPruneRejected: true,
      quarantine: { ownedOnly: true },
      owned: { ownedOnly: true },
    },
    host: {
      tracked: true,
      dedicatedPublicationRole: true,
      sharedWorker: false,
    },
    localDaemonProtocolVersion: 333,
    relayPeerProtocolVersion: 55,
  }
  const publicationReceipt = {
    schema: "chariox.path1.reviewed-publication-receipt.v1",
    status: "PASS",
    reviewed: true,
    authoritative: true,
    cloudRevision: "c".repeat(40),
    sourceRevision,
    repository,
    ossRevision: sourceRevision,
    finalIntegratedSourceHead: sourceRevision,
    cloudReleasePinsMatchFinalIntegratedSourceHead: true,
    localDaemonProtocolVersion: 333,
    relayPeerProtocolVersion: 55,
  }
  const bootstrapReceipt = {
    schema: "chariox.path1.publication-host-bootstrap.v1",
    ready: true,
    host: { role: "dedicated-publication", dedicated: true, sharedWorker: false },
    localDaemonProtocolVersion: 333,
    relayPeerProtocolVersion: 55,
    storage: { dedicatedVolumeFreeBytes: String(40n * 1024n ** 3n) },
  }
  const input = {
    mode: "apply",
    confirm: APPLY_GUARD,
    repoRoot: sourceRoot,
    repository,
    runRoot,
    journalPath,
    builderName: `chariox-publication-builder-${nonce}`,
    stagingNonce: nonce,
    reviewedPublicationReceipt: publicationReceipt,
    hostPreflightReceipt,
    bootstrapReceipt,
    env: {
      CHARIOX_SLICE_IMAGE_SIGNATURE_KEY: "fd://protected-cosign-key",
      COSIGN_PUBLIC_KEY: "fd://protected-cosign-public-key",
    },
    now: "2026-09-19T00:00:00.000Z",
    ...overrides,
  }
  if (overrides.hostPreflightReceipt) {
    input.hostPreflightReceipt = {
      ...hostPreflightReceipt,
      ...overrides.hostPreflightReceipt,
      checks: { ...hostChecks, ...(overrides.hostPreflightReceipt.checks ?? {}) },
      tools: {
        ...hostPreflightReceipt.tools,
        ...(overrides.hostPreflightReceipt.tools ?? {}),
        checksums: {
          ...hostPreflightReceipt.tools.checksums,
          ...(overrides.hostPreflightReceipt.tools?.checksums ?? {}),
        },
      },
    }
  }
  if (overrides.reviewedPublicationReceipt) {
    input.reviewedPublicationReceipt = { ...publicationReceipt, ...overrides.reviewedPublicationReceipt }
  }
  if (overrides.bootstrapReceipt) {
    input.bootstrapReceipt = { ...bootstrapReceipt, ...overrides.bootstrapReceipt }
  }
  return { input, scratch, runRoot, journalPath, hostPreflightReceipt, publicationReceipt, bootstrapReceipt }
}

function makeRunner(harness, variant = {}) {
  const calls = []
  const effectiveDigest = variant.digest ?? digest
  const defaultInspection = {
    Name: `${repository}@${effectiveDigest}`,
    Digest: effectiveDigest,
    Manifests: [{ Platform: "linux/amd64" }],
    Labels: { "io.chariox.runtime-source-revision": sourceRevision },
  }
  const run = (command, args, options = {}) => {
    calls.push({ command, args: [...args], options })
    const label = `${command} ${args.join(" ")}`
    if (variant.failAt && variant.failAt(command, args)) {
      const error = new Error(variant.failureMessage ?? "injected partial failure")
      if (variant.interrupted) error.code = "SIGINT"
      throw error
    }
    if (command === "git" && args[0] === "rev-parse") {
      return { stdout: variant.gitHead ?? sourceRevision, stderr: "" }
    }
    if (command === "git" && args[0] === "status") {
      return { stdout: variant.gitStatus ?? "", stderr: "" }
    }
    if (command === "docker" && args[0] === "buildx" && args[1] === "build") {
      const metadataIndex = args.indexOf("--metadata-file")
      if (metadataIndex >= 0 && variant.writeMetadata !== false) {
        writeFileSync(args[metadataIndex + 1], JSON.stringify({
          "containerimage.descriptor.digest": effectiveDigest,
        }))
      }
      if (variant.interruptAfterMetadata) {
        const error = new Error("interrupted while building")
        error.code = "SIGINT"
        throw error
      }
      return { stdout: "build output intentionally suppressed", stderr: "" }
    }
    if (command === "docker" && args[0] === "buildx" && args[1] === "inspect") {
      return { stdout: "{\"Name\":\"owned-builder\"}", stderr: "" }
    }
    if (command === "docker" && args[0] === "buildx" && args[1] === "imagetools") {
      const inspection = typeof variant.inspection === "function"
        ? variant.inspection(effectiveDigest)
        : variant.inspection ?? defaultInspection
      return { stdout: JSON.stringify(inspection), stderr: "" }
    }
    if (command === "cosign" && args.includes("verify-attestation")) {
      const statement = createSlsaV02Statement({
        reference: `${repository}@${effectiveDigest}`,
        digest: effectiveDigest,
        sourceRevision,
      })
      const evidence = {
        critical: {
          identity: { "docker-reference": repository },
          image: { "docker-manifest-digest": effectiveDigest },
        },
        statement: variant.attestationStatement ?? statement,
      }
      return { stdout: JSON.stringify([variant.attestationEvidence ?? evidence]), stderr: "" }
    }
    if (command === "cosign" && args.includes("verify")) {
      const evidence = {
        critical: {
          identity: { "docker-reference": repository },
          image: { "docker-manifest-digest": effectiveDigest },
        },
      }
      return { stdout: JSON.stringify([variant.signatureEvidence ?? evidence]), stderr: "" }
    }
    if (label.includes("buildx create") || label.includes("buildx rm")) {
      return { stdout: "", stderr: "" }
    }
    return { stdout: "", stderr: "" }
  }
  return { run, calls }
}

function commandNames(calls) {
  return calls.map(({ command, args }) => `${command} ${args.join(" ")}`)
}

function cleanupHarness(harness) {
  rmSync(harness.scratch, { recursive: true, force: true })
}

function exactReviewedPublicationReceipt() {
  return {
    schema: "chariox.path1.oss-publication-receipt.v1",
    status: "published",
    source: {
      cloudRevision: "c".repeat(40),
      ossRevision: sourceRevision,
      treeClean: true,
      revisionLabel: {
        name: "io.chariox.runtime-source-revision",
        value: sourceRevision,
      },
    },
    protocols: { localDaemon: 333, relayPeer: 55 },
    image: {
      reference: `${repository}@${digest}`,
      digest,
      manifests: [{ platform: "linux/amd64", digest }],
      signatureVerification: "verified",
      attestationVerification: "verified",
    },
    publication: { status: "succeeded", quarantine: "none" },
  }
}

test("plan mode is non-mutating, exposes the exact contract order, and reports the Cloud pin guard", () => {
  const harness = fixture({ mode: "plan" })
  try {
    const result = runPath1Publication(harness.input)
    assert.equal(result.mode, "plan")
    assert.equal(result.ready, true)
    assert.equal(result.warning, CLOUD_PIN_WARNING)
    assert.equal(result.commands.builder[0], "docker")
    assert.ok(result.commands.build.includes("--pull"))
    assert.ok(result.commands.build.includes("--push"))
    assert.equal(existsSync(harness.runRoot), false)
    assert.equal(existsSync(harness.journalPath), false)
  } finally {
    cleanupHarness(harness)
  }
})

test("consumes the nested reviewed publication receipt without hard-coded source heads", () => {
  const harness = fixture()
  harness.input.reviewedPublicationReceipt = exactReviewedPublicationReceipt()
  try {
    const result = runPath1Publication({ ...harness.input, mode: "plan" })
    assert.equal(result.sourceRevision, sourceRevision)
    assert.equal(result.repository, repository)
    assert.equal(result.ready, true)
  } finally {
    cleanupHarness(harness)
  }
})

test("consumes redacted Cloud bootstrap artifact checksum evidence", () => {
  const harness = fixture()
  harness.input.hostPreflightReceipt.tools = {
    docker: { daemonResponsive: true },
    buildx: { metadataOutputSupported: true },
    cosign: { installed: true },
  }
  harness.input.bootstrapReceipt = {
    schema: "chariox.path1.publication-host-bootstrap.v1",
    redacted: true,
    mode: "apply",
    ready: true,
    outcome: "applied",
    checks: {
      publicationVolumePostBuildSafety: true,
      artifactChecksumsVerified: true,
      trackedDedicatedPublicationRole: true,
      sharedWorkerRejected: true,
      exactReleaseHeads: true,
      exactProtocols: true,
      broadPruneRejected: true,
    },
    artifacts: {
      dockerEngine: { checksumVerified: true },
      buildx: { checksumVerified: true },
      cosign: { checksumVerified: true },
    },
    ownedCleanup: { broadPruneRejected: true },
    localDaemonProtocolVersion: 333,
    relayPeerProtocolVersion: 55,
  }
  try {
    const result = runPath1Publication({ ...harness.input, mode: "plan" })
    assert.equal(result.toolChecksums.docker, "[redacted-verified]")
    assert.equal(result.toolChecksums.buildx, "[redacted-verified]")
    assert.equal(result.toolChecksums.cosign, "[redacted-verified]")
  } finally {
    cleanupHarness(harness)
  }
})

test("apply mode uses the manifest sequence and emits only an immutable reference plus redacted receipt", () => {
  const harness = fixture()
  const fake = makeRunner(harness)
  try {
    const result = runPath1Publication({ ...harness.input, run: fake.run })
    assert.equal(result.reference, `${repository}@${digest}`)
    assert.equal(result.receipt.image.reference, result.reference)
    assert.equal(result.receipt.source.ossRevision, sourceRevision)
    assert.equal(result.receipt.source.revisionLabel.value, sourceRevision)
    assert.equal(result.receipt.image.manifests[0].platform, "linux/amd64")
    assert.equal(result.receipt.image.manifests.length, 1)
    assert.equal(result.receipt.publication.quarantine, "none")
    assert.equal(result.receipt.schema, "chariox.path1.oss-publication-receipt.v1")
    assert.doesNotMatch(JSON.stringify(result.receipt), /protected-cosign-key|protected-cosign-public-key/)

    const names = commandNames(fake.calls)
    assert.equal(names.filter((name) => name.startsWith("git ")).length, 2)
    assert.match(names[2], /^docker buildx create /)
    assert.match(names[3], /^docker buildx build /)
    assert.match(names[4], /^docker buildx imagetools inspect /)
    assert.match(names[5], /^cosign sign /)
    assert.match(names[6], /^cosign attest /)
    assert.match(names[7], /^cosign verify /)
    assert.match(names[8], /^cosign verify-attestation /)
    assert.equal(names[9], `docker buildx rm --force chariox-publication-builder-${harness.input.stagingNonce}`)
    assert.equal(names.some((name) => /login|prune|image rm|imagetools create|(?:^| )tag /.test(name)), false)
    assert.equal(names[3].match(/--tag/g)?.length, 1)
    assert.match(names[3], new RegExp(`${repository}:chariox-path1-staging-`))
    assert.equal(existsSync(harness.runRoot), false)

    const journal = JSON.parse(readFileSync(harness.journalPath, "utf8"))
    assert.equal(journal.schema, JOURNAL_SCHEMA)
    assert.equal(journal.status, "completed")
    assert.equal(journal.stages.builderRemoved, true)
    assert.equal(journal.stages.runRootRemoved, true)
    assert.equal(journal.owned.stagingTag.startsWith(`${repository}:chariox-path1-staging-`), true)
  } finally {
    cleanupHarness(harness)
  }
})

test("a completed owned journal resumes idempotently without creating or deleting anything again", () => {
  const harness = fixture()
  const first = makeRunner(harness)
  try {
    const initial = runPath1Publication({ ...harness.input, run: first.run })
    const second = makeRunner(harness)
    const resumed = runPath1Publication({ ...harness.input, run: second.run })
    assert.equal(resumed.resumed, true)
    assert.equal(resumed.reference, initial.reference)
    assert.deepEqual(second.calls, [])
  } finally {
    cleanupHarness(harness)
  }
})

test("every required host-preflight check fails closed", () => {
  for (const check of REQUIRED_PREFLIGHT_CHECKS) {
    const harness = fixture({ hostPreflightReceipt: { checks: { [check]: false } } })
    try {
      assert.throws(
        () => runPath1Publication({ ...harness.input, mode: "plan" }),
        new RegExp(`host preflight check failed: ${check}`),
        check,
      )
    } finally {
      cleanupHarness(harness)
    }
  }
})

test("missing package-write/signing inputs, shared workers, stale protocols, checksums, volume headroom, and cleanup are blocked", () => {
  const cases = [
    ["package write", { hostPreflightReceipt: { checks: { registryPackageWriteCapability: false } } }, /registryPackageWriteCapability/],
    ["signing key", { hostPreflightReceipt: { checks: { signingKeyInputPresent: false } } }, /signingKeyInputPresent/],
    ["signing password", { hostPreflightReceipt: { checks: { signingPasswordInputPresent: false } } }, /signingPasswordInputPresent/],
    ["shared worker", { hostPreflightReceipt: { checks: { sharedWorkerRejected: false } } }, /sharedWorkerRejected/],
    ["unverified checksum", { hostPreflightReceipt: { tools: { checksums: { docker: { sha256: checksum("c"), verified: false } } } } }, /tool checksum is not verified/],
    ["stale protocol", { bootstrapReceipt: { relayPeerProtocolVersion: 54 } }, /stale or mismatched protocol/],
    ["insufficient volume", { bootstrapReceipt: { storage: { dedicatedVolumeFreeBytes: "1" } } }, /insufficient post-build headroom/],
    ["broad cleanup", { hostPreflightReceipt: { cleanup: { command: "docker system prune --force" } } }, /broad Docker cleanup/],
  ]
  for (const [label, overrides, expected] of cases) {
    const harness = fixture(overrides)
    try {
      assert.throws(() => runPath1Publication({ ...harness.input, mode: "plan" }), expected, label)
    } finally {
      cleanupHarness(harness)
    }
  }
})

test("mutable tags, dirty/mismatched source heads, missing guards, and unmatched Cloud pins are refused before mutation", () => {
  const mutable = fixture({ repository: `${repository}:latest` })
  assert.throws(() => runPath1Publication({ ...mutable.input, mode: "plan" }), /mutable tagged/)
  cleanupHarness(mutable)

  const dirty = fixture()
  const dirtyRunner = makeRunner(dirty, { gitStatus: " M tracked-file" })
  assert.throws(() => runPath1Publication({ ...dirty.input, run: dirtyRunner.run }), /clean source tree/)
  assert.equal(dirtyRunner.calls.length, 2)
  cleanupHarness(dirty)

  const mismatched = fixture()
  const mismatchedRunner = makeRunner(mismatched, { gitHead: "b".repeat(40) })
  assert.throws(() => runPath1Publication({ ...mismatched.input, run: mismatchedRunner.run }), /authoritative reviewed publication SHA/)
  assert.equal(mismatchedRunner.calls.length, 1)
  cleanupHarness(mismatched)

  const unguarded = fixture({ confirm: undefined })
  const unguardedRunner = makeRunner(unguarded)
  assert.throws(() => runPath1Publication({ ...unguarded.input, confirm: undefined, run: unguardedRunner.run }), /requires --confirm/)
  assert.equal(unguardedRunner.calls.length, 0)
  cleanupHarness(unguarded)

  const pins = fixture({ reviewedPublicationReceipt: { cloudReleasePinsMatchFinalIntegratedSourceHead: false } })
  assert.throws(() => runPath1Publication(pins.input), /Cloud release pins must match/)
  cleanupHarness(pins)
})

test("metadata, registry, platform, source-label, signature, and attestation mismatches quarantine evidence", () => {
  const variants = [
    ["metadata", { digest: "sha256:not-a-digest" }, /Buildx metadata/],
    ["registry", { inspection: { Name: "ghcr.io/other/image", Digest: digest, Manifests: [{ Platform: "linux/amd64" }], Labels: { ["io.chariox.runtime-source-revision"]: sourceRevision } } }, /registry equality/],
    ["platform", { inspection: { Name: `${repository}@${digest}`, Digest: digest, Manifests: [{ Platform: "linux/arm64" }], Labels: { ["io.chariox.runtime-source-revision"]: sourceRevision } } }, /single linux\/amd64/],
    ["source label", { inspection: { Name: `${repository}@${digest}`, Digest: digest, Manifests: [{ Platform: "linux/amd64" }], Labels: { ["io.chariox.runtime-source-revision"]: "c".repeat(40) } } }, /source label/],
    ["signature", { signatureEvidence: { critical: { identity: { "docker-reference": repository }, image: { "docker-manifest-digest": `sha256:${"c".repeat(64)}` } } } }, /not bound to the published registry digest/],
    ["attestation", { attestationStatement: { predicateType: "https://slsa.dev/provenance/v1", subject: [{ name: `${repository}@${digest}`, digest: { sha256: digest.slice(7) } }], predicate: {} } }, /SLSA v0\.2 predicate type/],
  ]
  for (const [label, variant, expected] of variants) {
    const harness = fixture()
    const fake = makeRunner(harness, variant)
    try {
      assert.throws(() => runPath1Publication({ ...harness.input, run: fake.run }), expected, label)
      const names = commandNames(fake.calls)
      assert.equal(names.some((name) => /docker system prune|docker image rm|docker buildx prune/.test(name)), false)
      const journal = JSON.parse(readFileSync(harness.journalPath, "utf8"))
      assert.equal(journal.status, "quarantined")
      assert.equal(journal.quarantine.stagingTagRetained, true)
      assert.equal(existsSync(`${harness.runRoot}.quarantine`), true)
      if (label !== "metadata") assert.equal(journal.quarantine.immutableDigest, digest)
    } finally {
      cleanupHarness(harness)
    }
  }
})

test("partial failure quarantines the run root and cleans only the owned builder", () => {
  const harness = fixture()
  const fake = makeRunner(harness, {
    failAt: (command, args) => command === "cosign" && args.includes("attest"),
  })
  try {
    assert.throws(() => runPath1Publication({ ...harness.input, run: fake.run }), /partial failure/)
    const names = commandNames(fake.calls)
    assert.equal(names.at(-1), `docker buildx rm --force chariox-publication-builder-${harness.input.stagingNonce}`)
    assert.equal(names.some((name) => /prune|image rm|volume rm|docker rmi/.test(name)), false)
    assert.equal(existsSync(`${harness.runRoot}.quarantine`), true)
  } finally {
    cleanupHarness(harness)
  }
})

test("interruption records a resumable journal and safe resume reuses the owned builder and staging tag", () => {
  const harness = fixture()
  const interrupted = makeRunner(harness, { interruptAfterMetadata: true })
  try {
    assert.throws(() => runPath1Publication({ ...harness.input, run: interrupted.run }), /interrupted/i)
    const interruptedJournal = JSON.parse(readFileSync(harness.journalPath, "utf8"))
    assert.equal(interruptedJournal.status, "interrupted")
    assert.equal(interruptedJournal.stages.builderCreated, true)
    assert.equal(interruptedJournal.stages.buildCompleted, false)
    assert.equal(recoverInterruptedPublication(harness.journalPath).nextAction.includes("same reviewed receipt"), true)

    const resumed = makeRunner(harness)
    const result = runPath1Publication({ ...harness.input, run: resumed.run })
    assert.equal(result.reference, `${repository}@${digest}`)
    const names = commandNames(resumed.calls)
    assert.equal(names.some((name) => name.startsWith("docker buildx create")), false)
    assert.equal(names.some((name) => name.startsWith("docker buildx build")), false)
    assert.equal(names.some((name) => name.startsWith("docker buildx imagetools inspect")), true)
  } finally {
    cleanupHarness(harness)
  }
})

test("inline secrets are rejected and protected values never enter commands, journals, or receipts", () => {
  const secretReceipt = fixture({ reviewedPublicationReceipt: { token: "do-not-accept" } })
  assert.throws(() => runPath1Publication({ ...secretReceipt.input, mode: "plan" }), /inline secret material/)
  cleanupHarness(secretReceipt)

  const harness = fixture()
  const fake = makeRunner(harness)
  const protectedKey = "super-secret-private-key-material"
  const protectedPublic = "super-secret-public-key-material"
  try {
    const result = runPath1Publication({
      ...harness.input,
      env: {
        CHARIOX_SLICE_IMAGE_SIGNATURE_KEY: protectedKey,
        COSIGN_PUBLIC_KEY: protectedPublic,
      },
      run: fake.run,
    })
    assert.doesNotMatch(JSON.stringify(result.receipt), /super-secret/)
    assert.doesNotMatch(readFileSync(harness.journalPath, "utf8"), /super-secret/)
    for (const call of fake.calls) {
      assert.equal(call.args.includes(protectedKey), false)
      assert.equal(call.args.includes(protectedPublic), false)
    }
  } finally {
    cleanupHarness(harness)
  }
})

test("Cloud pin mismatch remains visible in plan output without permitting apply", () => {
  const harness = fixture({
    mode: "plan",
    reviewedPublicationReceipt: {
      cloudReleasePinsMatchFinalIntegratedSourceHead: undefined,
      finalIntegratedSourceHead: undefined,
      ossRevision: undefined,
    },
  })
  try {
    const result = runPath1Publication(harness.input)
    assert.equal(result.ready, false)
    assert.equal(result.blockedReason, CLOUD_PIN_WARNING)
    assert.throws(
      () => runPath1Publication({ ...harness.input, mode: "apply", confirm: APPLY_GUARD, run: makeRunner(harness).run }),
      /Cloud release pins must match/,
    )
  } finally {
    cleanupHarness(harness)
  }
})
