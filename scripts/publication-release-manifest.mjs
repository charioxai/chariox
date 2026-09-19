import { randomUUID } from "node:crypto"
import { spawnSync } from "node:child_process"
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { tmpdir } from "node:os"
import { basename, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

export const DOCKERFILE_FRONTEND =
  "docker/dockerfile:1@sha256:ecfaec9ed6d810b56388c508f4121597bfbba70d41a6dfeee4d8cad5f295fc32"
export const SOURCE_REVISION_LABEL = "io.chariox.runtime-source-revision"
export const SLSA_V02_PREDICATE_TYPE = "https://slsa.dev/provenance/v0.2"
export const IMAGE_DIGEST_PATTERN = /^sha256:[0-9a-f]{64}$/

export const BUILDKIT_CONTRACT = Object.freeze({
  image: "moby/buildkit",
  version: "v0.25.0",
  digest: "sha256:faffcac91decfb3b981234bf2762d88ed6c90771b689a3d8a5049cd0e874759a",
  reference: "moby/buildkit:v0.25.0@sha256:faffcac91decfb3b981234bf2762d88ed6c90771b689a3d8a5049cd0e874759a",
})

export const PUBLICATION_RESOURCE_CONTRACT = Object.freeze({
  cpuCount: 4,
  memoryAndSwap: "8GiB",
  dedicatedVolumeMinimum: "32GiB",
  maxParallelism: 1,
  gc: Object.freeze({
    reservedSpace: "2GiB",
    maxUsedSpace: "8GiB",
    minFreeSpace: "8GiB",
  }),
})

const SOURCE_REVISION_PATTERN = /^[0-9a-f]{40}$/
const REPOSITORY_PATTERN = /^[a-z0-9][a-z0-9._-]*(?::[0-9]+)?(?:\/[a-z0-9][a-z0-9._-]*)+$/
const NONCE_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._-]{0,47}$/
const DEDICATED_RUN_ROOT_PATTERN = /^chariox-publication-run-[A-Za-z0-9._-]+$/
const BUILDER_NAME_PATTERN = /^chariox-publication-builder-[A-Za-z0-9._-]+$/

function fail(message) {
  throw new Error(message)
}

function requireSourceRevision(sourceRevision) {
  if (!SOURCE_REVISION_PATTERN.test(sourceRevision ?? "")) {
    fail("publication requires an exact 40-character lowercase source SHA")
  }
  return sourceRevision
}

function requireRepository(repository) {
  if (!REPOSITORY_PATTERN.test(repository ?? "") || repository.includes("@")) {
    fail("publication requires a canonical untagged registry repository")
  }
  return repository
}

function requireDedicatedRunRoot(runRoot) {
  const absoluteRoot = resolve(runRoot ?? "")
  if (!absoluteRoot || !DEDICATED_RUN_ROOT_PATTERN.test(basename(absoluteRoot))) {
    fail("operation is limited to a dedicated publication run root")
  }
  return absoluteRoot
}

function requireBuilderName(builderName) {
  if (!BUILDER_NAME_PATTERN.test(builderName ?? "")) {
    fail("publication requires a dedicated builder name")
  }
  return builderName
}

function requireNonce(nonce) {
  if (!NONCE_PATTERN.test(nonce ?? "")) fail("publication staging nonce is invalid")
  return nonce
}

function runCommand(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd,
    env: options.env,
    encoding: "utf8",
  })
  if (result.error || result.status !== 0) {
    fail(`${command} command failed with status ${result.status ?? "unknown"}`)
  }
  return {
    stdout: result.stdout ?? "",
    stderr: result.stderr ?? "",
  }
}

function commandParts(command) {
  return [command[0], command.slice(1)]
}

function parseJson(text, description) {
  try {
    return JSON.parse(String(text ?? ""))
  } catch {
    fail(`${description} must be strict JSON`)
  }
}

function firstDefined(...values) {
  return values.find((value) => value !== undefined && value !== null)
}

function platformOf(manifest) {
  const value = firstDefined(
    manifest?.Platform,
    manifest?.platform,
    manifest?.Descriptor?.Platform,
    manifest?.Descriptor?.platform,
    manifest?.descriptor?.platform,
  )
  if (typeof value === "string") {
    const [os, architecture] = value.split("/")
    return { os, architecture }
  }
  return {
    os: firstDefined(value?.OS, value?.os),
    architecture: firstDefined(value?.Architecture, value?.architecture),
  }
}

function manifestEntries(inspection) {
  return firstDefined(
    inspection?.Manifests,
    inspection?.manifests,
    inspection?.Index?.Manifests,
    inspection?.index?.manifests,
  )
}

