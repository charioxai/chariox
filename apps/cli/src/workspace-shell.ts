export type WorkspaceShellEntry = {
  id: number
  command: string
  output: string
  ok: boolean
}

export function isWorkspaceShellCommand(input: string): boolean {
  return input.trimStart().startsWith("@")
}

export function workspaceShellCommandText(input: string): string {
  const trimmed = input.trimStart()
  return trimmed.startsWith("@") ? trimmed.slice(1).trim() : trimmed.trim()
}

export function appendWorkspaceShellEntry(
  entries: readonly WorkspaceShellEntry[],
  entry: WorkspaceShellEntry,
  limit = 80,
): WorkspaceShellEntry[] {
  return [...entries, entry].slice(-Math.max(1, limit))
}

export function renderWorkspaceShellTranscript(entries: readonly WorkspaceShellEntry[]): string {
  if (entries.length === 0) {
    return "@ help\nType @ <command> in the prompt below while the workflow screen is open."
  }
  return entries.map((entry) => {
    const prefix = entry.ok ? "" : "error: "
    const output = entry.output.trim() || (entry.ok ? "ok" : "error")
    return [`@ ${entry.command}`, `${prefix}${output}`].join("\n")
  }).join("\n\n")
}
