import type { KernelTransportControlRequest } from "./kernel-transport-requests.js"
import type { EncryptedRelayPayload } from "./kernel-transport-frames.js"

export type KernelSubscriptionScope = "session" | "room_list"

export type KernelSubscriptionState = {
  sessionId: string
  attachmentId: string
  scope: KernelSubscriptionScope
  relaySubscriptionId: string | null
  relayPublicKey: string | null
  relayDecryptEvent: ((payload: EncryptedRelayPayload) => string) | null
  /** Protocol 596: how often this subscription's `heartbeat` should arrive (at least 500 ms). */
  heartbeatIntervalMs?: number
}

export type KernelSubscriptionStart = {
  readonly subscription: KernelSubscriptionState
  readonly resumeFromEventId: number | null
  readonly resetLastReceivedEventId: boolean
}

const ROOM_LIST_SUBSCRIPTION_ID = "__room_list__"

export function createKernelSessionSubscriptionStart(input: {
  readonly previous: KernelSubscriptionState | null
  readonly lastReceivedEventId: number | null
  readonly sessionId: string
  readonly attachmentId: string
  readonly relaySubscriptionId: string | null
  readonly heartbeatIntervalMs?: number | undefined
}): KernelSubscriptionStart {
  const resumeFromEventId =
    input.previous?.scope === "session"
      && input.previous.sessionId === input.sessionId
      && input.previous.attachmentId === input.attachmentId
      ? input.lastReceivedEventId
      : null
  return {
    subscription: {
      sessionId: input.sessionId,
      attachmentId: input.attachmentId,
      scope: "session",
      relaySubscriptionId: input.relaySubscriptionId,
      relayPublicKey: null,
      relayDecryptEvent: null,
      ...heartbeatField(input.heartbeatIntervalMs),
    },
    resumeFromEventId,
    resetLastReceivedEventId: resumeFromEventId == null,
  }
}

export function createRoomListSubscriptionStart(input: {
  readonly previous: KernelSubscriptionState | null
  readonly lastReceivedEventId: number | null
  readonly relaySubscriptionId: string | null
  readonly heartbeatIntervalMs?: number | undefined
}): KernelSubscriptionStart {
  const resumeFromEventId =
    input.previous?.scope === "room_list" ? input.lastReceivedEventId : null
  return {
    subscription: {
      sessionId: ROOM_LIST_SUBSCRIPTION_ID,
      attachmentId: ROOM_LIST_SUBSCRIPTION_ID,
      scope: "room_list",
      relaySubscriptionId: input.relaySubscriptionId,
      relayPublicKey: null,
      relayDecryptEvent: null,
      ...heartbeatField(input.heartbeatIntervalMs),
    },
    resumeFromEventId,
    resetLastReceivedEventId: resumeFromEventId == null,
  }
}

function heartbeatField(heartbeatIntervalMs: number | undefined): { heartbeatIntervalMs?: number } {
  return heartbeatIntervalMs === undefined ? {} : { heartbeatIntervalMs }
}

export function kernelSubscriptionScopeValue(subscription: KernelSubscriptionState): string | undefined {
  return subscription.scope === "room_list" ? "room_list" : undefined
}

export function buildKernelSubscriptionTransportRequest(
  subscription: KernelSubscriptionState,
  resumeFromEventId: number | null,
): { readonly __kernel_transport: KernelTransportControlRequest } {
  const transportRequest: KernelTransportControlRequest = {
    type: "subscribe",
    session_id: subscription.sessionId,
    attachment_id: subscription.attachmentId,
    resume_from_event_id: resumeFromEventId,
  }
  if (subscription.heartbeatIntervalMs !== undefined) {
    transportRequest.heartbeat_interval_ms = subscription.heartbeatIntervalMs
  }
  const subscriptionScope = kernelSubscriptionScopeValue(subscription)
  if (subscriptionScope) {
    transportRequest.subscription_scope = subscriptionScope
  }
  return {
    __kernel_transport: transportRequest,
  }
}
