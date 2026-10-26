import assert from "node:assert/strict"
import test from "node:test"
import { readdirSync, readFileSync } from "node:fs"
import { COMMANDS, EXIT_CODES, REQUEST_EXAMPLES, CommandFailure, exitCodeFor, guideText, parseArguments, usageOf } from "./agent-command-table.js"
import {
  attachToSessionRequest, cancelActivePromptRequest, createSessionRequest, endSessionRequest, getSessionStateRequest,
  listProjectsRequest, listSessionsRequest, searchWorkspaceDirectoriesRequest, spawnAgentRequest, submitPromptRequest,
} from "./ipc-requests.js"

const knownFlags = new Set(COMMANDS.flatMap(spec => spec.flags.map(flag => flag.name)))

test("sudo refusal and expiry have separate typed CLI outcomes", () => {
  assert.equal(exitCodeFor("kernel_sudo_refused"), 3)
  assert.equal(exitCodeFor("kernel_sudo_expired"), 5)
  assert.match(guideText(), /kernel_sudo_refused/)
  assert.match(guideText(), /kernel_sudo_expired/)
  assert.equal(exitCodeFor("kernel_sudo_room_owner_local"), 3)
  assert.match(guideText(), /re-create the Room after enrollment/)
})

test("the guide mentions exactly the commands and flags the parser knows", () => {
  const guide = guideText()
  const names = [...guide.matchAll(/chariox ([a-z]+)(?: ([a-z]+))?/g)]
    .map(([, first, second]) => COMMANDS.some(spec => spec.name === `${first} ${second}`) ? `${first} ${second}` : first)
  for (const name of names) assert.ok(COMMANDS.some(spec => spec.name === name), `guide mentions unknown command ${name}`)
  for (const spec of COMMANDS) assert.ok(names.includes(spec.name), `guide omits command ${spec.name}`)
  const flags = new Set(guide.match(/--[a-z0-9-]+/g))
  for (const flag of flags) assert.ok(knownFlags.has(flag), `guide mentions unknown flag ${flag}`)
  for (const flag of knownFlags) assert.ok(flags.has(flag), `guide omits flag ${flag}`)
  assert.ok(guide.split("\n").length <= 105)
  assert.match(guide, /show it to the user verbatim and wait/)
  assert.match(guide, /exit 0 and a printed grant mean approved: go on to step 2/, "an agent must not mistake the waiting note for a pending approval")
})

test("the parser accepts every flag the table lists and refuses anything else", () => {
  for (const spec of COMMANDS) {
    const positionals = (spec.args ?? []).map(name => name.toLowerCase())
    for (const flag of spec.flags) {
      const parsed = parseArguments(spec.name, [...positionals, ...flag.value ? [flag.name, "x"] : [flag.name]])
      assert.equal(parsed.flags.get(flag.name), flag.value ? "x" : true)
      assert.deepEqual(parsed.positionals, positionals)
      assert.throws(() => parseArguments(spec.name, [...positionals, ...flag.value ? [flag.name, "x", flag.name, "y"] : [flag.name, flag.name]]), CommandFailure)
      if (flag.value) assert.throws(() => parseArguments(spec.name, [...positionals, flag.name]), CommandFailure)
    }
    assert.throws(() => parseArguments(spec.name, [...positionals, "--passkey", "never"]), (error: unknown) => error instanceof CommandFailure && error.code === "usage")
    assert.throws(() => parseArguments(spec.name, [...positionals, "stray"]), CommandFailure)
    if (positionals.length > 0) assert.throws(() => parseArguments(spec.name, positionals.slice(1)), CommandFailure, "a missing argument is a usage error")
  }
})

test("one table maps every kernel error code to its exit code", () => {
  const expected: Record<number, string[]> = {
    2: ["usage", "file_path_invalid"],
    3: ["kernel_access_human_only", "kernel_access_ended", "kernel_access_not_covered", "kernel_access_already_granted", "command_id_required", "duplicate_command_conflict", "file_permission_denied"],
    4: ["kernel_unknown", "kernel_ambiguous", "kernel_offline", "kernel_unreachable", "kernel_unpaired", "kernel_incompatible", "kernel_not_available"],
    5: ["kernel_request_timeout", "kernel_connection_lost", "kernel_outcome_unknown"],
  }
  for (const [exit, codes] of Object.entries(expected)) for (const code of codes) assert.equal(exitCodeFor(code), Number(exit), code)
  for (const code of ["kernel_request_overloaded", "ledger_unavailable", "anything_else", "file_not_found", "file_exists", "file_hash_mismatch", "file_offset_mismatch", "file_io_failed"]) assert.equal(exitCodeFor(code), 1)
  assert.deepEqual(EXIT_CODES.map(entry => entry.exit), [0, 2, 3, 4, 5, 6, 7])
})

const kernelSource = new URL("../../kernel/src/", import.meta.url)
const typesDirectory = new URL("local/api/types/", kernelSource)
const rust = [...readdirSync(typesDirectory).filter(name => name.endsWith(".rs")).map(name => readFileSync(new URL(name, typesDirectory), "utf8")),
  readFileSync(new URL("session/session_identity.rs", kernelSource), "utf8")].join("\n")
const requestVariants = new Map([...readFileSync(new URL("local/api/types/request.rs", kernelSource), "utf8").matchAll(/^    (\w+)\((\w+)\),$/gm)].map(match => [match[1]!, match[2]!]))

