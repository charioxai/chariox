import type { KernelEvent } from "./kernel-events.js"

export type IpcEnvelope<TResponse> = {
  response: TResponse | null
  error: string | null
}

export type KernelTransportResponseFrame<TResponse> = {
  type: "response"
  request_id: string
  response: TResponse | null
  error: KernelTransportError | null
}

export type KernelTransportEventFrame<TEvent = KernelEvent> = {
  type: "event"
  event_id: number
  event: TEvent
}

export type KernelTransportError = {
  code: string
  message: string
  retryable: boolean
}

export type RelayTarget = {
  daemon_id?: string | null
  daemon_alias?: string | null
}

export type RelayConnectFrame = {
  kind: "client_connect"
  auth_token: string
  target: RelayTarget
}

export type RelayConnectedFrame = {
  kind: "client_connected"
  target: RelayTarget
  daemon_public_key: string
}

export type EncryptedRelayPayload = {
  sender_public_key: string
  nonce: string
  ciphertext: string
}

export type RelayRequestFrame = {
  kind: "client_request"
  request_id: string
  target: RelayTarget
  encrypted_request: EncryptedRelayPayload
}

export type RelayResponseFrame<TResponse> = {
  kind: "client_response"
  request_id: string
  encrypted_response: EncryptedRelayPayload | null
  error: KernelTransportError | null
}

export type RelaySubscribeFrame = {
  kind: "client_subscribe"
  request_id: string
  subscription_id: string
  target: RelayTarget
  session_id: string
  attachment_id: string
  client_public_key: string
  subscription_scope?: string
  resume_from_event_id: number | null
}

export type RelayUnsubscribeFrame = {
  kind: "client_unsubscribe"
  request_id: string
  subscription_id: string
  client_public_key: string
}

export type RelayEventFrame = {
  kind: "client_event"
  subscription_id: string
  event_id: number
  encrypted_event: EncryptedRelayPayload
}

export type RelayCloseFrame = {
  kind: "close"
  reason: string
}

export type KernelSocketLane = "control" | "event"
