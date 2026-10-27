import assert from "node:assert/strict"
import test from "node:test"
import { TextRenderable, getLinkId, resolveRenderLib } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"

test("MP-08/MP-10/MP-11 native default attributes preserve the complete link ID", async () => {
  const harness = await createTestRenderer({ width: 80, height: 4, useThread: false })
  const url = "https://github.com/charioxai/chariox"
  const lib = resolveRenderLib() as ReturnType<typeof resolveRenderLib> & {
    linkAlloc(url: string): number
    linkGetUrl(id: number): string
    attributesWithLink(base: number, id: number): number
  }
  const linkId = lib.linkAlloc(url)
  const node = new TextRenderable(harness.renderer, {
    content: url,
    attributes: lib.attributesWithLink(1, linkId),
  })
  harness.renderer.root.add(node)
  try {
    await harness.renderOnce()
    const frame = lib.getCurrentBuffer(harness.renderer.rendererPtr)
    const actual = getLinkId(frame.buffers.attributes[8]!)
    assert.equal(actual, linkId, "the native u32 attribute pointer must retain all 24 link bits")
    assert.equal(lib.linkGetUrl(actual), url)
    node.attributes = 0
    await harness.renderOnce()
    const plain = lib.getCurrentBuffer(harness.renderer.rendererPtr)
    assert.equal(getLinkId(plain.buffers.attributes[8]!), 0, "plain attributes must not acquire neighboring link bits")
  } finally { harness.renderer.destroy() }
})
