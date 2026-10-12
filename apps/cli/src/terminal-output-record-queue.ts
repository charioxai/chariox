type TerminalOutputRecordQueueOptions<TimerHandle, RecordValue> = {
  delayMs: number
  immediateFirstRecord?: boolean
  maxRecordsPerFlush?: number
  maxPendingRecords?: number
  scheduleTimer: (callback: () => void, delayMs: number) => TimerHandle
  clearTimer: (timer: TimerHandle) => void
  processRecords: (records: RecordValue[]) => void
}

export type TerminalOutputRecordQueue<RecordValue> = {
  queue(records: RecordValue[]): void
  flush(): void
  drain(): void
  clearTimer(): void
  pendingCount(): number
  hasPendingFlush(): boolean
}

export function createTerminalOutputRecordQueue<TimerHandle, RecordValue>(
  options: TerminalOutputRecordQueueOptions<TimerHandle, RecordValue>,
): TerminalOutputRecordQueue<RecordValue> {
  let pendingTimer: TimerHandle | undefined
  let pendingRecords: RecordValue[] = []
  const maxRecordsPerFlush = Math.max(1, options.maxRecordsPerFlush ?? Number.POSITIVE_INFINITY)
  const maxPendingRecords = Math.max(1, options.maxPendingRecords ?? Number.POSITIVE_INFINITY)

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
    const records = pendingRecords.splice(0, maxRecordsPerFlush)
    options.processRecords(records)
    if (pendingRecords.length > 0) {
      scheduleFlush()
    }
  }

  const drain = () => {
    clearPendingTimer()
    while (pendingRecords.length > 0) {
      const records = pendingRecords.splice(0, maxRecordsPerFlush)
      options.processRecords(records)
      clearPendingTimer()
    }
  }

  const scheduleFlush = () => {
    if (pendingTimer !== undefined) {
      return
    }
    pendingTimer = options.scheduleTimer(() => {
      pendingTimer = undefined
      flush()
    }, options.delayMs)
  }

  return {
    queue(records) {
      if (records.length === 0) {
        return
      }
      // MP-08 / MP-10: render the leading record before opening the
      // coalescing window. Later records retain the bounded batching budget.
      if (options.immediateFirstRecord && pendingTimer === undefined && pendingRecords.length === 0) {
        scheduleFlush()
        pendingRecords.push(...records)
        options.processRecords(pendingRecords.splice(0, 1))
      } else {
        pendingRecords.push(...records)
      }
      // A busy producer can outrun the delayed flush. Process overflow now so
      // the queue stays bounded without dropping transcript records.
      while (pendingRecords.length > maxPendingRecords) {
        flush()
      }
      scheduleFlush()
    },
    flush,
    drain,
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
