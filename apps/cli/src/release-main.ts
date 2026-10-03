// Entry point of the compiled `chariox` release executable (scripts/compile.mjs).
// `--version` answers without starting the TUI, and `__claude-hook HANDLER` runs
// a native-Claude hook handler; anything else runs the CLI.
import process from "node:process"
import { pathToFileURL } from "node:url"

import { claudeHookCommand, releaseVersion } from "./release-build.js"

const argv = process.argv.slice(2)
if (argv.length === 1 && (argv[0] === "--version" || argv[0] === "-V")) {
  process.stdout.write(`chariox ${releaseVersion ?? "development"}\n`)
} else if (argv.length === 2 && argv[0] === claudeHookCommand) {
  await import(pathToFileURL(argv[1]!).href)
} else {
  await import("./index.js")
}
