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
import assert from "node:assert/strict"
import { execSync } from "node:child_process"
import { randomInt } from "node:crypto"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import path from "node:path"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  attachToSessionRequest,
  createSessionRequest,
  deleteSessionRequest,
  getSessionHistoryBlobContentRequest,
  getSessionHistoryOutlineRequest,
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
      for (const blob of turn.blobs.filter(blob => ["provider_output", "provider_error"].includes(blob.kind))) {
        const content = variant(await client.send(getSessionHistoryBlobContentRequest(sessionId, agentId, blob.blob_id)), "SessionHistoryBlobContent")
        turn.entries.push(...content.entries)
      }
      const entries = [...turn.entries, ...(turn.summary ? [turn.summary] : [])].map(page => page.entry)
      const errors = entries.filter(entry => entry.kind === "provider_error").map(entry => entry.text)
      const text = entries.filter(entry => entry.kind === "provider_output").map(entry => entry.text).join("")
      const runIds = [...new Set(entries.map(entry => entry.provider_run_id).filter(Boolean))]
      return { lifecycle: turn.lifecycle, text, errors, runIds }
    }
    await sleep(500)
  }
  throw new Error(`timed out waiting for turn ${marker}`)
}

const kernelUrl = option("kernel-url")
const workspace = path.resolve(option("workspace", process.cwd()))
const from = profile(option("from", ""))
const to = profile(option("to", ""))
const timeoutMs = Number(option("timeout-ms", "300000"))
const fillerTurns = Number(option("filler-turns", "0"))
const recallAttachmentBytes = Number(option("recall-attachment-bytes", "0"))
const TOPICS = ["tide pools", "bread baking", "lighthouses", "glaciers", "chess openings", "bees", "volcanoes", "paper making", "kites", "river deltas", "telescopes", "salt marshes"]
const evidenceRoot = path.resolve(option("evidence-root", path.join(process.env.HOME ?? process.cwd(), ".codex/evidence/model-switch-context")))
if (!kernelUrl || !from.provider || !to.provider) {
  console.error("usage: live-model-switch-context-drill.mjs --kernel-url ws://... --from provider:model --to provider:model [--workspace dir]")
  process.exit(2)
}

const facts = { codename: `${pick()}-${pick()}`, port: String(20000 + randomInt(40000)), fruit: `${pick()}fruit` }
const evidence = { from, to, facts, started_at_ms: Date.now(), turns: [] }
const client = new LocalIpcClient(kernelUrl)
let sessionId = null
let scratch = null
try {
  const created = variant(await client.send(createSessionRequest(workspace, workspace, `ctxswitch-${Date.now()}`, {
    provider: from.provider, model: from.model, effort: from.effort, account_profile: from.accountProfile,
    execution_mode: "build", permission_level: "yolo",
  })), "SessionCreated")
  sessionId = created.session.id
  const agentId = created.agent.id
  // The drill does not subscribe to events, so the kernel may reap an idle
  // attachment between turns (or drop it on restart); attach for each prompt.
  const attach = async () => variant(await client.send(attachToSessionRequest(sessionId, `ctxswitch-drill-${process.pid}`)), "SessionAttached").attachment
  const submit = async (label, prompt, files = []) => {
    const marker = `[${label}-${Date.now()}]`
    const attachment = await attach()
    variant(await client.send(submitPromptRequest(sessionId, attachment.id, agentId, `${marker} ${prompt}`, files)), "PromptSubmitted")
    return marker
  }
  const ask = async (label, prompt, files = [], followUp = null) => {
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
  await ask("fact-2", `Also remember: the staging deploy port is ${facts.port}, and my favourite fruit is the ${facts.fruit}. Do not use tools. Reply with just OK.`)

  for (let index = 0; index < fillerTurns; index++) {
    await ask(`filler-${index}`, `Without using tools, write one paragraph of about 120 words about ${TOPICS[index % TOPICS.length]}.`)
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
  evidence.switched_agent = { provider: switched.provider, model: switched.model, effort: switched.effort, account_profile: switched.account_profile }
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
  const recall = await ask("recall", "Without using any tools, answer from our conversation so far: what is the project codename, the staging deploy port, and my favourite fruit? Reply on one line exactly as codename=<value> port=<value> fruit=<value>, writing UNKNOWN for anything you were not told.", files,
    process.argv.includes("--queued-follow-up") ? "Without using tools, reply with just the word DONE." : null)
  evidence.recalled = Object.fromEntries(Object.entries(facts).map(([key, value]) => [key, recall.text.includes(value)]))
  evidence.passed = Object.values(evidence.recalled).every(Boolean)
  assert.ok(evidence.passed, `facts lost after switch: ${JSON.stringify(evidence.recalled)}`)
} catch (error) {
  evidence.passed = false
  evidence.failure = error instanceof Error ? error.message : String(error)
  process.exitCode = 1
} finally {
  evidence.session_id = sessionId
  if (sessionId && !process.argv.includes("--keep-session")) {
    await client.send(deleteSessionRequest(sessionId, workspace)).catch(() => {})
  }
  await client.close().catch(() => {})
  if (scratch) await rm(scratch, { recursive: true, force: true })
}

await mkdir(evidenceRoot, { recursive: true })
const evidencePath = path.join(evidenceRoot, `${from.provider}-${from.model}-to-${to.provider}-${to.model}-${Date.now()}.json`)
await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, "utf8")
console.log(`model switch drill ${evidence.passed ? "passed" : "failed"}: ${JSON.stringify(evidence.recalled ?? evidence.failure)}; evidence: ${evidencePath}`)
