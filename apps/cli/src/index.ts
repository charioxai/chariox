// Bun resolves `solid-js` through its "node" export condition to Solid's
// server build, which never runs effects or onMount. Load the client build
// instead (the same redirects as `@opentui/solid/bun-plugin`) before anything
// imports Solid, then start the CLI.
import { readFile } from "node:fs/promises"

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
