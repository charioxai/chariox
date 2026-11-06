import { createTestRenderer } from "@opentui/core/testing"
import { createClipboardController } from "./clipboard-controller.js"
import { bindRendererShortcutInput } from "./renderer-shortcut-input.js"
import { parseKeypress, StdinParser, TextRenderable, TextareaRenderable } from "@opentui/core"
import assert from "node:assert/strict"
import test from "node:test"
import {
  createCliStdinKeyController,
  type CliStdinKeyControllerDeps,
  type CliStdinKeyEvent,
} from "./cli-stdin-key-controller.js"
import { recordedDragStreams } from "./cli-stdin-drag-streams.test-fixture.js"

function selectionController() {
  const counts = { clears: 0, shortcuts: 0 }
  const controller = createCliStdinKeyController({
    createStdinParser: (onTimeoutFlush: () => void) => new StdinParser({ timeoutMs: 10, armTimeouts: true, onTimeoutFlush, useKittyKeyboard: true }),
    flushTextSelectionRebuild: () => { counts.clears++ },
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

// The CLI decodes shortcuts with its own parser, which the renderer's mouse
// reset never reaches. A drag whose release was lost must not leave Escape
// buffered into an Alt chord there either.
test("MP-08 / MP-10 the CLI parser recovers Escape and the next key after a missed mouse release", async () => {
  const seen: Array<{ name: string; meta?: boolean | undefined }> = []
  const controller = createCliStdinKeyController({
    createStdinParser: (onTimeoutFlush: () => void) => new StdinParser({ timeoutMs: 10, armTimeouts: true, onTimeoutFlush, useKittyKeyboard: true }),
    flushTextSelectionRebuild: () => {},
    dialogOverlayOpen: () => false,
    handleSessionBrowserKey: (event: { name: string; meta?: boolean }) => { seen.push({ name: event.name, meta: event.meta }); return true },
  } as unknown as CliStdinKeyControllerDeps)
  controller.handleData("\x1b[<0;1;1M")
  controller.handleData("\x1b")
  await new Promise((resolve) => setTimeout(resolve, 700))
  controller.handleData("c")
  assert.deepEqual(seen.map((key) => `${key.meta ? "meta+" : ""}${key.name}`), ["escape", "c"])
})

test("MP-08 / MP-10 terminal theme notifications retain selection and deferred rebuild through F6", () => {
  for (const buffer of [false, true]) for (const batched of [false, true]) for (const mode of [1, 2]) {
    let retained = true, rebuilds = 0
    const copied: string[] = [], replayed: string[] = [], shortcuts: string[] = []
    const controller = createCliStdinKeyController({
      createStdinParser: (onTimeoutFlush: () => void) => new StdinParser({ timeoutMs: 10, armTimeouts: true, onTimeoutFlush, useKittyKeyboard: true }),
      replayCopyKey: (event: CliStdinKeyEvent) => { replayed.push(event.name); return false },
      flushTextSelectionRebuild: () => { retained = false; rebuilds++ },
      dialogOverlayOpen: () => false,
      handleSessionBrowserKey: (event: CliStdinKeyEvent) => { shortcuts.push(event.name); return event.name !== "f6" },
      promptFocused: () => false,
      focusedInteractionActive: () => false,
      handleFocusedInteractionKey: () => false,
      copyPromptSelection: () => { copied.push(retained ? "retained transcript" : ""); return true },
    } as unknown as CliStdinKeyControllerDeps)
    const send = (bytes: string) => controller.handleData(buffer ? Buffer.from(bytes) : bytes)
    // The pinned real parser emits both notifications as empty-name key events.
    // Each must preserve copy replay ordering without dispatching ordinary shortcuts.
    send(`\x1b[?997;${3 - mode}n`)
    assert.equal(retained, true)
    assert.equal(rebuilds, 0)
    assert.deepEqual(shortcuts, [])
    send(`\x1b[?997;${mode}n` + (batched ? "\x1b[17~" : ""))
    assert.equal(retained, true)
    assert.equal(rebuilds, 0)
    if (!batched) send("\x1b[17~")
    assert.deepEqual(replayed, ["", "", "f6"])
    assert.deepEqual(shortcuts, ["f6"])
    assert.deepEqual(copied, ["retained transcript"])
    // A subsequent recognized key still clears selection and flushes the rebuild.
    send("x")
    assert.equal(retained, false)
    assert.equal(rebuilds, 1)
  }
})


for (const rawFirst of [false, true]) for (const buffer of [false, true]) for (const batched of [false, true]) for (const sameChunkEdit of [false, true]) for (const paste of [false, true]) for (const [kind, movement, selected] of [
  ["Shift+Left twice", "\x1b[1;2D\x1b[1;2D", "ft"],
  ["Shift+Home", "\x1b[1;2H", "draft"],
] as const) {
  test(`MP-08 / MP-10 prompt ${kind} survives raw routing and F6/replacement (${buffer ? "Buffer" : "string"}; stdin observer ${rawFirst ? "first" : "last"}; ${batched ? "coalesced" : "separate"}; ${paste ? "paste" : "typing"}; edit ${sameChunkEdit ? "coalesced" : "later"})`, async () => {
    const harness = await createTestRenderer({ width: 80, height: 8, useThread: false })
    const prompt = new TextareaRenderable(harness.renderer, { width: 40, height: 2, initialValue: "draft" })
    harness.renderer.root.add(prompt)
    prompt.focus()
    await harness.renderOnce()
    prompt.gotoBufferEnd()
    const copies: string[] = []
    const clipboard = createClipboardController({
      renderer: harness.renderer,
      promptInput: () => prompt,
      flashFooter: () => {},
      copyText: async (text) => { copies.push(text); return "copied" },
    })
    let rebuildDeferred = true, rebuilds = 0
    const raw = createCliStdinKeyController({
      hasPromptSelection: () => prompt.hasSelection(),
      flushTextSelectionRebuild: () => {
        if (rebuildDeferred) { rebuildDeferred = false; rebuilds++ }
      },
      dialogOverlayOpen: () => false,
      handleSessionBrowserKey: (event: CliStdinKeyEvent) => event.name !== "f6",
      promptFocused: () => true,
      focusedInteractionActive: () => false,
      handleFocusedInteractionKey: () => false,
      replayCopyKey: clipboard.replayCopyKey,
      copyPromptSelection: clipboard.copyCapturedSelection ?? clipboard.copyPromptSelection,
    } as unknown as CliStdinKeyControllerDeps)
    harness.renderer.keyInput.prependListener("keypress", event => clipboard.captureCopyKey(event))
    harness.renderer.keyInput.prependListener("paste", () => clipboard.capturePaste())
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
      const edit = paste ? "\x1b[200~Z\x1b[201~" : "Z"
      if (batched) await send(movement + "\x1b[17~" + (sameChunkEdit ? edit : ""))
      else {
        // Repeated selection commands must extend the same anchor.
        for (const key of movement.match(/\x1b\[[^A-Z]*[A-Z]/g)!) await send(key)
        assert.equal(prompt.getSelectedText(), selected)
        await send("\x1b[17~" + (sameChunkEdit ? edit : ""))
      }
      if (!sameChunkEdit) assert.equal(prompt.getSelectedText(), selected, "keyboard selection remains highlighted")
      assert.deepEqual(copies, [selected], "F6 copies the textarea's selected range")
      if (!sameChunkEdit) {
        assert.equal(rebuilds, 0, "selection keeps the waiting-room rebuild deferred")
        await send(edit)
      }
      assert.equal(prompt.plainText, kind === "Shift+Home" ? "Z" : "draZ", "typing replaces the selected range")
      assert.equal(prompt.hasSelection(), false)
      assert.equal(rebuilds, 1, "editing releases selection and flushes the deferred rebuild")
    } finally { disposeInput(); harness.renderer.destroy() }
  })
}

// MP-08 / MP-10: repeated copies in one chunk retain each event's range,
// including an empty selection before the first selection movement.
for (const rawFirst of [false, true]) for (const buffer of [false, true]) {
  test(`MP-08 / MP-10 copy snapshots precede later selection extension (${buffer ? "Buffer" : "string"}; stdin observer ${rawFirst ? "first" : "last"})`, async () => {
    const harness = await createTestRenderer({ width: 80, height: 8, useThread: false })
    const prompt = new TextareaRenderable(harness.renderer, { width: 40, height: 2, initialValue: "draft" })
    harness.renderer.root.add(prompt)
    prompt.focus()
    await harness.renderOnce()
    prompt.gotoBufferEnd()
    const copies: string[] = []
    const clipboard = createClipboardController({ renderer: harness.renderer, promptInput: () => prompt,
      flashFooter: () => {}, copyText: async text => { copies.push(text); return "copied" } })
    const raw = createCliStdinKeyController({
      replayCopyKey: clipboard.replayCopyKey,
      copyPromptSelection: clipboard.copyCapturedSelection ?? clipboard.copyPromptSelection,
      hasPromptSelection: () => prompt.hasSelection(),
      flushTextSelectionRebuild: () => {},
      dialogOverlayOpen: () => false, handleSessionBrowserKey: (event: CliStdinKeyEvent) => event.name !== "f6",
      commandCenterOpen: () => false, commandCenterQuery: () => "", isAttached: () => false,
      promptFocused: () => true, focusedInteractionActive: () => false,
      handleFocusedInteractionKey: () => false, handleQueuedPromptKey: () => false,
    } as unknown as CliStdinKeyControllerDeps)
    harness.renderer.keyInput.prependListener("keypress", event => clipboard.captureCopyKey(event))
    harness.renderer.keyInput.prependListener("paste", () => clipboard.capturePaste())
    const disposeInput = bindRendererShortcutInput({
      keyInput: harness.renderer.keyInput, enabled: () => true,
      handleEvent: raw.handleEvent, discardInput: () => {},
    })
    const rawInput = () => {}
    if (rawFirst) harness.renderer.stdin.prependListener("data", rawInput)
    else harness.renderer.stdin.on("data", rawInput)
    try {
      const bytes = "\x1b[17~\x1b[1;2D\x1b[17~\x1b[1;2D\x1b[17~\x1b[1;2D"
      harness.renderer.stdin.emit("data", buffer ? Buffer.from(bytes) : bytes)
      await Promise.resolve()
      assert.deepEqual(copies, ["t", "ft"], "empty F6 must stay empty; each later F6 copies its event-time range")
      assert.equal(prompt.getSelectedText(), "aft")
    } finally { disposeInput(); harness.renderer.destroy() }
  })
}

// MP-08 / MP-10: editing must invalidate transcript selection before a later
// decoded copy, even though raw routing/rebuilds wait for the entire chunk.
for (const rawFirst of [false, true]) for (const buffer of [false, true]) for (const batched of [false, true]) for (const paste of [false, true]) {
  test(`MP-08 / MP-10 transcript edit precedes copy (${buffer ? "Buffer" : "string"}; stdin observer ${rawFirst ? "first" : "last"}; ${batched ? "coalesced" : "separate"}; ${paste ? "paste" : "typing"})`, async () => {
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
    let rebuilds = 0
    const raw = createCliStdinKeyController({
      replayCopyKey: clipboard.replayCopyKey,
      copyPromptSelection: clipboard.copyCapturedSelection,
      hasPromptSelection: () => prompt.hasSelection(),
      flushTextSelectionRebuild: () => { rebuilds++ },
      dialogOverlayOpen: () => false, handleSessionBrowserKey: (event: CliStdinKeyEvent) => event.name !== "f6",
      commandCenterOpen: () => false, commandCenterQuery: () => "", isAttached: () => false,
      promptFocused: () => true, focusedInteractionActive: () => false,
      handleFocusedInteractionKey: () => false, handleQueuedPromptKey: () => false,
    } as unknown as CliStdinKeyControllerDeps)
    harness.renderer.keyInput.prependListener("keypress", event => clipboard.captureCopyKey(event))
    harness.renderer.keyInput.prependListener("paste", () => clipboard.capturePaste())
    const disposeInput = bindRendererShortcutInput({
      keyInput: harness.renderer.keyInput, enabled: () => true,
      handleEvent: raw.handleEvent, discardInput: () => {},
    })
    const rawInput = () => {}
    if (rawFirst) harness.renderer.stdin.prependListener("data", rawInput)
    else harness.renderer.stdin.on("data", rawInput)
    const send = async (bytes: string) => {
      harness.renderer.stdin.emit("data", buffer ? Buffer.from(bytes) : bytes)
      assert.equal(rebuilds, 0, "renderer invalidation must not flush the deferred rebuild")
      await Promise.resolve()
    }
    try {
      harness.renderer.startSelection(transcript, transcript.x, transcript.y)
      harness.renderer.updateSelection(transcript, transcript.x + 8, transcript.y, { finishDragging: true })
      assert.equal(harness.renderer.getSelection()?.getSelectedText(), "retained")
      const edit = paste ? "\x1b[200~z\x1b[201~" : "z"
      if (batched) await send(edit + "\x1b[17~")
      else { await send(edit); harness.renderer.stdin.emit("data", buffer ? Buffer.from("\x1b[17~") : "\x1b[17~"); await Promise.resolve() }
      assert.equal(prompt.plainText, "z")
      assert.equal(harness.renderer.getSelection(), null)
      assert.equal(rebuilds, 1, "raw edit flushes the deferred rebuild once")
      assert.deepEqual(copies, [], "an edit before F6 must not overwrite the clipboard with deselected transcript text")
    } finally { disposeInput(); harness.renderer.destroy() }
  })
}

// MP-08 / MP-10 reviewer0794: a native release never reaches the TUI.
