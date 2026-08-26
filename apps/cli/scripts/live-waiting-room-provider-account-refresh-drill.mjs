#!/usr/bin/env node
import assert from "node:assert/strict"
import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  createProviderAccountProfileRequest,
  deleteProviderAccountProfileDataRequest,
  getWaitingRoomInventoryRequest,
  listProviderAccountProfilesRequest,
} from "../dist/ipc-requests.js"

function option(name, fallback) {
  const index = process.argv.indexOf(`--${name}`)
  return index >= 0 ? process.argv[index + 1] : fallback
}

function variant(response, key) {
  const value = response?.[key]
  if (value == null) throw new Error(`expected ${key}, received ${JSON.stringify(Object.keys(response ?? {}))}`)
  return value
}

async function waitForEvent(events, baseline, predicate, timeoutMs, description) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const event = events.slice(baseline).find(predicate)
    if (event) return event
    await sleep(25)
  }
  throw new Error(`timed out waiting for ${description}`)
}

const kernelUrl = option("kernel-url", "ws://127.0.0.1:7777")
const timeoutMs = Number(option("timeout-ms", "10000"))
const provider = "claude"
const label = `Waiting room refresh drill ${process.pid}-${Date.now()}`
const events = []
const client = new LocalIpcClient(kernelUrl)
const dispose = client.onKernelEvent((event) => events.push(event))
let profileId = null
let passed = false
let failure = null
let cleanupFailure = null

try {
  await client.subscribeToWaitingRoomInventory()
  await waitForEvent(
    events,
    0,
    (event) => event.event === "waiting_room_inventory_changed",
    timeoutMs,
    "initial waiting-room inventory refresh",
  )
  const baseline = events.length
  const created = variant(
    await client.send(createProviderAccountProfileRequest(provider, label)),
    "ProviderAccountProfile",
  ).profile
  profileId = created.profile_id

  const refreshEvent = await waitForEvent(
    events,
    baseline,
    (event) => event.event === "waiting_room_inventory_changed",
    timeoutMs,
    "provider-account waiting-room refresh",
  )
  const inventory = variant(
    await client.send(getWaitingRoomInventoryRequest()),
    "WaitingRoomInventory",
  ).snapshot
  assert(
    inventory.provider_accounts?.some((profile) => (
      profile.provider === provider && profile.profile_id === profileId && profile.label === label
    )),
    "waiting-room inventory did not include the newly created provider account",
  )
  assert.equal(typeof refreshEvent.inventory_version, "string")
  passed = true
} catch (error) {
  failure = error
} finally {
  dispose()
  await client.close().catch(() => {})

  const cleanupClient = new LocalIpcClient(kernelUrl)
  try {
    const listed = variant(
      await cleanupClient.send(listProviderAccountProfilesRequest(provider)),
      "ProviderAccountProfilesListed",
    ).profiles
    const drillProfiles = listed.filter((profile) => profile.label === label)
    for (const profile of drillProfiles) {
      await cleanupClient.send(
        deleteProviderAccountProfileDataRequest(provider, profile.profile_id, profile.profile_id),
      )
    }
    const remaining = variant(
      await cleanupClient.send(listProviderAccountProfilesRequest(provider)),
      "ProviderAccountProfilesListed",
    ).profiles
    assert.equal(remaining.some((profile) => profile.label === label), false)
  } catch (error) {
    cleanupFailure = error
  } finally {
    await cleanupClient.close().catch(() => {})
  }
}

const evidence = {
  schema: "chariox.waiting_room_provider_account_refresh_drill.v1",
  kernel_url: kernelUrl,
  provider,
  profile_id: profileId,
  observed_events: events.map((event) => event.event),
  passed,
  cleanup_passed: cleanupFailure == null,
  completed_at_ms: Date.now(),
}
const evidenceRoot = "/Users/miguel/.codex/evidence/waiting-room-provider-account-refresh"
await mkdir(evidenceRoot, { recursive: true })
const evidencePath = path.join(evidenceRoot, `live-${Date.now()}.json`)
await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, "utf8")

if (failure && cleanupFailure) {
  throw new AggregateError([failure, cleanupFailure], "drill and cleanup both failed")
}
if (failure) throw failure
if (cleanupFailure) throw cleanupFailure
console.log(`waiting-room provider-account refresh drill passed; safe evidence: ${evidencePath}`)
