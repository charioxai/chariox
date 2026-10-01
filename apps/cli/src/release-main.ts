// Entry point of the compiled `chariox` release executable (scripts/compile.mjs).
// `--version` answers without starting the TUI; anything else runs the CLI.
import process from "node:process"

import { releaseVersion } from "./release-build.js"

const argv = process.argv.slice(2)
if (argv.length === 1 && (argv[0] === "--version" || argv[0] === "-V")) {
  process.stdout.write(`chariox ${releaseVersion ?? "development"}\n`)
} else {
  await import("./index.js")
}
