import assert from "node:assert/strict"
import test from "node:test"

import {
  createBackgroundPollerStartupController,
  type BackgroundPollerStartupControllerDeps,
} from "./background-poller-startup-controller.js"

test("background poller startup waits for mounted prompt and transcript refs", () => {
  const harness = createHarness({ ready: false, promptMounted: false })

  harness.controller.ensureStarted()

  assert.equal(harness.controller.started(), false)
  assert.deepEqual(harness.calls(), ["debug:ensure pollers:missing refs:false"])
})

test("background poller startup uses the event stream without the polling watchdog", () => {
  const harness = createHarness()

  harness.controller.ensureStarted()
  harness.controller.ensureStarted()

  assert.equal(harness.controller.started(), true)
  assert.deepEqual(harness.calls(), [
    "debug:ensure pollers:starting",
    "rebuild",
    "placeholder",
    "focus",
    "last-scroll:42",
    "add-resize",
    "poll-room-environment",
    "info:starting kernel event stream",
    "sync-events",
    "debug:ensure pollers:already started",
  ])
})

test("background poller startup starts polling mode when event streams are unavailable", () => {
  const harness = createHarness({
    attached: false,
    supportsKernelEventStream: false,
    transcriptScrollTop: 15,
  })

  harness.controller.ensureStarted()

  assert.deepEqual(harness.calls(), [
    "debug:ensure pollers:starting",
    "rebuild",
    "placeholder",
    "blur",
    "last-scroll:15",
    "add-resize",
    "poll-room-environment",
    "info:starting background pollers",
    "poll-output",
    "poll-notices",
    "poll-session",
    "watchdog-start",
  ])
})

test("background poller startup removes resize listener only after start", () => {
  const idleHarness = createHarness({ ready: false })
  idleHarness.controller.stop()
  assert.deepEqual(idleHarness.calls(), ["watchdog-stop"])

  const startedHarness = createHarness()
  startedHarness.controller.ensureStarted()
  startedHarness.clearCalls()
  startedHarness.controller.stop()

  assert.deepEqual(startedHarness.calls(), ["watchdog-stop", "remove-resize"])
})

function createHarness(options: {
  ready?: boolean
  promptMounted?: boolean
  attached?: boolean
  supportsKernelEventStream?: boolean
  transcriptScrollTop?: number
} = {}) {
  const calls: string[] = []
  const deps: BackgroundPollerStartupControllerDeps = {
    logger: {
      info: (message) => {
        calls.push(`info:${message}`)
      },
    },
    ready: () => options.ready ?? true,
    promptMounted: () => options.promptMounted ?? true,
    transcriptScrollTop: () => options.transcriptScrollTop ?? 42,
    setLastTranscriptScrollTop: (scrollTop) => {
      calls.push(`last-scroll:${scrollTop}`)
    },
    isAttached: () => options.attached ?? true,
    rebuildTranscript: () => {
      calls.push("rebuild")
    },
    syncPromptPlaceholder: () => {
      calls.push("placeholder")
    },
    focusPrompt: () => {
      calls.push("focus")
    },
    blurPrompt: () => {
      calls.push("blur")
    },
    addResizeListener: () => {
      calls.push("add-resize")
    },
    removeResizeListener: () => {
      calls.push("remove-resize")
    },
    supportsKernelEventStream: () => options.supportsKernelEventStream ?? true,
    syncKernelEventSubscription: () => {
      calls.push("sync-events")
    },
    pollOutput: () => {
      calls.push("poll-output")
    },
    pollNotices: () => {
      calls.push("poll-notices")
    },
    pollSessionState: () => {
      calls.push("poll-session")
    },
    pollRoomEnvironmentActivity: () => {
      calls.push("poll-room-environment")
    },
    startConnectionWatchdog: () => {
      calls.push("watchdog-start")
    },
    stopConnectionWatchdog: () => {
      calls.push("watchdog-stop")
    },
    logViewDebug: (message, fields) => {
      if (fields && "has_prompt_input" in fields) {
        calls.push(`debug:${message}:${String(fields.has_prompt_input)}`)
      } else {
        calls.push(`debug:${message}`)
      }
    },
  }
  return {
    controller: createBackgroundPollerStartupController(deps),
    calls: () => calls,
    clearCalls: () => {
      calls.length = 0
    },
  }
}
