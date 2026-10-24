import type { TerminalTurnState } from "./runtime-state.js"
export function terminalTurnIsCompleted(
  turn: TerminalTurnState,
): turn is TerminalTurnState & { readonly completedAtMs: number } {
  return turn.completedAtMs !== undefined
}

export function terminalTurnExpandedForDisplay(turn: TerminalTurnState): boolean {
  return !terminalTurnIsCompleted(turn) || turn.expanded
}

export function terminalTurnCompletionSortValue(turn: TerminalTurnState): number {
  return Math.max(turn.completedAtMs ?? 0, turn.lastEntryAtMs, turn.startedAtMs)
}

export function hasTerminalTurnId(turnId: string | null | undefined): turnId is string {
  return turnId !== undefined && turnId !== null
}
