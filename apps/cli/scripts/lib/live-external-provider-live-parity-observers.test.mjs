import assert from "node:assert/strict"
import test from "node:test"

import {
  historyOutlineTextsWithBlobContent,
  sampleDiagnostic,
  summarizeSamples,
  transcriptMarkerOutputText,
  webSample,
} from "./live-external-provider-live-parity-observers.mjs"

test("kernel marker evidence excludes user prompts and non-output history", async () => {
  const outline = {
    agents: [{
      agent_id: "agent-1",
      turns: [{
        user_prompt: pageEntry("user_prompt", "TOOL_STEP_20 FINAL_MARKER"),
        entries: [
          pageEntry("provider_output", "ASSISTANT_STEP_01"),
          pageEntry("provider_tool", "TOOL_STEP_01"),
          pageEntry("provider_status", "TOOL_STEP_19"),
        ],
        blobs: [],
      }],
    }],
  }

  const { text, markerText } = await historyOutlineTextsWithBlobContent({
    client: null,
    sessionId: "session-1",
    agentId: "agent-1",
    outline,
  })

  assert.equal(text, "TOOL_STEP_20 FINAL_MARKER\nASSISTANT_STEP_01\nTOOL_STEP_01\nTOOL_STEP_19")
  assert.equal(markerText, "ASSISTANT_STEP_01\nTOOL_STEP_01")
})

test("kernel marker evidence filters loaded history blob content", async () => {
  const outline = {
    agents: [{
      agent_id: "agent-1",
      turns: [{
        user_prompt: pageEntry("user_prompt", "TOOL_STEP_20"),
        entries: [],
        blobs: [{
          blob_id: "blob-tool",
          kind: "provider_tool",
          sequence_start: 1,
        }],
      }],
    }],
  }
  const client = {
    async send() {
      return {
        SessionHistoryBlobContent: {
          entries: [
            pageEntry("provider_tool", "TOOL_STEP_02"),
            pageEntry("provider_status", "TOOL_STEP_18"),
          ],
        },
      }
    },
  }

  const { text, markerText } = await historyOutlineTextsWithBlobContent({
    client,
    sessionId: "session-1",
    agentId: "agent-1",
    outline,
  })

  assert.equal(text, "TOOL_STEP_20\nTOOL_STEP_02\nTOOL_STEP_18")
  assert.equal(markerText, "TOOL_STEP_02")
})

test("web marker evidence excludes prompt and status text", async () => {
  const finalMarker = "FINAL_MARKER"
  const promptMarker = "EXTERNAL_PARITY_USER_PROMPT"
  const page = fakeWebPage([
    element("freeform-message freeform-user-prompt", `${promptMarker} TOOL_STEP_20 ${finalMarker}`),
    element("freeform-message freeform-agent-output", "ASSISTANT_STEP_01"),
    element("freeform-message freeform-agent-output freeform-tool-output freeform-activity-group", "ASSISTANT_STEP_19 TOOL_STEP_01"),
    element("freeform-message freeform-agent-output freeform-tool-output", "TOOL_STEP_01"),
    element("freeform-message freeform-agent-output freeform-status-output", "TOOL_STEP_19"),
  ])

  const sample = await webSample(page, "codex", "DRILL_MARKER", finalMarker, promptMarker)

  assert.match(sample.text, /TOOL_STEP_20/)
  assert.equal(sample.markerText, "ASSISTANT_STEP_01\nTOOL_STEP_01")
  assert.deepEqual(sample.assistantMarkers, ["ASSISTANT_STEP_01"])
  assert.deepEqual(sample.toolMarkers, ["TOOL_STEP_01"])
  assert.equal(sample.finalSeen, false)
  assert.equal(sample.promptOccurrences, 1)
})

test("TUI marker evidence includes assistant and tool entries only", () => {
  assert.equal(transcriptMarkerOutputText([
    { role: "user", text: "TOOL_STEP_20 FINAL_MARKER" },
    { role: "assistant", text: "ASSISTANT_STEP_01" },
    { role: "tool", text: "TOOL_STEP_01" },
    { role: "status", text: "TOOL_STEP_19" },
    { role: "reasoning", text: "ASSISTANT_STEP_19" },
  ]), "ASSISTANT_STEP_01\nTOOL_STEP_01")
})

test("TUI marker evidence includes visible collapsed output summaries without relabeling reasoning", () => {
  assert.equal(transcriptMarkerOutputText([
    {
      role: "tool",
      text: "",
      blobCollapsible: true,
      blobCollapsed: true,
      blobTitle: "1 tool called",
      blobSummary: "$ printf TOOL_STEP_02",
    },
    {
      role: "reasoning",
      text: "",
      blobCollapsible: true,
      blobCollapsed: true,
      blobTitle: "thinking",
      blobSummary: "ASSISTANT_STEP_19",
    },
  ]), "1 tool called\n$ printf TOOL_STEP_02")
})

