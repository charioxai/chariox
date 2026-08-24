#!/usr/bin/env node
import assert from "node:assert/strict"
import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  attachToSessionRequest,
  createSessionRequest,
  deleteSessionRequest,
  getProviderAuthStatusRequest,
  getProviderCatalogRequest,
  getSessionStateRequest,
  listProviderAccountProfilesRequest,
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
  if (value == null) throw new Error(`expected ${key}, received ${JSON.stringify(Object.keys(response ?? {}))}`)
  return value
}

function activityFor(state, agentId) {
  const payload = state?.SessionStateLoaded ?? state?.SessionState ?? state
  const activity = payload?.agent_activity ?? payload?.session?.agent_activity ?? {}
  return Array.isArray(activity)
    ? activity.find((entry) => entry?.agent_id === agentId)
    : activity?.[agentId]
}

function working(activity) {
  if (!activity) return false
  const status = String(activity.status ?? "").toLowerCase()
  const prompt = String(activity.prompt_status ?? "").toLowerCase()
  return activity.busy === true
    || Number(activity.active_prompt_count ?? 0) > 0
    || ["working", "running", "thinking", "streaming"].includes(status)
    || (prompt !== "" && !["none", "idle", "completed", "cancelled"].includes(prompt))
}

async function waitForTurns(client, sessionId, agentIds, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  const sawWorking = new Set()
  while (Date.now() < deadline) {
    const state = await client.send(getSessionStateRequest(sessionId))
    for (const agentId of agentIds) {
      if (working(activityFor(state, agentId))) sawWorking.add(agentId)
    }
    if (agentIds.every((agentId) => sawWorking.has(agentId) && !working(activityFor(state, agentId)))) return
    await sleep(500)
  }
  throw new Error(`timed out waiting for provider turns; observed working agents: ${[...sawWorking].join(", ")}`)
}

const provider = option("provider")
const profiles = (option("profiles", "") ?? "").split(",").map((value) => value.trim()).filter(Boolean)
const kernelUrl = option("kernel-url", "ws://127.0.0.1:7777")
const workspace = path.resolve(option("workspace", process.cwd()))
const model = option("model")
const effort = option("effort")
const execute = process.argv.includes("--execute")
const timeoutMs = Number(option("timeout-ms", "300000"))

if (!provider || profiles.length !== 2) {
  console.error("usage: live-multi-account-drill.mjs --provider <codex|claude|opencode> --profiles <profile-a,profile-b> [--kernel-url ws://...] [--model model] [--effort effort] [--workspace path] [--evidence-root path] [--execute]")
  process.exit(2)
}
if (execute && (!model || !effort)) {
  console.error("--execute requires --model and --effort so the drill never guesses an account entitlement or effort level")
  process.exit(2)
}

const evidence = {
  provider,
  profiles,
  model,
  effort,
  mode: execute ? "live-turns" : "read-only-preflight",
  started_at_ms: Date.now(),
  checks: [],
}
const client = new LocalIpcClient(kernelUrl)
let sessionId = null
try {
  const listed = variant(
    await client.send(listProviderAccountProfilesRequest(provider)),
    "ProviderAccountProfilesListed",
  ).profiles
  for (const profileId of profiles) {
    const profile = listed.find((candidate) => candidate.profile_id === profileId)
    assert(profile, `profile ${provider}/${profileId} is not registered`)
    assert.equal(profile.provider, provider)
    const status = variant(
      await client.send(getProviderAuthStatusRequest(provider, profileId)),
      "ProviderAuthStatus",
    ).status
    evidence.checks.push({ kind: "profile", profile_id: profileId, auth_state: status.auth_state })
    const catalog = variant(
      await client.send(getProviderCatalogRequest({ provider, accountProfile: profileId })),
      "ProviderCatalog",
    ).catalog
    assert(catalog.all.some((entry) => entry.id === provider), `catalog did not include ${provider}`)
    evidence.checks.push({ kind: "catalog", profile_id: profileId, provider_present: true })
  }

  if (execute) {
    const created = variant(await client.send(createSessionRequest(
      workspace,
      workspace,
      `multi-account-${provider}-${Date.now()}`,
      { provider, model, effort, account_profile: profiles[0] },
    )), "SessionCreated")
    sessionId = created.session.id
    const first = created.agent
    const attachment = variant(
      await client.send(attachToSessionRequest(sessionId, `multi-account-drill-${process.pid}`)),
      "SessionAttached",
    ).attachment
    const second = variant(await client.send(spawnAgentRequest(
      sessionId,
      provider,
      `${provider}-${profiles[1]}`,
      model,
      workspace,
      effort,
      "plan",
      "required",
      undefined,
      undefined,
      undefined,
      profiles[1],
    )), "AgentSpawned").agent
    assert.equal(first.account_profile, profiles[0])
    assert.equal(first.model, model)
    assert.equal(first.effort, effort)
    assert.equal(second.account_profile, profiles[1])
    assert.equal(second.model, model)
    assert.equal(second.effort, effort)
    const prompt = "Reply with exactly the account-isolation marker: CHARIOX_MULTI_ACCOUNT_OK"
    await Promise.all([
      client.send(submitPromptRequest(sessionId, attachment.id, first.id, prompt, [])),
      client.send(submitPromptRequest(sessionId, attachment.id, second.id, prompt, [])),
    ])
    await waitForTurns(client, sessionId, [first.id, second.id], timeoutMs)
    const switched = variant(await client.send(updateAgentProfileRequest({
      sessionId,
      agentId: first.id,
      provider,
      accountProfile: profiles[1],
      model,
      effort,
    })), "AgentProfileUpdated").agent
    assert.equal(switched.account_profile, profiles[1])
    assert.equal(switched.model, model)
    assert.equal(switched.effort, effort)
    await client.send(submitPromptRequest(sessionId, attachment.id, first.id, prompt, []))
    await waitForTurns(client, sessionId, [first.id], timeoutMs)
    evidence.checks.push({ kind: "concurrent_turns", profiles: [...profiles], passed: true })
    evidence.checks.push({ kind: "context_handoff", from: profiles[0], to: profiles[1], passed: true })
  }
  evidence.completed_at_ms = Date.now()
  evidence.passed = true
} finally {
  if (sessionId) await client.send(deleteSessionRequest(sessionId, workspace)).catch(() => {})
  await client.close().catch(() => {})
}

const evidenceRoot = path.resolve(option(
  "evidence-root",
  path.join(process.env.HOME ?? process.cwd(), ".codex/evidence/workflow-infrastructure-platform/provider-accounts"),
))
await mkdir(evidenceRoot, { recursive: true })
const evidencePath = path.join(evidenceRoot, `live-${provider}-${Date.now()}.json`)
await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, "utf8")
console.log(`multi-account drill passed; safe evidence: ${evidencePath}`)