export function renderBuildKitConfig() {
  return [
    "# Path 1 publication BuildKit contract; generated, not user-editable.",
    "[worker.oci]",
    "  max-parallelism = 1",
    '  reservedSpace = "2GiB"',
    '  maxUsedSpace = "8GiB"',
    '  minFreeSpace = "8GiB"',
    "",
  ].join("\n")
}

export function publicationContractManifest() {
  return {
    contractVersion: 1,
    frontend: DOCKERFILE_FRONTEND,
    buildkit: { ...BUILDKIT_CONTRACT },
    resources: {
      cpuCount: PUBLICATION_RESOURCE_CONTRACT.cpuCount,
      memoryAndSwap: PUBLICATION_RESOURCE_CONTRACT.memoryAndSwap,
      dedicatedVolumeMinimum: PUBLICATION_RESOURCE_CONTRACT.dedicatedVolumeMinimum,
      maxParallelism: PUBLICATION_RESOURCE_CONTRACT.maxParallelism,
      gc: { ...PUBLICATION_RESOURCE_CONTRACT.gc },
    },
    platform: "linux/amd64",
    sourceRevisionLabel: SOURCE_REVISION_LABEL,
    publication: {
      output: "REPOSITORY@DIGEST",
      descriptorDigestPattern: IMAGE_DIGEST_PATTERN.source,
      manifestCount: 1,
    },
    attestation: {
      predicateType: SLSA_V02_PREDICATE_TYPE,
      signatureKeyEnvironment: "CHARIOX_SLICE_IMAGE_SIGNATURE_KEY",
      publicKeyEnvironment: "COSIGN_PUBLIC_KEY",
    },
    cleanup: {
      allowed: ["dedicated builder", "dedicated publication run root"],
      broadDockerGarbageCollection: false,
    },
  }
}

export function createStagingTag({ repository, sourceRevision, stagingNonce }) {
  requireRepository(repository)
  requireSourceRevision(sourceRevision)
  requireNonce(stagingNonce)
  return `${repository}:chariox-path1-staging-${sourceRevision.slice(0, 12)}-${stagingNonce}`
}

export function validatePublicationDockerfile(dockerfile) {
  const text = String(dockerfile ?? "")
  if (text.split("\n", 1)[0] !== `# syntax=${DOCKERFILE_FRONTEND}`) {
    fail("publication Dockerfile frontend is not the reviewed digest")
  }
  const fromLines = text.match(/^FROM\s+(\S+)/gm) ?? []
  for (const line of fromLines) {
    const image = line.replace(/^FROM\s+/, "").split(/\s+/)[0]
    if (image === "js-toolchain") continue
    if (!/@sha256:[0-9a-f]{64}$/.test(image)) {
      fail("publication Dockerfile contains an unpinned external base")
    }
  }
  if (!text.includes("ARG CHARIOX_RUNTIME_SOURCE_REVISION")) {
    fail("publication Dockerfile must require CHARIOX_RUNTIME_SOURCE_REVISION")
  }
  if (!text.includes(`LABEL ${SOURCE_REVISION_LABEL}="\${CHARIOX_RUNTIME_SOURCE_REVISION}"`)) {
    fail("publication Dockerfile must label the exact source revision")
  }
  return true
}

