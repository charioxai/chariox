#!/usr/bin/env node
import { execFileSync } from "node:child_process"
import { createPrivateKey, sign, verify } from "node:crypto"
import { copyFile, lstat, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { renderInstallScript } from "./build-chariox-setup.mjs"
import { assemble, exportSliceBuildContext, PLATFORMS, publicKeyFromHex, signBundle, verifyBundle } from "./release-bundle.mjs"

const repository = "charioxai/chariox"
const releaseBase = `https://github.com/${repository}/releases/download`
const root = fileURLToPath(new URL("../", import.meta.url))
const platform = "linux-x64"

function parseArgs(args) {
  const required = ["--version", "--public-key", "--signing-key", "--executables"]
  const fields = [...required, "--control-plane-url"]
  const options = {}
  for (let i = 0; i < args.length; i += 2) {
    if (!fields.includes(args[i]) || !args[i + 1] || Object.hasOwn(options, args[i])) {
      throw new Error("Expected --version VERSION --public-key PUBLIC_HEX --signing-key PRIVATE_PEM --executables DIRECTORY [--control-plane-url URL]")
    }
    options[args[i]] = args[i + 1]
  }
  if (!required.every(key => options[key])) throw new Error(`Required arguments: ${required.join(" ")}`)
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(options["--version"])) throw new Error("Use a semantic version without the v")
  return options
}

function verifyRemoteTag(tag, commit) {
  const ref = `refs/tags/${tag}`
  const lines = execFileSync("git", ["ls-remote", "--tags", `https://github.com/${repository}.git`, ref, ref + "^{}"], {
    cwd: root, encoding: "utf8", timeout: 30_000,
  }).trim()
  if (!lines) return
  const tags = new Map(lines.split("\n").map(line => line.split("\t").reverse()))
  if ((tags.get(ref + "^{}") ?? tags.get(ref)) !== commit) {
    throw new Error(`GitHub tag ${tag} belongs to a different commit. Choose a new release version.`)
  }
}

// Staging the assets is independent of GitHub and can use a disposable HTTPS origin in drills.
export async function stageLinuxSetupRelease({ version, pin, signingKey, binaries, sourceCommit, releaseBase, controlPlaneUrl, output }) {
  const key = publicKeyFromHex(pin)
  const setup = join(output, "setup"), executables = join(output, "executables")
  const name = `chariox-${version}-${platform}`, bundle = join(output, name)
  const context = join(output, "slice-build-context")
  await mkdir(join(executables, "bin"), { recursive: true })
  for (const name of PLATFORMS[platform].bin) {
    const source = join(binaries, "bin", name), info = await lstat(source)
    if (!info.isFile() || !(info.mode & 0o100)) throw new Error(`${source} must be a regular executable`)
    await copyFile(source, join(executables, "bin", name))
  }
  const buildArgs = ["scripts/build-chariox-setup.mjs", "--target", platform, "--version", version,
    "--public-key", pin, "--release-base", releaseBase, "--output", setup]
  if (controlPlaneUrl) buildArgs.push("--control-plane-url", controlPlaneUrl)
  execFileSync(process.execPath, buildArgs, { cwd: root, stdio: "inherit", timeout: 120_000 })
  const bootstrap = join(setup, "install.sh")
  const template = await readFile(join(root, "deploy/setup/install.sh"), "utf8")
  await writeFile(bootstrap, renderInstallScript(template, { version, publicKeyHex: pin, releaseBase, linuxOnly: true }))
  await copyFile(join(setup, "chariox-setup"), join(executables, "bin/chariox-setup"))
  await exportSliceBuildContext(root, sourceCommit, context)
  await assemble({ platform, version, sourceCommit, executables, sliceBuildContext: context, output: bundle })
  await signBundle(bundle, signingKey)
  await verifyBundle(bundle, pin)

  const setupAsset = join(output, `chariox-setup-${version}-${platform}`)
  await copyFile(join(bundle, "bin/chariox-setup"), setupAsset)
  const setupBytes = await readFile(setupAsset)
  const detachedSignature = sign(null, setupBytes, createPrivateKey(await readFile(signingKey)))
  if (!verify(null, setupBytes, key, detachedSignature)) throw new Error("Setup signature does not match the approved public pin")
  await writeFile(setupAsset + ".sig", detachedSignature.toString("hex"))

  const archive = bundle + ".tar.gz"
  execFileSync("tar", ["-czf", archive, "-C", output, name], { stdio: "inherit", timeout: 300_000 })
  const manifest = bundle + ".manifest.json", signature = bundle + ".manifest.sig"
  await copyFile(join(bundle, "manifest.json"), manifest)
  await copyFile(join(bundle, "manifest.sig"), signature)
  return [bootstrap, setupAsset, setupAsset + ".sig", manifest, signature, archive]
}

async function publish(args) {
  const { "--version": version, "--public-key": pin, "--signing-key": pem,
    "--executables": binaries, "--control-plane-url": controlPlaneUrl } = parseArgs(args)
  const tag = `v${version}`
  const sourceCommit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim()
  verifyRemoteTag(tag, sourceCommit)

  const output = await mkdtemp(join(tmpdir(), "chariox-linux-release-"))
  try {
    const assets = await stageLinuxSetupRelease({
      version, pin, signingKey: resolve(pem), binaries: resolve(binaries), sourceCommit, releaseBase, controlPlaneUrl, output,
    })
    execFileSync("gh", ["release", "create", tag, "--repo", repository, "--target", sourceCommit, "--title", `Chariox ${version}`,
      "--notes", "Signed Chariox Setup and release bundle for Linux x86_64.",
      ...(version.includes("-") ? ["--prerelease"] : []), ...assets],
    { stdio: "inherit", timeout: 900_000 })
    console.log(`Linux installer: ${releaseBase}/${tag}/install.sh`)
  } finally {
    await rm(output, { recursive: true, force: true })
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    await publish(process.argv.slice(2))
  } catch (error) {
    console.error(`Linux Setup release failed: ${error.message}`)
    process.exitCode = 1
  }
}
