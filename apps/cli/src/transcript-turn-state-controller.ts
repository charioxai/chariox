export type TranscriptTurnStateControllerOptions = {
  initialCurrentTurnId: number | null
  initialNextTurnId: number
}

export function createTranscriptTurnStateController(options: TranscriptTurnStateControllerOptions) {
  let currentTurnId = options.initialCurrentTurnId
  let nextTurnId = options.initialNextTurnId

  return {
    getCurrentTurnId: () => currentTurnId,
    setCurrentTurnId: (turnId: number | null) => {
      currentTurnId = turnId
    },
    getNextTurnId: () => nextTurnId,
    setNextTurnId: (turnId: number) => {
      nextTurnId = turnId
    },
  }
}