export function buildPublicationPlan({
  repoRoot,
  repository,
  sourceRevision,
  runRoot,
  builderName,
  stagingNonce,
}) {
  const sourceRoot = resolve(repoRoot ?? "")
  const dedicatedRunRoot = requireDedicatedRunRoot(runRoot)
  const safeBuilderName = requireBuilderName(builderName)
  const safeRepository = requireRepository(repository)
  const safeSourceRevision = requireSourceRevision(sourceRevision)
  const safeNonce = requireNonce(stagingNonce)
  const stagingTag = createStagingTag({
    repository: safeRepository,
    sourceRevision: safeSourceRevision,
    stagingNonce: safeNonce,
  })
  const buildKitConfigPath = join(dedicatedRunRoot, "buildkitd.toml")
  const metadataFile = join(dedicatedRunRoot, "buildx-metadata.json")
  const predicateFile = join(dedicatedRunRoot, "slsa-v0.2-predicate.json")
  const builderCommand = [
    "docker",
    "buildx",
    "create",
    "--name",
    safeBuilderName,
    "--driver",
    "docker-container",
    "--driver-opt",
    `image=${BUILDKIT_CONTRACT.reference}`,
    "--buildkitd-config",
    buildKitConfigPath,
    "--use",
  ]
  const buildCommand = [
    "docker",
    "buildx",
    "build",
    "--builder",
    safeBuilderName,
    "--pull",
    "--platform",
    "linux/amd64",
    "--file",
    "docker/publication/Dockerfile",
    "--tag",
    stagingTag,
    "--metadata-file",
    metadataFile,
    "--build-arg",
    `CHARIOX_RUNTIME_SOURCE_REVISION=${safeSourceRevision}`,
    "--push",
    sourceRoot,
  ]
  const inspectCommand = [
    "docker",
    "buildx",
    "imagetools",
    "inspect",
    "--format",
    "{{json .}}",
    `${safeRepository}@<descriptor-digest>`,
  ]
  return {
    repoRoot: sourceRoot,
    repository: safeRepository,
    sourceRevision: safeSourceRevision,
    runRoot: dedicatedRunRoot,
    builderName: safeBuilderName,
    stagingTag,
    buildKitConfigPath,
    metadataFile,
    predicateFile,
    builderCommand,
    buildCommand,
    inspectCommand,
    inspectCommandForDigest: (digest) => [
      ...inspectCommand.slice(0, -1),
      `${safeRepository}@${digest}`,
    ],
    cleanupBuilderCommand: ["docker", "buildx", "rm", "--force", safeBuilderName],
  }
}

export function descriptorDigestFromBuildxMetadata(metadata) {
  const value = typeof metadata === "string"
    ? parseJson(metadata, "Buildx metadata")
    : metadata
  const digest = value?.["containerimage.descriptor.digest"]
  if (!IMAGE_DIGEST_PATTERN.test(digest ?? "")) {
    fail("Buildx metadata has no valid containerimage.descriptor.digest")
  }
  return digest
}

export function validateBuildxPublication({ repository, digest, inspection }) {
  requireRepository(repository)
  if (!IMAGE_DIGEST_PATTERN.test(digest ?? "")) {
    fail("descriptor digest must match ^sha256:[0-9a-f]{64}$")
  }
  const expectedReference = `${repository}@${digest}`
  const name = firstDefined(inspection?.Name, inspection?.name, inspection?.Reference, inspection?.reference)
  const inspectedDigest = firstDefined(inspection?.Digest, inspection?.digest)
  const inspectedRepository = typeof name === "string" ? name.split("@", 1)[0] : undefined
  if (
    inspectedRepository !== repository ||
    (name?.includes("@") && name !== expectedReference) ||
    (inspectedDigest !== undefined && inspectedDigest !== digest)
  ) {
    fail("Buildx inspection failed registry equality")
  }
  const manifests = manifestEntries(inspection)
  if (!Array.isArray(manifests) || manifests.length !== 1) {
    fail("publication must contain a single linux/amd64 manifest")
  }
  const platform = platformOf(manifests[0])
  if (platform.os !== "linux" || platform.architecture !== "amd64") {
    fail("publication must contain a single linux/amd64 manifest")
  }
  return expectedReference
}

export function buildCosignCommands({
  reference,
  predicateFile,
  signatureKeyEnvironment = "CHARIOX_SLICE_IMAGE_SIGNATURE_KEY",
  publicKeyEnvironment = "COSIGN_PUBLIC_KEY",
}) {
  if (!reference || !predicateFile) fail("Cosign commands require an immutable image reference and predicate file")
  return {
    sign: ["cosign", "sign", "--yes", "--key", `env://${signatureKeyEnvironment}`, reference],
    attest: [
      "cosign",
      "attest",
      "--yes",
      "--key",
      `env://${signatureKeyEnvironment}`,
      "--predicate",
      predicateFile,
      "--type",
      "slsaprovenance",
      reference,
    ],
    verify: ["cosign", "verify", "--key", `env://${publicKeyEnvironment}`, "--output", "json", reference],
    verifyAttestation: [
      "cosign",
      "verify-attestation",
      "--key",
      `env://${publicKeyEnvironment}`,
      "--type",
      "slsaprovenance",
      "--output",
      "json",
      reference,
    ],
  }
}

function boundCosignEvidence({ evidence, repository, digest }) {
  requireRepository(repository)
  if (!IMAGE_DIGEST_PATTERN.test(digest ?? "")) fail("Cosign verification requires an immutable image digest")
  const entries = Array.isArray(evidence) ? evidence : [evidence]
  const matchingEntry = entries.find((entry) => {
    const critical = entry?.critical
    return critical?.image?.["docker-manifest-digest"] === digest
      && (!critical.identity?.["docker-reference"] || critical.identity["docker-reference"] === repository)
  })
  if (!matchingEntry) fail("Cosign evidence is not bound to the published registry digest")
  return matchingEntry
}

