#!/usr/bin/env node
// Focused drill for App sets (protocol 361). Against a live kernel with at
// least one installed App:
//   node scripts/live-app-set-drill.mjs [--kernel-url ws://127.0.0.1:44240/kernel] [--evidence FILE]
// It reads the App set and checks that each entry matches what the owner's
// own requests show for that installation, and that no App data is present.
import assert from "node:assert/strict"
import { writeFile } from "node:fs/promises"

import { LocalIpcClient } from "../dist/ipc.js"

const args = process.argv.slice(2)
const option = (name) => { const index = args.indexOf(`--${name}`); return index >= 0 ? args[index + 1] : undefined }
const client = new LocalIpcClient(option("kernel-url") ?? "ws://127.0.0.1:44240/kernel")
try {
  const set = (await client.send({ GetAppSet: {} })).AppSet
  assert.equal(set?.schema, "chariox.app-set.v1")
  assert.ok(set.installations.length > 0, "the drill needs an installed App")
  const checked = []
  for (const entry of set.installations) {
    const app = { installation_id: entry.installation_id }
    const installation = (await client.send({ GetAppInstallation: app })).AppInstallation.installation
    assert.equal(entry.release.package_digest, installation.active_release.package_digest)
    const routes = (await client.send({ ListAppInboxRoutes: app })).AppInboxRoutes.routes
    const automations = (await client.send({ ListAppAutomations: app })).AppAutomations.automations
    const connections = (await client.send({ ListAppConnections: app })).AppConnections.connections
    assert.deepEqual(entry.inbox_routes.map((row) => row.route_id).sort(), routes.map((row) => row.route_id).sort())
    assert.deepEqual(entry.automations.map((row) => row.automation_id).sort(), automations.map((row) => row.automation_id).sort())
    assert.deepEqual(entry.connections.map((row) => row.connection_id).sort(), connections.map((row) => row.connection_id).sort())
    assert.equal(typeof entry.capabilities, "object")
    assert.deepEqual(Object.keys(entry).sort(),
      ["app_id", "automations", "capabilities", "connections", "inbox_routes", "installation_id", "release"])
    checked.push({ installation: entry.installation_id, version: entry.release.version,
      automations: automations.length, routes: routes.length, connections: connections.length })
  }
  const evidence = { checked_at: new Date().toISOString(), schema: set.schema, installations: checked }
  if (option("evidence")) await writeFile(option("evidence"), `${JSON.stringify(evidence, null, 2)}\n`)
  console.log(JSON.stringify(evidence))
  console.log("APP_SET_DRILL_PASS")
} finally {
  client.close?.()
}
