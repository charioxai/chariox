import type {
  RelayConnectFrame,
  RelayRequestFrame,
  RelaySubscribeFrame,
  RelayTarget,
  RelayUnsubscribeFrame,
} from "./kernel-transport-frames.js"
import { encryptRelayPayload, relayPublicKeyFromPrivateKey } from "./relay-crypto.js"

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
  const plaintext = Buffer.from(JSON.stringify({
    command_id: requestId,
    request,
  }), "utf8")
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

export function buildRelaySubscribeFrame(input: {
  readonly requestId: string
  readonly subscriptionId: string
  readonly target: RelayTarget | null
  readonly sessionId: string
  readonly attachmentId: string
  readonly clientPublicKey: string
  readonly resumeFromEventId: number | null
  readonly subscriptionScope?: string | undefined
}): RelaySubscribeFrame {
  const frame: RelaySubscribeFrame = {
    kind: "client_subscribe",
    request_id: input.requestId,
    subscription_id: input.subscriptionId,
    target: requireRelayTarget(input.target),
    session_id: input.sessionId,
    attachment_id: input.attachmentId,
    client_public_key: input.clientPublicKey,
    resume_from_event_id: input.resumeFromEventId,
  }
  if (input.subscriptionScope) {
    frame.subscription_scope = input.subscriptionScope
  }
  return frame
}

export function buildRelayUnsubscribeFrame(
  requestId: string,
  subscriptionId: string,
  privateKey: Buffer,
): RelayUnsubscribeFrame {
  return {
    kind: "client_unsubscribe",
    request_id: requestId,
    subscription_id: subscriptionId,
    client_public_key: relayPublicKeyFromPrivateKey(privateKey),
  }
}

export function requireRelayTarget(target: RelayTarget | null): RelayTarget {
  if (!target?.daemon_id && !target?.daemon_alias) {
    throw new Error("relay target daemon id or alias is required")
  }
  return target
}