export function validateCosignSignature({ evidence, repository, digest }) {
  boundCosignEvidence({ evidence, repository, digest })
  return true
}

export function createSlsaV02Statement({ reference, digest, sourceRevision }) {
  requireSourceRevision(sourceRevision)
  if (!IMAGE_DIGEST_PATTERN.test(digest ?? "")) fail("SLSA statement requires an immutable image digest")
  return {
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
}

function decodeJsonPayload(value) {
  if (typeof value !== "string") return value
  try {
    return JSON.parse(value)
  } catch {
    try {
      return JSON.parse(Buffer.from(value, "base64").toString("utf8"))
    } catch {
      try {
        return JSON.parse(Buffer.from(value, "base64url").toString("utf8"))
      } catch {
        return null
      }
    }
  }
}

function attestationStatement(evidence) {
  if (evidence?.statement) return evidence.statement
  if (evidence?.payload) {
    const decoded = decodeJsonPayload(evidence.payload)
    if (decoded?.payload) return decodeJsonPayload(decoded.payload)
    return decoded
  }
  return evidence
}

export function validateSlsaV02Attestation({ evidence, repository, digest, sourceRevision }) {
  requireSourceRevision(sourceRevision)
  const matchingEntry = boundCosignEvidence({ evidence, repository, digest })
  const statement = attestationStatement(matchingEntry)
  if (statement?.predicateType !== SLSA_V02_PREDICATE_TYPE) {
    fail("attestation must use the SLSA v0.2 predicate type")
  }
  const subject = Array.isArray(statement.subject) ? statement.subject : []
  const matchingSubject = subject.find(
    (entry) => entry?.digest?.sha256 === digest.slice("sha256:".length)
      && (entry.name === `${repository}@${digest}` || entry.name === repository),
  )
  if (!matchingSubject) fail("SLSA predicate subject does not match the published digest")
  const predicate = statement.predicate
  if (
    !predicate ||
    typeof predicate.builder?.id !== "string" ||
    typeof predicate.buildType !== "string" ||
    !predicate.invocation ||
    !Array.isArray(predicate.materials)
  ) {
    fail("SLSA v0.2 predicate body is incomplete")
  }
  if (predicate.invocation.parameters?.sourceRevision !== sourceRevision) {
    fail("SLSA predicate source revision does not match the clean source")
  }
  return true
}

export function quarantineDedicatedRunRoot(runRoot, reason, { rename = renameSync } = {}) {
  const dedicatedRunRoot = requireDedicatedRunRoot(runRoot)
  const quarantineRoot = `${dedicatedRunRoot}.quarantine`
  rename(dedicatedRunRoot, quarantineRoot)
  void reason
  return quarantineRoot
}

export function removeDedicatedRunRoot(runRoot, { remove = rmSync } = {}) {
  const dedicatedRunRoot = requireDedicatedRunRoot(runRoot)
  remove(dedicatedRunRoot, { recursive: true, force: true })
}

function requireProtectedKeyEnvironment(env) {
  for (const key of ["CHARIOX_SLICE_IMAGE_SIGNATURE_KEY", "COSIGN_PUBLIC_KEY"]) {
    if (!String(env?.[key] ?? "").trim()) fail(`required protected environment key is absent: ${key}`)
  }
}

function exactCleanSource({ repoRoot, sourceRevision, run }) {
  const head = run("git", ["rev-parse", "--verify", "HEAD"], { cwd: repoRoot }).stdout.trim()
  if (head !== sourceRevision) fail("clean source HEAD does not equal the required exact source SHA")
  const status = run("git", ["status", "--porcelain", "--untracked-files=all"], { cwd: repoRoot }).stdout.trim()
  if (status) fail("publication requires a clean source tree")
}

export function runPublication({
  repoRoot = process.cwd(),
  repository,
  sourceRevision,
  runRoot,
  builderName,
  stagingNonce = randomUUID().replace(/[^A-Za-z0-9._-]/g, "-").slice(0, 48),
  env = process.env,
  run = runCommand,
} = {}) {
  const sourceRoot = resolve(repoRoot)
  const safeSourceRevision = requireSourceRevision(sourceRevision)
  requireRepository(repository)
  requireProtectedKeyEnvironment(env)
  exactCleanSource({ repoRoot: sourceRoot, sourceRevision: safeSourceRevision, run })
  validatePublicationDockerfile(readFileSync(join(sourceRoot, "docker/publication/Dockerfile"), "utf8"))

  const ownedRunRoot = runRoot
    ? requireDedicatedRunRoot(runRoot)
    : mkdtempSync(join(tmpdir(), "chariox-publication-run-"))
  const plan = buildPublicationPlan({
    repoRoot: sourceRoot,
    repository,
    sourceRevision: safeSourceRevision,
    runRoot: ownedRunRoot,
    builderName: builderName ?? `chariox-publication-builder-${randomUUID().slice(0, 12)}`,
    stagingNonce,
  })
  mkdirSync(plan.runRoot, { recursive: true })
  writeFileSync(plan.buildKitConfigPath, renderBuildKitConfig(), "utf8")

  let builderCreated = false
  let quarantinedRunRoot = false
  let completed = false
  try {
    const [builderCommand, builderArgs] = commandParts(plan.builderCommand)
    run(builderCommand, builderArgs, { cwd: sourceRoot, env })
    builderCreated = true

    const [buildCommand, buildArgs] = commandParts(plan.buildCommand)
    run(buildCommand, buildArgs, { cwd: sourceRoot, env })
    const metadata = parseJson(readFileSync(plan.metadataFile, "utf8"), "Buildx metadata")
    const digest = descriptorDigestFromBuildxMetadata(metadata)
    const [inspectCommand, inspectArgs] = commandParts(plan.inspectCommandForDigest(digest))
    const inspection = parseJson(run(inspectCommand, inspectArgs, { cwd: sourceRoot, env }).stdout, "Buildx inspection")
    const reference = validateBuildxPublication({ repository, digest, inspection })
    const statement = createSlsaV02Statement({ reference, digest, sourceRevision: safeSourceRevision })
    writeFileSync(plan.predicateFile, `${JSON.stringify(statement, null, 2)}\n`, "utf8")
    const cosignCommands = buildCosignCommands({ reference, predicateFile: plan.predicateFile })
    for (const command of [cosignCommands.sign, cosignCommands.attest]) {
      const [executable, args] = commandParts(command)
      run(executable, args, { cwd: sourceRoot, env })
    }
    const [verifyCommand, verifyArgs] = commandParts(cosignCommands.verify)
    const signature = parseJson(
      run(verifyCommand, verifyArgs, { cwd: sourceRoot, env }).stdout,
      "Cosign signature",
    )
    validateCosignSignature({ evidence: signature, repository, digest })
    const [verifyAttestationCommand, verifyAttestationArgs] = commandParts(cosignCommands.verifyAttestation)
    const attestation = parseJson(
      run(verifyAttestationCommand, verifyAttestationArgs, { cwd: sourceRoot, env }).stdout,
      "Cosign attestation",
    )
    validateSlsaV02Attestation({
      evidence: attestation,
      repository,
      digest,
      sourceRevision: safeSourceRevision,
    })
    completed = true
    return reference
  } catch (error) {
    try {
      quarantineDedicatedRunRoot(plan.runRoot, error instanceof Error ? error.message : "publication mismatch")
      quarantinedRunRoot = true
    } catch {
      // Preserve the original fail-closed error if quarantine itself cannot complete.
    }
    throw error
  } finally {
    if (builderCreated) {
      const [cleanupCommand, cleanupArgs] = commandParts(plan.cleanupBuilderCommand)
      try {
        run(cleanupCommand, cleanupArgs, { cwd: sourceRoot, env })
      } catch {
        if (completed) fail("dedicated Buildx builder cleanup failed")
      }
    }
    if (completed && !quarantinedRunRoot && existsSync(plan.runRoot)) {
      removeDedicatedRunRoot(plan.runRoot)
    }
  }
}

function parseArguments(argv) {
  const values = {}
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]
    if (!argument.startsWith("--")) fail(`unexpected argument: ${argument}`)
    const key = argument.slice(2).replaceAll("-", "_")
    const value = argv[index + 1]
    if (!value || value.startsWith("--")) fail(`${argument} requires a value`)
    values[key] = value
    index += 1
  }
  return values
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const args = parseArguments(process.argv.slice(2))
    const reference = runPublication({
      repoRoot: args.repo_root ?? process.cwd(),
      repository: args.repository,
      sourceRevision: args.source_revision,
      runRoot: args.run_root,
      builderName: args.builder_name,
      stagingNonce: args.staging_nonce,
    })
    process.stdout.write(`${reference}\n`)
  } catch (error) {
    process.stderr.write(`publication blocked: ${error instanceof Error ? error.message : String(error)}\n`)
    process.exitCode = 1
  }
}
