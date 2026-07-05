import type { TerminalOutputRecord } from "./cli-types.js"
import { terminalRecordPromptHistoryText } from "@arroba/kernel-client/terminal-record-transcript"

export type TerminalOutputRecordProcessorDeps = {
  appendPromptEchoToSharedHistory: (text: string) => void
  processKernelTerminalOutputRecord: (record: TerminalOutputRecord) => void
}

export function createTerminalOutputRecordProcessor(
  deps: TerminalOutputRecordProcessorDeps,
) {
  const process = (record: TerminalOutputRecord) => {
    const text = Buffer.from(record.bytes).toString("utf8")
    const promptHistoryText = terminalRecordPromptHistoryText(record, text)
    if (promptHistoryText !== null) {
      deps.appendPromptEchoToSharedHistory(promptHistoryText)
    }
    deps.processKernelTerminalOutputRecord(record)
  }

  return {
    process,
  }
}
