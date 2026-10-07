import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"

// MP-11 F7: fail CI if a new public response/event bypasses the credential-free DTO.
for (const file of [
  "apps/kernel/src/local/api/types/response.rs",
  "apps/kernel/src/local/api/types/provider_control.rs",
  "apps/kernel/src/transport/kernel_protocol.rs",
]) {
  test(`MP-11 F7 public provider-run boundary: ${file}`, () => {
    const source = readFileSync(new URL(`../${file}`, import.meta.url), "utf8")
    assert.doesNotMatch(source, /\bRuntimeProviderRun\b/)
    assert.match(source, /PublicProviderRun/)
  })
}

import { runPublicProviderRunProtocolDrill } from "../apps/cli/scripts/public-provider-run-protocol-drill.mjs"

test("MP-11 F7 protocol drill refuses missing/mismatched tests and suppresses private diagnostics", () => {
  assert.throws(() => runPublicProviderRunProtocolDrill(), /MP-11 F7 requires/)
  for (const result of [
    { status: 0, stdout: "test result: ok. 0 passed; 0 failed;", stderr: "private-fixture-sentinel" },
    { status: 1, stdout: "private-fixture-sentinel", stderr: "private-fixture-sentinel" },
    { status: null, error: new Error("private-fixture-sentinel") },
  ]) {
    const report = runPublicProviderRunProtocolDrill("/synthetic/test", () => result)
    assert.equal(report.passed, false)
    assert.equal(JSON.stringify(report).includes("private-fixture-sentinel"), false)
  }
  const report = runPublicProviderRunProtocolDrill("/synthetic/test", () => ({ status: 0, stdout: "test result: ok. 1 passed; 0 failed;" }))
  assert.equal(report.passed, true)
  assert.equal(report.results.length, 3)
  const runtime = readFileSync(new URL("../apps/kernel/src/local/api/types.rs", import.meta.url), "utf8")
  assert.equal(report.localProtocol, Number(runtime.match(/LOCAL_DAEMON_PROTOCOL_VERSION: u32 = (\d+);/)[1]))
})
