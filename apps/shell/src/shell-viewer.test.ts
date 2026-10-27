import assert from "node:assert/strict"
import test from "node:test"
import { createShellViewer } from "./shell-viewer.js"
import { executeShellLine } from "@chariox/kernel-client/shell-script"
import { createDefaultShellContext } from "@chariox/kernel-client/shell-core"

const target = { sessionId: "room-1", agentId: "agent-2", sliceId: "slice-3" }

test("standalone shell resolves a scoped viewer URL without returning profile credentials", async () => {
  const requests: Record<string, unknown>[] = []
  const viewer = createShellViewer({ send: async (request) => {
    requests.push(request)
    return { CloudRelayStatus: { profile: { api_url: "https://cloud.test/api", cloud_session_token: "private-fixture" } } }
  } })
  assert.deepEqual(await viewer(target), { url: "https://cloud.test/view?view_target=room-1%3Aagent-2%3Aslice-3", opened: false })
  assert.deepEqual(requests, [{ CloudRelayStatus: null }])
})

test("standalone shell reports an unlinked kernel without inventing a Cloud URL", async () => {
  const viewer = createShellViewer({ send: async () => ({ CloudRelayStatus: { profile: null } }) })
  assert.equal(await viewer(target), null)
})

test("standalone shell does not conceal malformed status responses", async () => {
  const viewer = createShellViewer({ send: async () => ({ Unexpected: {} }) })
  await assert.rejects(viewer(target), /CloudRelayStatus/)
})

test("shell slice screen reaches the kernel profile through the shared command path", async () => {
  const client = { send: async (request: Record<string, unknown>) => {
    if ("GetSlice" in request) return { Slice: { slice: { id: "slice-3", display_endpoint: { kind: "selkies" } } } }
    if ("GetRoomEnvironmentSlice" in request) return { RoomEnvironmentSlice: { binding: { session_id: "room-1", slice_id: "slice-3" } } }
    if ("CloudRelayStatus" in request) return { CloudRelayStatus: { profile: { api_url: "https://cloud.test" } } }
    throw new Error("unexpected request")
  } }
  const output: string[] = []
  const result = await executeShellLine("slice screen slice-3", createDefaultShellContext({
    sessionId: "room-1", agentId: "agent-2", attachmentId: "attachment-1",
  }), { client, openRoomViewer: createShellViewer(client) }, line => output.push(line))
  assert.equal(result.ok, true)
  assert.match(output.join(""), /https:\/\/cloud.test\/view\?view_target=room-1%3Aagent-2%3Aslice-3/)
})
