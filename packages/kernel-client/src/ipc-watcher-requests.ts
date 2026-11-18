import type { WatcherManifest, WatcherScript } from "./kernel-types-watchers.js"

export function createWatcherRequest(sessionId: string, manifest: WatcherManifest, script: WatcherScript) {
  return { Watcher: { create: { session_id: sessionId, manifest, script } } }
}
export function updateWatcherRequest(watcherId: string, manifest: WatcherManifest, script: WatcherScript) {
  return { Watcher: { update: { watcher_id: watcherId, manifest, script } } }
}
export function dryRunWatcherRequest(watcherId: string) {
  return { Watcher: { dry_run: { watcher_id: watcherId } } }
}
export function activateWatcherRequest(watcherId: string) {
  return { Watcher: { activate: { watcher_id: watcherId } } }
}
export function pauseWatcherRequest(watcherId: string, paused: boolean) {
  return { Watcher: { pause: { watcher_id: watcherId, paused } } }
}
export function checkWatcherNowRequest(watcherId: string) {
  return { Watcher: { check_now: { watcher_id: watcherId } } }
}
export function renameWatcherRequest(watcherId: string, name: string) {
  return { Watcher: { rename: { watcher_id: watcherId, name } } }
}
export function removeWatcherRequest(watcherId: string) {
  return { Watcher: { remove: { watcher_id: watcherId } } }
}
export function listWatchersRequest() {
  return { Watcher: { list: {} } }
}
