import assert from "node:assert/strict"
import test from "node:test"
import { TextNodeRenderable, TextRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { applyTranscriptTextContent, linkifyTranscriptChunks } from "./transcript-text-render.js"
import { createClipboardController } from "./clipboard-controller.js"
import { createTranscriptLinkController } from "./transcript-link-controller.js"

declare const Bun: { stringWidth(text: string): number }

for (const prefix of ["λ prefix ", "👩‍💻 ", "界 λ ", "\t\n"]) test(`MP-08 / MP-10 inline URL opens without handoff after ${JSON.stringify(prefix)}`, async () => {
  const harness = await createTestRenderer({ width: 60, height: 16, useThread: false })
  const url = "https://www.wikipedia.org/?state=" + "abcdefgh".repeat(30)
  const text = new TextRenderable(harness.renderer, { width: 50, height: 14 })
  const opened: string[] = []
  const links = createTranscriptLinkController({ renderer: harness.renderer, localDesktop: () => true,
    openUrl: async url => { opened.push(url); return true }, flashFooter: () => {} })
  text.onMouseDown = links.handleMouseDown
  text.onMouseUp = links.handleMouseUp
  harness.renderer.root.add(text)
  try {
    const apply = process.env.TUIFIX_BASE_TEXT_RENDER
      ? (await import(process.env.TUIFIX_BASE_TEXT_RENDER)).applyTranscriptTextContent : applyTranscriptTextContent
    apply(text, { id: 1, role: "system", text: prefix + url })
    await harness.renderOnce()
    assert.ok(text.getTextChildren().some(node => node instanceof TextNodeRenderable && node.link?.url === url))
    const send = (bytes: string) => harness.renderer.stdin.emit("data", bytes)
    const rows = harness.captureCharFrame().split("\n")
    const y = rows.findIndex(row => row.includes("https://")) + 1
    assert.ok(y > 0)
    const row = rows[y-1]!
    const x = Bun.stringWidth(row.slice(0, row.indexOf("https://"))) + 1
    send(`\x1b[<0;${x};${y}M\x1b[<0;${x};${y}m`)
    await new Promise(resolve => setImmediate(resolve))
    assert.deepEqual(opened, [url])
    assert.equal(links.selectedUrl(), url, "F6/F7 use the full clicked URL, including its state")
    send(`\x1b[<0;${x};${y}M\x1b[<32;${x+10};${y}M\x1b[<0;${x+10};${y}m`)
    assert.deepEqual(opened, [url], "dragging text must not open a browser")
    assert.equal(links.selectedUrl(), null)
    harness.renderer.clearSelection()
    assert.equal(links.selectedUrl(), null)
  } finally { harness.renderer.destroy() }
})

test("MP-08 / MP-10 SSH click uses full-URL clipboard, never the SSH host's browser", async () => {
  const url = "https://www.wikipedia.org/?state=complete"
  const selection = { getSelectedText: () => "s" }
  const target = { plainText: url, getSelection: () => ({ start: 4, end: 5 }) }
  const copied: string[] = []
  const links = createTranscriptLinkController({ renderer: { getSelection: () => selection },
    localDesktop: () => false, openUrl: async () => { throw Error("wrong desktop") }, flashFooter: () => {} })
  links.handleMouseDown({ button: 0, x: 4, y: 1, target })
  links.handleMouseUp({ button: 0, x: 4, y: 1, target })
  await new Promise(resolve => setImmediate(resolve))
  const clipboard = createClipboardController({ renderer: { getSelection: () => selection, clearSelection: () => {}, copyToClipboardOSC52: () => false },
    promptInput: () => null, selectedLinkUrl: links.selectedUrl, flashFooter: () => {},
    copyText: async text => { copied.push(text); return "unavailable" } })
  const key = { name: "f6" }
  clipboard.captureCopyKey(key); clipboard.replayCopyKey(key); clipboard.copyCapturedSelection()
  await new Promise(resolve => setImmediate(resolve))
  assert.deepEqual(copied, [url])
  assert.equal(links.selectedUrl(), url)
  assert.equal(await links.activate("javascript:alert(1)"), false)
})

test("MP-08 / MP-10 highlighted bare URL chunks keep styles and the complete link", () => {
  const url = "https://www.wikipedia.org/?state=complete"
  const chunks = linkifyTranscriptChunks([{ __isChunk: true, text: "prefix https://www.", attributes: 1 }, { __isChunk: true, text: "wikipedia.org/?state=complete suffix", attributes: 2 }])
  assert.equal(chunks.map(chunk => chunk.text).join(""), "prefix " + url + " suffix")
  assert.deepEqual(chunks.map(chunk => [chunk.text, chunk.attributes, chunk.link?.url]), [
    ["prefix ", 1, undefined], ["https://www.", 1, url], ["wikipedia.org/?state=complete", 2, url], [" suffix", 2, undefined],
  ])
})
