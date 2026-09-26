import assert from "node:assert/strict"
import { createHash, generateKeyPairSync, verify } from "node:crypto"
import { chmod, lstat, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { spawnSync } from "node:child_process"
import test from "node:test"

const repositoryRoot = fileURLToPath(new URL("..", import.meta.url))
const builder = join(repositoryRoot, "scripts/build-managed-kernel-release.mjs")
const dockerfilePath = "apps/kernel/slice-linux-docker/docker/Dockerfile"
const artifactNames = ["chariox-kernel", "chariox-managed-bootstrap", "chariox-relay"]

function rawPublicKey(publicKey) {
  const der = publicKey.export({ format: "der", type: "spki" })
  return der.subarray(der.length - 32)
}

function digest(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`
}

function writeFakeDocker(path, { dockerfileDigest, artifacts, trace }) {
  const contents = `#!/usr/bin/env node
import assert from "node:assert/strict"
import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"
import path from "node:path"
import { createHash } from "node:crypto"
const args = process.argv.slice(2)
assert.deepEqual(args.slice(0, 2), ["buildx", "build"])
assert.equal(args.includes("--load"), false)
assert.equal(args.includes("--tag"), false)
for (const pair of [["--pull"], ["--platform", "linux/amd64"], ["--target", "managed-release-artifacts"]]) {
  const at = args.indexOf(pair[0])
  assert.notEqual(at, -1)
  if (pair.length > 1) assert.equal(args[at + 1], pair[1])
}
const source = args.at(-1)
const dockerfile = args[args.indexOf("--file") + 1]
assert.equal(dockerfile, path.join(source, ${JSON.stringify(dockerfilePath)}))
assert.equal(existsSync(path.join(source, ".git")), false)
assert.equal(existsSync(path.join(source, "working-tree-only")), false)
const sourceDigest = createHash("sha256").update(readFileSync(dockerfile)).digest("hex")
assert.equal(sourceDigest, ${JSON.stringify(dockerfileDigest)})
const outputOption = args[args.indexOf("--output") + 1]
assert.match(outputOption, /^type=local,dest=/)
const destination = outputOption.slice("type=local,dest=".length)
assert.equal(existsSync(destination), false)
mkdirSync(destination)
const artifacts = ${JSON.stringify(Object.fromEntries(Object.entries(artifacts).map(([name, bytes]) => [name, Buffer.from(bytes).toString("base64")])))}
for (const [name, encoded] of Object.entries(artifacts)) writeFileSync(path.join(destination, name), Buffer.from(encoded, "base64"))
appendFileSync(${JSON.stringify(trace)}, JSON.stringify(args) + "\\n")
`
  return writeFile(path, contents, { mode: 0o755 }).then(() => chmod(path, 0o755))
}

async function makeSourceFixture(root) {
  const sourceRepository = join(root, "source")
  const dockerfile = join(sourceRepository, dockerfilePath)
  const dockerfileBytes = await readFile(join(repositoryRoot, dockerfilePath))
  await mkdir(join(sourceRepository, "apps/kernel/slice-linux-docker/docker"), { recursive: true })
  await writeFile(dockerfile, dockerfileBytes)
  const git = (args) => spawnSync("git", args, {
    cwd: sourceRepository,
    encoding: "utf8",
    env: {
      ...process.env,
      GIT_AUTHOR_NAME: "Release Export Fixture",
      GIT_AUTHOR_EMAIL: "release-export@example.invalid",
      GIT_AUTHOR_DATE: "2000-01-01T00:00:00Z",
      GIT_COMMITTER_NAME: "Release Export Fixture",
      GIT_COMMITTER_EMAIL: "release-export@example.invalid",
      GIT_COMMITTER_DATE: "2000-01-01T00:00:00Z",
    },
  })
  let result = git(["init", "-q"])
  assert.equal(result.status, 0, result.stderr)
  result = git(["add", dockerfilePath])
  assert.equal(result.status, 0, result.stderr)
  result = git(["commit", "-q", "-m", "fixture"])
  assert.equal(result.status, 0, result.stderr)
  const sourceCommit = git(["rev-parse", "HEAD"]).stdout.trim()
  const sourceTree = git(["rev-parse", "HEAD^{tree}"]).stdout.trim()
  return { sourceRepository, dockerfile, dockerfileBytes, sourceCommit, sourceTree }
}

function runBuilder({ fixture, bin, output, signingKey, builderName }) {
  const args = [
    builder,
    "--source-repository", fixture.sourceRepository,
    "--source-commit", fixture.sourceCommit,
    "--builder-signing-key", signingKey,
    ...(builderName ? ["--builder", builderName] : []),
    "--output", output,
  ]
  return spawnSync(process.execPath, args, {
    encoding: "utf8",
    env: { ...process.env, PATH: `${bin}:${process.env.PATH ?? "/usr/bin:/bin"}` },
  })
}

test("Dockerfile has a scratch export target containing only the three signed release binaries", async () => {
  const dockerfile = await readFile(join(repositoryRoot, dockerfilePath), "utf8")
  const start = dockerfile.indexOf("FROM scratch AS managed-release-artifacts")
  assert.notEqual(start, -1)
  const end = dockerfile.indexOf("\nFROM ", start + 1)
  assert.notEqual(end, -1)
  const target = dockerfile.slice(start, end).trim().split("\n")
  assert.deepEqual(target.slice(1), [
    "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-kernel /chariox-kernel",
    "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-managed-bootstrap /chariox-managed-bootstrap",
    "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-relay /chariox-relay",
  ])
})

test("builder exports exact committed artifacts locally and from a named BuildKit builder, then signs their attestation", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-release-artifact-export-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fixture = await makeSourceFixture(root)
  const key = generateKeyPairSync("ed25519")
  const signingKey = join(root, "builder-signing-key.pem")
  await writeFile(signingKey, key.privateKey.export({ format: "pem", type: "pkcs8" }), { mode: 0o600 })
  const artifacts = Object.fromEntries(artifactNames.map((name) => [name, Buffer.from(`fixture artifact ${name}\n`)]))
  const bin = join(root, "bin")
  const trace = join(root, "docker-buildx-trace")
  await mkdir(bin)
  await writeFakeDocker(join(bin, "docker"), {
    dockerfileDigest: createHash("sha256").update(fixture.dockerfileBytes).digest("hex"),
    artifacts,
    trace,
  })

  await writeFile(fixture.dockerfile, Buffer.concat([fixture.dockerfileBytes, Buffer.from("# working tree drift\n")]))
  await writeFile(join(fixture.sourceRepository, "working-tree-only"), "must not be exported\n")

  const outputs = [join(root, "release-default"), join(root, "release-named-builder")]
  for (const [index, output] of outputs.entries()) {
    const result = runBuilder({
      fixture,
      bin,
      output,
      signingKey,
      builderName: index === 1 ? "capped-release-builder" : undefined,
    })
    assert.equal(result.status, 0, result.stderr)
    const exportedNames = await readdir(output)
    assert.deepEqual(exportedNames.sort(), [...artifactNames, "build-attestation.json", "build-attestation.sig", "builder-public-key"].sort())
    for (const name of artifactNames) {
      assert.deepEqual(await readFile(join(output, name)), artifacts[name])
      assert.equal((await lstat(join(output, name))).mode & 0o777, 0o755)
    }
    const attestationBytes = await readFile(join(output, "build-attestation.json"))
    const attestation = JSON.parse(attestationBytes)
    assert.equal(attestation.sourceCommit, fixture.sourceCommit)
    assert.equal(attestation.sourceTree, fixture.sourceTree)
    assert.equal(attestation.target, "x86_64-unknown-linux-gnu")
    assert.deepEqual(attestation.artifacts, artifactNames.map((name) => ({ name, sha256: digest(artifacts[name]) })))
    const signature = Buffer.from(await readFile(join(output, "build-attestation.sig"), "utf8"), "base64")
    assert.equal(verify(null, attestationBytes, key.publicKey, signature), true)
    assert.equal(await readFile(join(output, "builder-public-key"), "utf8"), rawPublicKey(key.publicKey).toString("base64"))
  }

  const invocations = (await readFile(trace, "utf8")).trim().split("\n").map((line) => JSON.parse(line))
  assert.equal(invocations.length, 2)
  assert.equal(invocations[0].includes("--builder"), false)
  const namedBuilder = invocations[1].indexOf("--builder")
  assert.notEqual(namedBuilder, -1)
  assert.equal(invocations[1][namedBuilder + 1], "capped-release-builder")
  for (const args of invocations) {
    assert.equal(args.includes("--load"), false)
    assert.equal(args.includes("--tag"), false)
    assert.equal(args.includes("run"), false)
    assert.equal(args.includes("image"), false)
    assert.equal(args[args.indexOf("--target") + 1], "managed-release-artifacts")
    assert.match(args[args.indexOf("--output") + 1], /^type=local,dest=/)
  }
})
