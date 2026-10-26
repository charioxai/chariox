import type { TerminalOutputRecord } from "./cli-types.js"
import { terminalRecordPromptHistoryText } from "@chariox/kernel-client/terminal-record-transcript"
import { splitTerminalOutputSequences } from "@chariox/kernel-client/terminal-output-sequences"

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
    for (const fragment of splitTerminalOutputSequences(record)) {
      deps.processKernelTerminalOutputRecord(fragment)
    }
  }

  return {
    process,
  }
}
