// MP-08 / MP-10 / MP-11: authoritative runtime versions for build/test consumers.
import { readFileSync } from "node:fs"
export const repositoryRoot = new URL("../../", import.meta.url)
const read = path => readFileSync(new URL(path, repositoryRoot), "utf8")
export const localProtocolVersion = Number(read("apps/kernel/src/local/api/types.rs").match(/pub const LOCAL_DAEMON_PROTOCOL_VERSION: u32 = (\d+);/)[1])
export const relayPeerProtocolVersion = Number(read("apps/kernel/src/transport/relay_peer.rs").match(/pub const RELAY_PEER_PROTOCOL_VERSION: u32 = (\d+);/)[1])
export const admittedPredecessorProtocols = [629, 636, 911, 923]
