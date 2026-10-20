// MP-07 / MP-08 / MP-10 / MP-11: every generated landing consumer must agree.
import { execFileSync } from "node:child_process"
import test from "node:test"
test("generated protocol consumers match the two runtime constants", () => {
  execFileSync(process.execPath, [new URL("generate-protocol-versions.mjs", import.meta.url).pathname, "--check"])
})

import assert from "node:assert/strict"
import { readFileSync, readdirSync } from "node:fs"
import { repositoryRoot } from "./lib/protocol-versions.mjs"
test("MP-08/MP-10/MP-11 protocol tests use the generated pair instead of landing-number literals", () => {
  const pins = []
  for (const directory of ["apps/kernel/src", "apps/cli/src", "packages/kernel-client/src"]) {
    const root = new URL(directory + "/", repositoryRoot)
    for (const file of readdirSync(root, { recursive: true })) {
      if (!file.endsWith(".rs") && !file.endsWith(".test.ts")) continue
      const source = readFileSync(new URL(file, root), "utf8")
      if (/assert(?:_eq!|\.equal)\(\s*(?:[\w:]+::)?(?:LOCAL_DAEMON_PROTOCOL_VERSION|RELAY_PEER_PROTOCOL_VERSION)\s*,\s*\d+/.test(source)) pins.push(directory + "/" + file)
    }
  }
  assert.deepEqual(pins, [], "derive current versions from the generated pair; retain historical floors with >= checks")
})
