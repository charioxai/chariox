export interface KernelClient {
 send<T = Record<string, unknown>>(request: unknown): Promise<T>
 onKernelEvent(callback: (event: {event: string; records?: Array<Record<string, unknown>>}) => void): () => void
 subscribeToKernelEvents(sessionId: string, attachmentId: string): Promise<void>
 close(): Promise<void>
}
export type AgentTrigger = {id: string; agent_id: string; enabled: boolean; source: {kind: "http"; ingress_id: string} | {kind: "schedule"}}
export type TriggerDelivery = {prompt_id: string; origin: "http" | "schedule"; prompt: string}
export type SessionHistoryPageEntry = {entry_index: number; fragment_start?: number; fragment_end?: number; total_chars: number; entry: {kind: string; text: string; timestamp_ms?: number; prompt_origin?: string | null}}
export type SessionHistoryOutlineTurn = {turn_id: string; prompt_id?: string | null; started_at_ms: number; completed_at_ms?: number | null; lifecycle: string; user_prompt: SessionHistoryPageEntry; entries: SessionHistoryPageEntry[]; blobs: Array<{blob_id: string}>; summary?: SessionHistoryPageEntry | null}
