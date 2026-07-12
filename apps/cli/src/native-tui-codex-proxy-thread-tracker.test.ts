import assert from "node:assert/strict"
import test from "node:test"

import { createCodexProjectionThreadTracker } from "./native-tui/codex-proxy.js"

test("codex projection keeps the TUI-started thread when upstream announces another thread", () => {
  const projectedThreadIds: string[] = []
  const debug: Array<{ label: string, payload: unknown }> = []
  const tracker = createCodexProjectionThreadTracker({
    setThreadId: (threadId) => projectedThreadIds.push(threadId),
    debug: (label, payload) => debug.push({ label, payload }),
  })

  tracker.bindTuiThread("tui-thread")
  tracker.observeUpstreamThread("provider-thread")

  assert.deepEqual(projectedThreadIds, ["tui-thread"])
  assert.deepEqual(debug, [{
    label: "upstream_thread_started_ignored_for_projection",
    payload: {
      tuiThreadId: "tui-thread",
      upstreamThreadId: "provider-thread",
    },
  }])
})

test("codex projection can initialize from upstream before the TUI starts a thread", () => {
  const projectedThreadIds: string[] = []
  const tracker = createCodexProjectionThreadTracker({
    setThreadId: (threadId) => projectedThreadIds.push(threadId),
    debug: () => {},
  })

  tracker.observeUpstreamThread("provider-thread")
  tracker.bindTuiThread("tui-thread")

  assert.deepEqual(projectedThreadIds, ["provider-thread", "tui-thread"])
})
