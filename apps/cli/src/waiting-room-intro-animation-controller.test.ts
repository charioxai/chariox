import assert from "node:assert/strict"
import test from "node:test"

import { createWaitingRoomIntroAnimationController } from "./waiting-room-intro-animation-controller.js"
import type { WaitingRoomState } from "./waiting-room.js"

test("waiting room intro animation advances while detached", () => {
  const harness = createHarness()

  harness.controller.tick()

  assert.equal(harness.state().introStep, 1)
  assert.deepEqual(harness.calls(), ["set:1", "rebuild"])
})

test("waiting room intro animation is idle when attached", () => {
  const harness = createHarness({ attached: true })

  harness.controller.tick()

  assert.equal(harness.state().introStep, 0)
  assert.deepEqual(harness.calls(), [])
})

test("waiting room intro animation is idle after the intro completes", () => {
  const harness = createHarness({ introStep: 12 })

  harness.controller.tick()

  assert.equal(harness.state().introStep, 12)
  assert.deepEqual(harness.calls(), [])
})

test("waiting room intro animation starts and stops one interval", () => {
  const harness = createHarness()

  harness.controller.start()
  harness.controller.start()
  harness.controller.stop()
  harness.controller.stop()

  assert.deepEqual(harness.calls(), ["interval:90", "clear:timer-1"])
})

function createHarness(options: {
  attached?: boolean
  introStep?: number
} = {}) {
  const calls: string[] = []
  let state = waitingRoomState(options.introStep ?? 0)
  const controller = createWaitingRoomIntroAnimationController<string>({
    intervalMs: 90,
    scheduleInterval: (_callback, intervalMs) => {
      calls.push(`interval:${intervalMs}`)
      return "timer-1"
    },
    clearInterval: (handle) => {
      calls.push(`clear:${handle}`)
    },
    isAttached: () => options.attached ?? false,
    getWaitingRoomState: () => state,
    setWaitingRoomState: (nextState) => {
      state = nextState
      calls.push(`set:${nextState.introStep}`)
    },
    rebuildTranscript: () => {
      calls.push("rebuild")
    },
  })

  return {
    controller,
    calls: () => calls,
    state: () => state,
  }
}

function waitingRoomState(introStep: number): WaitingRoomState {
  return {
    focus: "new",
    sessionIndex: 0,
    machineIndex: 0,
    remoteKernelIndex: 0,
    terminalIndex: 0,
    worktreeSelectionId: "current",
    providerId: "opencode",
    modelId: "default",
    effort: "medium",
    themeId: "system",
    introStep,
    keyState: {
      up: false,
      down: false,
      left: false,
      right: false,
    },
  }
}
