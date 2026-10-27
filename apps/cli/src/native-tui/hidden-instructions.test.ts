import assert from "node:assert/strict"
import test from "node:test"
import {
  hiddenInstructionsEnd,
  hiddenInstructionsStart,
  redactHiddenInstructions,
  redactHiddenInstructionsFromJson,
} from "./hidden-instructions.js"

test("redacts tagged workflow prompt components after the visible endpoint prompt", () => {
  const prompt = [
    "<endpoint-prompt>",
    "Visible request",
    "</endpoint-prompt>",
    "<node-level-prompt>",
    "Hidden node instructions",
    "</node-level-prompt>",
    "<workflow-runtime-instructions>",
    "Hidden runtime instructions",
    "</workflow-runtime-instructions>",
  ].join("\n")

  assert.equal(
    redactHiddenInstructions(prompt),
    "<endpoint-prompt>\nVisible request\n</endpoint-prompt>",
  )
})

test("continues to redact legacy workflow headings from stored prompts", () => {
  assert.equal(
    redactHiddenInstructions("Visible request\n\nWorkflow-level prompt:\nHidden legacy instructions"),
    "Visible request\n",
  )
})

test("redacts explicit hidden blocks recursively", () => {
  const value = {
    prompt: `Visible ${hiddenInstructionsStart}secret${hiddenInstructionsEnd}`,
    nested: [`Keep ${hiddenInstructionsStart}private${hiddenInstructionsEnd}`],
  }

  assert.deepEqual(redactHiddenInstructionsFromJson(value), {
    prompt: "Visible ",
    nested: ["Keep "],
  })
})
