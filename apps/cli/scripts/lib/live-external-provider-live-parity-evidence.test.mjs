import assert from "node:assert/strict"
import test from "node:test"

import {
  assertProviderTranscript,
  classifyClaudeJsonlTranscriptValues,
  classifyCodexJsonlTranscriptValues,
  classifyOpenCodeSqliteTranscriptRows,
  providerLimitations,
} from "./live-external-provider-live-parity-evidence.mjs"
import {
  requiredAssistantMarkers,
  requiredToolMarkers,
} from "./live-external-provider-live-parity-common.mjs"

test("OpenCode SQLite evidence classifies markers by message role and part type", () => {
  const finalMarker = "FINAL_EXTERNAL_PARITY_SUMMARY_DRILL"
  const promptMarker = "EXTERNAL_PARITY_USER_PROMPT_DRILL"
  const promptText = enumeratedPrompt(promptMarker, finalMarker)
  const evidence = classifyOpenCodeSqliteTranscriptRows([
    messageRow("message-user", "user"),
    messageRow("message-assistant", "assistant"),
    partRow("message-user", {
      type: "text",
      text: `${promptText} ${promptMarker}`,
    }),
    partRow("message-user", {
      type: "tool",
      state: { input: { command: "printf TOOL_STEP_19" } },
    }),
    partRow("message-assistant", {
      type: "reasoning",
      text: `ASSISTANT_STEP_01 ASSISTANT_STEP_02 TOOL_STEP_18 ${finalMarker}`,
    }),
    partRow("message-assistant", {
      type: "tool",
      state: {
        input: { command: `printf TOOL_STEP_01 ASSISTANT_STEP_03 ${finalMarker}` },
        output: "TOOL_STEP_02",
      },
    }),
    partRow("message-assistant", {
      type: "text",
      text: `ASSISTANT_STEP_20 ${finalMarker}`,
    }),
  ], { finalMarker, promptMarker })

  assert.deepEqual(evidence, {
    assistantMarkersSeen: ["ASSISTANT_STEP_20"],
    toolMarkersSeen: ["TOOL_STEP_01", "TOOL_STEP_02"],
    reasoningMarkersSeen: ["ASSISTANT_STEP_01", "ASSISTANT_STEP_02"],
    finalSeen: true,
    promptOccurrences: 2,
  })
})

test("OpenCode SQLite evidence requires final output from assistant text", () => {
  const finalMarker = "FINAL_EXTERNAL_PARITY_SUMMARY_DRILL"
  const evidence = classifyOpenCodeSqliteTranscriptRows([
    messageRow("message-user", "user"),
    messageRow("message-assistant", "assistant"),
    partRow("message-user", { type: "text", text: finalMarker }),
    partRow("message-assistant", { type: "reasoning", text: finalMarker }),
    partRow("message-assistant", { type: "tool", state: { output: finalMarker } }),
  ], { finalMarker, promptMarker: "PROMPT" })

  assert.equal(evidence.finalSeen, false)
})

test("Codex JSONL evidence excludes prompt and reasoning markers from assistant and tools", () => {
  const finalMarker = "FINAL_EXTERNAL_PARITY_SUMMARY_DRILL"
  const promptMarker = "EXTERNAL_PARITY_USER_PROMPT_DRILL"
  const prompt = enumeratedPrompt(promptMarker, finalMarker)
  const finalOutput = `ASSISTANT_STEP_20 ${finalMarker}`
  const evidence = classifyCodexJsonlTranscriptValues([
    codexMessage("developer", `ASSISTANT_STEP_02 TOOL_STEP_19 ${finalMarker}`),
    codexMessage("user", prompt),
    { type: "event_msg", payload: { type: "user_message", message: prompt } },
    {
      type: "response_item",
      payload: { type: "reasoning", summary: [{ type: "summary_text", text: `ASSISTANT_STEP_03 TOOL_STEP_18 ${finalMarker}` }] },
    },
    {
      type: "response_item",
      payload: {
        type: "custom_tool_call",
        name: "exec",
        input: `TOOL_STEP_01 ASSISTANT_STEP_04 ${finalMarker}`,
      },
    },
    {
      type: "response_item",
      payload: { type: "custom_tool_call_output", output: "TOOL_STEP_02" },
    },
    codexMessage("assistant", finalOutput, "output_text"),
    { type: "event_msg", payload: { type: "agent_message", message: finalOutput } },
  ], { finalMarker, promptMarker })

  assert.deepEqual(evidence, {
    assistantMarkersSeen: ["ASSISTANT_STEP_20"],
    toolMarkersSeen: ["TOOL_STEP_01", "TOOL_STEP_02"],
    reasoningMarkersSeen: ["ASSISTANT_STEP_03"],
    finalSeen: true,
    promptOccurrences: 1,
  })
})

