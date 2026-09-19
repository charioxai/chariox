import assert from "node:assert/strict"
import { test } from "node:test"

import {
  BUILDKIT_CONTRACT,
  PUBLICATION_RESOURCE_CONTRACT,
  SLSA_V02_PREDICATE_TYPE,
  buildCosignCommands,
  buildPublicationPlan,
  publicationContractManifest,
  quarantineDedicatedRunRoot,
  renderBuildKitConfig,
  validateCosignSignature,
  validateBuildxPublication,
  validateSlsaV02Attestation,
} from "./publication-release-manifest.mjs"

const sourceRevision = "a".repeat(40)
const repository = "ghcr.io/chariox/workflow-publication"
const digest = `sha256:${"b".repeat(64)}`
const reference = `${repository}@${digest}`

test("publication manifest fixes the reviewed BuildKit and dedicated resource contract", () => {
  assert.deepEqual(BUILDKIT_CONTRACT, {
    image: "moby/buildkit",
    version: "v0.25.0",
    digest: "sha256:faffcac91decfb3b981234bf2762d88ed6c90771b689a3d8a5049cd0e874759a",
    reference: "moby/buildkit:v0.25.0@sha256:faffcac91decfb3b981234bf2762d88ed6c90771b689a3d8a5049cd0e874759a",
  })
  assert.deepEqual(PUBLICATION_RESOURCE_CONTRACT, {
    cpuCount: 4,
    memoryAndSwap: "8GiB",
    dedicatedVolumeMinimum: "32GiB",
    maxParallelism: 1,
    gc: {
      reservedSpace: "2GiB",
      maxUsedSpace: "8GiB",
      minFreeSpace: "8GiB",
    },
  })
  assert.match(renderBuildKitConfig(), /max-parallelism = 1/)
  assert.match(renderBuildKitConfig(), /reservedSpace = "2GiB"/)
  assert.match(renderBuildKitConfig(), /maxUsedSpace = "8GiB"/)
  assert.match(renderBuildKitConfig(), /minFreeSpace = "8GiB"/)
})

test("publication plan requires a clean exact source and pull-enabled digest-pinned build", () => {
  const plan = buildPublicationPlan({
    repoRoot: "/workspace/chariox",
    repository,
    sourceRevision,
    runRoot: "/tmp/chariox-publication-run-test",
    builderName: "chariox-publication-builder-test",
    stagingNonce: "nonce-1",
  })

  assert.equal(plan.stagingTag, `${repository}:chariox-path1-staging-${sourceRevision.slice(0, 12)}-nonce-1`)
  assert.ok(plan.buildCommand.includes("--pull"))
  assert.ok(plan.buildCommand.includes("--platform"))
  assert.ok(plan.buildCommand.includes("linux/amd64"))
  assert.ok(plan.buildCommand.includes("--push"))
  assert.ok(plan.buildCommand.includes(`CHARIOX_RUNTIME_SOURCE_REVISION=${sourceRevision}`))
  assert.ok(plan.buildCommand.includes(plan.stagingTag))
  assert.equal(plan.buildCommand.at(-1), "/workspace/chariox")
  assert.ok(plan.builderCommand.some((argument) => argument.includes(BUILDKIT_CONTRACT.reference)))
  assert.ok(plan.builderCommand.includes(plan.buildKitConfigPath))
  assert.equal(plan.inspectCommand.at(-1), `${repository}@<descriptor-digest>`)
  assert.doesNotMatch(plan.builderCommand.join(" "), /prune/i)
  assert.doesNotMatch(plan.cleanupBuilderCommand.join(" "), /prune/i)
})

test("publication manifest is deterministic and exposes only its contract fields", () => {
  const first = publicationContractManifest()
  const second = publicationContractManifest()
  assert.deepEqual(first, second)
  assert.deepEqual(first.buildkit, BUILDKIT_CONTRACT)
  assert.deepEqual(first.resources, PUBLICATION_RESOURCE_CONTRACT)
  assert.equal(first.platform, "linux/amd64")
  assert.equal(first.sourceRevisionLabel, "io.chariox.runtime-source-revision")
  assert.equal(first.attestation.predicateType, SLSA_V02_PREDICATE_TYPE)
  assert.equal(first.cleanup.broadDockerGarbageCollection, false)
})

