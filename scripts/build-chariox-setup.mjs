#!/usr/bin/env node
// MP-07 / MP-08 / MP-11: generic unsigned Setup plus versioned bootstrap; no private keys.
import { spawnSync } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { defaultControlPlaneUrl } from "./lib/default-control-plane-url.mjs"
export function renderInstallScript(source, { version, publicKeyHex, releaseBase, linuxOnly = false }) {
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version) || !/^[a-f0-9]{64}$/.test(publicKeyHex) || !/^https:\/\/[A-Za-z0-9./_-]+$/.test(releaseBase)) throw new Error("invalid public Setup build inputs")
  const darwin = linuxOnly
    ? "printf '%s\\n' 'This release provides Setup for Linux x86_64. Signed macOS Setup is not available for this version.' >&2; exit 1"
    : "platform=darwin-arm64"
  return source.replaceAll("@VERSION@", version).replaceAll("@RELEASE_PUBLIC_KEY@", publicKeyHex)
    .replaceAll("@RELEASE_BASE@", releaseBase).replaceAll("@DARWIN_SETUP@", darwin)
    .replaceAll("@SETUP_SIGNATURE_INPUT@", linuxOnly ? "setup.digest" : "chariox-setup")
}
export async function linuxProviderArtifacts(repository) {
  const versions = Object.fromEntries((await readFile(join(repository, "deploy/runtime-release/provider-versions.env"), "utf8"))
    .trim().split("\n").map(line => line.split("=")))
  const { packages } = JSON.parse(await readFile(join(repository, "apps/kernel/slice-linux-docker/toolchain/package-lock.json"), "utf8"))
  const names = {
    claude: "@anthropic-ai/claude-code-linux-x64",
    codex: "@openai/codex-linux-x64",
    opencode: "opencode-linux-x64-baseline",
  }
  return Object.fromEntries(Object.entries(names).map(([cli, pkg]) => {
    const version = versions[`CHARIOX_${cli.toUpperCase()}_VERSION`]
    const artifact = packages[`node_modules/${pkg}`]
    const expectedVersion = cli === "codex" ? `${version}-linux-x64` : version
    if (!/^\d+\.\d+\.\d+$/.test(version) || artifact?.version !== expectedVersion
      || !artifact.resolved?.startsWith("https://registry.npmjs.org/")
      || !/^sha512-[A-Za-z0-9+/]{86}==$/.test(artifact.integrity)) {
      throw new Error(`${pkg} artifact must match the pinned provider version and integrity in the toolchain lockfile`)
    }
    return [cli, { version, url: artifact.resolved, integrity: artifact.integrity }]
  }))
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2), options = {}
  for (let i = 0; i < args.length; i += 2) { if (!/^--(version|public-key|release-base|control-plane-url|target|output)$/.test(args[i]) || !args[i + 1]) throw new Error("invalid Setup build arguments"); options[args[i].slice(2)] = args[i + 1] }
  const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..")
  const build = { version: options.version, publicKeyHex: options["public-key"], releaseBase: options["release-base"] ?? "https://github.com/charioxai/chariox/releases/download", controlPlaneUrl: options["control-plane-url"] ?? await defaultControlPlaneUrl(repository) }
  const script = renderInstallScript(await readFile(join(repository, "deploy/setup/install.sh"), "utf8"), build)
  if (!["linux-x64", "darwin-arm64"].includes(options.target)) throw new Error("unsupported Setup build target")
  const output = resolve(options.output)
  await mkdir(output, { recursive: true })
  const scratch = await mkdtemp(join(tmpdir(), "chariox-setup-build-"))
  try {
    const source = await readFile(join(repository, "deploy/runtime-release/extract-release.py"), "utf8")
    const providerArtifacts = options.target === "linux-x64" ? await linuxProviderArtifacts(repository) : undefined
    const entry = join(scratch, "entry.mjs")
    await writeFile(entry, `import { runSetup } from ${JSON.stringify(join(repository, "apps/setup/main.mjs"))};\nrunSetup(${JSON.stringify({ ...build, extractorSource: source, providerArtifacts })}).catch(error => {process.stderr.write("MP-07/MP-08/MP-11: Setup failed: " + (error instanceof Error ? error.message : "unknown installation failure") + "\\n");process.exitCode=1});\n`)
    const result = spawnSync("bun", ["build", "--compile", `--target=bun-${options.target}`, entry, "--outfile", join(output, "chariox-setup")], { stdio: "inherit" })
    if (result.status !== 0) throw new Error("unsigned Setup compilation failed")
    await writeFile(join(output, "install.sh"), script, { mode: 0o755 })
    await writeFile(join(output, "build.json"), JSON.stringify({ schema: "chariox.setup-build.v1", sourceCommit: spawnSync("git", ["rev-parse", "HEAD"], { cwd: repository, encoding: "utf8" }).stdout.trim(), ...build, target: options.target, signed: false }) + "\n")
  } finally { await rm(scratch, { recursive: true, force: true }) }
}
