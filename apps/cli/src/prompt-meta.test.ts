import assert from "node:assert/strict"
import test from "node:test"

import { formatPromptMetaLine } from "./prompt-meta.js"

test("formatPromptMetaLine renders provider, model, and effort values", () => {
  assert.equal(formatPromptMetaLine("opencode", "gpt-5.4", "high"), "OpenCode • GPT-5.4 • High")
})

test("formatPromptMetaLine handles defaults and provider-qualified models", () => {
  assert.equal(formatPromptMetaLine("opencode", "openai/gpt-5.4", ""), "OpenCode • GPT-5.4 OpenAI")
  assert.equal(formatPromptMetaLine("opencode", "github-copilot/gpt-5.4", ""), "OpenCode • GPT-5.4 GitHub-Copilot")
  assert.equal(formatPromptMetaLine("opencode", "default", "default"), "OpenCode • Default")
})
