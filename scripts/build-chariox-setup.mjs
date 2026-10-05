#!/usr/bin/env node
// MP-07 / MP-08 / MP-11: generic unsigned Setup plus versioned bootstrap; no private keys.
import { spawnSync } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
export function renderInstallScript(source, { version, publicKeyHex, releaseBase }) {
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version) || !/^[a-f0-9]{64}$/.test(publicKeyHex) || !/^https:\/\/[A-Za-z0-9./_-]+$/.test(releaseBase)) throw new Error("invalid public Setup build inputs")
  return source.replaceAll("@VERSION@", version).replaceAll("@RELEASE_PUBLIC_KEY@", publicKeyHex).replaceAll("@RELEASE_BASE@", releaseBase)
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2), options = {}
  for (let i = 0; i < args.length; i += 2) { if (!/^--(version|public-key|release-base|target|output)$/.test(args[i]) || !args[i + 1]) throw new Error("invalid Setup build arguments"); options[args[i].slice(2)] = args[i + 1] }
  const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..")
  const build = { version: options.version, publicKeyHex: options["public-key"], releaseBase: options["release-base"] ?? "https://github.com/charioxai/chariox/releases/download" }
  const script = renderInstallScript(await readFile(join(repository, "deploy/setup/install.sh"), "utf8"), build)
  if (!["linux-x64", "darwin-arm64"].includes(options.target)) throw new Error("unsupported Setup build target")
  const output = resolve(options.output)
  await mkdir(output, { recursive: true })
  const scratch = await mkdtemp(join(tmpdir(), "chariox-setup-build-"))
  try {
    const source = await readFile(join(repository, "deploy/managed-kernel/extract-release.py"), "utf8")
    const entry = join(scratch, "entry.mjs")
    await writeFile(entry, `import { runSetup } from ${JSON.stringify(join(repository, "apps/setup/main.mjs"))};\nrunSetup(${JSON.stringify({ ...build, extractorSource: source })}).catch(() => {process.stderr.write("MP-07/MP-08/MP-11: Setup failed; check signed release, user service and enrollment prerequisites\\n");process.exitCode=1});\n`)
    const result = spawnSync("bun", ["build", "--compile", `--target=bun-${options.target}`, entry, "--outfile", join(output, "chariox-setup")], { stdio: "inherit" })
    if (result.status !== 0) throw new Error("unsigned Setup compilation failed")
    await writeFile(join(output, "install.sh"), script, { mode: 0o755 })
    await writeFile(join(output, "build.json"), JSON.stringify({ schema: "chariox.setup-build.v1", sourceCommit: spawnSync("git", ["rev-parse", "HEAD"], { cwd: repository, encoding: "utf8" }).stdout.trim(), ...build, target: options.target, signed: false }) + "\n")
  } finally { await rm(scratch, { recursive: true, force: true }) }
}
