import assert from "node:assert/strict"
import test from "node:test"

import { latestSliceStartProgress, startSliceShowingProgress } from "./slice-start-progress.js"

const log = (...lines: string[]) => [{ text: lines.join("\n") }]

test("the newest provisioner progress line wins over older ones", () => {
  assert.equal(latestSliceStartProgress(log(
    "[slice-linux] progress: Verifying the runtime image 900 MB of 900 MB (100%)",
    "[slice-linux] progress: Loading the runtime image 450 MB of 900 MB (50%)",
  )), "Loading the runtime image 450 MB of 900 MB (50%)")
})

test("a source build reports its BuildKit step, and unrelated output reports nothing", () => {
  assert.equal(latestSliceStartProgress(log(
    "[slice-linux] progress: Building the runtime image from source (several minutes)",
    "#38 [rust-builder 18/18] RUN mkdir -p /opt/chariox-runtime-bin",
    "#38 381.3 warning: unused",
  )), "Building the runtime image from source: step 18 of 18")
  assert.equal(latestSliceStartProgress(log("[slice-linux] using compatible cached image")), undefined)
  assert.equal(latestSliceStartProgress([]), undefined)
})

test("a pending start shows progress from the kernel's logs and returns the started slice", async () => {
  const shown: string[] = []
  let finishStart: () => void = () => {}
  const client = {
    async send(request: Record<string, unknown>) {
      if ("StartSlice" in request) {
        await new Promise<void>((resolve) => { finishStart = resolve })
        return { SliceStarted: { slice: { id: "slice-1" } } }
      }
      shown.length === 0 && setTimeout(() => finishStart(), 1500)
      return { SliceLogs: { slice: { id: "slice-1" }, entries: log("[slice-linux] progress: Loading the runtime image 10 MB of 20 MB (50%)") } }
    },
  }
  const slice = await startSliceShowingProgress(client as never, "slice-1", (message) => shown.push(message))
  assert.equal(slice.id, "slice-1")
  assert.deepEqual(shown.slice(0, 1), ["starting slice: Loading the runtime image 10 MB of 20 MB (50%)"])
})
