import assert from "node:assert/strict"
import { test } from "node:test"
import { parsePrivilegedRun, runPrivilegedCommand } from "./privileged-command.js"

test("privileged CLI preserves argv after -- without interpreting command flags", () => {
  assert.deepEqual(parsePrivilegedRun(["--entry", "machine password", "--timeout", "5", "--machine", "this", "--", "echo", "--entry", "literal", "a b", "$(true)"]), {
    request: { OwnerVault: { action: "run_privileged", id: "machine password", timeout_seconds: 5, machine: "this", argv: ["echo", "--entry", "literal", "a b", "$(true)"] } },
    options: { socket: undefined },
  })
})

test("privileged CLI refuses incomplete requests and secret-bearing options", () => {
  for (const args of [[], ["--entry", "x"], ["--entry", "x", "--"], ["--", "whoami"], ["--entry", " ", "--", "whoami"], ["--entry", "x", "--password", "never", "--", "whoami"], ...["0", "241", "NaN", "1.5"].map(value => ["--entry", "x", "--timeout", value, "--", "whoami"])]) {
    assert.throws(() => parsePrivilegedRun(args))
  }
})

test("privileged help needs no connection and unrelated commands are ignored", async t => {
  const output: unknown[][] = []
  t.mock.method(console, "log", (...args: unknown[]) => { output.push(args) })
  assert.equal(await runPrivilegedCommand(["privileged", "run", "--help"]), true)
  assert.match(String(output), /--entry/)
  assert.equal(await runPrivilegedCommand(["other"]), false)
})
