type TerminalOutputRecordQueueOptions<TimerHandle, RecordValue> = {
  delayMs: number
  scheduleTimer: (callback: () => void, delayMs: number) => TimerHandle
  clearTimer: (timer: TimerHandle) => void
  processRecords: (records: RecordValue[]) => void
}

export type TerminalOutputRecordQueue<RecordValue> = {
  queue(records: RecordValue[]): void
  flush(): void
  clearTimer(): void
  pendingCount(): number
  hasPendingFlush(): boolean
}

export function createTerminalOutputRecordQueue<TimerHandle, RecordValue>(
  options: TerminalOutputRecordQueueOptions<TimerHandle, RecordValue>,
): TerminalOutputRecordQueue<RecordValue> {
  let pendingTimer: TimerHandle | undefined
  let pendingRecords: RecordValue[] = []

  const clearPendingTimer = () => {
    if (pendingTimer === undefined) {
      return
    }
    options.clearTimer(pendingTimer)
    pendingTimer = undefined
  }

  const flush = () => {
    clearPendingTimer()
    if (pendingRecords.length === 0) {
      return
    }
    const records = pendingRecords
    pendingRecords = []
    options.processRecords(records)
  }

  return {
    queue(records) {
      if (records.length === 0) {
        return
      }
      pendingRecords.push(...records)
      if (pendingTimer !== undefined) {
        return
      }
      pendingTimer = options.scheduleTimer(() => {
        pendingTimer = undefined
        flush()
      }, options.delayMs)
    },
    flush,
    clearTimer() {
      clearPendingTimer()
    },
    pendingCount() {
      return pendingRecords.length
    },
    hasPendingFlush() {
      return pendingTimer !== undefined
    },
  }
}
