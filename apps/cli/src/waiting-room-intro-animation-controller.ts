import { nextWaitingRoomIntroStep } from "./background-effects.js"
import type { WaitingRoomState } from "./waiting-room.js"

export type WaitingRoomIntroAnimationControllerDeps<TimerHandle> = {
  intervalMs: number
  scheduleInterval: (callback: () => void, intervalMs: number) => TimerHandle
  clearInterval: (handle: TimerHandle) => void
  isAttached: () => boolean
  getWaitingRoomState: () => WaitingRoomState
  setWaitingRoomState: (state: WaitingRoomState) => void
  rebuildTranscript: () => void
}

export type WaitingRoomIntroAnimationController = {
  start(): void
  stop(): void
  tick(): void
}

export function createWaitingRoomIntroAnimationController<TimerHandle>(
  deps: WaitingRoomIntroAnimationControllerDeps<TimerHandle>,
): WaitingRoomIntroAnimationController {
  let timer: TimerHandle | null = null

  const tick = () => {
    const state = deps.getWaitingRoomState()
    const nextIntroStep = nextWaitingRoomIntroStep(deps.isAttached(), state.introStep)
    if (nextIntroStep === null) {
      return
    }
    deps.setWaitingRoomState({
      ...state,
      introStep: nextIntroStep,
    })
    deps.rebuildTranscript()
  }

  return {
    start() {
      if (timer !== null) {
        return
      }
      timer = deps.scheduleInterval(tick, deps.intervalMs)
    },
    stop() {
      if (timer === null) {
        return
      }
      deps.clearInterval(timer)
      timer = null
    },
    tick,
  }
}
