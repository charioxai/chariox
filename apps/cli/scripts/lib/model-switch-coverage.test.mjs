import assert from "node:assert/strict"
import test from "node:test"
import { spawnSync } from "node:child_process"
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { agentSnapshot, assertContinuedPlacement, assertToolProbe, evidenceName, scoreFacts, scoreSummary } from "./model-switch-coverage.mjs"
test("OpenCode model IDs produce one evidence filename", () => {
  assert.equal(evidenceName({provider:"opencode",model:"opencode/big-pickle"}, {provider:"codex",model:"gpt-6"}, 1), "opencode-opencode-big-pickle-to-codex-gpt-6-1.json")
})
test("recall scoring matches whole tokens for each key, allowing extra tokens", () => {
  const facts = {codename:"amber-kestrel",port:"31234",created_file:"ctx-amber.txt"}
  assert.deepEqual(scoreSummary(scoreFacts("codename=amber-kestrel port=31234 created_file=ctx-amber.txt",facts)),{correct:3,total:3})
  assert.deepEqual(scoreSummary(scoreFacts("codename=UNKNOWN amber-kestrel port=312345 created_file=other",facts)),{correct:1,total:3})
})

test("evidence retains placement identity without transport credentials", () => {
  const source = agentSnapshot({provider:"codex", remote_execution:{worker_kernel_id:"worker",execution_lease_id:"lease",leased_agent_id:"agent",relay_token:"fixture-private",relay_url:"ws://relay"}})
  assert.equal(JSON.stringify(source).includes("fixture-private"),false)
  assertContinuedPlacement(source,source,{kernelRef:"worker"})
  assert.throws(()=>assertContinuedPlacement(source,{remote_execution:{...source.remote_execution,leased_agent_id:"other"}},{sliceRef:"slice"}))
  assert.throws(()=>assertContinuedPlacement({},source,{kernelRef:"worker"}))
})

const fileCommand = "echo amber-kestrel > ctx-amber.txt && cat ctx-amber.txt"
const cleanupCommand = "rm -f -- ctx-amber.txt && test ! -e ctx-amber.txt && echo CTXSWITCH_FILE_REMOVED"
const toolResult = (command = fileCommand, status = "completed", raw = "exit_code: 0", output = "amber-kestrel") => ({
  lifecycle: "completed", text: "amber-kestrel", tool_rows: [JSON.stringify({ tool: "bash", input: { command }, status, raw, output })],
})
test("file probes require matching successful tool output, independently of assistant text", () => {
  assert.throws(() => assertToolProbe({lifecycle:"completed",text:"amber-kestrel",tool_rows:[]},"amber-kestrel","file",fileCommand))
  assert.throws(() => assertToolProbe({...toolResult(),lifecycle:"failed"},"amber-kestrel","file",fileCommand))
  assert.throws(() => assertToolProbe(toolResult("pwd"),"amber-kestrel","file",fileCommand))
  assert.throws(() => assertToolProbe(toolResult(fileCommand,"error"),"amber-kestrel","file",fileCommand))
  assert.throws(() => assertToolProbe(toolResult(fileCommand,"completed","exit_code: 1"),"amber-kestrel","file",fileCommand))
  assert.throws(() => assertToolProbe(toolResult(fileCommand,"completed","exit_code: 0","wrong"),"amber-kestrel","file",fileCommand))
  assert.throws(() => assertToolProbe({...toolResult(),tool_rows:["pwd"]},"amber-kestrel","file",fileCommand))
  assertToolProbe({...toolResult(),text:"DONE"},"amber-kestrel","file",fileCommand)
  assertToolProbe(toolResult(`/bin/bash -lc '${fileCommand}'`),"amber-kestrel","file",fileCommand)
  // Claude reports shell failure as status:error; successful Bash results have no exit_code.
  assertToolProbe(toolResult(fileCommand,"completed",""),"amber-kestrel","file",fileCommand)
})
test("cleanup requires the absence check in the successful tool command", () => {
  const result = toolResult("rm -f -- ctx-amber.txt && echo CTXSWITCH_FILE_REMOVED", "completed", "exit_code: 0", "CTXSWITCH_FILE_REMOVED")
  result.text = "CTXSWITCH_FILE_REMOVED"
  assert.throws(()=>assertToolProbe(result,"CTXSWITCH_FILE_REMOVED","cleanup",cleanupCommand))
  assertToolProbe(toolResult(cleanupCommand,"completed","exit_code: 0","CTXSWITCH_FILE_REMOVED"),"CTXSWITCH_FILE_REMOVED","cleanup",cleanupCommand)
})


