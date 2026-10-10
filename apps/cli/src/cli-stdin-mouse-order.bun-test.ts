import assert from "node:assert/strict"
import test from "node:test"
import { ScrollBoxRenderable, TextRenderable, TextareaRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { bindRendererShortcutInput } from "./renderer-shortcut-input.js"
import { createClipboardController } from "./clipboard-controller.js"
import { createPromptSurfaceMouseController } from "./prompt-surface-mouse-controller.js"
import { createCliStdinKeyController, type CliStdinKeyControllerDeps } from "./cli-stdin-key-controller.js"

for (const rawFirst of [false, true]) for (const buffer of [false, true]) for (const paste of [false, true]) for (const delivery of ["separate", "down-coalesced", "drag-coalesced"] as const) {
  test(`MP-08 / MP-10 newer mouse selection survives earlier ${paste ? "paste" : "typing"} (${delivery}; stdin observer ${rawFirst ? "first" : "last"}; ${buffer ? "Buffer" : "string"})`, async () => {
    const harness = await createTestRenderer({ width: 80, height: 8, useThread: false })
    const transcript = new TextRenderable(harness.renderer, { content: "retained transcript", width: 40, height: 1 })
    const prompt = new TextareaRenderable(harness.renderer, { width: 40, height: 2 })
    harness.renderer.root.add(transcript)
    harness.renderer.root.add(prompt)
    prompt.focus()
    await harness.renderOnce()
    const copies: string[] = []
    const clipboard = createClipboardController({ renderer: harness.renderer, promptInput: () => prompt,
      flashFooter: () => {}, copyText: async text => { copies.push(text); return "copied" } })
    const raw = createCliStdinKeyController({
      flushTextSelectionRebuild: () => {},
      hasPromptSelection: () => prompt.hasSelection(),
      dialogOverlayOpen: () => false, handleSessionBrowserKey: () => true,
    } as unknown as CliStdinKeyControllerDeps)
    harness.renderer.keyInput.prependListener("keypress", event => clipboard.captureCopyKey(event))
    harness.renderer.keyInput.prependListener("paste", () => clipboard.capturePaste())
    const mouse = createPromptSurfaceMouseController({ delayMs: 0, scheduleTimer: setTimeout,
      isPrimaryButton: () => true, copyText: clipboard.copyTextWithFeedback, retainPromptFocus: () => prompt.focus() })
    transcript.onMouseUp = mouse.handleMouseUp
    harness.renderer.on("selection", selection => mouse.handleSelection(selection.getSelectedText()))
    const disposeInput = bindRendererShortcutInput({
      keyInput: harness.renderer.keyInput, enabled: () => true,
      handleEvent: raw.handleEvent, discardInput: () => {},
    })
    const rawInput = () => {}
    if (rawFirst) harness.renderer.stdin.prependListener("data", rawInput)
    else harness.renderer.stdin.on("data", rawInput)
    const send = async (bytes: string) => {
      harness.renderer.stdin.emit("data", buffer ? Buffer.from(bytes) : bytes)
      await Promise.resolve()
    }
    try {
      const edit = paste ? "\x1b[200~z\x1b[201~" : "z"
      const down = "\x1b[<0;1;1M", rest = "\x1b[<32;9;1M\x1b[<0;9;1m"
      if (delivery === "separate") { await send(edit); await send(down); await send(rest) }
      else if (delivery === "down-coalesced") { await send(edit + down); await send(rest) }
      else await send(edit + down + rest)
      assert.equal(prompt.plainText, "z")
      assert.equal(harness.renderer.getSelection()?.getSelectedText(), "retained")
      await new Promise(resolve => setTimeout(resolve, 10))
      assert.deepEqual(copies, ["retained"], "release copies the newer selection")
    } finally { disposeInput(); harness.renderer.destroy() }
  })
}

for (const rawFirst of [false, true]) for (const buffer of [false, true]) for (const paste of [false, true]) for (const delivery of ["separate", "release-coalesced", "drag-coalesced", "all-coalesced"] as const) {
  test(`MP-08 / MP-10 released mouse selection copies once before later ${paste ? "paste" : "typing"} (${delivery}; stdin observer ${rawFirst ? "first" : "last"}; ${buffer ? "Buffer" : "string"})`, async () => {
    const harness = await createTestRenderer({ width: 80, height: 8, useThread: false })
    const transcript = new TextRenderable(harness.renderer, { content: "retained transcript", width: 40, height: 1 })
    const prompt = new TextareaRenderable(harness.renderer, { width: 40, height: 2 })
    const transcriptPane = new ScrollBoxRenderable(harness.renderer, { width: 40, height: 1, focusable: true })
    transcriptPane.add(transcript)
    harness.renderer.root.add(transcriptPane)
    harness.renderer.root.add(prompt)
    prompt.focus()
    await harness.renderOnce()
    const copies: string[] = []
    const clipboard = createClipboardController({ renderer: harness.renderer, promptInput: () => prompt,
      flashFooter: () => {}, copyText: async text => { copies.push(text); return "copied" } })
    const mouse = createPromptSurfaceMouseController({ delayMs: 0, scheduleTimer: setTimeout,
      isPrimaryButton: () => true, copyText: clipboard.copyTextWithFeedback, retainPromptFocus: () => prompt.focus() })
    transcript.onMouseUp = mouse.handleMouseUp
    harness.renderer.on("selection", selection => mouse.handleSelection(selection.getSelectedText()))
    harness.renderer.keyInput.prependListener("keypress", event => clipboard.captureCopyKey(event))
    harness.renderer.keyInput.prependListener("paste", () => clipboard.capturePaste())
    const rawInput = () => {}
    if (rawFirst) harness.renderer.stdin.prependListener("data", rawInput)
    else harness.renderer.stdin.on("data", rawInput)
    const send = (bytes: string) => harness.renderer.stdin.emit("data", buffer ? Buffer.from(bytes) : bytes)
    try {
      const edit = paste ? "\x1b[200~z\x1b[201~" : "z"
      const down = "\x1b[<0;1;1M", drag = "\x1b[<32;9;1M", up = "\x1b[<0;9;1m"
      if (delivery === "separate") {
        send(down + drag + up)
        await new Promise(resolve => setTimeout(resolve, 10))
        send(edit)
      } else if (delivery === "release-coalesced") { send(down + drag); send(up + edit) }
      else if (delivery === "drag-coalesced") { send(down); send(drag + up + edit) }
      else send(down + drag + up + edit)
      assert.equal(prompt.plainText, "z", "the following edit still reaches the prompt")
      assert.equal(harness.renderer.getSelection(), null, "the following edit clears the highlight")
      await new Promise(resolve => setTimeout(resolve, 10))
      assert.deepEqual(copies, ["retained"], "release text is copied once even after its highlight clears")
    } finally { harness.renderer.destroy() }
  })
}
