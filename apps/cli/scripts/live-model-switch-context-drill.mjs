#!/usr/bin/env node
// Live drill: establish facts with one provider/model, switch the agent's
// model through UpdateAgentProfile (the agent-config path), then ask the new
// model to recall the facts without tools.
//
// usage: live-model-switch-context-drill.mjs --kernel-url ws://127.0.0.1:PORT --workspace DIR \
//   --from codex:gpt-5.5[:effort][@profile] --to claude:sonnet[:effort][@profile] [--evidence-root DIR]
//   [--before-switch-cmd CMD] [--after-switch-cmd CMD]  (e.g. restart the kernel at that point)
//   [--filler-turns N]  ordinary turns between the facts and the switch
//   [--recall-attachment-bytes N]  attach an N-byte text file to the recall prompt; a
//     Claude native target carries it as hidden context, next to the handoff
//   [--queued-follow-up]  submit a second prompt while the recall turn runs; the kernel
//     log shows the handoff rendered once for the new session
//   [--keep-session]  leave the drill session for inspection instead of deleting it
//   [--bulk-turns N --bulk-kb K]  scripted long session: N turns that each paste K KB of
//     notes and ask for a summary, so the history outgrows any fixed handoff
//   [--kernel-ref REF | --slice-ref REF]  home-owned leased or slice agent
//   [--rich-probes]  also probe an answer-only fact, a superseded decision, the current
//     task and next step, a file a tool created, and the latest tool result
//   [--allow-recall]  let the recall turn use the chariox.search_recall tool
//   [--round-trip]  after the recall, plant one more fact and switch back to the first profile
import assert from "node:assert/strict"
import { execSync } from "node:child_process"
import { randomInt, randomUUID } from "node:crypto"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import path from "node:path"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

import { agentSnapshot, assertContinuedPlacement, assertToolProbe, evidenceName, scoreFacts, scoreSummary } from "./lib/model-switch-coverage.mjs"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  attachToSessionRequest,
  createSessionRequest,
  deleteSessionRequest,
  getSessionHistoryBlobContentRequest,
  getSessionHistoryOutlineRequest,
  spawnAgentRequest,
  submitPromptRequest,
  updateAgentProfileRequest,
} from "../dist/ipc-requests.js"

function option(name, fallback = null) {
  const index = process.argv.indexOf(`--${name}`)
  return index >= 0 ? process.argv[index + 1] : fallback
}

function variant(response, key) {
  const value = response?.[key]
  if (value == null) throw new Error(`expected ${key}, received ${JSON.stringify(response).slice(0, 400)}`)
  return value
}

function profile(spec) {
  const [selection, accountProfile = null] = spec.split("@")
  const [provider, model = null, effort = null] = selection.split(":")
  return { provider, model, effort, accountProfile }
}

const WORDS = ["amber", "basalt", "cobalt", "dahlia", "ember", "fjord", "garnet", "harbor", "indigo", "juniper", "kestrel", "lagoon", "marlin", "nimbus", "obsidian", "pelican", "quartz", "raven", "saffron", "tundra"]
const pick = () => WORDS[randomInt(WORDS.length)]

async function turnOutput(client, sessionId, agentId, marker, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const outline = variant(await client.send(getSessionHistoryOutlineRequest(sessionId, [agentId], 3)), "SessionHistoryOutline")
    const turn = outline.agents.find(agent => agent.agent_id === agentId)?.turns.find(turn => turn.user_prompt?.entry?.text.includes(marker))
    if (turn && ["completed", "failed", "cancelled"].includes(turn.lifecycle)) {
      for (const blob of turn.blobs.filter(blob => ["provider_output", "provider_error", "provider_tool"].includes(blob.kind))) {
        const content = variant(await client.send(getSessionHistoryBlobContentRequest(sessionId, agentId, blob.blob_id)), "SessionHistoryBlobContent")
        turn.entries.push(...content.entries)
      }
      const entries = [...turn.entries, ...(turn.summary ? [turn.summary] : [])].map(page => page.entry)
      const errors = entries.filter(entry => entry.kind === "provider_error").map(entry => entry.text)
      const text = entries.filter(entry => entry.kind === "provider_output").map(entry => entry.text).join("")
      const runIds = [...new Set(entries.map(entry => entry.provider_run_id).filter(Boolean))]
      const tool_rows = entries.filter(entry => entry.kind === "provider_tool").map(entry => entry.text)
      return { lifecycle: turn.lifecycle, text, errors, runIds, tool_rows }
    }
    await sleep(500)
  }
  throw new Error(`timed out waiting for turn ${marker}`)
}

