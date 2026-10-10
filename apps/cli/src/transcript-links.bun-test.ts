import assert from "node:assert/strict"
import test from "node:test"
import { CodeRenderable, SyntaxStyle, TextRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { applyTranscriptTextContent } from "./transcript-text-render.js"
import type { TranscriptEntry } from "./cli-types.js"
import { linkifyTranscriptChunks } from "./transcript-hyperlinks.js"
import { installTerminalHyperlinkContent, rememberTerminalLinkChunks } from "./terminal-hyperlink-content.js"

test("MP-08/MP-10/MP-11 hyperlink repaint never replaces newer streamed text with cached content", async () => {
  const harness = await createTestRenderer({width:80,height:5,useThread:false})
  const syntax = SyntaxStyle.fromStyles({})
  const old = "OLD https://github.com/charioxai/chariox"
  const node = new CodeRenderable(harness.renderer,{content:old,syntaxStyle:syntax})
  harness.renderer.root.add(node)
  rememberTerminalLinkChunks(node,linkifyTranscriptChunks([{__isChunk:true,text:old}]),old)
  const dispose = installTerminalHyperlinkContent(harness.renderer)
  try {
    await harness.renderOnce()
    node.content = "NEW STREAM CONTENT"
    await harness.renderOnce()
    assert.ok(harness.captureCharFrame().includes("NEW STREAM CONTENT"))
    assert.ok(!harness.captureCharFrame().includes("OLD https://"))
  } finally { dispose(); harness.renderer.destroy(); syntax.destroy() }
})

test("MP-08/MP-10/MP-11 Markdown bare URLs spanning styled chunks receive one exact native target", () => {
  const url = "https://github.com/charioxai/chariox"
  const input = [{ __isChunk: true as const, text: "Read https://github.com/", attributes: 1 }, { __isChunk: true as const, text: "charioxai/chariox, then reply." }]
  const output = linkifyTranscriptChunks(input)
  assert.equal(output.map(chunk => chunk.text).join(""), input.map(chunk => chunk.text).join(""))
  assert.deepEqual(output.filter(chunk => chunk.link).map(chunk => chunk.link!.url), [url, url])
  assert.equal(output[1]!.attributes, 1)
  assert.ok(!linkifyTranscriptChunks([{__isChunk:true,text:"https://user:fake@example.org/"}]).some(chunk => chunk.link))
})

test("MP-08/MP-10/MP-11 transcript links keep exact targets and clean text, including trailing query punctuation", async () => {
  const harness = await createTestRenderer({width:80,height:8,useThread:false})
  const text = new TextRenderable(harness.renderer,{wrapMode:"char"})
  harness.renderer.root.add(text)
  try {
    const urls = ["https://github.com/charioxai/chariox", "https://example.org/authorize?state=abc!", "https://example.org/" + "x".repeat(150)]
    const content = `Read ${urls[0]}, then reply.\n` + urls.join("\n") + "\nhttps://user:fake@example.org/ file:///tmp/example"
    applyTranscriptTextContent(text,{id:1,role:"notice",text:content} as TranscriptEntry)
    await harness.renderOnce()
    const chunks = text.textNode.toChunks()
    assert.equal(chunks.map(value => value.text).join(""),content)
    assert.deepEqual(chunks.filter(value => value.link).map(value => value.link!.url),[urls[0],...urls])
    assert.ok(!chunks.some(value => value.link?.url.includes("fake")))
  } finally { harness.renderer.destroy() }
})
