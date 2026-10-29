// MP-07 / MP-08 / MP-10 / MP-11: every generated landing consumer must agree.
import { execFileSync } from "node:child_process"
import test from "node:test"
test("generated protocol consumers match the two runtime constants", () => {
  execFileSync(process.execPath, [new URL("generate-protocol-versions.mjs", import.meta.url).pathname, "--check"])
})

import assert from "node:assert/strict"
import { readFileSync, readdirSync } from "node:fs"
import { repositoryRoot, admittedPredecessorProtocols } from "./lib/protocol-versions.mjs"
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

// MP-08/MP-10/MP-11 #992: acc/agent-model 4d478c71 advertised 513/121
// while only ordering input. That deployed protocol must retain reply-first reads.
test("MP-08/MP-10/MP-11 mirror read ordering has a floor above the unordered target", () => {
  const pair = JSON.parse(readFileSync(new URL("fixtures/protocol-versions.json", repositoryRoot), "utf8"))
  assert.ok(pair.local >= 516, "coordinator-allocated mirror ordering floor is 516, above target 513")
  assert.ok(pair.relayPeer >= 124, "paired peer allocation is 124, above target 121")
})

test("managed upgrades retain the current and previous releases and the target", () => {
  const pair = JSON.parse(readFileSync(new URL("fixtures/protocol-versions.json", repositoryRoot), "utf8"))
  const policy = JSON.parse(readFileSync(new URL("apps/kernel/managed-upgrade-protocol-transitions.json", repositoryRoot), "utf8"))
  assert.deepEqual(admittedPredecessorProtocols, [629, 636])
  assert.deepEqual(policy.upgradeFrom, [...admittedPredecessorProtocols, pair.local])
  assert.deepEqual(policy.rollbackTo, policy.upgradeFrom)
})

test("dictation603 keeps its allocated protocol floors", () => {
  const pair = JSON.parse(readFileSync(new URL("fixtures/protocol-versions.json", repositoryRoot), "utf8"))
  assert.ok(pair.local >= 603)
  assert.ok(pair.relayPeer >= 209)
})
