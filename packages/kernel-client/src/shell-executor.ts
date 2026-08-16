import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import { executeShellLocalCommand } from "./shell-local-command.js"
import { executeAgentCommand } from "./shell-agent-command.js"
import {
  executeEnvironmentCommand,
  executeConnectorCommand,
  executeExtensionCommand,
  executeMcpCommand,
  executeScriptCommand,
  executeSkillCommand,
} from "./shell-capability-command.js"
import { executeRecallCommand } from "./shell-recall-command.js"
import {
  executeConfigCommand,
  executeCredentialCommand,
} from "./shell-config-command.js"
import { executeContextCommand } from "./shell-context-command.js"
import {
  executeClientCommand,
  executeMachineCommand,
  executeRelayCommand,
} from "./shell-remote-command.js"
import { executeSessionCommand } from "./shell-session-command.js"
import { executeCloudCommand } from "./shell-cloud-command.js"
import { executeSliceCommand } from "./shell-slice-command.js"
import { executePromptCommand } from "./shell-prompt-command.js"
import { executeProviderCommand } from "./shell-provider-command.js"
import { executeKernelCommand } from "./shell-kernel-command.js"
import { executeStopCommand } from "./shell-stop-command.js"
import type { ShellPlacementDeps } from "./shell-placement.js"
import { executeWorkflowCommand } from "./shell-workflow-command.js"
import { executeWorkspaceCommand } from "./shell-workspace-command.js"
import { executeNotificationCommand } from "./shell-notification-command.js"
import { executePromptSettingsCommand } from "./shell-prompt-settings-command.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellExecutorDeps = ShellPlacementDeps & {
  client: ShellKernelClient
  clientId?: string | undefined
  readSecret?: ((prompt: string) => Promise<string>) | undefined
}

export async function executeShellCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  if (parsed.kind === "empty") {
    return { ok: true, message: "" }
  }
  if (parsed.kind === "invalid") {
    return { ok: false, message: parsed.reason ?? "invalid command" }
  }
  if (parsed.kind === "tui-only") {
    return { ok: false, message: parsed.reason ?? `${parsed.command ?? "command"} is only available in the TUI client` }
  }
  if (parsed.kind === "shell-local") {
    return executeShellLocalCommand(parsed, context)
  }
  switch (parsed.command) {
    case "session":
      return executeSessionCommand(parsed, context, deps)
    case "agent":
    case "agents":
      return executeAgentCommand(parsed, context, deps)
    case "kernel":
      return executeKernelCommand(parsed, context, deps)
    case "client":
      return executeClientCommand(parsed, deps)
    case "machine":
      return executeMachineCommand(parsed, deps)
    case "slice":
      return executeSliceCommand(parsed, context, deps)
    case "relay":
      return executeRelayCommand(parsed, deps)
    case "cloud":
      return executeCloudCommand(parsed, context, deps)
    case "config":
      return executeConfigCommand(parsed, deps)
    case "credential":
      return executeCredentialCommand(parsed, deps, context)
    case "mcp":
      return executeMcpCommand(parsed, context, deps)
    case "skill":
    case "skills":
      return executeSkillCommand(parsed, context, deps)
    case "env":
    case "environment":
      return executeEnvironmentCommand(parsed, context, deps)
    case "script":
      return executeScriptCommand(parsed, context, deps)
    case "connector":
      return executeConnectorCommand(parsed, context, deps)
    case "extension":
      return executeExtensionCommand(parsed, context, deps)
    case "workflow":
      return executeWorkflowCommand(parsed, context, deps)
    case "notifications":
      return executeNotificationCommand(parsed.args, deps.client)
    case "settings":
      if (parsed.args[0] !== "prompts") {
        return { ok: false, message: "usage: settings prompts list|reset <id> [--confirm]|reset-all [--confirm]" }
      }
      return executePromptSettingsCommand(parsed.args.slice(1), deps.client)
    case "workspace":
      return executeWorkspaceCommand(parsed, context, deps)
    case "recall":
      return executeRecallCommand(parsed, context, deps)
    case "prompt":
      return executePromptCommand(parsed, context, deps)
    case "stop":
    case "cancel":
      return executeStopCommand(parsed, context, deps)
    case "provider":
      return executeProviderCommand(parsed, context, deps)
    case "context":
      return executeContextCommand(context, deps)
    default:
      return {
        ok: false,
        message: `${parsed.command ?? "command"} is not implemented in chariox-shell yet`,
      }
  }
}
