#!/usr/bin/env node
import { execFileSync } from "node:child_process"
import { createHash, verify } from "node:crypto"
import { copyFile, lstat, mkdir, readFile, writeFile } from "node:fs/promises"
import { join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { renderInstallScript } from "./build-chariox-setup.mjs"
import { assemble, exportSliceBuildContext, PLATFORMS, publicKeyFromHex, verifyBundle } from "./release-bundle.mjs"

const root = fileURLToPath(new URL("../", import.meta.url))
const platform = "linux-x64"

export async function prepareLinuxSetup({ version, pin, binaries, sourceCommit, releaseBase, controlPlaneUrl, output }) {
  publicKeyFromHex(pin)
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
  await mkdir(join(output, "signing"))
  await copyFile(bootstrap, join(output, "signing/install.sh"))
  await copyFile(join(bundle, "manifest.json"), join(output, "signing/manifest.json"))
  await writeFile(join(output, "signing/request.json"), JSON.stringify({
    publicKey: pin,
    manifestSha256: sha256(await readFile(join(bundle, "manifest.json"))),
    installSha256: sha256(await readFile(bootstrap)),
  }, null, 2) + "\n")
}

const sha256 = bytes => createHash("sha256").update(bytes).digest("hex")

export async function assembleLinuxSetup({ version, pin, output, signed }) {
  const name = `chariox-${version}-${platform}`
  const bundle = join(output, name)
  const setup = `chariox-setup-${version}-${platform}`
  const request = JSON.parse(await readFile(join(output, "signing/request.json")))
  const install = await readFile(join(output, "signing/install.sh"))
  const manifest = await readFile(join(bundle, "manifest.json"))
  if (request.publicKey !== pin || sha256(install) !== request.installSha256 ||
      sha256(manifest) !== request.manifestSha256) throw new Error("Linux Setup signing inputs changed")
  const setupBytes = await readFile(join(bundle, "bin/chariox-setup"))
  const signatures = {}
  for (const [file, bytes] of [["install.sh", install], ["manifest.json", manifest], ["setup", Buffer.from("chariox-linux-setup-sha256:" + sha256(setupBytes))]]) {
    const signature = await readFile(join(signed, file + ".sig"), "utf8")
    if (!/^[a-f0-9]{128}$/.test(signature) || !verify(null, bytes, publicKeyFromHex(pin), Buffer.from(signature, "hex"))) {
      throw new Error(file + " signature does not match approved public pin")
    }
    signatures[file] = signature
  }
  await writeFile(join(bundle, "manifest.sig"), signatures["manifest.json"])
  const receipt = await verifyBundle(bundle, pin)
  if (receipt.version !== version || receipt.platform !== platform) throw new Error("unexpected Linux bundle")
  const assets = join(output, "assets")
  await mkdir(assets)
  await writeFile(join(assets, "install.sh"), install)
  await copyFile(join(bundle, "bin/chariox-setup"), join(assets, setup))
  await writeFile(join(assets, setup + ".sig"), signatures.setup)
  await writeFile(join(assets, name + ".manifest.json"), manifest)
  await writeFile(join(assets, name + ".manifest.sig"), signatures["manifest.json"])
  execFileSync("tar", ["-czf", join(assets, name + ".tar.gz"), "-C", output, name], { timeout: 300_000 })
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [action, ...args] = process.argv.slice(2)
  const required = action === "prepare"
    ? ["version", "public-key", "executables", "release-base", "control-plane-url", "output"]
    : ["version", "public-key", "output", "signed"]
  const options = {}
  for (let i = 0; i < args.length; i += 2) {
    const field = args[i].slice(2)
    if (!args[i].startsWith("--") || !required.includes(field) || !args[i + 1] || Object.hasOwn(options, field)) throw new Error("invalid Linux Setup arguments")
    options[field] = args[i + 1]
  }
  if (!["prepare", "assemble"].includes(action) || !required.every(field => options[field])) {
    throw new Error(`Usage: prepare-linux-setup.mjs prepare|assemble ${required.map(field => "--" + field + " VALUE").join(" ")}`)
  }
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(options.version)) throw new Error("invalid Linux Setup version")
  const common = { version: options.version, pin: options["public-key"], output: resolve(options.output) }
  if (action === "prepare") {
    const sourceCommit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim()
    await prepareLinuxSetup({ ...common, sourceCommit, binaries: resolve(options.executables),
      releaseBase: options["release-base"], controlPlaneUrl: options["control-plane-url"] })
  } else await assembleLinuxSetup({ ...common, signed: resolve(options.signed) })
}
