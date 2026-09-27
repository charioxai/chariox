#!/usr/bin/env node
// Focused drill for provider resource filters (protocol 362). Against a live
// kernel with an event connection whose resources share a connection scope
// (for example Slack channels in one workspace):
//   node scripts/live-resource-filter-drill.mjs --connection ID \
//     [--kernel-url ws://127.0.0.1:44240/kernel] [--evidence FILE]
// Resources that share a scope must each carry a filter that tells them apart.
import assert from "node:assert/strict"
import { writeFile } from "node:fs/promises"

import { LocalIpcClient } from "../dist/ipc.js"

const options = parseArgs(process.argv.slice(2))
const client = new LocalIpcClient(options.kernelUrl)
try {
  const page = (await client.send({ ListEventConnectionResources: {
    connection_id: options.connection, query: null, cursor: null, limit: 100,
  } })).EventConnectionResourcesPage.page
  const byScope = Map.groupBy(page.resources, (resource) => resource.connection_scope)
  const shared = [...byScope.values()].filter((resources) => resources.length > 1).flat()
    .filter((resource) => resource.filter)
  assert.ok(shared.length > 1, "at least two resources share a scope and carry filters")
  const filters = new Set(shared.map((resource) => JSON.stringify(resource.filter)))
  assert.equal(filters.size, shared.length, "resources that share a scope have distinct filters")
  const evidence = page.resources.map(({ name, kind, connection_scope, filter }) => ({ name, kind, connection_scope, filter }))
  if (options.evidence) await writeFile(options.evidence, `${JSON.stringify(evidence, null, 2)}\n`)
  console.log(JSON.stringify(evidence, null, 2))
  console.log("RESOURCE_FILTER_DRILL_PASS")
} finally {
  client.close?.()
}

function parseArgs(argv) {
  const values = {}
  for (let index = 0; index < argv.length; index += 2) {
    assert.match(argv[index] ?? "", /^--/, `unexpected argument ${argv[index]}`)
    values[argv[index].slice(2)] = argv[index + 1]
  }
  assert.ok(values.connection, "--connection is required")
  return {
    connection: values.connection,
    kernelUrl: values["kernel-url"] ?? "ws://127.0.0.1:44240/kernel",
    evidence: values.evidence,
  }
}
