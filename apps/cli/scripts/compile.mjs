#!/usr/bin/env bun
// Compiles the CLI/TUI into one self-contained `chariox` executable with Bun.
//
//   pnpm --filter @chariox/cli run build
//   bun apps/cli/scripts/compile.mjs --target linux-x64|darwin-arm64 --outfile PATH [--version VERSION]
//
// The executable embeds the Bun runtime, every JavaScript dependency, OpenTUI's
// native renderer for the target (libopentui, loaded with bun:ffi) and the
// tree-sitter worker with its grammars, so users need no Bun, Node or pnpm.
//
// Build each target on its own platform: pnpm installs only the host's OpenTUI
// native package, and the script refuses a target whose package is missing.
import { realpathSync } from "node:fs"
import { access, readFile } from "node:fs/promises"
import { createRequire } from "node:module"
import path from "node:path"
import { fileURLToPath } from "node:url"

export const targets = {
  // The baseline build runs on x86-64 CPUs without AVX2, like the Rust binaries.
  "linux-x64": { bun: "bun-linux-x64-baseline", os: "linux", arch: "x64" },
  "darwin-arm64": { bun: "bun-darwin-arm64", os: "darwin", arch: "arm64" },
}

// OpenTUI picks its native package at run time from the host platform. A
// compiled executable has no node_modules, so the import must name the
// target's package at build time for Bun to embed it.
const platformImport = "import(`@opentui/core-${process.platform}-${process.arch}/index.ts`)"

export function pinOpenTuiPlatform(code, target) {
  const occurrences = code.split(platformImport).length - 1
  if (occurrences === 0) return { code, pinned: 0 }
  const specifier = `@opentui/core-${target.os}-${target.arch}/index.ts`
  return { code: code.replaceAll(platformImport, `import(${JSON.stringify(specifier)})`), pinned: occurrences }
}

export function parseCompileArgs(argv) {
  const options = { version: undefined, target: undefined, outfile: undefined }
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index]
    const value = argv[index + 1]
    if (!["--target", "--outfile", "--version"].includes(flag)) throw new Error(`unknown argument: ${flag}`)
    if (value === undefined || value.startsWith("--")) throw new Error(`${flag} needs a value`)
    const key = flag.slice(2)
    if (options[key] !== undefined) throw new Error(`${flag} given twice`)
    options[key] = value
    index += 1
  }
  if (!options.target || !Object.hasOwn(targets, options.target)) {
    throw new Error(`--target must be one of ${Object.keys(targets).join(", ")}`)
  }
  if (!options.outfile) throw new Error("--outfile is required")
  if (options.version !== undefined && !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(options.version)) {
    throw new Error("--version must be a semantic version such as 0.2.0 or 0.2.0-rc.1")
  }
  return options
}

// Bun serves embedded entry points from this virtual root in a compiled executable.
const embeddedRoot = "/$bunfs/root/"

export async function compileCli(options) {
  const target = targets[options.target]
  const appDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
  const repoRoot = path.resolve(appDir, "../..")
  const entry = path.join(appDir, "dist/release-main.js")
  await access(entry).catch(() => {
    throw new Error("apps/cli/dist is missing; run `pnpm --filter @chariox/cli run build` first")
  })
  const requireFromCli = createRequire(path.join(appDir, "package.json"))
  const coreDir = path.dirname(realpathSync(requireFromCli.resolve("@opentui/core/package.json")))
  const nativePackage = `@opentui/core-${target.os}-${target.arch}`
  let nativeModule
  try {
    nativeModule = createRequire(path.join(coreDir, "package.json")).resolve(`${nativePackage}/index.ts`)
  } catch {
    throw new Error(`${nativePackage} is not installed; compile ${options.target} on a ${options.target} host`)
  }
  // The tree-sitter worker runs from its own embedded entry point.
  const parserWorker = path.join(coreDir, "parser.worker.js")
  const workerPath = embeddedRoot + path.relative(repoRoot, parserWorker).split(path.sep).join("/")

  let pinned = 0
  let nativeLoaderPinned = 0
  const result = await Bun.build({
    entrypoints: [entry, parserWorker],
    root: repoRoot,
    compile: {
      target: target.bun,
      outfile: path.resolve(options.outfile),
      // A user's working directory must not configure the CLI.
      autoloadDotenv: false,
      autoloadBunfig: false,
    },
    define: {
      CHARIOX_RELEASE_VERSION: JSON.stringify(options.version ?? "development"),
      OTUI_TREE_SITTER_WORKER_PATH: JSON.stringify(workerPath),
    },
    plugins: [{
      name: "chariox-release",
      setup(build) {
        // The hyperlink capability helper must use the embedded renderer,
        // rather than look for a source node_modules tree at run time.
        build.onLoad({ filter: /[\\/]terminal-render-library\.js$/ }, () => {
          nativeLoaderPinned += 1
          return {
            contents: `export async function loadTerminalRenderLibrary() { return (await import(${JSON.stringify(nativeModule)})).default; }`,
            loader: "js",
          }
        })
        // Bun resolves solid-js through its "node" condition to Solid's server
        // build, which never runs effects: load the client build instead, as
        // @opentui/solid/bun-plugin does for the TUI started from source.
        for (const [filter, client] of [
          [/[\\/]solid-js[\\/]dist[\\/]server\.js$/, "solid.js"],
          [/[\\/]solid-js[\\/]store[\\/]dist[\\/]server\.js$/, "store.js"],
        ]) {
          build.onLoad({ filter }, async (args) => ({
            contents: await readFile(path.join(path.dirname(args.path), client), "utf8"),
            loader: "js",
          }))
        }
        build.onLoad({ filter: /[\\/]@opentui[\\/]core[\\/][^\\/]+\.js$/ }, async (args) => {
          const rewritten = pinOpenTuiPlatform(await readFile(args.path, "utf8"), target)
          pinned += rewritten.pinned
          return { contents: rewritten.code, loader: "js" }
        })
      },
    }],
  })
  if (!result.success) {
    throw new Error(`bun build failed:\n${result.logs.map((log) => String(log)).join("\n")}`)
  }
  if (pinned !== 1) throw new Error(`expected one OpenTUI platform import, pinned ${pinned}`)
  if (nativeLoaderPinned !== 1) throw new Error(`expected one terminal renderer loader, pinned ${nativeLoaderPinned}`)
  return { outfile: path.resolve(options.outfile), target: options.target, bunTarget: target.bun, nativePackage, workerPath }
}

if (import.meta.main) {
  try {
    const receipt = await compileCli(parseCompileArgs(process.argv.slice(2)))
    process.stdout.write(`${JSON.stringify(receipt)}\n`)
  } catch (error) {
    process.stderr.write(`compile: ${error instanceof Error ? error.message : String(error)}\n`)
    process.exit(1)
  }
}
