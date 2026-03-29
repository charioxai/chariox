import assert from "node:assert/strict"
import test from "node:test"

import { formatPromptMetaLine, formatPromptMetaParts, formatPromptUsageMeta } from "./prompt-meta.js"

test("formatPromptMetaLine renders provider, model, and effort values", () => {
  assert.equal(formatPromptMetaLine("opencode", "gpt-5.4", "high"), "OpenCode • GPT-5.4 • High")
})

test("formatPromptMetaLine handles defaults and provider-qualified models", () => {
  assert.equal(formatPromptMetaLine("opencode", "openai/gpt-5.4", ""), "OpenCode • OpenAI GPT-5.4")
  assert.equal(formatPromptMetaLine("opencode", "github-copilot/gpt-5.4", ""), "OpenCode • GitHub-Copilot GPT-5.4")
  assert.equal(formatPromptMetaLine("opencode", "default", "default"), "OpenCode • Default")
})

test("formatPromptMetaParts assigns bright tones per value", () => {
  assert.deepEqual(formatPromptMetaParts("opencode", "openai/gpt-5.4", "high"), [
    { kind: "provider", text: "OpenCode", tone: "primary" },
    { kind: "model", text: "OpenAI GPT-5.4", tone: "secondary" },
    { kind: "variant", text: "High", tone: "primary" },
  ])
  assert.deepEqual(formatPromptMetaParts("anthropic", "claude-3.7-sonnet", "low"), [
    { kind: "provider", text: "Anthropic", tone: "warning" },
    { kind: "model", text: "Claude-3.7-Sonnet", tone: "warning" },
    { kind: "variant", text: "Low", tone: "success" },
  ])
})

test("formatPromptUsageMeta renders token totals and usage bars", () => {
  assert.deepEqual(formatPromptUsageMeta(12345, 20000, 10), {
    tokensLabel: "12,345 tok",
    usagePercent: 62,
    usageLabel: "62%",
    barFilled: "======",
    barEmpty: "----",
  })
})

test("formatPromptUsageMeta falls back to token-only metadata without a limit", () => {
  assert.deepEqual(formatPromptUsageMeta(512, null, 8), {
    tokensLabel: "512 tok",
    usagePercent: null,
    usageLabel: "",
    barFilled: "",
    barEmpty: "--------",
  })
})
