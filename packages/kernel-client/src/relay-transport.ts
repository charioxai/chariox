import type {
  RelayConnectFrame,
  RelayRequestFrame,
  RelayTarget,
} from "./kernel-transport-frames.js"
import { encryptRelayPayload } from "./relay-crypto.js"

export function buildRelayConnectFrame(authToken: string | null, target: RelayTarget | null): RelayConnectFrame {
  if (!authToken) {
    throw new Error("relay auth token is required")
  }
  if (!target?.daemon_id && !target?.daemon_alias) {
    throw new Error("relay target daemon id or alias is required")
  }
  return {
    kind: "client_connect",
    auth_token: authToken,
    target: target ?? {},
  }
}

export function normalizeRelayRequest(
  requestId: string,
  request: unknown,
  target: RelayTarget | null,
  daemonPublicKey: string | null,
): { frame: RelayRequestFrame; privateKey: Buffer } {
  const resolvedTarget = requireRelayTarget(target)
  if (!daemonPublicKey) {
    throw new Error("relay daemon public key is required")
  }
  const plaintext = Buffer.from(JSON.stringify(request), "utf8")
  const { privateKey, payload } = encryptRelayPayload(daemonPublicKey, plaintext)
  return {
    frame: {
      kind: "client_request",
      request_id: requestId,
      target: resolvedTarget,
      encrypted_request: payload,
    },
    privateKey,
  }
}

export function requireRelayTarget(target: RelayTarget | null): RelayTarget {
  if (!target?.daemon_id && !target?.daemon_alias) {
    throw new Error("relay target daemon id or alias is required")
  }
  return target
}
