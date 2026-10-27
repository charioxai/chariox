// MP-08 / MP-10 / MP-11: stock screen options and detached launch metadata.
import assert from "node:assert/strict"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { runClaudeKernelScreen } from "./claude-kernel-screen.js"

test("detached Claude screen uses stock options and forwards its kernel banner", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-screen-test-"))
  const originalPath = process.env.PATH
  const originalWrite = process.stderr.write
  let output = ""
  try {
    await writeFile(path.join(root, "screen"), `#!/bin/sh
case "$1" in
  -dmS)
    for arg in "$@"; do [ "$arg" != "-Logfile" ] || exit 64; done
    printf '%s' "$2" > "${root}/name"
    printf '%s' "$CHARIOX_CLAUDE_NATIVE_BANNER_FILE" > "${root}/banner"
    touch "${root}/alive" ;;
  -ls)
    if [ -e "${root}/alive" ]; then
      if [ -e "${root}/listed" ]; then
        rm "${root}/alive"
        printf '  chariox session: session-test\\n  provider run: run-test\\n' > "$(cat "${root}/banner")"
      else touch "${root}/listed"; fi
      printf '1.%s\\n' "$(cat "${root}/name")"
      exit 1
    else exit 1; fi ;;
esac
`, { mode: 0o700 })
    process.env.PATH = `${root}:${originalPath}`
    process.stderr.write = ((chunk: string | Uint8Array) => { output += chunk.toString(); return true }) as typeof process.stderr.write
    await runClaudeKernelScreen([], root)
    assert.match(output, /chariox session: session-test/)
    assert.match(output, /screen: chariox-claude-/)
  } finally {
    process.env.PATH = originalPath
    process.stderr.write = originalWrite
    await rm(root, { recursive: true, force: true })
  }
})
