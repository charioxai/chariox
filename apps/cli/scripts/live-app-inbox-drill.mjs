#!/usr/bin/env node
// Focused drill for the App inbox (protocol 353). Against a live kernel with
// an installed App that declares an incoming event and handles it:
//   node scripts/live-app-inbox-drill.mjs --installation ID --event NAME \
//     --payload JSON [--conflict-payload JSON] [--kernel-url ws://127.0.0.1:44240/kernel] [--evidence FILE]
// It creates a route, sends a test occurrence, checks duplicate and conflict
// answers, waits for delivery, and removes the route.
import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { writeFile } from "node:fs/promises"

import { LocalIpcClient } from "../dist/ipc.js"

const options = parseArgs(process.argv.slice(2))
const client = new LocalIpcClient(options.kernelUrl)
const evidence = { started_at: new Date().toISOString(), steps: [] }
const route = `drill-${randomUUID().slice(0, 8)}`
const occurrence = `occ-${randomUUID().slice(0, 8)}`
const installation = options.installation
// A failed run still removes the route it made.
let created = false

try {
  const answer = await client.send({ CreateAppInboxRoute: {
    installation_id: installation, route_id: route, event_name: options.event,
    source_event_type: "drill.event", source_event_version: 1,
  } })
  assert.ok(answer.AppInboxRoutes?.routes.some((row) => row.route_id === route), JSON.stringify(answer))
  created = true
  evidence.steps.push({ step: "create", route })

  const send = (payload) => client.send({ TestAppInboxRoute: {
    installation_id: installation, route_id: route, occurrence_id: occurrence, payload,
  } })
  const first = (await send(options.payload)).AppInboxOccurrenceAccepted
  assert.equal(first?.duplicate, false, "first occurrence must be new")
  const again = (await send(options.payload)).AppInboxOccurrenceAccepted
  assert.equal(again?.duplicate, true, "the same occurrence must be a duplicate")
  const conflict = await send(options.conflictPayload).catch((error) => ({ error: String(error) }))
  assert.equal(conflict.AppRequestFailed?.code ?? null, "conflict", `changed payload answered ${JSON.stringify(conflict)}`)
  evidence.steps.push({ step: "dedupe", first, again, conflict })

  let row
  for (let attempt = 0; attempt < 120; attempt += 1) {
    const listed = await client.send({ ListAppInboxRoutes: { installation_id: installation } })
    row = listed.AppInboxRoutes?.routes.find((candidate) => candidate.route_id === route)
    if (row && row.pending === 0) break
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  assert.deepEqual([row?.pending, row?.delivered], [0, 1], `delivery did not settle: ${JSON.stringify(row)}`)
  evidence.steps.push({ step: "delivered", row })

  const removed = await client.send({ RemoveAppInboxRoute: { installation_id: installation, route_id: route } })
  assert.ok(!removed.AppInboxRoutes?.routes.some((candidate) => candidate.route_id === route))
  created = false
  evidence.steps.push({ step: "remove" })
  evidence.result = "passed"
  console.log(`App inbox drill passed: route ${route}, occurrence ${occurrence} delivered once`)
} catch (error) {
  evidence.result = "failed"
  evidence.error = String(error?.stack ?? error)
  process.exitCode = 1
  console.error(error)
} finally {
  if (created) {
    await client.send({ RemoveAppInboxRoute: { installation_id: installation, route_id: route } }).catch(() => {})
  }
  evidence.finished_at = new Date().toISOString()
  if (options.evidence) await writeFile(options.evidence, `${JSON.stringify(evidence, null, 2)}\n`)
  client.close?.()
}

function parseArgs(argv) {
  const values = {}
  for (let index = 0; index < argv.length; index += 2) {
    assert.match(argv[index] ?? "", /^--/, `unexpected argument ${argv[index]}`)
    values[argv[index].slice(2)] = argv[index + 1]
  }
  for (const name of ["installation", "event", "payload"]) assert.ok(values[name], `--${name} is required`)
  const payload = JSON.parse(values.payload)
  return {
    installation: values.installation,
    event: values.event,
    payload,
    conflictPayload: values["conflict-payload"] ? JSON.parse(values["conflict-payload"]) : { ...payload, drill_changed: true },
    kernelUrl: values["kernel-url"] ?? "ws://127.0.0.1:44240/kernel",
    evidence: values.evidence,
  }
}
