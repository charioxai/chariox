// MP-07 / MP-08 / MP-10 / MP-11: regenerate landing pins from the two Rust constants.
import { readFileSync, writeFileSync } from "node:fs"
import { repositoryRoot, localProtocolVersion, relayPeerProtocolVersion } from "./lib/protocol-versions.mjs"
const check = process.argv.includes("--check")
function update(path, content) {
  const url = new URL(path, repositoryRoot)
  if (check) {
    if (readFileSync(url, "utf8") !== content) throw new Error(`${path} is stale; run pnpm protocol:generate`)
  } else writeFileSync(url, content)
}
update("fixtures/protocol-versions.json", JSON.stringify({ local: localProtocolVersion, relayPeer: relayPeerProtocolVersion }, null, 2) + "\n")
// These snapshots compare their allocation with the current runtime pair.
// Their shape hashes remain explicit in the focused protocol tests.
const currentSnapshots = [
  "apps/kernel/src/local/api/tests/protocol_shapes/files-watch-1054.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/agent-browser-owner-action-993.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/app-dev-push-933.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/app-dev-register-933.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/app-view-inspection-933.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/browser-mirror2-completion-550.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/browser-mirror2-font-absent-567.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/browser-mirror2-font-failures-550.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/browser-mirror2-font-requirements-550.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/browser-mirror2-viewport-first-629.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/browser-mirror2-visited-links-629.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/browser-pointer-cursor-document-mode-920.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/browser-verification-992.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/client-app-view-999.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/kernel-browser-vault-fill-941.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/leased-computer-507.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/outside-events.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/owner-agent-cancellation-987.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/owner-managed-event-connections-1000.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/provider-tool-background-lifecycle.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/provider-tool-images-910.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/remote-agent-binding-worktree-1000.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/room-pin-588.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/session-feed-596.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/session-outline-tool-image-count-1000.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/session-processes-954.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/web-bot-auth-946.json",
  "apps/kernel/src/local/api/tests/protocol_shapes/workflow-gate-1002.json",
  "fixtures/aggregate-agent-model-protocol.json",
  "fixtures/async-questions-protocol.json",
  "fixtures/historical-workflow-protocol.json",
  "fixtures/landing8-protocol.json",
  "fixtures/owner-vault-protocol.json",
  "fixtures/totp-protocol.json"
]
const localKeys = new Set(["local", "local_protocol", "protocol_version"])
for (const path of currentSnapshots) {
  const text = readFileSync(new URL(path, repositoryRoot), "utf8")
  // Preserve formatting: some snapshot hashes bind the exact file bytes.
  let content = text.replace(/^(  "(?:local|local_protocol|protocol_version|peer|peer_protocol|relayPeer|relay_peer_protocol_version)"\s*:\s*)\d+/gm,
    (match, prefix) => prefix + (localKeys.has(prefix.match(/"([^"]+)"/)[1]) ? localProtocolVersion : relayPeerProtocolVersion))
  if (path.endsWith("remote-agent-binding-worktree-1000.json")) {
    content = content.replace(/("relay_peer_protocol_version"\s*:\s*)\d+/g, `$1${relayPeerProtocolVersion}`)
  }
  update(path, content)
}
const clientPath = "packages/kernel-client/src/kernel-types.ts"
update(clientPath, readFileSync(new URL(clientPath, repositoryRoot), "utf8").replace(/export const LOCAL_DAEMON_PROTOCOL_VERSION = \d+/, `export const LOCAL_DAEMON_PROTOCOL_VERSION = ${localProtocolVersion}`))
const dockerPath = "docker/publication/Dockerfile"
update(dockerPath, readFileSync(new URL(dockerPath, repositoryRoot), "utf8").replace(/ARG CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION=\d+/g, `ARG CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION=${localProtocolVersion}`))
