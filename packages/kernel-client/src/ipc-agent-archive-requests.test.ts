import assert from "node:assert/strict"
import test from "node:test"
import { archiveAgentRequest, listArchivedAgentsRequest, supportsReversibleAgentArchive } from "./ipc-agent-requests.js"

test("agent archive discovery and restoration use the shared protocol", () => {
  assert.deepEqual(listArchivedAgentsRequest("room"), { ListArchivedAgents: { session_id: "room" } })
  assert.deepEqual(archiveAgentRequest("room", "agent", false), { ArchiveAgent: { session_id: "room", agent_id: "agent", archived: false } })
})

test("agent archive UI requires the expanded cleanup protocol", () => {
  for (const version of [null, undefined, 592, 629, 635]) assert.equal(supportsReversibleAgentArchive(version), false)
  for (const version of [636, 637]) assert.equal(supportsReversibleAgentArchive(version), true)
})
