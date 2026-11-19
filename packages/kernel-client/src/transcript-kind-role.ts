export type ProviderTranscriptKind =
  | "provider_output"
  | "provider_reasoning"
  | "provider_tool"
  | "provider_error"
  | "provider_status"
  | "context_compacted"

export type ProviderTranscriptRole =
  | "assistant"
  | "error"
  | "reasoning"
  | "status"
  | "compaction"
  | "tool"

export function providerTranscriptRoleForKind(kind: ProviderTranscriptKind): ProviderTranscriptRole
export function providerTranscriptRoleForKind(kind: string): ProviderTranscriptRole | null
export function providerTranscriptRoleForKind(kind: string): ProviderTranscriptRole | null {
  switch (kind) {
    case "provider_output":
      return "assistant"
    case "provider_reasoning":
      return "reasoning"
    case "provider_tool":
      return "tool"
    case "provider_error":
      return "error"
    case "provider_status":
      return "status"
    case "context_compacted":
      return "compaction"
    default:
      return null
  }
}
