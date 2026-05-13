import type { KernelTransportControlRequest } from "./kernel-transport-requests.js"

export type KernelSubscriptionScope = "session" | "waiting_room_inventory"

export type KernelSubscriptionState = {
  sessionId: string
  attachmentId: string
  scope: KernelSubscriptionScope
  relaySubscriptionId: string | null
  relayPrivateKey: Buffer | null
}

export type KernelSubscriptionStart = {
  readonly subscription: KernelSubscriptionState
  readonly resumeFromEventId: number | null
  readonly resetLastReceivedEventId: boolean
}

const WAITING_ROOM_INVENTORY_SUBSCRIPTION_ID = "__waiting_room_inventory__"

export function createKernelSessionSubscriptionStart(input: {
  readonly previous: KernelSubscriptionState | null
  readonly lastReceivedEventId: number | null
  readonly sessionId: string
  readonly attachmentId: string
  readonly relaySubscriptionId: string | null
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
      relayPrivateKey: null,
    },
    resumeFromEventId,
    resetLastReceivedEventId: resumeFromEventId == null,
  }
}

export function createWaitingRoomInventorySubscriptionStart(input: {
  readonly previous: KernelSubscriptionState | null
  readonly lastReceivedEventId: number | null
  readonly relaySubscriptionId: string | null
}): KernelSubscriptionStart {
  const resumeFromEventId =
    input.previous?.scope === "waiting_room_inventory" ? input.lastReceivedEventId : null
  return {
    subscription: {
      sessionId: WAITING_ROOM_INVENTORY_SUBSCRIPTION_ID,
      attachmentId: WAITING_ROOM_INVENTORY_SUBSCRIPTION_ID,
      scope: "waiting_room_inventory",
      relaySubscriptionId: input.relaySubscriptionId,
      relayPrivateKey: null,
    },
    resumeFromEventId,
    resetLastReceivedEventId: resumeFromEventId == null,
  }
}

export function kernelSubscriptionScopeValue(subscription: KernelSubscriptionState): string | undefined {
  return subscription.scope === "waiting_room_inventory" ? "waiting_room_inventory" : undefined
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
  const subscriptionScope = kernelSubscriptionScopeValue(subscription)
  if (subscriptionScope) {
    transportRequest.subscription_scope = subscriptionScope
  }
  return {
    __kernel_transport: transportRequest,
  }
}