function runDrill(scenario, placement, roundTrip = false) {
  const root = mkdtempSync(path.join(tmpdir(), "ctxswitch-drill-fixture-"))
  try {
    const result = spawnSync(process.execPath, [
      "--import", fileURLToPath(new URL("./model-switch-drill-fixture.mjs", import.meta.url)),
      fileURLToPath(new URL("../live-model-switch-context-drill.mjs", import.meta.url)),
      "--kernel-url", "ws://fixture.invalid", "--workspace", root,
      "--evidence-root", path.join(root, "evidence"), "--rich-probes",
      "--from", "codex:source:low@creator", "--to", "claude:target:low@broken-target",
      `--${placement}`, "execution-place", ...(roundTrip ? ["--round-trip"] : []),
    ], { encoding: "utf8", timeout: 10000, env: { ...process.env, CTXSWITCH_FIXTURE_ROOT: root, CTXSWITCH_FIXTURE_SCENARIO: scenario } })
    assert.ifError(result.error)
    const evidenceDir = path.join(root, "evidence")
    const evidence = JSON.parse(readFileSync(path.join(evidenceDir, readdirSync(evidenceDir)[0]), "utf8"))
    const trace = JSON.parse(readFileSync(path.join(root, "trace.json"), "utf8"))
    return { ...result, evidence, ...trace }
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
}

for (const placement of ["kernel-ref", "slice-ref"]) {
  test(`${placement}: cleanup uses the creator before a failing target switch`, () => {
    const result = runDrill("target-failure", placement)
    assert.equal(result.status, 1)
    assert.match(result.evidence.failure, /facts lost after switch/)
    assert.equal(result.fileExists, false, "the placed scratch file must be removed despite target failure")
    const cleanup = result.calls.find(call => call.label === "file-cleanup")
    assert.equal(cleanup.account_profile, "creator")
    assert.equal(cleanup.provider, "codex")
    assert.equal(cleanup.agent_id, "placed-agent")
    assert.equal(cleanup[placement.replace("-", "_")], "execution-place")
    assert.ok(result.calls.indexOf(cleanup) < result.calls.findIndex(call => call.label === "tool"))
    assert.ok(result.calls.indexOf(cleanup) < result.calls.findIndex(call => call.label === "switch"))
    assert.equal(result.evidence.file_cleanup.verified, true)
  })
}

for (const scenario of ["file-readback-failure", "file-submit-failure"]) {
  test(`${scenario}: cleanup runs after a partially completed file probe`, () => {
    const result = runDrill(scenario, "slice-ref")
    assert.equal(result.status, 1)
    assert.equal(result.fileExists, false)
    assert.equal(result.calls.filter(call => call.label === "file-cleanup").length, 1)
    assert.equal(result.calls.some(call => call.label === "switch"), false)
    assert.equal(result.evidence.file_cleanup.verified, true)
  })
}

test("cleanup failure is visible in the final log and fails the drill", () => {
  const result = runDrill("cleanup-failure", "kernel-ref")
  assert.equal(result.status, 1)
  assert.equal(result.evidence.file_cleanup.verified, false)
  assert.match(result.stdout.trim().split("\n").at(-1), /model switch drill failed:.*file cleanup/)
})

test("matching assistant reply and unrelated tool do not verify the file probe", () => {
  const result = runDrill("unrelated-file-tool", "slice-ref")
  assert.equal(result.status, 1)
  assert.notEqual(result.evidence.file_probe?.verified, true)
  assert.equal(result.evidence.file_cleanup.verified, true)
  assert.equal(result.fileExists, false)
  assert.equal(result.calls.some(call => call.label === "switch"), false)
})

test("matching assistant marker and failed shell result do not verify cleanup", () => {
  const result = runDrill("cleanup-tool-failure-matching-reply", "kernel-ref")
  assert.equal(result.status, 1)
  assert.equal(result.evidence.file_cleanup.verified, false)
  assert.equal(result.fileExists, true, "failed cleanup must retain the recovery filename")
  assert.match(result.stdout.trim().split("\n").at(-1), /model switch drill failed:.*file cleanup/)
})

test("successful round trip keeps whole-token recall scores after early cleanup", () => {
  const result = runDrill("success", "slice-ref", true)
  assert.equal(result.status, 0, result.stderr)
  assert.equal(result.fileExists, false)
  assert.deepEqual(result.evidence.scores, { after_switch: { correct: 9, total: 9 }, after_return: { correct: 10, total: 10 } })
  assert.equal(result.calls.filter(call => call.label === "file-cleanup").length, 1)
})
