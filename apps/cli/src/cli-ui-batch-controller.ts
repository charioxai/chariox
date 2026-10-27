type BatchFunction = (callback: () => void) => void

export type CliUiBatchControllerOptions = {
  batch: BatchFunction
  flushDeferredUpdates: () => void
}

export function createCliUiBatchController(options: CliUiBatchControllerOptions) {
  let depth = 0

  const run = (callback: () => void) => {
    depth += 1
    try {
      options.batch(callback)
    } finally {
      depth -= 1
      if (depth === 0) {
        options.flushDeferredUpdates()
      }
    }
  }

  return {
    isBatched: () => depth > 0,
    run,
  }
}
