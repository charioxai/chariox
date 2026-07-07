import assert from "node:assert/strict"
import test from "node:test"

import type { TerminalOutputRecord } from "./cli-types.js"
import { createTerminalOutputRecordProcessor } from "./terminal-output-record-processor.js"
import {
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
} from "@arroba/kernel-client/external-provider-observation"

test("terminal output record processor stores prompt echoes before kernel processing", () => {
  const harness = processorHarness()
  const record = terminalRecord("prompt_echo", "hello")

  harness.processor.process(record)

  assert.deepEqual(harness.calls, [
    "history:hello",
    "kernel:prompt_echo",
  ])
  assert.equal(harness.records[0], record)
})

test("terminal output record processor forwards non-prompt records without history writes", () => {
  const harness = processorHarness()
  const record = terminalRecord("provider_output", "answer")

  harness.processor.process(record)

  assert.deepEqual(harness.calls, ["kernel:provider_output"])
  assert.equal(harness.records[0], record)
})

test("terminal output record processor follows shared prompt history projection", () => {
  const harness = processorHarness()
  const record = terminalRecord("prompt_echo", "token count", {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  })

  harness.processor.process(record)

  assert.deepEqual(harness.calls, ["kernel:prompt_echo"])
  assert.equal(harness.records[0], record)
})

function processorHarness() {
  const harness = {
    calls: [] as string[],
    records: [] as TerminalOutputRecord[],
    processor: null as ReturnType<typeof createTerminalOutputRecordProcessor> | null,
  }
  harness.processor = createTerminalOutputRecordProcessor({
    appendPromptEchoToSharedHistory: (text) => {
      harness.calls.push(`history:${text}`)
    },
    processKernelTerminalOutputRecord: (record) => {
      harness.records.push(record)
      harness.calls.push(`kernel:${record.kind}`)
    },
  })
  return harness as typeof harness & {
    processor: ReturnType<typeof createTerminalOutputRecordProcessor>
  }
}

function terminalRecord(
  kind: TerminalOutputRecord["kind"],
  text: string,
  fields: Partial<TerminalOutputRecord> = {},
): TerminalOutputRecord {
  return {
    kind,
    bytes: [...Buffer.from(text, "utf8")],
    ...fields,
    timestamp_ms: fields.timestamp_ms ?? 1,
  }
}