test("Claude JSONL evidence excludes prompt, metadata, and thinking markers from assistant and tools", () => {
  const finalMarker = "FINAL_EXTERNAL_PARITY_SUMMARY_DRILL"
  const promptMarker = "EXTERNAL_PARITY_USER_PROMPT_DRILL"
  const prompt = enumeratedPrompt(promptMarker, finalMarker)
  const evidence = classifyClaudeJsonlTranscriptValues([
    {
      type: "last-prompt",
      message: { content: prompt },
    },
    {
      type: "user",
      message: {
        role: "user",
        content: [
          { type: "text", text: prompt },
          { type: "tool_result", content: "TOOL_STEP_02" },
        ],
      },
    },
    {
      type: "user",
      isMeta: true,
      message: { role: "user", content: [{ type: "text", text: "ASSISTANT_STEP_06 TOOL_STEP_17" }] },
    },
    {
      type: "assistant",
      message: {
        role: "assistant",
        content: [
          { type: "thinking", thinking: `ASSISTANT_STEP_03 TOOL_STEP_18 ${finalMarker}` },
          { type: "tool_use", input: { command: `TOOL_STEP_01 ASSISTANT_STEP_04 ${finalMarker}` } },
          { type: "text", text: `ASSISTANT_STEP_20 ${finalMarker}` },
        ],
      },
    },
    {
      type: "assistant",
      message: {
        role: "assistant",
        model: "<synthetic>",
        content: [{ type: "text", text: "ASSISTANT_STEP_05" }],
      },
    },
  ], { finalMarker, promptMarker })

  assert.deepEqual(evidence, {
    assistantMarkersSeen: ["ASSISTANT_STEP_20"],
    toolMarkersSeen: ["TOOL_STEP_01", "TOOL_STEP_02"],
    reasoningMarkersSeen: ["ASSISTANT_STEP_03"],
    finalSeen: true,
    promptOccurrences: 1,
  })
})

test("provider transcript assertions remain strict for missing OpenCode assistant text", () => {
  const result = { assertions: [] }
  assertProviderTranscript(result, {
    found: true,
    assistantMarkersSeen: ["ASSISTANT_STEP_20"],
    toolMarkersSeen: requiredToolMarkers,
    finalSeen: true,
    promptOccurrences: 1,
  }, "provider transcript")

  assert.equal(
    result.assertions.find((entry) => entry.name === "provider transcript saw all assistant markers")?.passed,
    false,
  )
})

test("provider limitations distinguish semantic provider output gaps from Chariox import loss", () => {
  const providerOutputLimitation = assistantTextLimitation({
    provider: "codex",
    providerMarkers: ["ASSISTANT_STEP_20"],
    kernelMarkers: ["ASSISTANT_STEP_20"],
  })
  assert.equal(providerOutputLimitation.status, "not_observed")
  assert.equal(providerOutputLimitation.classification, "provider_output_limitation")

  const charioxBug = assistantTextLimitation({
    provider: "claude",
    providerMarkers: requiredAssistantMarkers,
    kernelMarkers: ["ASSISTANT_STEP_20"],
  })
  assert.equal(charioxBug.status, "not_observed")
  assert.equal(charioxBug.classification, "chariox_bug")
})

test("provider limitations classify missing semantic tool events as provider output gaps", () => {
  const limitations = providerLimitations("claude", {
    providerTranscript: {
      found: true,
      semanticEvidence: true,
      assistantMarkersSeen: requiredAssistantMarkers,
      toolMarkersSeen: ["TOOL_STEP_20"],
      finalSeen: true,
      promptOccurrences: 1,
    },
    kernel: {
      assistantMarkersSeen: requiredAssistantMarkers,
      toolMarkersSeen: ["TOOL_STEP_20"],
      finalSeen: true,
      samples: [],
      statuses: [],
    },
  })

  const toolCalls = limitations.find((entry) => entry.metadata === "tool_calls")
  assert.equal(toolCalls.status, "not_observed")
  assert.equal(toolCalls.classification, "provider_output_limitation")
})

function assistantTextLimitation({ provider, providerMarkers, kernelMarkers }) {
  const limitations = providerLimitations(provider, {
    providerTranscript: {
      found: true,
      semanticEvidence: true,
      assistantMarkersSeen: providerMarkers,
      toolMarkersSeen: requiredToolMarkers,
      finalSeen: true,
      promptOccurrences: 1,
    },
    kernel: {
      assistantMarkersSeen: kernelMarkers,
      toolMarkersSeen: requiredToolMarkers,
      finalSeen: true,
      samples: [],
      statuses: [],
    },
  })
  return limitations.find((entry) => entry.metadata === "assistant_text")
}

function codexMessage(role, text, contentType = "input_text") {
  return {
    type: "response_item",
    payload: {
      type: "message",
      role,
      content: [{ type: contentType, text }],
    },
  }
}

function enumeratedPrompt(promptMarker, finalMarker) {
  return [promptMarker, ...requiredAssistantMarkers, ...requiredToolMarkers, finalMarker].join(" ")
}

function messageRow(id, role) {
  return {
    kind: "message",
    id,
    data: JSON.stringify({ role }),
  }
}

function partRow(messageId, data) {
  return {
    kind: "part",
    message_id: messageId,
    data,
  }
}
