import { strict as assert } from "node:assert"
import test from "node:test"

import { createTranscriptParserRegistration } from "./transcript-parser-registration.js"

test("transcript parser registration adds parsers only once", () => {
  const calls: string[][] = []
  const registration = createTranscriptParserRegistration({
    parsers: ["one", "two"],
    addDefaultParsers: (parsers) => {
      calls.push([...parsers])
    },
  })

  assert.equal(registration.ensureRegistered(), true)
  assert.equal(registration.ensureRegistered(), false)
  assert.deepEqual(calls, [["one", "two"]])
})
