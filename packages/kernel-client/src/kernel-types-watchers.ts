/** A watcher: a supervised script on a kernel that is a notification source of its owner. */
export type WatcherMode = { poll: { every_s: number } } | "stream"
export type WatcherCoalesce = "every" | "latest_per_key"
export interface WatcherEventDeclaration { name: string; keyed?: boolean }
export interface WatcherManifest {
  name: string; sentence: string; mode: WatcherMode; command: string[];
  events: WatcherEventDeclaration[]; coalesce?: WatcherCoalesce; debounce_s?: number
}
export interface WatcherScript { file_name: string; text: string }
export type WatcherState = "draft" | "active" | "paused"
export type WatcherHealth =
  | { state: "ok" }
  | { state: "failing"; since_ms: number; reason: string }
  | { state: "gap"; since_ms: number }
  | { state: "noisy"; since_ms: number }
export interface WatcherEvent { event: string; key?: string; text: string }
export interface WatcherSummary {
  watcher_id: string; source_id: string; kernel_id: string; session_id: string; creator_agent_id: string | null;
  manifest: WatcherManifest; state: WatcherState; health: WatcherHealth; reading: string | null;
  last_check_ms: number | null; last_fired_ms: number | null; fired_count: number; dry_run: WatcherDryRun | null
}
export interface WatcherDryRun { reading: string | null; history: boolean; events: WatcherEvent[] }
export type WatcherResponse = { Watcher: { watcher: WatcherSummary } }
export type WatchersResponse = { Watchers: { watchers: WatcherSummary[] } }
export type WatcherDryRunResponse = { WatcherDryRun: { result: WatcherDryRun } }
export type WatcherRemovedResponse = { WatcherRemoved: { watcher_id: string } }
