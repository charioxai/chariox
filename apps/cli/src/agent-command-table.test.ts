import assert from "node:assert/strict"
import test from "node:test"
import { COMMANDS, EXIT_CODES, CommandFailure, exitCodeFor, guideText, parseFlags } from "./agent-command-table.js"

const knownFlags = new Set(COMMANDS.flatMap(spec => spec.flags.map(flag => flag.name)))

test("the guide mentions exactly the commands and flags the parser knows", () => {
  const guide = guideText()
  const names = [...guide.matchAll(/chariox ([a-z]+)(?: ([a-z]+))?/g)]
    .map(([, first, second]) => COMMANDS.some(spec => spec.name === `${first} ${second}`) ? `${first} ${second}` : first)
  for (const name of names) assert.ok(COMMANDS.some(spec => spec.name === name), `guide mentions unknown command ${name}`)
  for (const spec of COMMANDS) assert.ok(names.includes(spec.name), `guide omits command ${spec.name}`)
  const flags = new Set(guide.match(/--[a-z-]+/g))
  for (const flag of flags) assert.ok(knownFlags.has(flag), `guide mentions unknown flag ${flag}`)
  for (const flag of knownFlags) assert.ok(flags.has(flag), `guide omits flag ${flag}`)
  assert.ok(guide.split("\n").length <= 40)
  assert.match(guide, /show it to the user verbatim and wait/)
  assert.match(guide, /exit 0 and a printed grant mean approved: go on to step 2/, "an agent must not mistake the waiting note for a pending approval")
})

test("the parser accepts every flag the table lists and refuses anything else", () => {
  for (const spec of COMMANDS) {
    for (const flag of spec.flags) {
      const parsed = parseFlags(spec.name, flag.value ? [flag.name, "x"] : [flag.name])
      assert.equal(parsed.get(flag.name), flag.value ? "x" : true)
      assert.throws(() => parseFlags(spec.name, flag.value ? [flag.name, "x", flag.name, "y"] : [flag.name, flag.name]), CommandFailure)
      if (flag.value) assert.throws(() => parseFlags(spec.name, [flag.name]), CommandFailure)
    }
    assert.throws(() => parseFlags(spec.name, ["--passkey", "never"]), (error: unknown) => error instanceof CommandFailure && error.code === "usage")
    assert.throws(() => parseFlags(spec.name, ["stray"]), CommandFailure)
  }
})

test("one table maps every machine error code to its exit code", () => {
  const expected: Record<number, string[]> = {
    3: ["kernel_access_human_only", "kernel_access_ended", "kernel_access_not_covered", "kernel_access_already_granted", "command_id_required", "duplicate_command_conflict"],
    4: ["machine_unknown", "machine_ambiguous", "machine_offline", "machine_unreachable", "machine_unpaired", "machine_incompatible", "machine_not_available"],
    5: ["machine_request_timeout", "machine_connection_lost", "machine_outcome_unknown"],
  }
  for (const [exit, codes] of Object.entries(expected)) for (const code of codes) assert.equal(exitCodeFor(code), Number(exit), code)
  assert.equal(exitCodeFor("usage"), 2)
  for (const code of ["kernel_request_overloaded", "ledger_unavailable", "anything_else"]) assert.equal(exitCodeFor(code), 1)
  assert.deepEqual(EXIT_CODES.map(entry => entry.exit), [0, 2, 3, 4, 5, 6])
})