function requestFields(struct: string): { name: string; required: boolean }[] | null {
  const body = rust.match(new RegExp(`pub struct ${struct}(?:;|\\s*\\{([\\s\\S]*?)\\n\\})`))
  assert.ok(body, `${struct} not found in the kernel sources`)
  if (body[1] === undefined) return null
  const fields: { name: string; required: boolean }[] = []
  let optional = false
  let skipped = false
  for (const line of body[1].split("\n").map(text => text.trim())) {
    if (line.startsWith("#[serde(")) { optional ||= /\bdefault\b/.test(line); skipped ||= /\bskip\b/.test(line) }
    const field = line.match(/^pub (\w+): (.+),$/)
    if (!field) continue
    if (!skipped) fields.push({ name: field[1]!, required: !optional && !field[2]!.startsWith("Option<") })
    optional = skipped = false
  }
  return fields
}

test("every request example is one kernel request variant with the kernel's field names", () => {
  assert.deepEqual(REQUEST_EXAMPLES.map(entry => Object.keys(entry.body)[0]), [
    "ListSessions", "ListProjects", "SearchWorkspaceDirectories", "CreateSession", "AttachToSession", "GetSessionState",
    "SpawnAgent", "SubmitPrompt", "CancelActivePrompt", "RunShellCommand", "EndSession", "RequestKernelSudo",
  ])
  const guide = guideText()
  for (const { body } of REQUEST_EXAMPLES) {
    const parsed = JSON.parse(JSON.stringify(body)) as Record<string, unknown>
    const [variant, ...extra] = Object.keys(parsed)
    assert.deepEqual(extra, [])
    const struct = requestVariants.get(variant!)
    assert.ok(struct, `${variant} is not a request variant in request.rs`)
    assert.ok(guide.includes(JSON.stringify(body)), `the guide omits the ${variant} example`)
    const fields = requestFields(struct)
    const given = parsed[variant!]
    if (fields === null) { assert.equal(given, null); continue }
    const names = Object.keys(given as object)
    for (const name of names) assert.ok(fields.some(field => field.name === name), `${variant} has no field ${name}`)
    for (const field of fields.filter(entry => entry.required)) assert.ok(names.includes(field.name), `${variant} example omits required ${field.name}`)
  }
})

test("request examples equal what the kernel-client builders produce", () => {
  const byName = new Map(REQUEST_EXAMPLES.map(entry => [Object.keys(entry.body)[0]!, entry.body]))
  const built = [
    listSessionsRequest(), listProjectsRequest(), searchWorkspaceDirectoriesRequest("NAME", 20), createSessionRequest("/PATH", "/PATH"),
    attachToSessionRequest("SESSION_ID", "CLIENT_NAME"), getSessionStateRequest("SESSION_ID"), spawnAgentRequest("SESSION_ID", "codex"),
    submitPromptRequest("SESSION_ID", "ATTACHMENT_ID", null, "PROMPT", []), cancelActivePromptRequest("SESSION_ID", "ATTACHMENT_ID", null),
    endSessionRequest("SESSION_ID"),
  ]
  for (const request of built) assert.deepEqual(byName.get(Object.keys(request)[0]!), request)
})

test("the guide tells the agent to read parse errors and to give state-changing requests a command id", () => {
  const guide = guideText()
  assert.match(guide, /cannot parse[^\n]*read/)
  assert.match(guide, /changes state[^\n]*--command-id/)
})

test("the guide steps cover watching a kernel and an attachment that is kept alive", () => {
  const guide = guideText()
  assert.match(guide, /4\. Watch events on a kernel: chariox watch --kernel NAME --session ID --attachment ID/)
  assert.match(guide, /An AttachToSession you make on another kernel is kept alive for you while your access lasts; send DetachFromSession when you are done with it\./)
  assert.doesNotMatch(guide, /lapses/)
  assert.deepEqual(COMMANDS.find(spec => spec.name === "watch")?.flags.map(flag => flag.name), ["--kernel", "--session", "--attachment", "--from", "--socket"])
})

test("watch's usage line shows the flags it cannot run without as required", () => {
  assert.equal(usageOf("watch"), "Usage: chariox watch --kernel NAME --session SESSION_ID --attachment ATTACHMENT_ID [--from EVENT_ID] [--socket PATH]")
})

test("the guide teaches moving files and resuming a transfer", () => {
  const guide = guideText()
  assert.match(guide, /5\. Move files: chariox files put\|get --kernel NAME/)
  assert.match(guide, /put and get resume after an interruption: run the same command again/)
  assert.match(guide, /Listing, stat, hashing and small reads go through a terminal/)
  for (const verb of ["get", "put"]) assert.ok(COMMANDS.some(spec => spec.name === `files ${verb}`))
  for (const verb of ["stat", "ls"]) assert.ok(!COMMANDS.some(spec => spec.name === `files ${verb}`))
  assert.ok(COMMANDS.find(spec => spec.name === "files get")!.flags.some(flag => flag.name === "--sha256" && flag.value === "HEX"))
})

test("the guide teaches long-running commands in a terminal that outlives the agent", () => {
  const guide = guideText()
  assert.match(guide, /6\. Run long commands: chariox terminal open\|send\|read\|wait\|close/)
  assert.match(guide, /keep running if you disconnect; read them later with --since/)
})
