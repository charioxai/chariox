#!/usr/bin/env node

import { createHash, createPrivateKey, createPublicKey, randomBytes, sign } from "node:crypto"
import { spawnSync } from "node:child_process"
import { chmod, lstat, mkdir, mkdtemp, open, readFile, rename, rm, stat, writeFile } from "node:fs/promises"
import { basename, dirname, join, resolve } from "node:path"
import { tmpdir } from "node:os"

const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex")
const BUILD_TARGET = "x86_64-unknown-linux-gnu"
const BUILDER_STAGE = "rust-builder"
const BUILDER_DOCKERFILE = "apps/kernel/slice-linux-docker/docker/Dockerfile"
const REQUIRED_OPTIONS = ["source-repository", "source-commit", "builder-signing-key", "output"]

function usage() {
  return "usage: build-managed-kernel-release --source-repository <git-worktree> --source-commit <40-hex-commit> --builder-signing-key <ed25519-key> --output <new-directory>"
}

function parseOptions(argv) {
  const options = new Map()
  for (let index = 0; index < argv.length; index += 2) {
    const option = argv[index]
    const value = argv[index + 1]
    if (!option?.startsWith("--") || !value || value.startsWith("--")) throw new Error(usage())
    const name = option.slice(2)
    if (!REQUIRED_OPTIONS.includes(name) || options.has(name)) throw new Error(usage())
    options.set(name, name === "source-commit" ? value : resolve(value))
  }
  if (options.size !== REQUIRED_OPTIONS.length) throw new Error(usage())
  return Object.fromEntries(options)
}

function git(repository, args, encoding = "utf8") {
  const result = spawnSync("git", args, {
    cwd: repository,
    encoding,
    maxBuffer: 512 * 1024 * 1024,
    env: gitEnvironment(),
  })
  if (result.status !== 0) throw new Error(`git ${args[0]} failed: ${result.stderr.toString().trim()}`)
  return result.stdout
}

function gitEnvironment() {
  return {
    PATH: process.env.PATH ?? "/usr/bin:/bin",
    HOME: "/nonexistent",
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_CONFIG_GLOBAL: "/dev/null",
    GIT_NO_REPLACE_OBJECTS: "1",
    LC_ALL: "C",
  }
}

async function materializeGitTree(repository, commit, destination) {
  const listing = git(repository, ["ls-tree", "-r", "-z", "--full-tree", commit], null)
  for (const record of listing.toString("utf8").split("\0").filter(Boolean)) {
    const match = /^(100644|100755) blob ([a-f0-9]{40})\t([^\0]+)$/.exec(record)
    if (!match || match[3].startsWith("/") || match[3].split("/").includes("..")) {
      throw new Error("source commit contains an unsupported Git object")
    }
    const path = join(destination, match[3])
    await mkdir(dirname(path), { recursive: true, mode: 0o700 })
    await writeFile(path, git(repository, ["cat-file", "blob", match[2]], null), {
      flag: "wx",
      mode: match[1] === "100755" ? 0o755 : 0o644,
    })
  }
}

async function requirePrivateKey(path) {
  const metadata = await lstat(path)
  if (metadata.isSymbolicLink() || !metadata.isFile() || metadata.size > 64 * 1024) {
    throw new Error("builder signing key must be a bounded regular file")
  }
  if (process.platform !== "win32" && (metadata.mode & 0o077) !== 0) {
    throw new Error("builder signing key must not be readable by group or other users")
  }
  const key = createPrivateKey({ key: await readFile(path), format: "pem", type: "pkcs8" })
  if (key.asymmetricKeyType !== "ed25519") throw new Error("builder signing key must be Ed25519")
  return key
}

function rawPublicKey(privateKey) {
  const der = createPublicKey(privateKey).export({ format: "der", type: "spki" })
  if (!der.subarray(0, ED25519_SPKI_PREFIX.length).equals(ED25519_SPKI_PREFIX)) {
    throw new Error("builder signing key produced an unsupported public key")
  }
  return der.subarray(ED25519_SPKI_PREFIX.length)
}

async function sha256File(path) {
  return `sha256:${createHash("sha256").update(await readFile(path)).digest("hex")}`
}

