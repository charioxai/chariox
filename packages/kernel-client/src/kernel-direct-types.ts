// MP-08/MP-10/MP-11: transport ports shared by Web and native terminal clients.
export const relayMaxMessageBytes = 64 * 1024 * 1024
export type RelayTarget = { readonly daemon_id?: string | null; readonly daemon_alias?: string | null }
export type EncryptedRelayPayload = { readonly sender_public_key: string; readonly nonce: string; readonly ciphertext: string }
export type RelayCloseFrame = { readonly kind: "close"; readonly reason: string; readonly kernel_reload?: boolean }
export type SocketCloseDiagnostic = { readonly code?: number; readonly reason?: string; readonly wasClean?: boolean; readonly readyState?: number; readonly kernelReload?: boolean }
export type DirectSocketInput = {
  readonly lane: "control" | "event" | "inventory_event"
  readonly relayToken: string
  readonly target: RelayTarget
  readonly attempt: number
  readonly targetForLogs: RelayTarget
  readonly logTransport: (level: "debug" | "info" | "warn", event: string, fields: Record<string, unknown>) => void
  readonly onSettled: () => void
  readonly onConnected: (socket: WebSocket, key: string) => void
  readonly onActivity?: () => void
  readonly onMessage: (data: unknown, receivedAt?: number) => void
  readonly onClosed: (socket: WebSocket, message: string, diagnostic: SocketCloseDiagnostic) => void
}
export function parseFrame<T>(data: unknown): T | null {
  try { return JSON.parse(typeof data === "string" ? data : String(data)) as T } catch { return null }
}
