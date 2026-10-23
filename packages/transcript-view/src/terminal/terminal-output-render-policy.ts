import type {
  TerminalOutputEntry,
} from "./runtime-state.js"
import { terminalRuntimeNoticeIsTaskStatus } from "../terminal-runtime-notice.js"

type RenderableEntry = Pick<TerminalOutputEntry, "role" | "source" | "externalObservation" | "key" | "text">

export function terminalOutputEntryShouldRenderInAgentPane(
  entry: RenderableEntry,
): boolean {
  return entry.role !== "status"
    || (entry.source !== "local_terminal_status"
      && !entry.externalObservation
      && entry.key.startsWith("runtime-notice:")
      && terminalRuntimeNoticeIsTaskStatus(entry.text))
}

export function terminalOutputEntryShouldRenderInTurn(
  entry: RenderableEntry,
): boolean {
  return terminalOutputEntryShouldRenderInAgentPane(entry)
}
