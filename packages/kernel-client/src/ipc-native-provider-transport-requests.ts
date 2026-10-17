import type { NativeProviderTransportOperation } from "./kernel-types-provider.js"

// MP-08 / MP-10 / MP-11: coordinator-allocated native display transport.
export const nativeProviderTransportMinimumProtocolVersion = 531
export function nativeProviderTransportRequest(options: {
  readonly sessionId: string
  readonly attachmentId: string
  readonly agentId: string
  readonly providerRunId: string
  readonly operation: NativeProviderTransportOperation
}) {
  return { NativeProviderTransport: {
    session_id: options.sessionId, attachment_id: options.attachmentId,
    agent_id: options.agentId, provider_run_id: options.providerRunId,
    operation: options.operation,
  } }
}
