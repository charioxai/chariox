import test from "node:test"
import assert from "node:assert/strict"
import { chooseNotificationSource } from "./notification-source-reference.js"
import type { WorkflowNotificationSourceSummary } from "./kernel-types.js"

const source = (id: string, name: string, kernel: string, available = true): WorkflowNotificationSourceSummary =>
  ({ source_id: id, kernel_id: kernel, session_id: "room", workflow_id: "w", exit_node_id: "n", exit_ordinal: 1, name, display_name: name, available })
const sources = [source("wf_a", "Release pipeline-1", "Studio"), source("wf_b", "Release pipeline-2", "Studio"), source("wf_c", "Deploy", "Laptop", false)]

test("a source name with spaces leaves the rest as the prompt", () => {
  assert.deepEqual(chooseNotificationSource(["Studio/Release", "pipeline-1", "summarize", "the", "failures"], sources), { source: { workflow: { source_id: "wf_a" } }, prompt: "summarize the failures" })
  assert.deepEqual(chooseNotificationSource(["Release", "pipeline-2"], sources), { source: { workflow: { source_id: "wf_b" } }, prompt: "" })
  assert.deepEqual(chooseNotificationSource(["wf_b", "go"], sources), { source: { workflow: { source_id: "wf_b" } }, prompt: "go" })
})
test("an App event names the installation and the event", () => {
  assert.deepEqual(chooseNotificationSource(["app:github/issue_opened", "triage"], sources), { source: { app: { installation_id: "github", event_name: "issue_opened" } }, prompt: "triage" })
  assert.ok("error" in chooseNotificationSource(["app:github"], sources))
})
test("unknown, ambiguous and offline sources are refused with what to do next", () => {
  assert.match((chooseNotificationSource(["Nope"], sources) as { error: string }).error, /no such source/)
  assert.match((chooseNotificationSource(["Deploy"], sources) as { error: string }).error, /offline/)
  const twice = [...sources, source("wf_d", "Release pipeline-1", "Laptop")]
  assert.match((chooseNotificationSource(["Release", "pipeline-1"], twice) as { error: string }).error, /more than one/)
  assert.deepEqual(chooseNotificationSource(["Laptop/Release", "pipeline-1"], twice), { source: { workflow: { source_id: "wf_d" } }, prompt: "" })
  assert.ok("error" in chooseNotificationSource([], sources))
})