test("publication validation accepts exactly one registry-equal linux amd64 manifest", () => {
  assert.equal(
    validateBuildxPublication({
      repository,
      digest,
      inspection: {
        Name: reference,
        Digest: digest,
        Manifests: [{ Platform: "linux/amd64" }],
      },
    }),
    reference,
  )
  assert.throws(
    () => validateBuildxPublication({
      repository,
      digest,
      inspection: { Name: "ghcr.io/other/image", Digest: digest, Manifests: [{ Platform: "linux/amd64" }] },
    }),
    /registry equality/,
  )
  assert.throws(
    () => validateBuildxPublication({
      repository,
      digest,
      inspection: { Name: reference, Digest: digest, Manifests: [{ Platform: "linux/amd64" }, { Platform: "linux/arm64" }] },
    }),
    /single linux\/amd64/,
  )
  assert.throws(
    () => validateBuildxPublication({
      repository,
      digest: "sha256:not-a-digest",
      inspection: { Name: reference, Digest: digest, Manifests: [{ Platform: "linux/amd64" }] },
    }),
    /descriptor digest/,
  )
})

test("publication Cosign commands are key-based and bind verification to the immutable reference", () => {
  const commands = buildCosignCommands({ reference, predicateFile: "/run/predicate.json" })
  for (const command of [commands.sign, commands.attest, commands.verify, commands.verifyAttestation]) {
    assert.ok(command.includes(reference))
    assert.doesNotMatch(command.join(" "), /fulcio|rekor|identity-token|keyless/i)
  }
  assert.ok(commands.sign.includes("env://CHARIOX_SLICE_IMAGE_SIGNATURE_KEY"))
  assert.ok(commands.attest.includes("env://CHARIOX_SLICE_IMAGE_SIGNATURE_KEY"))
  assert.ok(commands.verify.includes("env://COSIGN_PUBLIC_KEY"))
  assert.ok(commands.verifyAttestation.includes("env://COSIGN_PUBLIC_KEY"))
  assert.ok(commands.verifyAttestation.includes("slsaprovenance"))
})

test("publication Cosign signature verification binds the signed manifest digest", () => {
  const evidence = [{
    critical: {
      identity: { "docker-reference": repository },
      image: { "docker-manifest-digest": digest },
    },
  }]
  assert.equal(validateCosignSignature({ evidence, repository, digest }), true)
  assert.throws(
    () => validateCosignSignature({ evidence, repository, digest: `sha256:${"c".repeat(64)}` }),
    /not bound to the published registry digest/,
  )
})

test("publication attestation validation enforces SLSA v0.2 predicate body and image digest", () => {
  const statement = {
    _type: "https://in-toto.io/Statement/v0.1",
    subject: [{ name: reference, digest: { sha256: digest.slice("sha256:".length) } }],
    predicateType: SLSA_V02_PREDICATE_TYPE,
    predicate: {
      builder: { id: "https://chariox.dev/publication/builder" },
      buildType: "https://chariox.dev/publication/v1",
      invocation: { parameters: { sourceRevision } },
      materials: [],
    },
  }
  const evidence = {
    critical: {
      identity: { "docker-reference": repository },
      image: { "docker-manifest-digest": digest },
    },
    statement,
  }
  assert.doesNotThrow(() => validateSlsaV02Attestation({ evidence: [evidence], repository, digest, sourceRevision }))
  assert.throws(
    () => validateSlsaV02Attestation({ evidence: [{ ...evidence, statement: { ...statement, predicateType: "https://slsa.dev/provenance/v1" } }], repository, digest, sourceRevision }),
    /SLSA v0\.2 predicate type/,
  )
})

test("publication mismatch quarantine is limited to a dedicated run root", () => {
  const renames = []
  const quarantined = quarantineDedicatedRunRoot(
    "/tmp/chariox-publication-run-test",
    "descriptor digest mismatch",
    { rename: (from, to) => renames.push([from, to]) },
  )
  assert.equal(quarantined, "/tmp/chariox-publication-run-test.quarantine")
  assert.deepEqual(renames, [[
    "/tmp/chariox-publication-run-test",
    "/tmp/chariox-publication-run-test.quarantine",
  ]])
  assert.throws(
    () => quarantineDedicatedRunRoot("/tmp/unsafe", "mismatch", { rename: () => {} }),
    /dedicated publication run root/,
  )
})
