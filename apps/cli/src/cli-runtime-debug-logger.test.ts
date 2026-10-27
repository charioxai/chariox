import { strict as assert } from "node:assert"
import test from "node:test"

import type { RuntimeProviderRun, TranscriptEntry } from "./cli-types.js"
import type { SessionFocusedStatusBadge } from "@chariox/kernel-client/session-runtime-status"
import { createCliRuntimeDebugLogger } from "./cli-runtime-debug-logger.js"

test("runtime debug logger gates view debug fields behind the debug flag", () => {
  const records: string[] = []
  createLogger(records, false).logView("state changed")
  assert.deepEqual(records, [])

  createLogger(records, true).logView("state changed", { extra: 1 })
  assert.deepEqual(records, [
    "debug:view debug: state changed:split:true:agent-1:true:1",
  ])
})

test("runtime debug logger projects provider run metadata", () => {
  const records: string[] = []
  const logger = createLogger(records, true)

  logger.logProviderRun("provider loaded", providerRun(), { source: "test" })

  assert.deepEqual(records, [
    "debug:provider loaded:opencode:gpt-5:running:test",
  ])
})

test("runtime debug logger filters transcript output roles and normalizes preview text", () => {
  const records: string[] = []
  const logger = createLogger(records, true)

  logger.logVisibleTranscriptOutput("user" as TranscriptEntry["role"], "ignored", false)
  logger.logVisibleTranscriptOutput("assistant", "hello\n\nworld", true, "merge-1")

  assert.deepEqual(records, [
    "info:applied visible transcript output:assistant:hello world:agent-1:agent-2:merge-1:true",
  ])
})

test("runtime debug logger emits focused badge changes once per state", () => {
  const records: string[] = []
  const logger = createLogger(records, true)
  const badge = focusedBadge("IDLE", "idle")

  logger.logFocusedBadgeChange(badge)
  logger.logFocusedBadgeChange(badge)
  logger.resetFocusedBadgeChange()
  logger.logFocusedBadgeChange(badge)
  logger.logFocusedBadgeChange(focusedBadge("WORKING", "working"))

  assert.deepEqual(records, [
    "info:focused status badge changed:IDLE:agent-1:agent-2",
    "info:focused status badge changed:IDLE:agent-1:agent-2",
    "info:focused status badge changed:WORKING:agent-1:agent-2",
  ])
})

function createLogger(records: string[], debugLogsEnabled: boolean) {
  return createCliRuntimeDebugLogger({
    logger: {
      debug(message, fields) {
        if (message === "view debug: state changed") {
          records.push(`debug:${message}:${fields?.layout}:${fields?.split_active}:${fields?.focused_agent_id}:${fields?.has_transcript_scrollbox}:${fields?.extra}`)
          return
        }
        records.push(`debug:${message}:${fields?.provider}:${fields?.provider_model}:${fields?.provider_state}:${fields?.source}`)
      },
      info(message, fields) {
        if (message === "applied visible transcript output") {
          records.push(`info:${message}:${fields?.role}:${fields?.preview}:${fields?.focused_agent_id}:${fields?.visible_agent_id}:${fields?.merge_key}:${fields?.merged}`)
          return
        }
        records.push(`info:${message}:${fields?.label}:${fields?.focused_agent_id}:${fields?.visible_agent_id}`)
      },
    },
    debugLogsEnabled,
    getResponseLayout: () => "split",
    splitAgentResponseMode: () => true,
    isAttached: () => true,
    getAgentCount: () => 2,
    getFocusedAgentId: () => "agent-1",
    hasTranscriptScrollbox: () => true,
    getVisibleTranscriptAgentId: () => "agent-2",
  })
}

function providerRun(): RuntimeProviderRun {
  return {
    id: "run-1",
    provider: "opencode",
    model: "gpt-5",
    variant: null,
    usage_tokens_total: 123,
    state: "running",
  } as unknown as RuntimeProviderRun
}

function focusedBadge(label: string, tone: "idle" | "working"): SessionFocusedStatusBadge {
  return {
    label,
    tone,
    parts: [{ label, tone }],
  }
}
