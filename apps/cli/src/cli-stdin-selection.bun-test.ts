import { parseKeypress } from "@opentui/core"
import assert from "node:assert/strict"
import test from "node:test"
import {
  createCliStdinKeyController,
  type CliStdinKeyControllerDeps,
} from "./cli-stdin-key-controller.js"

test("MP-08 / MP-10 batched SGR mouse reports preserve the drag and deferred rebuild", () => {
  const reports = ["\x1b[<32;11;5M", "\x1b[<32;12;5M", "\x1b[<0;12;5m"]
  for (const raw of [reports.slice(0, 2).join(""), reports.join("")]) {
    for (const chunk of [raw, Buffer.from(raw)]) {
      let clears = 0
      let shortcuts = 0
      const controller = createCliStdinKeyController({
        parseKeypress,
        clearTextSelection: () => { clears++ },
        dialogOverlayOpen: () => false,
        handleSessionBrowserKey: () => { shortcuts++; return true },
      } as unknown as CliStdinKeyControllerDeps)

      assert.equal(controller.handleData(chunk), false)
      assert.equal(clears, 0)
      assert.equal(shortcuts, 0)
    }
  }
})

test("MP-08 / MP-10 F6 is distinct from Ctrl+C with the legacy keyboard protocol", () => {
  assert.equal(parseKeypress(Buffer.from("\x1b[17~"), { useKittyKeyboard: false })?.name, "f6")
  const interrupt = parseKeypress(Buffer.from("\x03"), { useKittyKeyboard: false })
  assert.equal(interrupt?.name, "c")
  assert.equal(interrupt?.ctrl, true)
  assert.equal(interrupt?.shift, false)
})

// MP-08 / MP-10: these real parser inputs must release retained selection so
// the root's clearTextSelection callback can flush waiting-room rebuilds.
for (const [label, raw] of [
  ["bracketed paste", "\x1b[200~pasted text\x1b[201~"],
  ["batched text", "ab"],
] as const) {
  test(`MP-08 / MP-10 ${label} clears retained selection before returning`, () => {
    for (const chunk of [raw, Buffer.from(raw)]) {
      let clears = 0
      let shortcuts = 0
      const controller = createCliStdinKeyController({
        parseKeypress,
        clearTextSelection: () => { clears++ },
        handleSessionBrowserKey: () => { shortcuts++; return true },
      } as unknown as CliStdinKeyControllerDeps)

      assert.equal(parseKeypress(chunk, { useKittyKeyboard: true })?.name, "")
      assert.equal(controller.handleData(chunk), false)
      assert.equal(clears, 1)
      assert.equal(shortcuts, 0)
    }
  })
}