async function build(options) {
  if (!/^[a-f0-9]{40}$/.test(options["source-commit"])) {
    throw new Error("source commit must be a full lowercase Git commit ID")
  }
  const repository = options["source-repository"]
  const repositoryMetadata = await lstat(repository)
  if (repositoryMetadata.isSymbolicLink() || !repositoryMetadata.isDirectory()) {
    throw new Error("source repository must be a directory, not a symlink")
  }
  const commit = git(repository, ["rev-parse", "--verify", `${options["source-commit"]}^{commit}`]).trim()
  if (commit !== options["source-commit"]) throw new Error("source commit did not resolve exactly")
  const tree = git(repository, ["rev-parse", `${commit}^{tree}`]).trim()
  const signingKey = await requirePrivateKey(options["builder-signing-key"])
  if (await stat(options.output).then(() => true, () => false)) throw new Error("output must not exist")

  const scratch = await mkdtemp(join(tmpdir(), "chariox-managed-build."))
  const source = join(scratch, "source")
  const pending = join(dirname(options.output), `.new-${basename(options.output)}-${process.pid}`)
  const builderTag = `chariox-managed-builder:${tree}-${process.pid}-${randomBytes(8).toString("hex")}`
  const dockerEnvironment = Object.fromEntries(
    ["PATH", "HOME", "DOCKER_HOST"].flatMap((name) => process.env[name] ? [[name, process.env[name]]] : []),
  )
  try {
    await mkdir(source, { mode: 0o700 })
    await mkdir(dirname(options.output), { recursive: true })
    await rm(pending, { recursive: true, force: true })
    await mkdir(pending, { mode: 0o755 })
    await materializeGitTree(repository, commit, source)
    const dockerBuild = spawnSync(
      "docker",
      [
        "build", "--pull", "--platform", "linux/amd64", "--target", BUILDER_STAGE,
        "--file", join(source, BUILDER_DOCKERFILE),
        "--tag", builderTag,
        source,
      ],
      {
        stdio: "inherit",
        env: dockerEnvironment,
      },
    )
    if (dockerBuild.status !== 0) throw new Error(`locked managed release builder image failed with status ${dockerBuild.status}`)
    const inspected = spawnSync(
      "docker",
      ["image", "inspect", "--format", "{{.Id}}", builderTag],
      { encoding: "utf8", env: dockerEnvironment },
    )
    if (inspected.status !== 0) throw new Error(`managed release builder image inspection failed: ${inspected.stderr.trim()}`)
    const builderImage = inspected.stdout.trim()
    if (!/^sha256:[a-f0-9]{64}$/.test(builderImage)) {
      throw new Error("managed release builder image ID is invalid")
    }
    for (const name of ["chariox-kernel", "chariox-managed-bootstrap"]) {
      const sourceBinary = join(pending, name)
      const output = await open(sourceBinary, "wx", 0o755)
      let copied
      try {
        copied = spawnSync(
          "docker",
          [
            "run", "--rm", "--pull=never", "--platform", "linux/amd64", "--entrypoint", "cat",
            builderImage,
            `/opt/chariox-source/target/release/${name}`,
          ],
          { encoding: "utf8", env: dockerEnvironment, stdio: ["ignore", output.fd, "pipe"] },
        )
      } finally {
        await output.close()
      }
      if (copied.status !== 0) throw new Error(`copy ${name} from managed builder failed: ${copied.stderr.trim()}`)
      const metadata = await lstat(sourceBinary)
      if (metadata.isSymbolicLink() || !metadata.isFile()) throw new Error(`build did not produce ${name}`)
      await chmod(sourceBinary, 0o755)
    }
    const attestation = Buffer.from(JSON.stringify({
      schemaVersion: 1,
      sourceCommit: commit,
      sourceTree: tree,
      target: BUILD_TARGET,
      artifacts: [
        { name: "chariox-kernel", sha256: await sha256File(join(pending, "chariox-kernel")) },
        { name: "chariox-managed-bootstrap", sha256: await sha256File(join(pending, "chariox-managed-bootstrap")) },
      ],
    }))
    await writeFile(join(pending, "build-attestation.json"), attestation, { flag: "wx", mode: 0o644 })
    await writeFile(
      join(pending, "build-attestation.sig"),
      sign(null, attestation, signingKey).toString("base64"),
      { flag: "wx", mode: 0o644 },
    )
    await writeFile(join(pending, "builder-public-key"), rawPublicKey(signingKey).toString("base64"), {
      flag: "wx",
      mode: 0o644,
    })
    await rename(pending, options.output)
  } finally {
    spawnSync("docker", ["image", "rm", "-f", builderTag], { stdio: "ignore", env: dockerEnvironment })
    await rm(pending, { recursive: true, force: true })
    await rm(scratch, { recursive: true, force: true })
  }
}

try {
  await build(parseOptions(process.argv.slice(2)))
} catch (error) {
  process.stderr.write(`${basename(process.argv[1])}: ${error instanceof Error ? error.message : String(error)}\n`)
  process.exitCode = 1
}
