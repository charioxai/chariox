import assert from "node:assert/strict"
import test from "node:test"

import type { PromptMetaPart, PromptUsageMeta } from "@chariox/kernel-client/prompt-meta"
import type { PromptMetaRenderableRefs } from "./prompt-meta-renderer.js"
import { createPromptMetaRenderController } from "./prompt-meta-render-controller.js"

test("prompt meta render controller renders with current refs and usage", () => {
  const parts: PromptMetaPart[] = [{ kind: "provider", text: "OpenCode", tone: "primary" }]
  const usage: PromptUsageMeta = {
    tokensLabel: "42 tok",
    usagePercent: 4,
    usageLabel: "4%",
    barFilled: "=",
    barEmpty: "---------",
  }
  let refs = { providerText: "provider" } as unknown as PromptMetaRenderableRefs
  const rendered: Array<{
    refs: PromptMetaRenderableRefs
    parts: PromptMetaPart[]
    usage: PromptUsageMeta | null
  }> = []
  const controller = createPromptMetaRenderController({
    refs: () => refs,
    getUsage: () => usage,
    renderMeta: (nextRefs, nextParts, nextUsage) => {
      rendered.push({ refs: nextRefs, parts: nextParts, usage: nextUsage })
    },
  })

  controller.setRenderables(parts)
  refs = { modelText: "model" } as unknown as PromptMetaRenderableRefs
  controller.setRenderables([])

  assert.deepEqual(rendered, [
    { refs: { providerText: "provider" } as unknown as PromptMetaRenderableRefs, parts, usage },
    { refs: { modelText: "model" } as unknown as PromptMetaRenderableRefs, parts: [], usage },
  ])
})

test("prompt meta render controller owns refs when no external refs are provided", () => {
  const rendered: PromptMetaRenderableRefs[] = []
  let refAssignments = 0
  const controller = createPromptMetaRenderController({
    getUsage: () => null,
    onRefAssigned: () => {
      refAssignments += 1
    },
    renderMeta: (refs) => {
      rendered.push(refs)
    },
  })

  controller.assignRef("providerText", "provider" as unknown as PromptMetaRenderableRefs["providerText"])
  controller.assignRefCallback("modelText")("model" as unknown as PromptMetaRenderableRefs["modelText"])
  controller.setRenderables([])

  assert.equal(rendered[0]?.providerText, "provider")
  assert.equal(rendered[0]?.modelText, "model")
  assert.equal(refAssignments, 2)
})
