// Bun resolves `solid-js` through its "node" export condition to Solid's
// server build, which never runs effects or onMount. Load the client build
// instead (the same redirects as `@opentui/solid/bun-plugin`) before anything
// imports Solid, then start the CLI.
import { readFile } from "node:fs/promises"

import { parseArgs } from "./cli-options.js"

// Admit ordinary TUI arguments before loading rendering, logging or runtime
// modules. Subcommands own their parsers and validate before runtime actions.
const argv = process.argv.slice(2)
if (!["access", "sudo", "app", "logs", "codex", "claude", "opencode", "publication", "deployments", "deployed", "cloud", "login", "setup"].includes(argv[0] ?? "")) {
  try {
    parseArgs(argv)
  } catch (error) {
    process.stderr.write(`error: ${error instanceof Error ? error.message : String(error)}\n`)
    process.exit(1)
  }
}

type OnLoad = (args: { path: string }) => Promise<{ contents: string; loader: "js" }>
declare const Bun: {
  plugin(plugin: { name: string; setup(build: { onLoad(options: { filter: RegExp }, load: OnLoad): void }): void }): void
}

const clientBuild = (from: string, to: string): OnLoad => async (args) => ({
  contents: await readFile(args.path.slice(0, -from.length) + to, "utf8"),
  loader: "js",
})

Bun.plugin({
  name: "solid-client-build",
  setup(build) {
    build.onLoad({ filter: /[\\/]solid-js[\\/]dist[\\/]server\.js$/ }, clientBuild("server.js", "solid.js"))
    build.onLoad({ filter: /[\\/]solid-js[\\/]store[\\/]dist[\\/]server\.js$/ }, clientBuild("server.js", "store.js"))
  },
})

await import("./cli-main.js")
