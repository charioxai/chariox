export type WorkspaceLiveSyncModeCommandInput = "off" | "managed" | "tracked"
export type WorkspaceLiveSyncModeInput = WorkspaceLiveSyncModeCommandInput | "unrestricted"
export type WorkspaceLiveSyncModeProtocolValue = "managed" | "tracked" | "unrestricted"

export function parseWorkspaceLiveSyncModeCommand(value: string): WorkspaceLiveSyncModeCommandInput | null {
  if (value === "off" || value === "managed" || value === "tracked") return value
  return null
}

export function workspaceLiveSyncModeProtocolValue(
  mode: WorkspaceLiveSyncModeInput,
): WorkspaceLiveSyncModeProtocolValue {
  return mode === "off" ? "unrestricted" : mode
}
