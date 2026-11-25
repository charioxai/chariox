import assert from "node:assert/strict"
import test from "node:test"
import { customChoiceLine } from "./interaction-custom-choice-render.js"

test("a free-text option the agent gave no label or hint is a plain box with no text of ours", () => {
  assert.equal(customChoiceLine(1, { label: "" }, "", false, false), "  2. ")
  assert.equal(customChoiceLine(1, { label: "", placeholder: null }, "teal", true, true), "> 2. teal_")
})

test("the agent's label and hint show as given, a label alone brings no hint", () => {
  assert.equal(customChoiceLine(1, { label: "Something else", placeholder: "Another colour" }, "", false, false), "  2.Something else: <Another colour>")
  assert.equal(customChoiceLine(1, { label: "Your colour" }, "", false, false), "  2.Your colour: ")
  assert.equal(customChoiceLine(1, { label: "Passphrase", input_kind: "secret" }, "abc", false, false), "  2.Passphrase: ***")
})
