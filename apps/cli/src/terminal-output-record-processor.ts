import type { TerminalOutputRecord } from "./cli-types.js"

export type TerminalOutputRecordProcessorDeps = {
  appendPromptEchoToSharedHistory: (text: string) => void
  processKernelTerminalOutputRecord: (record: TerminalOutputRecord) => void
}

export function createTerminalOutputRecordProcessor(
  deps: TerminalOutputRecordProcessorDeps,
) {
  const process = (record: TerminalOutputRecord) => {
    if (record.kind === "prompt_echo") {
      deps.appendPromptEchoToSharedHistory(Buffer.from(record.bytes).toString("utf8"))
    }
    deps.processKernelTerminalOutputRecord(record)
  }

  return {
    process,
  }
}
