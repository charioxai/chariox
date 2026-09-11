import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"

const root = new URL("../../../", import.meta.url)
const read = (path) => readFile(new URL(path, root), "utf8")

const [localTypes, relayPeer, workflowTool, formatter, clientRequests, shellCommand] = await Promise.all([
  read("apps/kernel/src/local/api/types.rs"),
  read("apps/kernel/src/transport/relay_peer.rs"),
  read("apps/kernel/src/runtime/state/workflow_tool.rs"),
  read("apps/kernel/src/runtime/state/workflow_event_reply_attribution.rs"),
  read("packages/kernel-client/src/ipc-event-publication-requests.ts"),
  read("packages/kernel-client/src/shell-workflow-event-publication-command.ts"),
])

assert.match(localTypes, /LOCAL_DAEMON_PROTOCOL_VERSION: u32 = 324/)
assert.match(relayPeer, /RELAY_PEER_PROTOCOL_VERSION: u32 = 51/)
assert.equal(
  (workflowTool.match(/format_event_reply\(/g) ?? []).length,
  2,
  "reply_to_event and notification.reply must share the kernel formatter",
)
assert.match(formatter, /PROVIDER_RUN_ATTRIBUTION_MARKER/)
assert.match(formatter, /contains\(PROVIDER_RUN_ATTRIBUTION_MARKER\)/)
assert.match(formatter, /authoritative provider run attribution could not be resolved/)
assert.match(clientRequests, /UpdateWorkflowEventBinding/)
assert.match(shellCommand, /--provider-run-attribution/)

console.log("reviewer runtime-attribution protocol drill passed")
