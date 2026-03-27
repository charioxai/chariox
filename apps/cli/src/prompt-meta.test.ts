import assert from "node:assert/strict"
import test from "node:test"

import { formatPromptMetaLine, formatPromptMetaParts } from "./prompt-meta.js"

test("formatPromptMetaLine renders provider, model, and effort values", () => {
  assert.equal(formatPromptMetaLine("opencode", "gpt-5.4", "high"), "OpenCode • GPT-5.4 • High")
})

test("formatPromptMetaLine handles defaults and provider-qualified models", () => {
  assert.equal(formatPromptMetaLine("opencode", "openai/gpt-5.4", ""), "OpenCode • GPT-5.4 OpenAI")
  assert.equal(formatPromptMetaLine("opencode", "github-copilot/gpt-5.4", ""), "OpenCode • GPT-5.4 GitHub-Copilot")
  assert.equal(formatPromptMetaLine("opencode", "default", "default"), "OpenCode • Default")
})

test("formatPromptMetaParts assigns bright tones per value", () => {
  assert.deepEqual(formatPromptMetaParts("opencode", "openai/gpt-5.4", "high"), [
    { kind: "provider", text: "OpenCode", tone: "primary" },
    { kind: "model", text: "GPT-5.4 OpenAI", tone: "secondary" },
    { kind: "variant", text: "High", tone: "primary" },
  ])
  assert.deepEqual(formatPromptMetaParts("anthropic", "claude-3.7-sonnet", "low"), [
    { kind: "provider", text: "Anthropic", tone: "warning" },
    { kind: "model", text: "Claude-3.7-Sonnet", tone: "warning" },
    { kind: "variant", text: "Low", tone: "success" },
  ])
})
