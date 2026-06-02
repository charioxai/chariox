export type WorkspaceLiveSyncModeCommandInput = "off" | "managed" | "tracked"
export type WorkspaceLiveSyncModeInput = WorkspaceLiveSyncModeCommandInput | "unrestricted"
export type WorkspaceLiveSyncModeProtocolValue = "managed" | "tracked" | "unrestricted"
export type WorkspaceLiveSyncModeLabelInput = WorkspaceLiveSyncModeProtocolValue | null | undefined

export function parseWorkspaceLiveSyncModeCommand(value: string): WorkspaceLiveSyncModeCommandInput | null {
  if (value === "off" || value === "managed" || value === "tracked") return value
  return null
}

export function workspaceLiveSyncModeProtocolValue(
  mode: WorkspaceLiveSyncModeInput,
): WorkspaceLiveSyncModeProtocolValue {
  return mode === "off" ? "unrestricted" : mode
}

export function formatWorkspaceLiveSyncModeLabel(mode: WorkspaceLiveSyncModeLabelInput): string {
  if (mode === "managed" || mode === "tracked") {
    return `${mode} (selected workspace/worktree only; other repositories unrestricted)`
  }
  if (mode === "unrestricted") {
    return "off"
  }
  return "config default"
}

export function formatWorkspaceLiveSyncModeChangeMessage(
  mode: WorkspaceLiveSyncModeInput,
  options: { readonly action?: "set" | "enabled" } = {},
): string {
  if (mode === "off" || mode === "unrestricted") {
    return "current session workspace live sync disabled; other repositories remain unrestricted"
  }
  const label = formatWorkspaceLiveSyncModeLabel(mode)
  return options.action === "enabled"
    ? `current session workspace live sync enabled: ${label}`
    : `current session workspace live sync set to ${label}`
}

export function formatWorkspaceLiveSyncDefaultModeChangeMessage(mode: WorkspaceLiveSyncModeInput): string {
  if (mode === "off" || mode === "unrestricted") {
    return "default workspace live sync for new sessions disabled; other repositories remain unrestricted"
  }
  return `default workspace live sync for new sessions set to ${formatWorkspaceLiveSyncModeLabel(mode)}`
}
