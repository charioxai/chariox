import assert from "node:assert/strict"
import test from "node:test"
import { TextRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { applyTranscriptTextContent } from "./transcript-text-render.js"
import type { TranscriptEntry } from "./cli-types.js"

test("MP-08/MP-10/MP-11 transcript links keep exact targets and clean text, including trailing query punctuation", async () => {
  const harness = await createTestRenderer({width:80,height:8,useThread:false})
  const text = new TextRenderable(harness.renderer,{wrapMode:"char"})
  harness.renderer.root.add(text)
  try {
    const urls = ["https://github.com/charioxai/chariox", "https://example.org/authorize?state=abc!", "https://example.org/" + "x".repeat(150)]
    const content = urls.join("\n") + "\nhttps://user:fake@example.org/ file:///tmp/example"
    applyTranscriptTextContent(text,{id:1,role:"notice",text:content} as TranscriptEntry)
    await harness.renderOnce()
    const chunks = text.textNode.toChunks()
    assert.equal(chunks.map(value => value.text).join(""),content)
    assert.deepEqual(chunks.filter(value => value.link).map(value => value.link!.url),urls)
    assert.ok(!chunks.some(value => value.link?.url.includes("fake")))
  } finally { harness.renderer.destroy() }
})
