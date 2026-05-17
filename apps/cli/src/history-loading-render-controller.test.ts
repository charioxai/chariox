import assert from "node:assert/strict"
import test from "node:test"

import { createHistoryLoadingRenderController } from "./history-loading-render-controller.js"

test("history loading render controller delegates current refs and loading state", () => {
  const rendered: unknown[] = []
  let box: string | undefined = "box-a"
  let text: string | undefined = "text-a"
  let loading = true
  const controller = createHistoryLoadingRenderController({
    renderer: "renderer",
    box: () => box,
    text: () => text,
    loading: () => loading,
    assignText: (value) => {
      text = value
    },
    renderIndicator: (options) => {
      rendered.push(options)
    },
  })

  controller.render()
  box = "box-b"
  loading = false
  controller.render()

  assert.equal(rendered.length, 2)
  const first = rendered[0] as {
    box: string
    text: string
    loading: boolean
    renderer: string
    assignText: (value: string | undefined) => void
  }
  assert.equal(first.box, "box-a")
  assert.equal(first.text, "text-a")
  assert.equal(first.loading, true)
  assert.equal(first.renderer, "renderer")
  first.assignText("text-b")
  assert.equal(text, "text-b")

  const second = rendered[1] as { box: string; text: string; loading: boolean; renderer: string }
  assert.equal(second.box, "box-b")
  assert.equal(second.text, "text-a")
  assert.equal(second.loading, false)
  assert.equal(second.renderer, "renderer")
})
