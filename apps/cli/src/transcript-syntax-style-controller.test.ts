import assert from "node:assert/strict"
import test from "node:test"

import { createTranscriptSyntaxStyleController } from "./transcript-syntax-style-controller.js"

test("transcript syntax style controller recreates theme-derived style", () => {
  let revision = 0
  const controller = createTranscriptSyntaxStyleController({
    createStyle: () => ({ revision: ++revision }),
  })

  assert.deepEqual(controller.current(), { revision: 1 })
  assert.deepEqual(controller.reset(), { revision: 2 })
  assert.deepEqual(controller.current(), { revision: 2 })
})