test("surface summaries do not recover markers from diagnostic prompt text", () => {
  const summary = summarizeSamples("web", [{
    text: "TOOL_STEP_20 FINAL_MARKER",
    markerText: "ASSISTANT_STEP_01",
    assistantMarkers: ["ASSISTANT_STEP_01"],
    toolMarkers: [],
    finalSeen: false,
    promptOccurrences: 1,
    status: "WORKING",
  }], "FINAL_MARKER")

  assert.deepEqual(summary.assistantMarkersSeen, ["ASSISTANT_STEP_01"])
  assert.deepEqual(summary.toolMarkersSeen, [])
  assert.equal(summary.finalSeen, false)
})

test("surface summaries use final-and-later samples for authoritative marker coverage", () => {
  const summary = summarizeSamples("web", [{
    markerText: "ASSISTANT_STEP_01 TOOL_STEP_01",
    assistantMarkers: ["ASSISTANT_STEP_01"],
    toolMarkers: ["TOOL_STEP_01"],
    finalSeen: false,
    status: "WORKING",
  }, {
    markerText: "ASSISTANT_STEP_20 TOOL_STEP_20 FINAL_MARKER",
    assistantMarkers: ["ASSISTANT_STEP_20"],
    toolMarkers: ["TOOL_STEP_20"],
    finalSeen: true,
    status: "IDLE",
  }, {
    markerText: "ASSISTANT_STEP_19 TOOL_STEP_19 FINAL_MARKER",
    assistantMarkers: ["ASSISTANT_STEP_19"],
    toolMarkers: ["TOOL_STEP_19"],
    finalSeen: true,
    status: "IDLE",
  }], "FINAL_MARKER")

  assert.deepEqual(summary.assistantMarkersSeen, ["ASSISTANT_STEP_19", "ASSISTANT_STEP_20"])
  assert.deepEqual(summary.toolMarkersSeen, ["TOOL_STEP_19", "TOOL_STEP_20"])
  assert.equal(summary.finalSeen, true)
  assert.equal(summary.firstFinalSampleIndex, 1)
  assert.equal(summary.preFinalMaxAssistantMarkers, 1)
  assert.equal(summary.preFinalMaxToolMarkers, 1)
})

test("wait diagnostics omit full transcript payloads", () => {
  const diagnostic = sampleDiagnostic({
    at: "2026-08-07T12:00:00.000Z",
    surface: "kernel",
    status: "IDLE",
    text: "large transcript payload",
    markerText: "ASSISTANT_STEP_01",
    assistantMarkers: ["ASSISTANT_STEP_01"],
    toolMarkers: ["TOOL_STEP_01", "TOOL_STEP_02"],
    finalSeen: false,
    promptOccurrences: 1,
  })

  assert.deepEqual(diagnostic, {
    at: "2026-08-07T12:00:00.000Z",
    surface: "kernel",
    status: "IDLE",
    error: null,
    assistantMarkerCount: 1,
    toolMarkerCount: 2,
    finalSeen: false,
    promptOccurrences: 1,
  })
  assert.doesNotMatch(JSON.stringify(diagnostic), /large transcript payload/)
})

function pageEntry(kind, text) {
  return {
    entry_index: 1,
    entry: { kind, text },
  }
}

function element(className, textContent) {
  return {
    classList: new Set(className.split(" ")),
    textContent,
  }
}

function fakeWebPage(children) {
  return {
    async evaluate(callback, args) {
      class FakeHTMLElement {}
      const output = Object.assign(new FakeHTMLElement(), {
        textContent: children.map((child) => child.textContent).join("\n"),
        scrollHeight: 100,
        clientHeight: 100,
        scrollTop: 0,
        querySelectorAll(selector) {
          if (!selector.includes("freeform-agent-output")) return []
          return children.filter((child) =>
            child.classList.has("freeform-agent-output")
            && !child.classList.has("freeform-reasoning-output")
            && !child.classList.has("freeform-status-output")
            && !child.classList.has("freeform-error-output")
            && (!selector.includes(":not(.freeform-activity-group)") || !child.classList.has("freeform-activity-group")),
          )
        },
      })
      const badge = { textContent: "IDLE" }
      const previousDocument = globalThis.document
      const previousHTMLElement = globalThis.HTMLElement
      globalThis.document = {
        scrollingElement: output,
        querySelector(selector) {
          return selector === "[data-terminal-output]" ? output : null
        },
        querySelectorAll(selector) {
          if (selector === ".freeform-status-badge") return [badge]
          if (selector === ".freeform-user-prompt") {
            return children.filter((child) => child.classList.has("freeform-user-prompt"))
          }
          return []
        },
      }
      globalThis.HTMLElement = FakeHTMLElement
      try {
        return callback(args)
      } finally {
        globalThis.document = previousDocument
        globalThis.HTMLElement = previousHTMLElement
      }
    },
  }
}
