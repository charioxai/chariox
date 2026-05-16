import type { BackendProviderId } from "./provider-catalog.js"
import type { ParsedProviderNamespaceCommand } from "./provider-command-catalog.js"

export type ProviderNamespaceSubmitDecision = {
  ok: true
  forwardedCommand: string
} | {
  ok: false
  message: string
}

export function validateProviderNamespaceSubmit(options: {
  command: ParsedProviderNamespaceCommand
  focusedProvider: BackendProviderId | null | undefined
  workflowScreenShowing: boolean
  pendingAttachmentCount: number
}): ProviderNamespaceSubmitDecision {
  const command = options.command
  if (command.provider !== options.focusedProvider) {
    return {
      ok: false,
      message: options.focusedProvider
        ? `${command.raw.split(/\s+/, 1)[0]} is unavailable while the focused agent uses ${options.focusedProvider}`
        : "provider-native commands require a focused OpenCode, Codex, or Claude Code agent",
    }
  }
  if (!command.forwardedCommand) {
    return {
      ok: false,
      message: `usage: ${command.raw} <provider-command>`,
    }
  }
  if (options.workflowScreenShowing) {
    return {
      ok: false,
      message: "provider-native commands are unavailable while the workflow screen owns the prompt",
    }
  }
  if (options.pendingAttachmentCount > 0) {
    return {
      ok: false,
      message: "provider-native commands do not support attachments",
    }
  }
  return {
    ok: true,
    forwardedCommand: command.forwardedCommand,
  }
}
