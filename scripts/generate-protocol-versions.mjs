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
// MP-08/MP-10/MP-11: keep the historical workflow shape but follow the pair.
const historyPath = "fixtures/historical-workflow-protocol.json"
const history = JSON.parse(readFileSync(new URL(historyPath, repositoryRoot), "utf8"))
update(historyPath, JSON.stringify({ ...history, local: localProtocolVersion, relayPeer: relayPeerProtocolVersion }, null, 2) + "\n")
const processPath = "apps/kernel/src/local/api/tests/protocol_shapes/session-processes-954.json"
const processes = JSON.parse(readFileSync(new URL(processPath, repositoryRoot), "utf8"))
update(processPath, JSON.stringify({ ...processes, protocol_version: localProtocolVersion, relay_peer_protocol_version: relayPeerProtocolVersion }, null, 2) + "\n")
const clientPath = "packages/kernel-client/src/kernel-types.ts"
update(clientPath, readFileSync(new URL(clientPath, repositoryRoot), "utf8").replace(/export const LOCAL_DAEMON_PROTOCOL_VERSION = \d+/, `export const LOCAL_DAEMON_PROTOCOL_VERSION = ${localProtocolVersion}`))
const dockerPath = "docker/publication/Dockerfile"
update(dockerPath, readFileSync(new URL(dockerPath, repositoryRoot), "utf8").replace(/ARG CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION=\d+/g, `ARG CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION=${localProtocolVersion}`))
