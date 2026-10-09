import { parseKeypress, StdinParser } from "@opentui/core"
import assert from "node:assert/strict"
import test from "node:test"
import {
  createCliStdinKeyController,
  type CliStdinKeyControllerDeps,
} from "./cli-stdin-key-controller.js"
import { recordedDragStreams } from "./cli-stdin-drag-streams.test-fixture.js"

function selectionController() {
  const counts = { clears: 0, shortcuts: 0 }
  const controller = createCliStdinKeyController({
    createStdinParser: (onTimeoutFlush: () => void) => new StdinParser({ timeoutMs: 10, armTimeouts: true, onTimeoutFlush, useKittyKeyboard: true }),
    clearTextSelection: () => { counts.clears++ },
    dialogOverlayOpen: () => false,
    handleSessionBrowserKey: () => { counts.shortcuts++; return true },
  } as unknown as CliStdinKeyControllerDeps)
  return { controller, counts }
}

test("MP-08 / MP-10 batched SGR mouse reports preserve the drag and deferred rebuild", () => {
  const reports = ["\x1b[<32;11;5M", "\x1b[<32;12;5M", "\x1b[<0;12;5m"]
  for (const raw of [reports.slice(0, 2).join(""), reports.join("")]) {
    for (const chunk of [raw, Buffer.from(raw)]) {
      const { controller, counts } = selectionController()
      assert.equal(controller.handleData(chunk), false)
      assert.deepEqual(counts, { clears: 0, shortcuts: 0 })
    }
  }
})

// MP-08 / MP-10: stdin chunks may split a report anywhere, including right
// after ESC or inside the CSI parameters. No split of a real drag may act as
// a key press, and the next real key must still be decoded afterwards.
test("MP-08 / MP-10 recorded drags split at every byte boundary keep the selection", () => {
  for (const [name, reports] of Object.entries(recordedDragStreams)) {
    const stream = reports.join("")
    const deliveries = [
      ...Array.from({ length: stream.length - 1 }, (_, i) => [stream.slice(0, i + 1), stream.slice(i + 1)]),
      [...stream],
    ]
    for (const chunks of deliveries) {
      const { controller, counts } = selectionController()
      chunks.forEach((chunk, i) => controller.handleData(i % 2 ? Buffer.from(chunk) : chunk))
      const at = `${name} split ${JSON.stringify(chunks[0])}`
      assert.deepEqual(counts, { clears: 0, shortcuts: 0 }, at)
      controller.handleData("x")
      assert.deepEqual(counts, { clears: 1, shortcuts: 1 }, at)
    }
  }
})

test("MP-08 / MP-10 a lone legacy ESC is still decoded as escape after the timeout", async () => {
  const { controller, counts } = selectionController()
  controller.handleData("\x1b")
  assert.deepEqual(counts, { clears: 0, shortcuts: 0 })
  await new Promise((resolve) => setTimeout(resolve, 50))
  assert.deepEqual(counts, { clears: 1, shortcuts: 1 })
})

test("MP-08 / MP-10 terminal focus reports keep the selection", () => {
  for (const raw of ["\x1b[I", "\x1b[O"]) {
    const { controller, counts } = selectionController()
    controller.handleData(raw)
    assert.deepEqual(counts, { clears: 0, shortcuts: 0 })
  }
})

test("MP-08 / MP-10 F6 is distinct from Ctrl+C with the legacy keyboard protocol", () => {
  assert.equal(parseKeypress(Buffer.from("\x1b[17~"), { useKittyKeyboard: false })?.name, "f6")
  const interrupt = parseKeypress(Buffer.from("\x03"), { useKittyKeyboard: false })
  assert.equal(interrupt?.name, "c")
  assert.equal(interrupt?.ctrl, true)
  assert.equal(interrupt?.shift, false)
})

// MP-08 / MP-10: real input must release retained selection so the root's
// clearTextSelection callback can flush waiting-room rebuilds.
test("MP-08 / MP-10 bracketed paste clears retained selection before returning", () => {
  for (const chunk of ["\x1b[200~pasted text\x1b[201~", Buffer.from("\x1b[200~pasted text\x1b[201~")]) {
    const { controller, counts } = selectionController()
    assert.equal(controller.handleData(chunk), false)
    assert.deepEqual(counts, { clears: 1, shortcuts: 0 })
  }
})

test("MP-08 / MP-10 batched typing is decoded as the same keys typed one by one", () => {
  for (const chunk of ["ab", Buffer.from("ab")]) {
    const { controller, counts } = selectionController()
    assert.equal(controller.handleData(chunk), true)
    assert.deepEqual(counts, { clears: 2, shortcuts: 2 })
  }
})

test("MP-08 / MP-10 native copy owns F7 and Esc before dialogs/selection clearing", async () => {
  let active = false
  const { controller, counts } = selectionControllerWithNativeMode()
  function selectionControllerWithNativeMode() {
    const counts = { clears: 0, shortcuts: 0, nativeKeys: [] as string[] }
    const controller = createCliStdinKeyController({
      createStdinParser: (onTimeoutFlush: () => void) => new StdinParser({ timeoutMs: 10, armTimeouts: true, onTimeoutFlush, useKittyKeyboard: true }),
      nativeSelectionActive: () => active,
      handleNativeSelectionKey: (event: import("./cli-stdin-key-controller.js").CliStdinKeyEvent) => {
        if (event.name === "f7" || active && event.name === "escape") {
          active = !active; counts.nativeKeys.push(event.name); return true
        }
        return active
      },
      clearTextSelection: () => { counts.clears++ },
      dialogOverlayOpen: () => true,
      closeActiveDialogOverlay: () => { counts.shortcuts++ },
      handleSessionBrowserKey: () => { counts.shortcuts++; return true },
    } as unknown as CliStdinKeyControllerDeps)
    return { controller, counts }
  }
  controller.handleData("\x1b[18~")
  assert.equal(active, true)
  controller.handleData("\x1b[200~paste\x1b[201~")
  controller.handleData("x")
  controller.handleData("\x1b")
  await new Promise(resolve => setTimeout(resolve, 50))
  assert.equal(active, false)
  assert.deepEqual(counts, { clears: 0, shortcuts: 0, nativeKeys: ["f7", "escape"] })
})