const kernelUrl = option("kernel-url")
const workspace = path.resolve(option("workspace", process.cwd()))
const from = profile(option("from", ""))
const to = profile(option("to", ""))
const placement = { kernelRef: option("kernel-ref"), sliceRef: option("slice-ref") }
if (placement.kernelRef && placement.sliceRef) throw new Error("select either --kernel-ref or --slice-ref")
const timeoutMs = Number(option("timeout-ms", "300000"))
const fillerTurns = Number(option("filler-turns", "0"))
const recallAttachmentBytes = Number(option("recall-attachment-bytes", "0"))
const bulkTurns = Number(option("bulk-turns", "0"))
const bulkBytes = Number(option("bulk-kb", "40")) * 1024
const richProbes = process.argv.includes("--rich-probes")
const TOPICS = ["tide pools", "bread baking", "lighthouses", "glaciers", "chess openings", "bees", "volcanoes", "paper making", "kites", "river deltas", "telescopes", "salt marshes"]
const evidenceRoot = path.resolve(option("evidence-root", path.join(process.env.HOME ?? process.cwd(), ".codex/evidence/model-switch-context")))
if (!kernelUrl || !from.provider || !to.provider) {
  console.error("usage: live-model-switch-context-drill.mjs --kernel-url ws://... --from provider:model --to provider:model [--workspace dir]")
  process.exit(2)
}

