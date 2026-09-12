import assert from "node:assert/strict"
import { createHash, generateKeyPairSync, sign } from "node:crypto"
import { chmod, glob, lstat, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { spawnSync } from "node:child_process"
import test from "node:test"

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const packager = join(repositoryRoot, "scripts/package-managed-kernel-release.mjs")

test("managed release stages every local Dockerfile COPY source", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-managed-build-context-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const sourceCommit = process.env.CHARIOX_TEST_SOURCE_COMMIT ?? git(["rev-parse", "HEAD"])
  const sourceTree = git(["rev-parse", `${sourceCommit}^{tree}`])
  const artifacts = await makeArtifacts(root, sourceCommit, sourceTree)
  const output = join(root, "release")
  const packaged = spawnSync(
    process.execPath,
    [
      packager,
      "--kernel", artifacts.kernel,
      "--supervisor", artifacts.supervisor,
      "--relay", artifacts.relay,
      "--builder-attestation", artifacts.builderAttestation,
      "--builder-attestation-signature", artifacts.builderAttestationSignature,
      "--trusted-builder-public-key", artifacts.trustedBuilderPublicKey,
      "--signing-key", artifacts.signingKey,
      "--source-repository", repositoryRoot,
      "--source-commit", sourceCommit,
      "--output", output,
    ],
    {
      encoding: "utf8",
      env: { ...process.env, SOURCE_DATE_EPOCH: "946684800" },
      timeout: 30_000,
    },
  )
  assert.equal(packaged.status, 0, packaged.stderr)

  const stagedContext = join(output, "rootfs/usr/lib/chariox/slice-build-context")
  const dockerfile = await readFile(
    join(stagedContext, "apps/kernel/slice-linux-docker/docker/Dockerfile"),
    "utf8",
  )
  const sources = dockerfileCopySources(dockerfile)
  assert.ok(sources.length > 0, "fixture should discover local Dockerfile COPY sources")
  const missing = []
  for (const source of sources) {
    if (!await copySourceExists(stagedContext, source)) missing.push(source)
  }
  assert.deepEqual(missing, [], "packaged slice build context is missing Dockerfile COPY sources")
})

test("Dockerfile COPY source checks understand directories and globs", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-docker-copy-sources-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  await mkdir(join(root, "assets/nested"), { recursive: true })
  await writeFile(join(root, "assets/config.json"), "{}\n")
  await writeFile(join(root, "assets/first.txt"), "first\n")
  await writeFile(join(root, "assets/nested/second.txt"), "second\n")

  const sources = dockerfileCopySources(`
COPY assets/nested /nested
COPY assets/*.txt /text/
COPY ["assets/config.json", "/config.json"]
COPY --from=builder /output/generated /generated
`)
  assert.deepEqual(sources, ["assets/nested", "assets/*.txt", "assets/config.json"])
  for (const source of sources) assert.equal(await copySourceExists(root, source), true)
  assert.equal(await copySourceExists(root, "assets/*.missing"), false)
})

function dockerfileCopySources(dockerfile) {
  const instructions = dockerfile.replaceAll(/\\\r?\n/g, " ").split(/\r?\n/)
  const sources = []
  for (const instruction of instructions) {
    const match = /^\s*COPY\s+(.+)$/i.exec(instruction)
    if (!match) continue
    const tokens = dockerTokens(match[1])
    let index = 0
    let fromStage = false
    while (tokens[index]?.startsWith("--")) {
      const option = tokens[index]
      if (option === "--from" || option === "--chown" || option === "--chmod") {
        if (option === "--from") fromStage = true
        index += 2
      } else {
        if (option.startsWith("--from=")) fromStage = true
        index += 1
      }
    }
    if (!fromStage) sources.push(...tokens.slice(index, -1))
  }
  return sources
}

function dockerTokens(argumentsText) {
  if (argumentsText.trimStart().startsWith("[")) {
    const tokens = JSON.parse(argumentsText)
    assert.ok(Array.isArray(tokens) && tokens.every((token) => typeof token === "string"))
    return tokens
  }
  const tokens = []
  let token = ""
  let quote = null
  let escaped = false
  for (const character of argumentsText.trim()) {
    if (escaped) {
      token += character
      escaped = false
    } else if (character === "\\" && quote !== "'") {
      escaped = true
    } else if (quote) {
      if (character === quote) quote = null
      else token += character
    } else if (character === "'" || character === '"') {
      quote = character
    } else if (/\s/.test(character)) {
      if (token) tokens.push(token)
      token = ""
    } else {
      token += character
    }
  }
  assert.equal(quote, null, "unterminated quote in Dockerfile COPY")
  assert.equal(escaped, false, "unterminated escape in Dockerfile COPY")
  if (token) tokens.push(token)
  assert.ok(tokens.length >= 2, "Dockerfile COPY must have a source and destination")
  return tokens
}

async function copySourceExists(contextRoot, source) {
  const relativeSource = source.replace(/^\/+/, "").replace(/^\.\//, "")
  assert.ok(relativeSource && !relativeSource.split("/").includes(".."), `unsafe Dockerfile COPY source ${source}`)
  if (/[*?[\]{}]/.test(relativeSource)) {
    for await (const _ of glob(relativeSource, { cwd: contextRoot })) return true
    return false
  }
  return lstat(join(contextRoot, relativeSource)).then(() => true, () => false)
}

async function makeArtifacts(root, sourceCommit, sourceTree) {
  const contents = {
    kernel: "kernel fixture\n",
    supervisor: "supervisor fixture\n",
    relay: "relay fixture\n",
  }
  const paths = {}
  for (const [name, bytes] of Object.entries(contents)) {
    paths[name] = join(root, name)
    await writeFile(paths[name], bytes, { mode: 0o755 })
    await chmod(paths[name], 0o755)
  }
  const builderKeys = generateKeyPairSync("ed25519")
  const releaseKeys = generateKeyPairSync("ed25519")
  const digest = (bytes) => `sha256:${createHash("sha256").update(bytes).digest("hex")}`
  const attestation = Buffer.from(JSON.stringify({
    schemaVersion: 1,
    sourceCommit,
    sourceTree,
    target: "x86_64-unknown-linux-gnu",
    artifacts: [
      { name: "chariox-kernel", sha256: digest(contents.kernel) },
      { name: "chariox-managed-bootstrap", sha256: digest(contents.supervisor) },
      { name: "chariox-relay", sha256: digest(contents.relay) },
    ],
  }))
  const builderAttestation = join(root, "build-attestation.json")
  const builderAttestationSignature = join(root, "build-attestation.sig")
  const trustedBuilderPublicKey = join(root, "builder-public-key")
  const signingKey = join(root, "release-key.pem")
  await writeFile(builderAttestation, attestation)
  await writeFile(builderAttestationSignature, sign(null, attestation, builderKeys.privateKey).toString("base64"))
  await writeFile(trustedBuilderPublicKey, rawPublicKey(builderKeys.publicKey).toString("base64"))
  await writeFile(signingKey, releaseKeys.privateKey.export({ format: "pem", type: "pkcs8" }), { mode: 0o600 })
  await chmod(signingKey, 0o600)
  return { ...paths, builderAttestation, builderAttestationSignature, trustedBuilderPublicKey, signingKey }
}

function rawPublicKey(publicKey) {
  const der = publicKey.export({ format: "der", type: "spki" })
  return der.subarray(der.length - 32)
}

function git(arguments_) {
  const result = spawnSync("git", arguments_, { cwd: repositoryRoot, encoding: "utf8" })
  assert.equal(result.status, 0, result.stderr)
  return result.stdout.trim()
}
