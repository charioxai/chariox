import type { LocalIpcClient } from "./ipc.js"
import type { SliceLogEntry, SliceRecord } from "./cli-types.js"
import { getSliceLogs, startSlice } from "./slice-api.js"

// The provisioner (provision-linux-docker-slice.sh, released-slice-image.py) logs
// "[slice-linux] progress: <stage> [<done> MB of <total> MB (<n>%)]"; a source build
// shows BuildKit's plain "#12 [stage 3/18] RUN ..." steps instead.
const PROVISIONER_PROGRESS = /^\[slice-linux\] progress: (.+)$/
const BUILD_STEP = /^#\d+ \[(?:[\w.-]+ +)?(\d+)\/(\d+)\] (?:RUN|COPY|ADD|FROM)\b/
const POLL_MS = 1000
const TAIL_LINES = 400

/** The newest first-start progress the provisioner has logged, as one line for the footer. */
export function latestSliceStartProgress(entries: readonly Pick<SliceLogEntry, "text">[]): string | undefined {
  const lines = entries.flatMap((entry) => entry.text.split("\n"))
  for (let index = lines.length - 1; index >= 0; index -= 1) {
    const progress = PROVISIONER_PROGRESS.exec(lines[index]!)
    if (progress) {
      return progress[1]
    }
    const step = BUILD_STEP.exec(lines[index]!)
    if (step) {
      return `Building the runtime image from source: step ${step[1]} of ${step[2]}`
    }
  }
  return undefined
}

/**
 * Starts a slice and, while the kernel's provisioner loads or builds the runtime image,
 * shows what it reports. The first start can take minutes, so it must never be silent.
 * The start request carries the outcome; a failed progress read only skips one update.
 */
export async function startSliceShowingProgress(
  client: LocalIpcClient,
  sliceRef: string,
  show: (message: string) => void,
): Promise<SliceRecord> {
  let running = true
  const reporting = (async () => {
    while (running) {
      await new Promise((resolve) => setTimeout(resolve, POLL_MS))
      const progress = await getSliceLogs(client, sliceRef, TAIL_LINES)
        .then(({ entries }) => latestSliceStartProgress(entries), () => undefined)
      if (running && progress) {
        show(`starting slice: ${progress}`)
      }
    }
  })()
  try {
    return await startSlice(client, sliceRef)
  } finally {
    running = false
    await reporting
  }
}
