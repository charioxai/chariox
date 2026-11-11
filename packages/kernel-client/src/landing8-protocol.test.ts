// MP-08 / MP-09 / MP-10 / MP-11: a client at local 1003 reads the Landing 8 shapes from the fixture the
// kernel round-trips (`protocol_shapes/landing8.rs`).
import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"
import { agentTriggersForAgent } from "./agent-trigger-state.js"
import { hideAgentRequest } from "./ipc-agent-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION, type AgentTrigger, type PasskeyPrompt } from "./kernel-types.js"

type Landing8Fixture = {
  local: number
  hideAgent: unknown
  notificationTrigger: AgentTrigger
  appInstallPrompt: PasskeyPrompt
}

const fixture = async () =>
  JSON.parse(await readFile(new URL("../../../fixtures/landing8-protocol.json", import.meta.url), "utf8")) as Landing8Fixture

test("a client at 1003 sends HideAgent, reads a notification trigger's source_available and an App install prompt", async () => {
  const shapes = await fixture()
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, shapes.local)
  assert.deepEqual(hideAgentRequest("room-1", "agent-1", true), shapes.hideAgent)
  const [trigger] = agentTriggersForAgent({ agent_triggers: [shapes.notificationTrigger] }, "agent-1")
  assert.equal(trigger?.offline, true, "an unavailable notification source shows its trigger offline")
  assert.equal(trigger?.label, "Notification · Nightly build")
  const prompt = shapes.appInstallPrompt
  assert.equal(prompt.kind, "app_install")
  assert.equal(prompt.app_install?.operation, "update")
  assert.equal(prompt.app_install?.request_id, "install-1")
})