const facts = { codename: `${pick()}-${pick()}`, port: String(20000 + randomInt(40000)), fruit: `${pick()}fruit` }
if (richProbes) {
  Object.assign(facts, {
    cache_database: `${pick()}db`, current_task: `migrate-${pick()}-${pick()}`, next_step: `write-${pick()}-rollback`,
    created_file: `ctx-${pick()}-${randomUUID().slice(0, 8)}.txt`, python_output: null, release_name: null,
  })
}
// Notes a scripted long session pastes: numbered, varied lines, about 4 bytes per token.
const notes = (turn) => {
  const lines = []
  for (let line = 0; lines.join("\n").length < bulkBytes; line++) {
    lines.push(`Note ${turn}.${line}: the ${pick()} ${pick()} report measured ${randomInt(100000)} units near the ${pick()} ${pick()} station, ${pick()} grade ${randomInt(1, 9)}.`)
  }
  return lines.join("\n")
}
const evidence = { from, to, placement, facts, started_at_ms: Date.now(), turns: [] }
const client = new LocalIpcClient(kernelUrl)
let sessionId = null
let scratch = null
let ask = null
let fileProbeAttempted = false
try {
  const created = variant(await client.send(createSessionRequest(workspace, workspace, `ctxswitch-${Date.now()}`, {
    provider: from.provider, model: from.model, effort: from.effort, account_profile: from.accountProfile,
    execution_mode: "build", permission_level: "yolo",
  })), "SessionCreated")
  sessionId = created.session.id
  const agent = placement.kernelRef || placement.sliceRef
    ? variant(await client.send(spawnAgentRequest(sessionId, from.provider, "ctxswitch-coverage", from.model,
      undefined, from.effort, "build", "yolo", placement.kernelRef ?? undefined, undefined,
      placement.sliceRef ?? undefined, from.accountProfile)), "AgentSpawned").agent
    : created.agent
  const agentId = agent.id
  evidence.source_agent = agentSnapshot(agent)
  assertContinuedPlacement(evidence.source_agent, evidence.source_agent, placement)

  // The drill does not subscribe to events, so the kernel may reap an idle
  // attachment between turns (or drop it on restart); attach for each prompt.
  const attach = async () => variant(await client.send(attachToSessionRequest(sessionId, `ctxswitch-drill-${process.pid}`)), "SessionAttached").attachment
  const submit = async (label, prompt, files = []) => {
    const marker = `[${label}-${Date.now()}]`
    const attachment = await attach()
    variant(await client.send(submitPromptRequest(sessionId, attachment.id, agentId, `${marker} ${prompt}`, files)), "PromptSubmitted")
    return marker
  }
  ask = async (label, prompt, files = [], followUp = null) => {
    const marker = await submit(label, prompt, files)
    const followUpMarker = followUp && await submit(`${label}-follow-up`, followUp)
    const result = await turnOutput(client, sessionId, agentId, marker, timeoutMs)
    evidence.turns.push({ label, prompt, ...result })
    console.log(`${label}: ${result.lifecycle} ${JSON.stringify(result.text.slice(0, 300))}`)
    if (followUpMarker) {
      const next = await turnOutput(client, sessionId, agentId, followUpMarker, timeoutMs)
      evidence.turns.push({ label: `${label}-follow-up`, prompt: followUp, ...next })
      console.log(`${label}-follow-up: ${next.lifecycle} ${JSON.stringify(next.text.slice(0, 300))}`)
    }
    return result
  }

  await ask("fact-1", `Remember this for later in our conversation: the project codename is ${facts.codename}. Do not use tools. Reply with just OK.`)
  if (richProbes) {
    const release = await ask("release", "Do not use tools. Invent a release name made of two lowercase English words joined by a hyphen, and reply with just that name.")
    facts.release_name = release.text.trim().toLowerCase().match(/[a-z]+-[a-z]+/)?.[0] ?? "missing-release"
    await ask("decision-old", "We will use SQLite for the cache. Do not use tools. Reply with just OK.")
  }
  const bulk = async (from, to) => {
    for (let index = from; index < to; index++) {
      await ask(`bulk-${index}`, `Reference notes, part ${index}:\n${notes(index)}\n\nWithout using tools, summarize these notes in about 80 words.`)
    }
  }
  await bulk(0, Math.floor(bulkTurns / 2))
  await ask("fact-2", `Also remember: the staging deploy port is ${facts.port}. Do not use tools. Reply with just OK.`)
  await bulk(Math.floor(bulkTurns / 2), bulkTurns)
  await ask("fact-3", `And my favourite fruit is the ${facts.fruit}. Do not use tools. Reply with just OK.`)

  for (let index = 0; index < fillerTurns; index++) {
    await ask(`filler-${index}`, `Without using tools, write one paragraph of about 120 words about ${TOPICS[index % TOPICS.length]}.`)
  }
  if (richProbes) {
    await ask("decision-new", `Decision changed: we will use ${facts.cache_database} for the cache instead of SQLite. Our current task is ${facts.current_task}, and the next step is ${facts.next_step}. Do not use tools. Reply with just OK.`)
    fileProbeAttempted = true
    const file = await ask("file", `In your current workspace, use one shell command to create the relative file ${facts.created_file} containing the line ${facts.codename}, then read that file back with cat. Reply with just the line read from the file.`)
    assertToolProbe(file, facts.codename, "file creation/readback")
    evidence.file_probe = { filename: facts.created_file, workspace_relative: true, verified: true }
    const [a, b] = [randomInt(1000, 9999), randomInt(1000, 9999)]
    facts.python_output = String(a * b)
    await ask("tool", `Run the shell command python3 -c "print(${a}*${b})" and reply with just its output.`)
  }

  const hook = (name) => {
    const cmd = option(name)
    if (!cmd) return
    execSync(cmd, { stdio: "inherit" })
    evidence[name] = cmd
  }
  hook("before-switch-cmd")
  const switched = variant(await client.send(updateAgentProfileRequest({
    sessionId, agentId, provider: to.provider, model: to.model, effort: to.effort, accountProfile: to.accountProfile,
  })), "AgentProfileUpdated").agent
  evidence.switched_agent = agentSnapshot(switched)
  assertContinuedPlacement(evidence.source_agent, evidence.switched_agent, placement)
  hook("after-switch-cmd")

  const files = []
  if (recallAttachmentBytes > 0) {
    scratch = await mkdtemp(path.join(tmpdir(), "ctxswitch-drill-"))
    const notes = path.join(scratch, "notes.txt")
    const line = "Reference notes unrelated to the question; ignore them.\n"
    await writeFile(notes, line.repeat(Math.ceil(recallAttachmentBytes / line.length)).slice(0, recallAttachmentBytes), "utf8")
    files.push({ url: `file://${notes}`, mime: "text/plain", filename: "notes.txt" })
    evidence.recall_attachment_bytes = recallAttachmentBytes
  }
  const questions = {
    codename: "the project codename", port: "the staging deploy port", fruit: "my favourite fruit",
    release_name: "the release name you invented", cache_database: "the database we use for the cache now",
    current_task: "our current task", next_step: "the next step", created_file: "the name of the file you created (name only)",
    python_output: "the output of the last python3 command you ran", colour: "my favourite colour",
  }
  const tools = process.argv.includes("--allow-recall")
    ? "You may use the chariox.search_recall tool, but no other tool"
    : "Without using any tools"
  const probe = async (label, files = [], followUp = null) => {
    const keys = Object.keys(facts)
    const recall = await ask(label, `${tools}, answer from our conversation so far: ${keys.map(key => questions[key]).join("; ")}. Reply on one line exactly as ${keys.map(key => `${key}=<value>`).join(" ")}, writing UNKNOWN for anything you were not told.`, files, followUp)
    return scoreFacts(recall.text, facts)
  }
  evidence.recalled = await probe("recall", files,
    process.argv.includes("--queued-follow-up") ? "Without using tools, reply with just the word DONE." : null)
  // A round trip switches back to the first profile after one more turn there.
  if (process.argv.includes("--round-trip")) {
    facts.colour = `${pick()}-blue`
    await ask("fact-4", `One more thing to remember: my favourite colour is ${facts.colour}. Do not use tools. Reply with just OK.`)
    const returned = variant(await client.send(updateAgentProfileRequest({
      sessionId, agentId, provider: from.provider, model: from.model, effort: from.effort, accountProfile: from.accountProfile,
    })), "AgentProfileUpdated").agent
    evidence.returned_agent = agentSnapshot(returned)
    assertContinuedPlacement(evidence.source_agent, evidence.returned_agent, placement)
    evidence.recalled_after_return = await probe("recall-return")
  }
  evidence.scores = { after_switch: scoreSummary(evidence.recalled), after_return: evidence.recalled_after_return && scoreSummary(evidence.recalled_after_return) }
  evidence.passed = [evidence.recalled, evidence.recalled_after_return ?? {}].every(recalled => Object.values(recalled).every(Boolean))
  assert.ok(evidence.passed, `facts lost after switch: ${JSON.stringify([evidence.recalled, evidence.recalled_after_return])}`)
} catch (error) {
  evidence.passed = false
  evidence.failure = error instanceof Error ? error.message : String(error)
  process.exitCode = 1
} finally {
  evidence.session_id = sessionId
  // Use the execution agent's filesystem before ending its home-owned session.
  if (fileProbeAttempted && ask) {
    try {
      const removed = await ask("file-cleanup", `In your current workspace, use one shell command to remove only the relative file ${facts.created_file}, verify that it no longer exists, and print CTXSWITCH_FILE_REMOVED. Reply with just that marker.`)
      assertToolProbe(removed, "CTXSWITCH_FILE_REMOVED", "file cleanup")
      evidence.file_cleanup = { filename: facts.created_file, verified: true }
    } catch (error) {
      evidence.file_cleanup = { filename: facts.created_file, verified: false, failure: error.message }
      evidence.passed = false
      evidence.failure ??= `file cleanup failed: ${error.message}`
      process.exitCode = 1
    }
  }
  if (sessionId && !process.argv.includes("--keep-session")) {
    await client.send(deleteSessionRequest(sessionId, workspace)).catch(() => {})
  }
  await client.close().catch(() => {})
  if (scratch) await rm(scratch, { recursive: true, force: true })
  if (facts.created_file && !placement.kernelRef && !placement.sliceRef) await rm(path.join(workspace, facts.created_file), { force: true })
}

await mkdir(evidenceRoot, { recursive: true })
const evidencePath = path.join(evidenceRoot, evidenceName(from, to, Date.now()))
await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, "utf8")
console.log(`model switch drill ${evidence.passed ? "passed" : "failed"}: ${JSON.stringify(evidence.recalled ?? evidence.failure)}; evidence: ${evidencePath}`)
