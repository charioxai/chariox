import { resolve as resolvePath } from "node:path"

import type {
  AgentInstance,
  ArrobaMcpServerConfig,
  ArrobaSkillMetadata,
  McpImportOutcome,
  PromptQueueItem,
  PromptSubmittedPayload,
  QueuedWorkflowLaunch,
  SliceDisplayEndpoint,
  SliceRecord,
  RuntimeSession,
  CloudCollaborator,
  CloudSessionMember,
  SessionInvite,
  SessionHistoryPage,
  SessionHistoryPageEntry,
  HistoryEvent,
  SemanticHistoryMatch,
  SessionConfigState,
  SessionMember,
  SkillImportOutcome,
  WorkflowDefinition,
  WorkflowEdgeDefinition,
  WorkflowEndpointDefinition,
  WorkflowNodeDefinition,
  WorkflowPublicationDefinition,
  WorkflowPublicationPairingCodeRecord,
  WorkflowPublicationSenderCredential,
  WorkflowPublicationTrustedSender,
  WorkflowRun,
  WorkflowWatchdogDefinition,
  WorkspaceLinkDefinition,
} from "./kernel-types.js"
import {
  addWorkflowEdgeRequest,
  addWorkflowNodeRequest,
  aliasAgentRequest,
  aliasWorkflowEndpointRequest,
  aliasWorkflowRequest,
  attachToSessionRequest,
  attachWorkspaceLinkRequest,
  bindWorkflowEndpointRequest,
  cancelActivePromptRequest,
  cancelWorkflowRunRequest,
  clearQueuedWorkflowLaunchesRequest,
  acceptCloudSessionInviteRequest,
  cloudRelayStatusRequest,
  createCloudSessionInviteRequest,
  type CreateWorkflowPublicationOptions,
  createSessionInviteRequest,
  createWorkspaceLinkRequest,
  createWorkflowEndpointRequest,
  createWorkflowPublicationPairCodeRequest,
  createWorkflowPublicationRequest,
  createWorkflowRequest,
  createWorkflowWatchdogRequest,
  createSessionRequest,
  createSliceRequest,
  deleteKernelRequest,
  deleteSliceRequest,
  focusAgentRequest,
  getMcpServerRequest,
  getSessionHistoryRequest,
  getSessionStateRequest,
  getSkillRequest,
  getSliceDisplayEndpointRequest,
  getSliceRequest,
  getWorkflowPublicationRequest,
  getWorkflowRunRequest,
  grantAgentCapabilityRequest,
  importMcpServersRequest,
  importSliceProviderAuthRequest,
  importSkillsRequest,
  invokeWorkflowEndpointRequest,
  installMcpServerRequest,
  installSkillRequest,
  joinSessionInviteRequest,
  listWorkflowPublicationSendersRequest,
  listCloudCollaboratorsRequest,
  listCloudSessionMembersRequest,
  getUserConfigRequest,
  getUserConfigSchemaRequest,
  detachWorkspaceLinkRequest,
  listAgentsRequest,
  listMcpServersRequest,
  listQueuedWorkflowLaunchesRequest,
  listSessionMembersRequest,
  listSessionsRequest,
  listSkillsRequest,
  listSlicesRequest,
  listWorkspaceLinksRequest,
  listWorkflowWatchdogsRequest,
  listWorkflowPublicationsRequest,
  listWorkflowRunsRequest,
  listWorkflowsRequest,
  launchProviderRunRequest,
  pumpTerminalOutputRequest,
  removeQueuedWorkflowLaunchRequest,
  removeWorkflowEdgeRequest,
  removeWorkflowNodeRequest,
  removeWorkflowWatchdogRequest,
  revokeAgentCapabilityRequest,
  redeemWorkflowPublicationPairCodeRequest,
  revokeSessionInviteRequest,
  revokeWorkflowPublicationSenderRequest,
  disableWorkflowPublicationRequest,
  resolveWorkflowRequest,
  resumeWorkflowRunRequest,
  resolveSessionRequest,
  startSliceRequest,
  setWorkflowFlushContextRequest,
  setWorkflowIntermediateOutputSchemaRequest,
  setWorkflowLaunchPolicyRequest,
  setWorkflowNodeCanCompleteRunRequest,
  setWorkflowNodeCanEmitIntermediateOutputRequest,
  setWorkflowNodeIntermediateOutputSchemaRequest,
  setWorkflowNodeMaxTurnsRequest,
  setWorkflowRunOutputSchemaRequest,
  setWorkflowWatchdogEnabledRequest,
  searchHistoryRequest,
  semanticSearchHistoryRequest,
  showWorkspaceLinkRequest,
  spawnAgentRequest,
  stopSliceRequest,
  submitPromptRequest,
  cycleAgentFocusRequest,
  uninstallMcpServerRequest,
  uninstallSkillRequest,
  updateAgentConfigRequest,
  updateAgentProfileRequest,
  updateAgentSubstitutesRequest,
  updateSessionConfigRequest,
  updateMcpServerRequest,
  updateSkillRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  formatAgentListSummary,
  formatAgentRef,
  formatAgentSubstituteSummary,
} from "./shell-agent-format.js"
import {
  formatAgentCapabilityGrants,
  formatMcpImportOutcome,
  formatMcpList,
  formatSkillImportOutcome,
  formatSkillList,
} from "./shell-capability-format.js"
import {
  formatHistoryEvents,
  formatPromptBlob,
  formatPromptReply,
  formatPromptSummary,
  formatSemanticHistoryMatches,
} from "./shell-history-format.js"
import {
  executeConfigCommand,
  executeCredentialCommand,
} from "./shell-config-command.js"
import {
  executeClientCommand,
  executeMachineCommand,
  executeRelayCommand,
} from "./shell-remote-command.js"
import {
  formatCloudCollaborators,
  formatCloudMembers,
  formatSessionInvite,
  formatSessionList,
  formatSessionMembers,
} from "./shell-session-format.js"
import { executeProviderCommand } from "./shell-provider-command.js"
import {
  parsePlacementOptions,
  resolveShellPlacement,
  type LocalGitWorktreeOptions,
  type ShellPlacementDeps,
} from "./shell-placement.js"
import { writeWorkflowPublicationExportPackage } from "./shell-workflow-publication-export.js"
import {
  formatQueuedWorkflowLaunches,
  formatWorkflowDetails,
  formatWorkflowLabel,
  formatWorkflowList,
  formatWorkflowPublicationLabel,
  formatWorkflowPublications,
  formatWorkflowPublicationSenders,
  formatWorkflowRunList,
  formatWorkflowWatchdogs,
} from "./shell-workflow-format.js"
import {
  formatWorkspaceLinkDetails,
  formatWorkspaceLinks,
} from "./shell-workspace-format.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellExecutorDeps = ShellPlacementDeps & {
  client: ShellKernelClient
  clientId?: string | undefined
  readSecret?: ((prompt: string) => Promise<string>) | undefined
  prepareLocalGitWorktree?: ((options: LocalGitWorktreeOptions) => Promise<string>) | undefined
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
      return executeCredentialCommand(parsed, deps)
    case "mcp":
      return executeMcpCommand(parsed, context, deps)
    case "skill":
      return executeSkillCommand(parsed, context, deps)
    case "workflow":
      return executeWorkflowCommand(parsed, context, deps)
    case "workspace":
      return executeWorkspaceCommand(parsed, context, deps)
    case "history":
      return executeHistoryCommand(parsed, context, deps)
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
        message: `${parsed.command ?? "command"} is not implemented in arroba-shell yet`,
      }
  }
}

function executeShellLocalCommand(parsed: ParsedShellCommand, context: ShellContext): ShellCommandResult {
  const [first, second] = parsed.args
  switch (parsed.command) {
    case "help":
      return {
        ok: true,
        message: [
          "arroba-shell commands:",
          "session list|new|attach|use|members|invite|join|mode|permissions",
          "kernel delete",
          "agent list|spawn|focus|cycle|mode|permissions|substitute",
          "client invite create|join|list|record|revoke",
          "machine invite create|join|list|kernels|approve|rename|revoke",
          "slice list|create|status|start|stop|delete|auth import|screen",
          "relay status",
          "config show|path|keys|schema|set|unset|managed-io",
          "credential list|set|delete",
          "mcp list|show|install|update|uninstall|import|grant|revoke|grants",
          "skill list|show|install|update|uninstall|import|grant|revoke|grants",
          "workspace link create|list|show|attach|detach",
          "workflow list|new|show|run|runs|cancel|resume|node|edge|endpoint",
          "history search|semantic-search",
          "prompt [agent-ref] <prompt> [--wait] [--show-reply|--show-summary]",
          "provider status|login|logout|reauth|processes",
          "stop",
          "context",
          "pwd",
          "set provider|model|effort <value>",
          "use session|agent|workflow <ref>",
          "vars",
          "unset <name>",
          "exit",
        ].join("\n"),
      }
    case "pwd":
      return { ok: true, message: context.worktree }
    case "set": {
      if (first !== "provider" && first !== "model" && first !== "effort") {
        return { ok: false, message: "usage: set provider|model|effort <value>" }
      }
      if (!second) {
        return { ok: false, message: `usage: set ${first} <value>` }
      }
      return {
        ok: true,
        message: `${first} = ${second}`,
        contextUpdates: { [first]: second },
      }
    }
    case "use": {
      if (first !== "session" && first !== "agent" && first !== "workflow") {
        return { ok: false, message: "usage: use session|agent|workflow <ref>" }
      }
      if (!second) {
        return { ok: false, message: `usage: use ${first} <ref>` }
      }
      const key = first === "session" ? "sessionId" : first === "agent" ? "agentId" : "workflowId"
      return {
        ok: true,
        message: `current ${first} = ${second}`,
        contextUpdates: { [key]: second },
      }
    }
    case "vars": {
      const entries = Object.entries(context.variables)
      return {
        ok: true,
        message: entries.length === 0
          ? "no variables bound"
          : entries.map(([name, value]) => `$${name} = ${value}`).join("\n"),
      }
    }
    case "unset": {
      if (!first) {
        return { ok: false, message: "usage: unset <name>" }
      }
      const nextVariables = { ...context.variables }
      delete nextVariables[first]
      return {
        ok: true,
        message: `unset $${first}`,
        variableRemovals: [first],
        data: { variables: nextVariables },
      }
    }
    case "exit":
    case "quit":
      return { ok: true, message: "exit", data: { exit: true } }
    case "source":
    case "run":
      return { ok: false, message: "script execution is not implemented yet" }
    default:
      return { ok: false, message: `${parsed.command ?? "command"} is not implemented in arroba-shell yet` }
  }
}

async function executeContextCommand(context: ShellContext, deps: ShellExecutorDeps): Promise<ShellCommandResult> {
  let session: RuntimeSession | null = null
  if (context.sessionId) {
    try {
      const response = await deps.client.send(getSessionStateRequest(context.sessionId))
      session = expectSessionState(response)
    } catch {
      session = null
    }
  }
  return { ok: true, message: formatShellContext(context, session), data: { context, session } }
}

function formatShellContext(context: ShellContext, session: RuntimeSession | null = null): string {
  const currentAgent = context.agentId
    ? session?.agents.find((agent) => agent.id === context.agentId || agent.agent_ref === context.agentId || agent.alias === context.agentId) ?? null
    : null
  const currentAgentId = currentAgent?.id ?? context.agentId ?? null
  const currentAgentBusy = currentAgentId
    ? Boolean(session?.prompt_states?.[currentAgentId]?.active_prompt)
      || Boolean(session?.prompt_states?.[currentAgentId]?.queued_prompts?.length)
      || Boolean(session?.active_prompt?.target_agent_id === currentAgentId)
      || Boolean(session?.queued_prompts?.some((prompt) => prompt.target_agent_id === currentAgentId))
      || Boolean(currentAgent?.is_processing)
    : false
  const agentLabel = currentAgent
    ? `${currentAgent.agent_ref}${currentAgent.alias ? ` (${currentAgent.alias})` : ""}${currentAgentBusy ? " (busy)" : ""}`
    : `${context.agentId ?? "-"}${currentAgentBusy ? " (busy)" : ""}`
  const sessionMode = parseExecutionMode(session?.config_state?.values?.["agents.mode"]) ?? "build"
  const sessionPermissions = parsePermissionLevel(session?.config_state?.values?.["agents.permissions"]) ?? "yolo"
  const effectiveAgentMode = currentAgent?.execution_mode_override ?? sessionMode
  const effectiveAgentPermissions = currentAgent?.permission_level_override ?? sessionPermissions
  const lines = [
    `workspace: ${context.workspace}`,
    `worktree: ${context.worktree}`,
    `session: ${context.sessionId ?? "-"}`,
    `attachment: ${context.attachmentId ?? "-"}`,
    `agent: ${agentLabel}`,
    `mode: ${currentAgent ? `${effectiveAgentMode} (agent${currentAgent.execution_mode_override ? "-override" : "-session"})` : sessionMode}`,
    `permissions: ${currentAgent ? `${effectiveAgentPermissions} (agent${currentAgent.permission_level_override ? "-override" : "-session"})` : sessionPermissions}`,
    `workflow: ${context.workflowId ?? "-"}`,
    `provider: ${context.provider}`,
    `model: ${context.model}`,
    `effort: ${context.effort}`,
  ]
  const variables = Object.entries(context.variables)
  if (variables.length === 0) {
    lines.push("vars: -")
  } else {
    lines.push("vars:")
    for (const [name, value] of variables) {
      lines.push(`  $${name} = ${value}`)
    }
  }
  return lines.join("\n")
}

async function executeSessionCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [action, ...args] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listSessionsRequest())
      const sessions = expectVariant<{ sessions: RuntimeSession[] }>(response, "SessionsListed").sessions
      return {
        ok: true,
        message: formatSessionList(sessions, context.sessionId),
        data: { sessions },
      }
    }
    case "new":
    case "create": {
      const placement = parsePlacementOptions(args, false)
      if (placement.error) {
        return { ok: false, message: placement.error }
      }
      if (placement.options.positional.length > 1) {
        return { ok: false, message: "usage: session new [directory] [--dir <directory>] [--worktree <directory> --branch <branch>]" }
      }
      const worktree = (await resolveShellPlacement(placement.options, context.worktree, "session working directory", deps))
        ?? context.worktree
      const response = await deps.client.send(createSessionRequest(context.workspace, worktree))
      const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionCreated")
      const session = payload.session
      const attachmentId = await attachShellSession(session.id, deps)
      const contextUpdates = {
        sessionId: session.id,
        ...(attachmentId ? { attachmentId } : {}),
        agentId: session.focused_agent_id ?? undefined,
        workspace: session.workspace_id,
        worktree: session.worktree_id,
      }
      return resourceResult(
        `created session ${session.alias ?? session.id} in ${session.worktree_id}`,
        parsed.assignment,
        session.id,
        contextUpdates,
        { session },
      )
    }
    case "attach":
    case "use": {
      const sessionRef = args[0]
      if (!sessionRef) {
        return { ok: false, message: `usage: session ${action} <ref>` }
      }
      const response = await deps.client.send(resolveSessionRequest(sessionRef, context.workspace))
      const session = expectVariant<{ session: RuntimeSession }>(response, "SessionResolved").session
      const attachmentId = await attachShellSession(session.id, deps)
      const contextUpdates = {
        sessionId: session.id,
        ...(attachmentId ? { attachmentId } : context.attachmentId ? { attachmentId: context.attachmentId } : {}),
        agentId: session.focused_agent_id ?? undefined,
        workspace: session.workspace_id,
        worktree: session.worktree_id,
      }
      return resourceResult(
        `current session = ${session.alias ?? session.id}`,
        parsed.assignment,
        session.id,
        contextUpdates,
        { session },
      )
    }
    case "mode": {
      if (!context.sessionId) {
        return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
      }
      const nextMode = parseExecutionMode(args[0])
      if (!args[0]) {
        const response = await deps.client.send(getSessionStateRequest(context.sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        return {
          ok: true,
          message: `session mode = ${parseExecutionMode(session.config_state?.values?.["agents.mode"]) ?? "build"}`,
          data: { session },
        }
      }
      if (!nextMode) {
        return { ok: false, message: "usage: session mode <build|plan>" }
      }
      const attachmentId = await resolveShellAttachmentId(context, deps)
      if (!attachmentId.ok) {
        return { ok: false, message: attachmentId.message }
      }
      const response = await deps.client.send(
        updateSessionConfigRequest(context.sessionId, attachmentId.attachmentId, { "agents.mode": nextMode }, false),
      )
      const payload = expectVariant<{ session: RuntimeSession; config: SessionConfigState }>(response, "SessionConfigUpdated")
      return {
        ok: true,
        message: `session mode = ${nextMode}`,
        data: payload,
      }
    }
    case "permissions": {
      if (!context.sessionId) {
        return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
      }
      const nextLevel = parsePermissionLevel(args[0])
      if (!args[0]) {
        const response = await deps.client.send(getSessionStateRequest(context.sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        return {
          ok: true,
          message: `session permissions = ${parsePermissionLevel(session.config_state?.values?.["agents.permissions"]) ?? "yolo"}`,
          data: { session },
        }
      }
      if (!nextLevel) {
        return { ok: false, message: "usage: session permissions <required|yolo>" }
      }
      const attachmentId = await resolveShellAttachmentId(context, deps)
      if (!attachmentId.ok) {
        return { ok: false, message: attachmentId.message }
      }
      const response = await deps.client.send(
        updateSessionConfigRequest(context.sessionId, attachmentId.attachmentId, { "agents.permissions": nextLevel }, false),
      )
      const payload = expectVariant<{ session: RuntimeSession; config: SessionConfigState }>(response, "SessionConfigUpdated")
      return {
        ok: true,
        message: `session permissions = ${nextLevel}`,
        data: payload,
      }
    }
    case "members": {
      const sessionId = args[0] ?? context.sessionId
      if (!sessionId) {
        return { ok: false, message: "usage: session members [session-ref]" }
      }
      const response = await deps.client.send(listSessionMembersRequest(sessionId))
      const payload = expectVariant<{ members: SessionMember[]; invites: SessionInvite[] }>(response, "SessionMembersListed")
      return { ok: true, message: formatSessionMembers(payload.members, payload.invites), data: payload }
    }
    case "invite": {
      const [inviteAction, maxUsesRaw] = args
      if (inviteAction !== "create") {
        return { ok: false, message: "usage: session invite create [max-uses]" }
      }
      if (!context.sessionId) {
        return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
      }
      const maxUses = maxUsesRaw ? Number.parseInt(maxUsesRaw, 10) : 1
      if (!Number.isFinite(maxUses) || maxUses <= 0) {
        return { ok: false, message: "usage: session invite create [max-uses]" }
      }
      const response = await deps.client.send(createSessionInviteRequest(context.sessionId, null, maxUses))
      const payload = expectVariant<{ invite: { invite: SessionInvite; invite_token: string }; session: RuntimeSession }>(response, "SessionInviteCreated")
      return {
        ok: true,
        message: formatSessionInvite(payload.invite.invite, payload.invite.invite_token),
        data: payload,
        contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
      }
    }
    case "join": {
      const [inviteToken, userId] = args
      if (!inviteToken || !userId) {
        return { ok: false, message: "usage: session join <invite-token> <user-id>" }
      }
      const response = await deps.client.send(joinSessionInviteRequest(inviteToken, userId))
      const payload = expectVariant<{ member: SessionMember; session: RuntimeSession }>(response, "SessionInviteJoined")
      const attachmentId = await attachShellSession(payload.session.id, deps)
      return {
        ok: true,
        message: `joined session ${payload.session.alias ?? payload.session.id} as ${payload.member.user_id}`,
        data: payload,
        contextUpdates: {
          sessionId: payload.session.id,
          ...(attachmentId ? { attachmentId } : {}),
          agentId: payload.session.focused_agent_id ?? undefined,
          workspace: payload.session.workspace_id,
          worktree: payload.session.worktree_id,
        },
      }
    }
    case "revoke-invite": {
      const inviteRef = args[0]
      if (!context.sessionId || !inviteRef) {
        return { ok: false, message: "usage: session revoke-invite <invite-id>" }
      }
      const response = await deps.client.send(revokeSessionInviteRequest(context.sessionId, inviteRef))
      const payload = expectVariant<{ invite: SessionInvite; session: RuntimeSession }>(response, "SessionInviteRevoked")
      return {
        ok: true,
        message: `revoked session invite ${payload.invite.invite_id}`,
        data: payload,
        contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
      }
    }
    default:
      return { ok: false, message: "usage: session list|new|attach|use|members|invite|join|revoke-invite|mode|permissions" }
  }
}

async function executeWorkspaceCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [resource, action, ...args] = parsed.args
  if (resource !== "link") {
    return { ok: false, message: "usage: workspace link create|list|show|attach|detach" }
  }
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }
  switch (action) {
    case "create":
    case "new": {
      const name = args[0]
      if (!name) {
        return { ok: false, message: "usage: workspace link create <name>" }
      }
      const response = await deps.client.send(createWorkspaceLinkRequest(sessionId, name))
      const payload = expectVariant<{ link: WorkspaceLinkDefinition; session: RuntimeSession }>(response, "WorkspaceLinkCreated")
      return resourceResult(
        `created workspace link ${payload.link.name} (${payload.link.link_id})`,
        parsed.assignment,
        payload.link.link_id,
        { sessionId: payload.session.id },
        payload,
      )
    }
    case undefined:
    case "list":
    case "ls": {
      const response = await deps.client.send(listWorkspaceLinksRequest(sessionId))
      const payload = expectVariant<{ links: WorkspaceLinkDefinition[] }>(response, "WorkspaceLinksListed")
      return { ok: true, message: formatWorkspaceLinks(payload.links), data: payload }
    }
    case "show": {
      const linkRef = args[0]
      if (!linkRef) {
        return { ok: false, message: "usage: workspace link show <name-or-id>" }
      }
      const response = await deps.client.send(showWorkspaceLinkRequest(sessionId, linkRef))
      const payload = expectVariant<{ link: WorkspaceLinkDefinition }>(response, "WorkspaceLinkShown")
      return { ok: true, message: formatWorkspaceLinkDetails(payload.link), data: payload }
    }
    case "attach": {
      const linkRef = args[0]
      const repoRoot = args[1] ? resolvePath(context.worktree, args[1]) : context.worktree
      if (!linkRef) {
        return { ok: false, message: "usage: workspace link attach <name-or-id> [repo-root]" }
      }
      const response = await deps.client.send(attachWorkspaceLinkRequest(sessionId, linkRef, repoRoot))
      const payload = expectVariant<{ link: WorkspaceLinkDefinition; session: RuntimeSession }>(response, "WorkspaceLinkAttached")
      return {
        ok: true,
        message: `attached ${repoRoot} to workspace link ${payload.link.name}`,
        data: payload,
        contextUpdates: { sessionId: payload.session.id },
      }
    }
    case "detach": {
      const linkRef = args[0]
      const repoRoot = args[1] ? resolvePath(context.worktree, args[1]) : context.worktree
      if (!linkRef) {
        return { ok: false, message: "usage: workspace link detach <name-or-id> [repo-root]" }
      }
      const response = await deps.client.send(detachWorkspaceLinkRequest(sessionId, linkRef, repoRoot))
      const payload = expectVariant<{ link: WorkspaceLinkDefinition; detached: unknown[]; session: RuntimeSession }>(response, "WorkspaceLinkDetached")
      return {
        ok: true,
        message: `detached ${payload.detached.length} workspace link attachment${payload.detached.length === 1 ? "" : "s"} from ${payload.link.name}`,
        data: payload,
        contextUpdates: { sessionId: payload.session.id },
      }
    }
    default:
      return { ok: false, message: "usage: workspace link create|list|show|attach|detach" }
  }
}

async function executeKernelCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [action, ...args] = parsed.args
  if (action !== "delete" || args.length > 0) {
    return { ok: false, message: "usage: kernel delete" }
  }
  const response = await deps.client.send(deleteKernelRequest())
  const payload = expectVariant<{ kernel_id: string; deleted_sessions: RuntimeSession[] }>(response, "KernelDeleted")
  const deletedCurrentSession = context.sessionId
    ? payload.deleted_sessions.some((session) => session.id === context.sessionId)
    : false
  return {
    ok: true,
    message: `deleted kernel ${payload.kernel_id} (${payload.deleted_sessions.length} session${payload.deleted_sessions.length === 1 ? "" : "s"})`,
    contextUpdates: deletedCurrentSession
      ? { sessionId: undefined, attachmentId: undefined, agentId: undefined }
      : undefined,
    data: payload,
  }
}

async function executeAgentCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }

  const [action, ...args] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listAgentsRequest(sessionId))
      const agents = expectVariant<{ agents: AgentInstance[] }>(response, "AgentsListed").agents
      return { ok: true, message: formatAgentListSummary(agents), data: { agents } }
    }
    case "spawn": {
      const parsedSpawn = parsePlacementOptions(args, true)
      if (parsedSpawn.error) {
        return { ok: false, message: parsedSpawn.error }
      }
      const [alias, model] = parsedSpawn.options.positional
      if (parsedSpawn.options.positional.length > 2) {
        return { ok: false, message: "usage: agent spawn [alias] [model] [--dir <directory>] [--worktree <directory> --branch <branch>] [--kernel <kernel-ref>|--slice <slice-ref>]" }
      }
      if (parsedSpawn.options.kernelRef && (parsedSpawn.options.directory || parsedSpawn.options.gitWorktree || parsedSpawn.options.branch || parsedSpawn.options.fromRef)) {
        return { ok: false, message: "usage: agent spawn [alias] [model] --kernel <kernel-ref> uses the worker kernel default directory" }
      }
      if (parsedSpawn.options.sliceRef && (parsedSpawn.options.directory || parsedSpawn.options.gitWorktree || parsedSpawn.options.branch || parsedSpawn.options.fromRef)) {
        return { ok: false, message: "usage: agent spawn [alias] [model] --slice <slice-ref> uses the slice worker default directory" }
      }
      const worktree = await resolveShellPlacement(parsedSpawn.options, context.worktree, "agent working directory", deps)
      const response = await deps.client.send(spawnAgentRequest(
        sessionId,
        context.provider,
        alias,
        model ?? context.model,
        worktree,
        context.effort,
        undefined,
        undefined,
        parsedSpawn.options.kernelRef,
        undefined,
        parsedSpawn.options.sliceRef,
      ))
      const agent = expectVariant<{ agent: AgentInstance }>(response, "AgentSpawned").agent
      const placement = agent.remote_execution
        ? parsedSpawn.options.sliceRef
          ? ` in slice ${parsedSpawn.options.sliceRef}`
          : ` on ${parsedSpawn.options.kernelRef ?? agent.remote_execution.worker_machine_id}`
        : agent.worktree_id ? ` in ${agent.worktree_id}` : ""
      return resourceResult(
        `spawned agent ${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}${placement}`,
        parsed.assignment,
        agent.id,
        { agentId: agent.id },
        { agent },
      )
    }
    case "focus": {
      const agentRef = args[0]
      if (!agentRef) {
        return { ok: false, message: "usage: agent focus <agent-id>" }
      }
      const response = await deps.client.send(focusAgentRequest(sessionId, agentRef))
      const agent = expectVariant<{ agent: AgentInstance }>(response, "AgentFocused").agent
      return resourceResult(
        `current agent = ${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`,
        parsed.assignment,
        agent.id,
        { agentId: agent.id },
        { agent },
      )
    }
    case "cycle": {
      const response = await deps.client.send(cycleAgentFocusRequest(sessionId))
      const agent = expectVariant<{ agent: AgentInstance | null }>(response, "AgentFocusCycled").agent
      if (!agent) {
        return { ok: true, message: "no agents to cycle" }
      }
      return resourceResult(
        `current agent = ${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`,
        parsed.assignment,
        agent.id,
        { agentId: agent.id },
        { agent },
      )
    }
    case "alias":
    case "name": {
      const reference = args.length > 1 ? args[0] : context.agentId
      const rawAlias = (args.length > 1 ? args.slice(1) : args).join(" ").trim()
      const resolved = await resolveShellAgent(context, deps, reference)
      if (!resolved.ok) {
        return { ok: false, message: args[0] ? resolved.message : "usage: agent alias [agent-ref] <alias|clear>" }
      }
      if (!rawAlias) {
        return { ok: true, message: `${formatAgentRef(resolved.agent)} alias = ${resolved.agent.alias ?? "<none>"}`, data: { agent: resolved.agent } }
      }
      const shouldClearAgentAlias = rawAlias === "clear" || rawAlias === "none" || rawAlias === "-"
      const response = await deps.client.send(aliasAgentRequest(
        sessionId,
        resolved.agent.id,
        shouldClearAgentAlias ? "" : rawAlias,
      ))
      const payload = expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentAliased")
      return { ok: true, message: `${formatAgentRef(payload.agent)} alias = ${payload.agent.alias ?? "<none>"}`, data: payload }
    }
    case "provider":
    case "model":
    case "variant": {
      const resolved = await resolveShellAgent(context, deps, args.length > 1 ? args[0] : context.agentId)
      const rawValue = args.length > 1 ? args.slice(1).join(" ").trim() : args.join(" ").trim()
      if (!resolved.ok) {
        return { ok: false, message: args[0] ? resolved.message : `usage: agent ${action} [agent-ref] <value>` }
      }
      if (!rawValue) {
        const response = await deps.client.send(getSessionStateRequest(sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        const agent = session.agents.find((entry) => entry.id === resolved.agent.id) ?? resolved.agent
        const value = action === "provider"
          ? agent.provider
          : action === "model"
            ? agent.model ?? "<none>"
            : agent.effort ?? "<none>"
        return { ok: true, message: `${formatAgentRef(agent)} ${action} = ${value}`, data: { session, agent } }
      }
      const shouldClearEffort = action === "variant" && ["clear", "none", "-", "default"].includes(rawValue)
      const response = await deps.client.send(updateAgentProfileRequest({
        sessionId,
        agentId: resolved.agent.id,
        ...(action === "provider" ? { provider: rawValue } : {}),
        ...(action === "model" ? { model: rawValue } : {}),
        ...(action === "variant" && !shouldClearEffort ? { effort: rawValue } : {}),
        ...(shouldClearEffort ? { clearEffort: true } : {}),
      }))
      const payload = expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentProfileUpdated")
      const value = action === "provider"
        ? payload.agent.provider
        : action === "model"
          ? payload.agent.model ?? "<none>"
          : payload.agent.effort ?? "<none>"
      return { ok: true, message: `${formatAgentRef(payload.agent)} ${action} = ${value}`, data: payload }
    }
    case "mode": {
      const firstArgIsMode = args[0] === "inherit" || parseExecutionMode(args[0]) != null
      const resolved = await resolveShellAgent(context, deps, firstArgIsMode ? context.agentId : args[0])
      if (!resolved.ok) {
        return { ok: false, message: args[0] ? resolved.message : "usage: agent mode [agent-ref] <build|plan|inherit>" }
      }
      const rawValue = args.length > 1 ? args[1] : firstArgIsMode ? args[0] : undefined
      if (!rawValue) {
        const response = await deps.client.send(getSessionStateRequest(sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        const agent = session.agents.find((entry) => entry.id === resolved.agent.id) ?? resolved.agent
        const sessionMode = parseExecutionMode(session.config_state?.values?.["agents.mode"]) ?? "build"
        const effectiveMode = agent.execution_mode_override ?? sessionMode
        const source = agent.execution_mode_override ? "agent" : "session"
        return { ok: true, message: `${formatAgentRef(agent)} mode = ${effectiveMode} (${source})`, data: { session, agent } }
      }
      if (rawValue !== "inherit" && !parseExecutionMode(rawValue)) {
        return { ok: false, message: "usage: agent mode [agent-ref] <build|plan|inherit>" }
      }
      const response = await deps.client.send(updateAgentConfigRequest({
        sessionId,
        agentId: resolved.agent.id,
        executionMode: rawValue === "inherit" ? null : parseExecutionMode(rawValue),
        clearExecutionMode: rawValue === "inherit",
      }))
      const payload = expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentConfigUpdated")
      const sessionMode = parseExecutionMode(payload.session.config_state?.values?.["agents.mode"]) ?? "build"
      const effectiveMode = payload.agent.execution_mode_override ?? sessionMode
      return { ok: true, message: `${formatAgentRef(payload.agent)} mode = ${effectiveMode}${rawValue === "inherit" ? " (session)" : " (agent)"}`, data: payload }
    }
    case "permissions": {
      const firstArgIsPermission = args[0] === "inherit" || parsePermissionLevel(args[0]) != null
      const resolved = await resolveShellAgent(context, deps, firstArgIsPermission ? context.agentId : args[0])
      if (!resolved.ok) {
        return { ok: false, message: args[0] ? resolved.message : "usage: agent permissions [agent-ref] <required|yolo|inherit>" }
      }
      const rawValue = args.length > 1 ? args[1] : firstArgIsPermission ? args[0] : undefined
      if (!rawValue) {
        const response = await deps.client.send(getSessionStateRequest(sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        const agent = session.agents.find((entry) => entry.id === resolved.agent.id) ?? resolved.agent
        const sessionLevel = parsePermissionLevel(session.config_state?.values?.["agents.permissions"]) ?? "yolo"
        const effectiveLevel = agent.permission_level_override ?? sessionLevel
        const source = agent.permission_level_override ? "agent" : "session"
        return { ok: true, message: `${formatAgentRef(agent)} permissions = ${effectiveLevel} (${source})`, data: { session, agent } }
      }
      if (rawValue !== "inherit" && !parsePermissionLevel(rawValue)) {
        return { ok: false, message: "usage: agent permissions [agent-ref] <required|yolo|inherit>" }
      }
      const response = await deps.client.send(updateAgentConfigRequest({
        sessionId,
        agentId: resolved.agent.id,
        permissionLevel: rawValue === "inherit" ? null : parsePermissionLevel(rawValue),
        clearPermissionLevel: rawValue === "inherit",
      }))
      const payload = expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentConfigUpdated")
      const sessionLevel = parsePermissionLevel(payload.session.config_state?.values?.["agents.permissions"]) ?? "yolo"
      const effectiveLevel = payload.agent.permission_level_override ?? sessionLevel
      return { ok: true, message: `${formatAgentRef(payload.agent)} permissions = ${effectiveLevel}${rawValue === "inherit" ? " (session)" : " (agent)"}`, data: payload }
    }
    case "substitute":
    case "subs":
      return executeAgentSubstituteCommand(args, context, deps, sessionId)
    default:
      return { ok: false, message: "usage: agent list|spawn|focus|cycle|alias|provider|model|variant|mode|permissions|substitute" }
  }
}

async function executeAgentSubstituteCommand(
  args: string[],
  context: ShellContext,
  deps: ShellExecutorDeps,
  sessionId: string,
): Promise<ShellCommandResult> {
  const [subcommand = "list", ...rawArgs] = args
  const agentFlagIndex = rawArgs.indexOf("--agent")
  const agentRefFromFlag = agentFlagIndex >= 0 ? rawArgs[agentFlagIndex + 1] : undefined
  const filteredArgs = agentFlagIndex >= 0
    ? rawArgs.filter((_, index) => index !== agentFlagIndex && index !== agentFlagIndex + 1)
    : rawArgs
  const resolved = await resolveShellAgent(context, deps, agentRefFromFlag)
  if (!resolved.ok) {
    return { ok: false, message: resolved.message }
  }
  const agent = resolved.agent
  if (subcommand === "list" || subcommand === "ls") {
    return { ok: true, message: formatAgentSubstituteSummary(agent), data: { agent } }
  }
  const update = async (action: Record<string, unknown>) => {
    const response = await deps.client.send(updateAgentSubstitutesRequest({
      sessionId,
      agentId: agent.id,
      action: action as never,
    }))
    return expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentConfigUpdated")
  }
  if (subcommand === "add") {
    const provider = filteredArgs[0]
    const model = filteredArgs[1]
    const variantIndex = filteredArgs.indexOf("--variant")
    const variant = variantIndex >= 0 ? filteredArgs[variantIndex + 1] : undefined
    const kernelIndex = filteredArgs.indexOf("--kernel")
    const kernelId = kernelIndex >= 0 ? filteredArgs[kernelIndex + 1] : undefined
    const worktreeIndex = filteredArgs.indexOf("--worktree")
    const worktreeId = worktreeIndex >= 0 ? filteredArgs[worktreeIndex + 1] : undefined
    if (!provider || !model) {
      return { ok: false, message: "usage: agent substitute add <provider> <model> [--variant v] [--kernel k] [--worktree dir] [--agent a]" }
    }
    const payload = await update({
      Add: {
        provider,
        model,
        variant: variant ?? null,
        kernel_id: kernelId ?? null,
        worktree_id: worktreeId ?? null,
      },
    })
    return { ok: true, message: `${formatAgentRef(payload.agent)} substitute added: ${provider}/${model}${variant ? `/${variant}` : ""}`, data: payload }
  }
  if (subcommand === "remove" || subcommand === "rm") {
    const index = Number.parseInt(filteredArgs[0] ?? "", 10)
    if (!Number.isFinite(index)) {
      return { ok: false, message: "usage: agent substitute remove <index> [--agent a]" }
    }
    const payload = await update({ Remove: { index } })
    return { ok: true, message: `${formatAgentRef(payload.agent)} substitute ${index} removed`, data: payload }
  }
  if (subcommand === "clear") {
    const payload = await update({ Clear: {} })
    return { ok: true, message: `${formatAgentRef(payload.agent)} substitutes cleared`, data: payload }
  }
  if (subcommand === "timeout") {
    const timeoutMs = parseSubstitutionTimeoutMs(filteredArgs[0])
    if (timeoutMs === undefined && filteredArgs[0] !== "inherit" && filteredArgs[0] !== "default") {
      return { ok: false, message: "usage: agent substitute timeout <ms|Ns|inherit> [--agent a]" }
    }
    const payload = await update({ SetTimeout: { timeout_ms: timeoutMs ?? null } })
    return { ok: true, message: `${formatAgentRef(payload.agent)} substitute timeout: ${timeoutMs == null ? "default" : `${timeoutMs}ms`}`, data: payload }
  }
  if (subcommand === "activate") {
    const index = Number.parseInt(filteredArgs[0] ?? "", 10)
    if (!Number.isFinite(index)) {
      return { ok: false, message: "usage: agent substitute activate <index> [--agent a]" }
    }
    const payload = await update({ Activate: { index, reason: "manual" } })
    const profile = payload.agent.substitutes?.[index]
    if (!profile) {
      return { ok: false, message: `${formatAgentRef(payload.agent)} substitute ${index} is not available`, data: payload }
    }
    const response = await deps.client.send(launchProviderRunRequest(
      sessionId,
      profile.provider,
      "default",
      profile.model,
      profile.variant ?? "",
      payload.agent.id,
    ))
    return { ok: true, message: `${formatAgentRef(payload.agent)} activated substitute ${index}: ${profile.provider}/${profile.model}`, data: { ...payload, launch: response }, contextUpdates: { agentId: payload.agent.id } }
  }
  if (subcommand === "primary") {
    const payload = await update({ Primary: {} })
    const response = await deps.client.send(launchProviderRunRequest(
      sessionId,
      payload.agent.provider,
      "default",
      payload.agent.model ?? context.model,
      payload.agent.effort ?? context.effort,
      payload.agent.id,
    ))
    return { ok: true, message: `${formatAgentRef(payload.agent)} returned to primary profile`, data: { ...payload, launch: response }, contextUpdates: { agentId: payload.agent.id } }
  }
  return { ok: false, message: "usage: agent substitute list|add|remove|clear|timeout|activate|primary" }
}

type ParsedPromptArgs = {
  agentRef?: string | undefined
  prompt: string
  wait: boolean
  showReply: boolean
  showSummary: boolean
}

async function executePromptCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  if (!context.sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }
  const attachmentId = await resolveShellAttachmentId(context, deps)
  if (!attachmentId.ok) {
    return { ok: false, message: attachmentId.message }
  }
  const promptArgs = await parsePromptArgs(parsed.args, context, deps)
  if (!promptArgs.ok) {
    return { ok: false, message: promptArgs.message }
  }
  const target = promptArgs.agent
  const promptText = promptArgs.options.prompt.endsWith("\n")
    ? promptArgs.options.prompt
    : `${promptArgs.options.prompt}\n`
  const response = await deps.client.send(submitPromptRequest(
    context.sessionId,
    attachmentId.attachmentId,
    target.id,
    promptText,
    [],
  ))
  const payload = expectVariant<PromptSubmittedPayload>(response, "PromptSubmitted")
  const prompt = extractSubmittedPrompt(payload, target.id)
  const promptId = prompt?.id ?? "unknown-prompt"
  const waitForCompletion = promptArgs.options.wait || promptArgs.options.showReply || promptArgs.options.showSummary
  if (!waitForCompletion) {
    return {
      ok: true,
      message: `prompt ${promptId} submitted to ${formatAgentRef(target)}`,
      data: { prompt, session: payload.session },
      contextUpdates: { agentId: target.id },
    }
  }

  const completedSession = await waitForPromptCompletion(context.sessionId, attachmentId.attachmentId, target.id, promptId, deps)
  const history = await readPromptHistory(context.sessionId, target.id, promptText, deps)
  const lines = [`prompt ${promptId} completed`]
  if (promptArgs.options.showReply) {
    lines.push(formatPromptBlob(promptId, "reply", formatPromptReply(history)))
  } else if (promptArgs.options.showSummary) {
    lines.push(formatPromptBlob(promptId, "summary", formatPromptSummary(history)))
  }
  return {
    ok: true,
    message: lines.join("\n"),
    data: { prompt, session: completedSession, history },
    contextUpdates: { agentId: target.id },
  }
}

async function parsePromptArgs(
  args: string[],
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<
  | { ok: true; agent: AgentInstance; options: ParsedPromptArgs }
  | { ok: false; message: string }
> {
  const positional: string[] = []
  let wait = false
  let showReply = false
  let showSummary = false
  for (const arg of args) {
    const normalized = normalizeShellFlag(arg)
    if (normalized === "--wait") {
      wait = true
    } else if (normalized === "--show-reply") {
      showReply = true
    } else if (normalized === "--show-summary") {
      showSummary = true
    } else {
      positional.push(arg)
    }
  }
  if (showReply && showSummary) {
    return { ok: false, message: "usage: prompt [agent-ref] <prompt> [--wait] [--show-reply|--show-summary]" }
  }
  if (positional.length === 0) {
    return { ok: false, message: "usage: prompt [agent-ref] <prompt> [--wait] [--show-reply|--show-summary]" }
  }

  let agentRef: string | undefined
  let promptParts = positional
  if (positional.length > 1) {
    const explicitAgent = await tryResolveShellAgent(context, deps, positional[0])
    if (explicitAgent.ok) {
      agentRef = positional[0]
      promptParts = positional.slice(1)
      return {
        ok: true,
        agent: explicitAgent.agent,
        options: { agentRef, prompt: promptParts.join(" "), wait, showReply, showSummary },
      }
    }
  }
  const defaultAgent = await resolveShellAgent(context, deps, undefined)
  if (!defaultAgent.ok) {
    return { ok: false, message: defaultAgent.message.replace("usage: mcp|skill grants <agent-ref>", "usage: prompt [agent-ref] <prompt>") }
  }
  return {
    ok: true,
    agent: defaultAgent.agent,
    options: { prompt: promptParts.join(" "), wait, showReply, showSummary },
  }
}

function normalizeShellFlag(value: string): string {
  return value.startsWith("—") ? `--${value.slice(1)}` : value
}

async function executeHistoryCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [action, ...rest] = parsed.args
  if (action !== "search" && action !== "semantic-search") {
    return { ok: false, message: "usage: history search <query> | history semantic-search [--agent] <query>" }
  }
  const semanticMode = action === "semantic-search" && rest[0] === "--agent" ? "agent" : "knn"
  const queryArgs = semanticMode === "agent" ? rest.slice(1) : rest
  const query = queryArgs.join(" ").trim()
  if (!query) {
    return { ok: false, message: `usage: history ${action} <query>` }
  }
  const filters = {
    session_id: context.sessionId ?? null,
    limit: action === "semantic-search" ? 20 : 50,
  }
  if (action === "semantic-search") {
    const response = await deps.client.send(semanticSearchHistoryRequest(query, { ...filters, mode: semanticMode }))
    const payload = expectVariant<{
      results?: SemanticHistoryMatch[]
      unavailable_reason?: string | null
      answer?: string | null
    }>(response, "SemanticHistoryEvents")
    const unavailable = payload.unavailable_reason?.trim()
    if (unavailable) {
      return { ok: false, message: unavailable }
    }
    return {
      ok: true,
      message: [payload.answer?.trim(), formatSemanticHistoryMatches(payload.results ?? [])].filter(Boolean).join("\n\n"),
      format: "text",
    }
  }
  const response = await deps.client.send(searchHistoryRequest(query, filters))
  const payload = expectVariant<{ events?: HistoryEvent[] }>(response, "HistoryEvents")
  return {
    ok: true,
    message: formatHistoryEvents(payload.events ?? []),
    format: "text",
  }
}

async function executeSliceCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [action, first, ...rest] = parsed.args
  switch (action ?? "list") {
    case "list":
    case "ls": {
      const response = await deps.client.send(listSlicesRequest())
      const slices = expectVariant<{ slices: SliceRecord[] }>(response, "SlicesListed").slices
      return { ok: true, message: formatSlices(slices), data: { slices } }
    }
    case "create": {
      if (!first) {
        return { ok: false, message: "usage: slice create <name> [--kernel <worker-kernel-ref>] [--display-url <url>]" }
      }
      let workerKernelRef: string | undefined
      let displayUrl: string | undefined
      for (let index = 0; index < rest.length; index += 1) {
        const arg = rest[index]
        const value = rest[index + 1]
        if (arg === "--kernel" && value && !value.startsWith("--")) {
          workerKernelRef = value
          index += 1
        } else if (arg === "--display-url" && value && !value.startsWith("--")) {
          displayUrl = value
          index += 1
        } else if (arg?.startsWith("--")) {
          return { ok: false, message: `unknown or incomplete slice create option: ${arg}` }
        } else if (arg) {
          return { ok: false, message: "usage: slice create <name> [--kernel <worker-kernel-ref>] [--display-url <url>]" }
        }
      }
      const response = await deps.client.send(createSliceRequest({
        name: first,
        workspaceMount: context.worktree,
        workerKernelRef: workerKernelRef ?? null,
        displayUrl: displayUrl ?? null,
      }))
      const slice = expectVariant<{ slice: SliceRecord }>(response, "SliceCreated").slice
      return resourceResult(`created slice ${formatSliceLabel(slice)}`, parsed.assignment, slice.id, {}, { slice })
    }
    case "status":
    case "show": {
      const sliceRef = first ?? await focusedAgentSliceRef(context, deps)
      const response = await deps.client.send(getSliceRequest(sliceRef))
      const slice = expectVariant<{ slice: SliceRecord }>(response, "Slice").slice
      return { ok: true, message: formatSlice(slice), data: { slice } }
    }
    case "start": {
      const sliceRef = first ?? await focusedAgentSliceRef(context, deps)
      const response = await deps.client.send(startSliceRequest(sliceRef))
      const slice = expectVariant<{ slice: SliceRecord }>(response, "SliceStarted").slice
      return { ok: true, message: `started slice ${formatSliceLabel(slice)}`, data: { slice } }
    }
    case "stop": {
      const sliceRef = first ?? await focusedAgentSliceRef(context, deps)
      const response = await deps.client.send(stopSliceRequest(sliceRef))
      const slice = expectVariant<{ slice: SliceRecord }>(response, "SliceStopped").slice
      return { ok: true, message: `stopped slice ${formatSliceLabel(slice)}`, data: { slice } }
    }
    case "delete":
    case "rm": {
      if (!first) {
        return { ok: false, message: "usage: slice delete <slice-ref>" }
      }
      const response = await deps.client.send(deleteSliceRequest(first))
      const slice = expectVariant<{ slice: SliceRecord }>(response, "SliceDeleted").slice
      return { ok: true, message: `deleted slice ${formatSliceLabel(slice)}`, data: { slice } }
    }
    case "screen": {
      const sliceRef = first ?? await focusedAgentSliceRef(context, deps)
      const response = await deps.client.send(getSliceDisplayEndpointRequest(sliceRef))
      const endpoint = expectVariant<{ endpoint: SliceDisplayEndpoint }>(response, "SliceDisplayEndpoint").endpoint
      return { ok: true, message: endpoint.url, data: { endpoint } }
    }
    case "auth": {
      if (first !== "import") {
        return { ok: false, message: "usage: slice auth import [slice-ref] <provider>" }
      }
      const sliceRef = rest.length > 1 ? rest[0]! : await focusedAgentSliceRef(context, deps)
      const provider = rest.length > 1 ? rest[1] : rest[0]
      if (!provider) {
        return { ok: false, message: "usage: slice auth import [slice-ref] <provider>" }
      }
      const response = await deps.client.send(importSliceProviderAuthRequest(sliceRef, provider))
      const payload = expectVariant<{ slice: SliceRecord; provider: string; status: string }>(response, "SliceProviderAuthImported")
      return { ok: true, message: `slice ${formatSliceLabel(payload.slice)} auth import ${payload.provider}: ${payload.status}`, data: payload }
    }
    default:
      return { ok: false, message: "usage: slice list|create|status|start|stop|delete|auth import|screen" }
  }
}

async function executeCloudCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [area, action, ...args] = parsed.args
  if (area === "invite" && action === "create") {
    if (!context.sessionId) {
      return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
    }
    const maxUses = args[0] ? Number.parseInt(args[0], 10) : 1
    if (!Number.isFinite(maxUses) || maxUses <= 0) {
      return { ok: false, message: "usage: cloud invite create [max-uses]" }
    }
    const localResponse = await deps.client.send(createSessionInviteRequest(context.sessionId, null, maxUses))
    const local = expectVariant<{ invite: { invite: SessionInvite; invite_token: string }; session: RuntimeSession }>(
      localResponse,
      "SessionInviteCreated",
    )
    const cloudResponse = await deps.client.send(createCloudSessionInviteRequest(context.sessionId, {
      displayName: local.session.alias ?? local.session.id,
      maxUses,
    }))
    const cloud = expectVariant<{ invite: { invite_token: string; invite_id: string } }>(
      cloudResponse,
      "CloudSessionInviteCreated",
    )
    return {
      ok: true,
      message: [
        `cloud invite ${cloud.invite.invite_id}`,
        `cloud_invite=${cloud.invite.invite_token}`,
        `local_invite=${local.invite.invite_token}`,
      ].join("\n"),
      data: { cloud, local },
      contextUpdates: { sessionId: local.session.id, agentId: local.session.focused_agent_id ?? undefined },
    }
  }
  if (area === "invite" && action === "accept") {
    const inviteToken = args[0]
    const localInviteToken = args[1]
    if (!inviteToken) {
      return { ok: false, message: "usage: cloud invite accept <cloud-invite-token> [local-invite-token]" }
    }
    const cloudResponse = await deps.client.send(acceptCloudSessionInviteRequest(inviteToken))
    const cloud = expectVariant<{ acceptance: { user_id: string } }>(cloudResponse, "CloudSessionInviteAccepted")
    if (!localInviteToken) {
      return {
        ok: true,
        message: `accepted cloud invite as ${cloud.acceptance.user_id}; provide local invite token to join the kernel session`,
        data: cloud,
      }
    }
    const joinResponse = await deps.client.send(joinSessionInviteRequest(localInviteToken, cloud.acceptance.user_id))
    const joined = expectVariant<{ member: SessionMember; session: RuntimeSession }>(joinResponse, "SessionInviteJoined")
    const attachmentId = await attachShellSession(joined.session.id, deps)
    return {
      ok: true,
      message: `joined session ${joined.session.alias ?? joined.session.id} as ${joined.member.user_id}`,
      data: { cloud, joined },
      contextUpdates: {
        sessionId: joined.session.id,
        ...(attachmentId ? { attachmentId } : {}),
        agentId: joined.session.focused_agent_id ?? undefined,
        workspace: joined.session.workspace_id,
        worktree: joined.session.worktree_id,
      },
    }
  }
  if ((area === "members" && !action) || (area === "members" && action === "list")) {
    const sessionId = context.sessionId
    if (!sessionId) {
      return { ok: false, message: "usage: cloud members [list]" }
    }
    const response = await deps.client.send(listCloudSessionMembersRequest(sessionId))
    const payload = expectVariant<{ members: CloudSessionMember[] }>(response, "CloudSessionMembersListed")
    return { ok: true, message: formatCloudMembers(payload.members), data: payload }
  }
  if ((area === "collaborators" && !action) || (area === "collaborators" && action === "list")) {
    const response = await deps.client.send(listCloudCollaboratorsRequest())
    const payload = expectVariant<{ collaborators: CloudCollaborator[] }>(response, "CloudCollaboratorsListed")
    return { ok: true, message: formatCloudCollaborators(payload.collaborators), data: payload }
  }
  if (!area || area === "status") {
    const response = await deps.client.send(cloudRelayStatusRequest())
    const payload = expectVariant<{ profile: { account_slug?: string; email?: string } | null }>(response, "CloudRelayStatus")
    return {
      ok: true,
      message: payload.profile ? `cloud profile ${payload.profile.account_slug ?? payload.profile.email ?? "configured"}` : "cloud profile not configured",
      data: payload,
    }
  }
  return { ok: false, message: "usage: cloud invite create|accept | cloud members | cloud collaborators | cloud status" }
}

async function executeMcpCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [action, name] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listMcpServersRequest(context.workspace))
      const mcps = expectVariant<{ mcps: ArrobaMcpServerConfig[] }>(response, "McpServersListed").mcps
      return { ok: true, message: formatMcpList(mcps), data: { mcps } }
    }
    case "show": {
      if (!name) {
        return { ok: false, message: "usage: mcp show <name>" }
      }
      const response = await deps.client.send(getMcpServerRequest(context.workspace, name))
      const mcp = expectVariant<{ mcp: ArrobaMcpServerConfig }>(response, "McpServer").mcp
      return { ok: true, message: JSON.stringify(mcp, null, 2), data: { mcp }, format: "json" }
    }
    case "install":
    case "update": {
      const config = parseMcpInstallConfig(action === "install" ? parsed.args : ["install", ...parsed.args.slice(1)])
      if (!config) {
        return { ok: false, message: `usage: mcp ${action} <name> --command <cmd> [--arg value] [--env VAR] | mcp ${action} <name> --url <url> [--bearer-token-env-var VAR]` }
      }
      const request = action === "install"
        ? installMcpServerRequest(context.workspace, config as unknown as Record<string, unknown>)
        : updateMcpServerRequest(context.workspace, config as unknown as Record<string, unknown>)
      const response = await deps.client.send(request)
      const variant = action === "install" ? "McpServerInstalled" : "McpServerUpdated"
      const mcp = expectVariant<{ mcp: ArrobaMcpServerConfig }>(response, variant).mcp
      return { ok: true, message: `${action === "install" ? "installed" : "updated"} MCP ${mcp.name}`, data: { mcp } }
    }
    case "uninstall":
    case "remove": {
      if (!name) {
        return { ok: false, message: `usage: mcp ${action} <name>` }
      }
      const response = await deps.client.send(uninstallMcpServerRequest(context.workspace, name))
      const removed = expectVariant<{ name: string }>(response, "McpServerUninstalled").name
      return { ok: true, message: `uninstalled MCP ${removed}`, data: { name: removed } }
    }
    case "import": {
      const provider = name
      const importName = parsed.args[2] ?? null
      if (!provider) {
        return { ok: false, message: "usage: mcp import <codex|opencode> [name]" }
      }
      const response = await deps.client.send(importMcpServersRequest(context.workspace, provider, importName))
      const outcome = expectVariant<{ outcome: McpImportOutcome }>(response, "McpServersImported").outcome
      return { ok: true, message: formatMcpImportOutcome(outcome), data: { outcome } }
    }
    case "grant":
    case "revoke": {
      const agentRef = name
      const grantName = parsed.args[2]
      if (!agentRef || !grantName) {
        return { ok: false, message: `usage: mcp ${action} <agent-ref> <name>` }
      }
      const request = action === "grant"
        ? grantAgentCapabilityRequest(context.workspace, agentRef, "mcp", grantName)
        : revokeAgentCapabilityRequest(agentRef, "mcp", grantName)
      const response = await deps.client.send(request)
      const variant = action === "grant" ? "AgentCapabilityGranted" : "AgentCapabilityRevoked"
      const agent = expectVariant<{ agent: AgentInstance }>(response, variant).agent
      return { ok: true, message: `${action === "grant" ? "granted" : "revoked"} MCP ${grantName} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, data: { agent }, contextUpdates: { agentId: agent.id } }
    }
    case "grants":
    case "agent": {
      const agent = await resolveShellAgent(context, deps, name)
      if (!agent.ok) {
        return { ok: false, message: agent.message }
      }
      return { ok: true, message: formatAgentCapabilityGrants(agent.agent, "mcp"), data: { agent: agent.agent } }
    }
    default:
      return { ok: false, message: "usage: mcp list|show|install|update|uninstall|import|grant|revoke|grants" }
  }
}

async function executeSkillCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [action, name] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listSkillsRequest(context.workspace))
      const skills = expectVariant<{ skills: ArrobaSkillMetadata[] }>(response, "SkillsListed").skills
      return { ok: true, message: formatSkillList(skills), data: { skills } }
    }
    case "show": {
      if (!name) {
        return { ok: false, message: "usage: skill show <name>" }
      }
      const response = await deps.client.send(getSkillRequest(context.workspace, name))
      const skill = expectVariant<{ skill: ArrobaSkillMetadata }>(response, "Skill").skill
      return { ok: true, message: JSON.stringify(skill, null, 2), data: { skill }, format: "json" }
    }
    case "install":
    case "update": {
      if (!name) {
        return { ok: false, message: `usage: skill ${action} <path>` }
      }
      const response = await deps.client.send(action === "install"
        ? installSkillRequest(context.workspace, name)
        : updateSkillRequest(context.workspace, name))
      const variant = action === "install" ? "SkillInstalled" : "SkillUpdated"
      const skill = expectVariant<{ skill: ArrobaSkillMetadata }>(response, variant).skill
      return { ok: true, message: `${action === "install" ? "installed" : "updated"} skill ${skill.name}`, data: { skill } }
    }
    case "uninstall":
    case "remove": {
      if (!name) {
        return { ok: false, message: `usage: skill ${action} <name>` }
      }
      const response = await deps.client.send(uninstallSkillRequest(context.workspace, name))
      const skill = expectVariant<{ skill: ArrobaSkillMetadata }>(response, "SkillUninstalled").skill
      return { ok: true, message: `uninstalled skill ${skill.name}`, data: { skill } }
    }
    case "import": {
      const provider = name
      const importName = parsed.args[2] ?? null
      if (!provider) {
        return { ok: false, message: "usage: skill import <codex|opencode> [name]" }
      }
      const response = await deps.client.send(importSkillsRequest(context.workspace, provider, importName))
      const outcome = expectVariant<{ outcome: SkillImportOutcome }>(response, "SkillsImported").outcome
      return { ok: true, message: formatSkillImportOutcome(outcome), data: { outcome } }
    }
    case "grant":
    case "revoke": {
      const agentRef = name
      const grantName = parsed.args[2]
      if (!agentRef || !grantName) {
        return { ok: false, message: `usage: skill ${action} <agent-ref> <name>` }
      }
      const request = action === "grant"
        ? grantAgentCapabilityRequest(context.workspace, agentRef, "skill", grantName)
        : revokeAgentCapabilityRequest(agentRef, "skill", grantName)
      const response = await deps.client.send(request)
      const variant = action === "grant" ? "AgentCapabilityGranted" : "AgentCapabilityRevoked"
      const agent = expectVariant<{ agent: AgentInstance }>(response, variant).agent
      return { ok: true, message: `${action === "grant" ? "granted" : "revoked"} skill ${grantName} ${action === "grant" ? "to" : "from"} ${agent.agent_ref}`, data: { agent }, contextUpdates: { agentId: agent.id } }
    }
    case "grants":
    case "agent": {
      const agent = await resolveShellAgent(context, deps, name)
      if (!agent.ok) {
        return { ok: false, message: agent.message }
      }
      return { ok: true, message: formatAgentCapabilityGrants(agent.agent, "skill"), data: { agent: agent.agent } }
    }
    default:
      return { ok: false, message: "usage: skill list|show|install|update|uninstall|import|grant|revoke|grants" }
  }
}

async function executeWorkflowCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }

  const [action, ...args] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listWorkflowsRequest(sessionId))
      const workflows = expectVariant<{ workflows: WorkflowDefinition[] }>(response, "WorkflowsListed").workflows
      return { ok: true, message: formatWorkflowList(workflows, context.workflowId), data: { workflows } }
    }
    case "new":
    case "create": {
      const response = await deps.client.send(createWorkflowRequest(sessionId, args[0] ?? null))
      const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowCreated")
      return resourceResult(
        `created workflow ${formatWorkflowLabel(payload.workflow)}`,
        parsed.assignment,
        payload.workflow.id,
        { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
        payload,
      )
    }
    case "show": {
      const workflowRef = args[0] ?? context.workflowId
      if (!workflowRef) {
        return { ok: false, message: "usage: workflow show <workflow-ref>" }
      }
      const response = await deps.client.send(resolveWorkflowRequest(sessionId, workflowRef))
      const workflow = expectVariant<{ workflow: WorkflowDefinition }>(response, "WorkflowResolved").workflow
      return { ok: true, message: formatWorkflowDetails(workflow), data: { workflow }, contextUpdates: { workflowId: workflow.id } }
    }
    case "alias": {
      const [workflowRef, alias] = args
      if (!workflowRef || !alias) {
        return { ok: false, message: "usage: workflow alias <workflow-ref> <alias>" }
      }
      const response = await deps.client.send(aliasWorkflowRequest(sessionId, workflowRef, alias))
      const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowAliased")
      return { ok: true, message: `workflow ${payload.workflow.id} aliased as ${payload.workflow.alias}`, data: payload, contextUpdates: { workflowId: payload.workflow.id } }
    }
    case "run":
    case "start": {
      const [workflowRef, endpointRef, ...promptParts] = args
      if (!workflowRef || !endpointRef) {
        return { ok: false, message: `usage: workflow ${action} <workflow-ref> <endpoint-ref> [prompt]` }
      }
      const prompt = promptParts.join(" ").trim() || null
      const response = await deps.client.send(invokeWorkflowEndpointRequest(sessionId, workflowRef, endpointRef, prompt))
      if ("WorkflowRunInvoked" in response) {
        const payload = response.WorkflowRunInvoked as {
          workflow_run: WorkflowRun
          workflow: WorkflowDefinition
          endpoint: WorkflowEndpointDefinition
          session: RuntimeSession
        }
        return {
          ok: true,
          message: `started workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
          data: payload,
          contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
        }
      }
      const payload = expectVariant<{
        queued_launch: QueuedWorkflowLaunch
        workflow: WorkflowDefinition
        endpoint: WorkflowEndpointDefinition
        session: RuntimeSession
      }>(response, "WorkflowRunQueued")
      return {
        ok: true,
        message: `queued workflow launch ${payload.queued_launch.id}; active workflow run in session`,
        data: payload,
        contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
      }
    }
    case "launch-policy": {
      const value = args[0]?.trim().toLowerCase()
      if (!value) {
        const response = await deps.client.send(getSessionStateRequest(sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        return { ok: true, message: `workflow launch policy: ${session.workflow_launch_policy ?? "reject"}`, data: { session } }
      }
      if (value !== "reject" && value !== "queue") {
        return { ok: false, message: "usage: workflow launch-policy <reject|queue>" }
      }
      const response = await deps.client.send(setWorkflowLaunchPolicyRequest(sessionId, value))
      const payload = expectVariant<{ session: RuntimeSession }>(response, "WorkflowLaunchPolicyUpdated")
      return { ok: true, message: `workflow launch policy set to ${value}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
    }
    case "flush-context": {
      const first = args[0]?.trim().toLowerCase()
      const firstIsValue = first === "true" || first === "false"
      const workflowRef = firstIsValue ? context.workflowId : (args[0] ?? context.workflowId)
      const value = firstIsValue ? first : args[1]?.trim().toLowerCase()
      if (!workflowRef || (value !== "true" && value !== "false")) {
        return { ok: false, message: "usage: workflow flush-context [workflow-ref] <true|false>" }
      }
      const response = await deps.client.send(setWorkflowFlushContextRequest(sessionId, workflowRef, value === "true"))
      const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowFlushContextUpdated")
      return { ok: true, message: `workflow ${payload.workflow.id} flush-context set to ${String(payload.workflow.flush_agent_context_before_run ?? true)}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
    }
    case "run-output-schema":
    case "intermediate-output-schema": {
      const explicit = args.length >= 2 ? args[0] : null
      const workflowRef = explicit ?? context.workflowId
      const rawValue = explicit ? args[1] : args[0]
      if (!workflowRef || rawValue === undefined) {
        return { ok: false, message: `usage: workflow ${action} [workflow-ref] <schema-ref|none>` }
      }
      const schemaRef = rawValue.trim().toLowerCase() === "none" ? null : rawValue
      const response = await deps.client.send(action === "run-output-schema"
        ? setWorkflowRunOutputSchemaRequest(sessionId, workflowRef, schemaRef)
        : setWorkflowIntermediateOutputSchemaRequest(sessionId, workflowRef, schemaRef))
      const variant = action === "run-output-schema" ? "WorkflowRunOutputSchemaUpdated" : "WorkflowIntermediateOutputSchemaUpdated"
      const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(response, variant)
      const field = action === "run-output-schema" ? "run-output-schema" : "intermediate-output-schema"
      const value = action === "run-output-schema" ? payload.workflow.run_output_schema_ref : payload.workflow.intermediate_output_schema_ref
      return { ok: true, message: `workflow ${payload.workflow.id} ${field} set to ${value ?? "none"}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
    }
    case "max-turns": {
      const value = args[0]?.trim().toLowerCase()
      if (!value) {
        return { ok: false, message: "usage: workflow max-turns <count|off>" }
      }
      const nextValue = value === "off" || value === "0"
        ? "0"
        : Number.isFinite(Number(value)) ? String(Math.max(1, Math.floor(Number(value)))) : null
      if (!nextValue) {
        return { ok: false, message: "usage: workflow max-turns <count|off>" }
      }
      const attachmentId = await resolveShellAttachmentId(context, deps)
      if (!attachmentId.ok) {
        return { ok: false, message: attachmentId.message }
      }
      const response = await deps.client.send(updateSessionConfigRequest(sessionId, attachmentId.attachmentId, { "workflow.max_turns": nextValue }, false))
      const payload = expectVariant<{ session: RuntimeSession; config: SessionConfigState }>(response, "SessionConfigUpdated")
      return { ok: true, message: nextValue === "0" ? "workflow max turns disabled" : `workflow max turns set to ${nextValue}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
    }
    case "runs": {
      const response = await deps.client.send(listWorkflowRunsRequest(sessionId, args[0] ?? null))
      const workflowRuns = expectVariant<{ workflow_runs: WorkflowRun[] }>(response, "WorkflowRunsListed").workflow_runs
      return { ok: true, message: formatWorkflowRunList(workflowRuns, args[0] ?? null), data: { workflow_runs: workflowRuns } }
    }
    case "run-show":
    case "run-get": {
      const workflowRunRef = args[0]
      if (!workflowRunRef) {
        return { ok: false, message: `usage: workflow ${action} <run-ref>` }
      }
      const response = await deps.client.send(getWorkflowRunRequest(sessionId, workflowRunRef))
      const workflowRun = expectVariant<{ workflow_run: WorkflowRun }>(response, "WorkflowRun").workflow_run
      return { ok: true, message: JSON.stringify(workflowRun, null, 2), data: { workflow_run: workflowRun }, format: "json" }
    }
    case "cancel":
    case "resume": {
      const workflowRunRef = args[0]
      if (!workflowRunRef) {
        return { ok: false, message: `usage: workflow ${action} <run-ref>` }
      }
      const response = await deps.client.send(action === "cancel"
        ? cancelWorkflowRunRequest(sessionId, workflowRunRef)
        : resumeWorkflowRunRequest(sessionId, workflowRunRef))
      const variant = action === "cancel" ? "WorkflowRunCancelled" : "WorkflowRunResumed"
      const payload = expectVariant<{ workflow_run: WorkflowRun; session: RuntimeSession }>(response, variant)
      return {
        ok: true,
        message: `${action === "cancel" ? "cancelled" : "resumed"} workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
        data: payload,
        contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
      }
    }
    case "node":
      return executeWorkflowNodeCommand(args, parsed, context, deps)
    case "edge":
      return executeWorkflowEdgeCommand(args, context, deps)
    case "endpoint":
      return executeWorkflowEndpointCommand(args, context, deps)
    case "publication":
    case "publish":
      return executeWorkflowPublicationCommand(args, context, deps)
    case "watchdog":
      return executeWorkflowWatchdogCommand(args, context, deps)
    case "queue":
      return executeWorkflowQueueCommand(args, context, deps)
    default:
      return { ok: false, message: "usage: workflow list|new|show|alias|run|runs|run-show|cancel|resume|node|edge|endpoint|publication|watchdog|queue" }
  }
}

async function executeWorkflowNodeCommand(
  args: string[],
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId!
  const [action, maybeWorkflowRef, maybeNodeOrAgent] = args
  const workflowRef = args.length >= 3 ? maybeWorkflowRef : context.workflowId
  const target = args.length >= 3 ? maybeNodeOrAgent : maybeWorkflowRef
  if (action === "add") {
    if (!workflowRef || !target) {
      return { ok: false, message: "usage: workflow node add [workflow-ref] <agent-ref>" }
    }
    const agent = await resolveShellAgent(context, deps, target)
    if (!agent.ok) {
      return { ok: false, message: agent.message }
    }
    const response = await deps.client.send(addWorkflowNodeRequest(sessionId, workflowRef, agent.agent.id))
    const payload = expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowNodeAdded")
    return resourceResult(
      `added workflow node ${payload.node.id} for agent ${agent.agent.agent_ref}`,
      parsed.assignment,
      payload.node.id,
      { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
      payload,
    )
  }
  if (action === "remove") {
    if (!workflowRef || !target) {
      return { ok: false, message: "usage: workflow node remove [workflow-ref] <node-id>" }
    }
    const response = await deps.client.send(removeWorkflowNodeRequest(sessionId, workflowRef, target))
    const payload = expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowNodeRemoved")
    return { ok: true, message: `removed workflow node ${payload.node.id}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  if (
    action === "can-complete-run"
    || action === "can-emit-intermediate-output"
    || action === "intermediate-output-schema"
    || action === "max-turns"
  ) {
    const explicitWorkflowRef = args.length >= 4 ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const nodeId = explicitWorkflowRef ? args[2] : args[1]
    const value = explicitWorkflowRef ? args[3] : args[2]
    if (!workflowRef || !nodeId || value === undefined) {
      return { ok: false, message: "usage: workflow node can-complete-run|can-emit-intermediate-output|intermediate-output-schema|max-turns [workflow-ref] <node-id> <value>" }
    }
    let request: Record<string, unknown>
    let variant: string
    let renderedValue: string
    if (action === "can-complete-run" || action === "can-emit-intermediate-output") {
      const normalized = value.trim().toLowerCase()
      if (normalized !== "true" && normalized !== "false") {
        return { ok: false, message: `usage: workflow node ${action} [workflow-ref] <node-id> <true|false>` }
      }
      const bool = normalized === "true"
      request = action === "can-complete-run"
        ? setWorkflowNodeCanCompleteRunRequest(sessionId, workflowRef, nodeId, bool)
        : setWorkflowNodeCanEmitIntermediateOutputRequest(sessionId, workflowRef, nodeId, bool)
      variant = action === "can-complete-run" ? "WorkflowNodeCanCompleteRunUpdated" : "WorkflowNodeCanEmitIntermediateOutputUpdated"
      renderedValue = normalized
    } else if (action === "intermediate-output-schema") {
      const schemaRef = value.trim().toLowerCase() === "none" ? null : value
      request = setWorkflowNodeIntermediateOutputSchemaRequest(sessionId, workflowRef, nodeId, schemaRef)
      variant = "WorkflowNodeIntermediateOutputSchemaUpdated"
      renderedValue = schemaRef ?? "none"
    } else {
      const normalized = value.trim().toLowerCase()
      const maxTurns = normalized === "none" ? null : Number.parseInt(normalized, 10)
      if (maxTurns !== null && (!Number.isFinite(maxTurns) || maxTurns <= 0)) {
        return { ok: false, message: "usage: workflow node max-turns [workflow-ref] <node-id> <count|none>" }
      }
      request = setWorkflowNodeMaxTurnsRequest(sessionId, workflowRef, nodeId, maxTurns)
      variant = "WorkflowNodeMaxTurnsUpdated"
      renderedValue = maxTurns === null ? "none" : String(maxTurns)
    }
    const response = await deps.client.send(request)
    const payload = expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, variant)
    return { ok: true, message: `workflow node ${payload.node.id} ${action} set to ${renderedValue}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  return { ok: false, message: "usage: workflow node add [workflow-ref] <agent-ref> | remove [workflow-ref] <node-id> | can-complete-run|can-emit-intermediate-output|intermediate-output-schema|max-turns ..." }
}

async function executeWorkflowEdgeCommand(
  args: string[],
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId!
  const [action] = args
  if (action === "add") {
    const explicitWorkflowRef = args.length >= 4 ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const fromNodeId = explicitWorkflowRef ? args[2] : args[1]
    const toNodeId = explicitWorkflowRef ? args[3] : args[2]
    if (!workflowRef || !fromNodeId || !toNodeId) {
      return { ok: false, message: "usage: workflow edge add [workflow-ref] <from-node-id> <to-node-id>" }
    }
    const response = await deps.client.send(addWorkflowEdgeRequest(sessionId, workflowRef, fromNodeId, toNodeId))
    const payload = expectVariant<{ edge: WorkflowEdgeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowEdgeAdded")
    return { ok: true, message: `added workflow edge ${payload.edge.id}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  if (action === "remove") {
    const explicitWorkflowRef = args.length >= 3 ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const edgeId = explicitWorkflowRef ? args[2] : args[1]
    if (!workflowRef || !edgeId) {
      return { ok: false, message: "usage: workflow edge remove [workflow-ref] <edge-id>" }
    }
    const response = await deps.client.send(removeWorkflowEdgeRequest(sessionId, workflowRef, edgeId))
    const payload = expectVariant<{ edge: WorkflowEdgeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowEdgeRemoved")
    return { ok: true, message: `removed workflow edge ${payload.edge.id}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  return { ok: false, message: "usage: workflow edge add [workflow-ref] <from-node-id> <to-node-id> | remove [workflow-ref] <edge-id>" }
}

async function executeWorkflowEndpointCommand(
  args: string[],
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId!
  const [action] = args
  if (action === "new" || action === "create") {
    const explicitWorkflowRef = args.length >= 3 ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const entryNodeId = explicitWorkflowRef ? args[2] : args[1]
    const alias = explicitWorkflowRef ? args[3] : args[2]
    if (!workflowRef || !entryNodeId) {
      return { ok: false, message: "usage: workflow endpoint new [workflow-ref] <entry-node-id> [alias]" }
    }
    const response = await deps.client.send(createWorkflowEndpointRequest(sessionId, workflowRef, entryNodeId, alias ?? null))
    const payload = expectVariant<{ endpoint: WorkflowEndpointDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowEndpointCreated")
    return { ok: true, message: `created workflow endpoint ${payload.endpoint.id}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  if (action === "alias") {
    const explicitWorkflowRef = args.length >= 4 ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const endpointRef = explicitWorkflowRef ? args[2] : args[1]
    const alias = explicitWorkflowRef ? args[3] : args[2]
    if (!workflowRef || !endpointRef || !alias) {
      return { ok: false, message: "usage: workflow endpoint alias [workflow-ref] <endpoint-ref> <alias>" }
    }
    const response = await deps.client.send(aliasWorkflowEndpointRequest(sessionId, workflowRef, endpointRef, alias))
    const payload = expectVariant<{ endpoint: WorkflowEndpointDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowEndpointAliased")
    return { ok: true, message: `workflow endpoint ${payload.endpoint.id} aliased as ${payload.endpoint.alias}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  if (action === "bind") {
    const explicitWorkflowRef = args.length >= 4 ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const endpointRef = explicitWorkflowRef ? args[2] : args[1]
    const entryNodeId = explicitWorkflowRef ? args[3] : args[2]
    if (!workflowRef || !endpointRef || !entryNodeId) {
      return { ok: false, message: "usage: workflow endpoint bind [workflow-ref] <endpoint-ref> <entry-node-id>" }
    }
    const response = await deps.client.send(bindWorkflowEndpointRequest(sessionId, workflowRef, endpointRef, entryNodeId))
    const payload = expectVariant<{ endpoint: WorkflowEndpointDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowEndpointBound")
    return { ok: true, message: `workflow endpoint ${payload.endpoint.id} bound to node ${payload.endpoint.entry_node_id}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  return { ok: false, message: "usage: workflow endpoint new [workflow-ref] <entry-node-id> [alias] | alias [workflow-ref] <endpoint-ref> <alias> | bind [workflow-ref] <endpoint-ref> <entry-node-id>" }
}

async function executeWorkflowPublicationCommand(
  args: string[],
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }

  const [action = "list", ...rest] = args
  if (action === "list" || action === "ls") {
    const response = await deps.client.send(listWorkflowPublicationsRequest(sessionId))
    const publications = expectVariant<{ publications: WorkflowPublicationDefinition[] }>(response, "WorkflowPublicationsListed").publications
    return { ok: true, message: formatWorkflowPublications(publications), data: { publications } }
  }

  if (action === "show" || action === "get") {
    const publicationRef = rest[0]
    if (!publicationRef) {
      return { ok: false, message: "usage: workflow publication show <publication-ref>" }
    }
    const response = await deps.client.send(getWorkflowPublicationRequest(sessionId, publicationRef))
    const publication = expectVariant<{ publication: WorkflowPublicationDefinition }>(response, "WorkflowPublication").publication
    return { ok: true, message: JSON.stringify(publication, null, 2), data: { publication }, format: "json" }
  }

  if (action === "export") {
    const [publicationRef, outputDirectory, ...optionArgs] = rest
    if (!publicationRef || !outputDirectory) {
      return { ok: false, message: "usage: workflow publication export <publication-ref> <directory> [--kernel-url <url>]" }
    }
    const options = parseWorkflowPublicationExportOptions(optionArgs)
    if (!options.ok) return { ok: false, message: options.message }
    const response = await deps.client.send(getWorkflowPublicationRequest(sessionId, publicationRef))
    const publication = expectVariant<{ publication: WorkflowPublicationDefinition }>(response, "WorkflowPublication").publication
    const outputRoot = resolvePath(context.worktree ?? context.workspace ?? process.cwd(), outputDirectory)
    const packageFiles = await writeWorkflowPublicationExportPackage(publication, outputRoot, options.kernelUrl)
    return {
      ok: true,
      message: `exported workflow publication ${formatWorkflowPublicationLabel(publication)} to ${outputRoot}`,
      data: { publication, outputRoot, files: packageFiles },
    }
  }

  if (action === "disable" || action === "remove") {
    const publicationRef = rest[0]
    if (!publicationRef) {
      return { ok: false, message: "usage: workflow publication disable <publication-ref>" }
    }
    const response = await deps.client.send(disableWorkflowPublicationRequest(sessionId, publicationRef))
    const payload = expectVariant<{ publication: WorkflowPublicationDefinition; session: RuntimeSession }>(response, "WorkflowPublicationDisabled")
    return {
      ok: true,
      message: `disabled workflow publication ${payload.publication.id}`,
      data: payload,
      contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
    }
  }

  if (action === "pair-code") {
    const publicationRef = rest[0]
    if (!publicationRef) {
      return { ok: false, message: "usage: workflow publication pair-code <publication-ref> [--expires-ms N] [--max-uses N]" }
    }
    const options = parseNumericFlags(rest.slice(1), ["--expires-ms", "--max-uses"])
    if (!options.ok) return { ok: false, message: options.message }
    const response = await deps.client.send(createWorkflowPublicationPairCodeRequest(
      sessionId,
      publicationRef,
      options.values["--expires-ms"] ?? null,
      options.values["--max-uses"] ?? null,
    ))
    const payload = expectVariant<{ pair_code: WorkflowPublicationPairingCodeRecord; session: RuntimeSession }>(response, "WorkflowPublicationPairCodeCreated")
    return {
      ok: true,
      message: `pair_code ${payload.pair_code.code.code_id}\n${payload.pair_code.pair_code}`,
      data: payload,
      contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
    }
  }

  if (action === "redeem-code") {
    const [publicationRef, pairCode, ...displayNameParts] = rest
    if (!publicationRef || !pairCode) {
      return { ok: false, message: "usage: workflow publication redeem-code <publication-ref> <pair-code> [display-name]" }
    }
    const displayName = displayNameParts.join(" ").trim() || null
    const response = await deps.client.send(redeemWorkflowPublicationPairCodeRequest(
      sessionId,
      publicationRef,
      pairCode,
      displayName,
      ["http"],
    ))
    const payload = expectVariant<{ sender_credential: WorkflowPublicationSenderCredential; session: RuntimeSession }>(response, "WorkflowPublicationSenderPaired")
    return {
      ok: true,
      message: `sender ${payload.sender_credential.sender.sender_id}\n${payload.sender_credential.credential}`,
      data: payload,
      contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
    }
  }

  if (action === "senders") {
    const publicationRef = rest[0]
    if (!publicationRef) {
      return { ok: false, message: "usage: workflow publication senders <publication-ref>" }
    }
    const response = await deps.client.send(listWorkflowPublicationSendersRequest(sessionId, publicationRef))
    const senders = expectVariant<{ senders: WorkflowPublicationTrustedSender[] }>(response, "WorkflowPublicationSendersListed").senders
    return { ok: true, message: formatWorkflowPublicationSenders(senders), data: { senders } }
  }

  if (action === "revoke-sender") {
    const [publicationRef, senderRef] = rest
    if (!publicationRef || !senderRef) {
      return { ok: false, message: "usage: workflow publication revoke-sender <publication-ref> <sender-ref>" }
    }
    const response = await deps.client.send(revokeWorkflowPublicationSenderRequest(sessionId, publicationRef, senderRef))
    const payload = expectVariant<{ sender: WorkflowPublicationTrustedSender; session: RuntimeSession }>(response, "WorkflowPublicationSenderRevoked")
    return {
      ok: true,
      message: `revoked workflow publication sender ${payload.sender.sender_id}`,
      data: payload,
      contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
    }
  }

  if (action === "create" || action === "new") {
    const parsed = parseWorkflowPublicationCreateOptions(rest, context.workflowId)
    if (!parsed.ok) {
      return { ok: false, message: parsed.message }
    }
    const response = await deps.client.send(createWorkflowPublicationRequest(
      sessionId,
      parsed.workflowRef,
      parsed.endpointRef,
      parsed.options,
    ))
    const payload = expectVariant<{ publication: WorkflowPublicationDefinition; session: RuntimeSession }>(response, "WorkflowPublicationCreated")
    return resourceResult(
      `created workflow publication ${formatWorkflowPublicationLabel(payload.publication)}`,
      undefined,
      payload.publication.id,
      { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined, workflowId: payload.publication.workflow_id },
      payload,
    )
  }

  return { ok: false, message: "usage: workflow publication list|create|show|export|disable|pair-code|redeem-code|senders|revoke-sender" }
}

async function executeWorkflowWatchdogCommand(
  args: string[],
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId!
  const [action] = args
  if (action === "list" || !action) {
    const workflowRef = args[1] ?? null
    const response = await deps.client.send(listWorkflowWatchdogsRequest(sessionId, workflowRef))
    const watchdogs = expectVariant<{ watchdogs: WorkflowWatchdogDefinition[] }>(response, "WorkflowWatchdogsListed").watchdogs
    return { ok: true, message: formatWorkflowWatchdogs(watchdogs), data: { watchdogs } }
  }
  if (action === "enable" || action === "disable") {
    const watchdogRef = args[1]
    if (!watchdogRef) {
      return { ok: false, message: `usage: workflow watchdog ${action} <watchdog-ref>` }
    }
    const response = await deps.client.send(setWorkflowWatchdogEnabledRequest(sessionId, watchdogRef, action === "enable"))
    const payload = expectVariant<{ watchdog: WorkflowWatchdogDefinition; session: RuntimeSession }>(response, "WorkflowWatchdogUpdated")
    return { ok: true, message: `${action === "enable" ? "enabled" : "disabled"} workflow watchdog ${payload.watchdog.id}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  if (action === "remove") {
    const watchdogRef = args[1]
    if (!watchdogRef) {
      return { ok: false, message: "usage: workflow watchdog remove <watchdog-ref>" }
    }
    const response = await deps.client.send(removeWorkflowWatchdogRequest(sessionId, watchdogRef))
    const payload = expectVariant<{ watchdog: WorkflowWatchdogDefinition; session: RuntimeSession }>(response, "WorkflowWatchdogRemoved")
    return { ok: true, message: `removed workflow watchdog ${payload.watchdog.id}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  if (action === "add") {
    const explicitWorkflowRef = args[3] === "every" ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const endpointRef = explicitWorkflowRef ? args[2] : args[1]
    const everyLiteral = explicitWorkflowRef ? args[3] : args[2]
    const intervalLiteral = explicitWorkflowRef ? args[4] : args[3]
    const optionStart = explicitWorkflowRef ? 5 : 4
    if (!workflowRef || !endpointRef || everyLiteral !== "every" || !intervalLiteral) {
      return { ok: false, message: "usage: workflow watchdog add [workflow-ref] <endpoint-ref> every <Ns|Nm|Nh|Nd> [skip|queue] [prompt]" }
    }
    const intervalSeconds = parseWatchdogIntervalSeconds(intervalLiteral)
    if (!intervalSeconds) {
      return { ok: false, message: "watchdog interval must be like 30s, 5m, 1h, or 1d" }
    }
    const hasPolicy = args[optionStart] === "skip" || args[optionStart] === "queue"
    const policy = (hasPolicy ? args[optionStart] : "skip") as "skip" | "queue"
    const prompt = args.slice(optionStart + (hasPolicy ? 1 : 0)).join(" ").trim() || "Run the workflow exactly as instructed."
    const response = await deps.client.send(createWorkflowWatchdogRequest(sessionId, workflowRef, endpointRef, intervalSeconds, prompt, policy))
    const payload = expectVariant<{ watchdog: WorkflowWatchdogDefinition; workflow: WorkflowDefinition; endpoint: WorkflowEndpointDefinition; session: RuntimeSession }>(response, "WorkflowWatchdogCreated")
    return { ok: true, message: `created workflow watchdog ${payload.watchdog.id}`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  return { ok: false, message: "usage: workflow watchdog add|list|enable|disable|remove" }
}

async function executeWorkflowQueueCommand(
  args: string[],
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId!
  const action = args[0] ?? "list"
  if (action === "list") {
    const response = await deps.client.send(listQueuedWorkflowLaunchesRequest(sessionId))
    const queuedLaunches = expectVariant<{ queued_launches: QueuedWorkflowLaunch[] }>(response, "QueuedWorkflowLaunchesListed").queued_launches
    return { ok: true, message: formatQueuedWorkflowLaunches(queuedLaunches), data: { queued_launches: queuedLaunches } }
  }
  if (action === "flush" || action === "clear") {
    const response = await deps.client.send(clearQueuedWorkflowLaunchesRequest(sessionId))
    const payload = expectVariant<{ queued_launches: QueuedWorkflowLaunch[]; session: RuntimeSession }>(response, "QueuedWorkflowLaunchesCleared")
    return { ok: true, message: payload.queued_launches.length === 0 ? "workflow queue already empty" : `cleared ${payload.queued_launches.length} queued workflow launch${payload.queued_launches.length === 1 ? "" : "es"}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  if (action === "remove") {
    const queueItemRef = args[1]
    if (!queueItemRef) {
      return { ok: false, message: "usage: workflow queue remove <queue-item-ref>" }
    }
    const response = await deps.client.send(removeQueuedWorkflowLaunchRequest(sessionId, queueItemRef))
    const payload = expectVariant<{ queued_launch: QueuedWorkflowLaunch; session: RuntimeSession }>(response, "QueuedWorkflowLaunchRemoved")
    return { ok: true, message: `removed queued workflow launch ${payload.queued_launch.id}`, data: payload, contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
  }
  return { ok: false, message: "usage: workflow queue [list|flush|remove <queue-item-ref>]" }
}

async function executeStopCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  if (parsed.args.length > 0) {
    return { ok: false, message: "usage: stop" }
  }
  if (!context.sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }
  const attachmentId = await resolveShellAttachmentId(context, deps)
  if (!attachmentId.ok) {
    return { ok: false, message: attachmentId.message }
  }
  const response = await deps.client.send(cancelActivePromptRequest(context.sessionId, attachmentId.attachmentId))
  const payload = expectVariant<{ cancellation: { prompt?: { id?: string | null } | null } }>(response, "PromptCancelled")
  return { ok: true, message: `cancellation requested${payload.cancellation.prompt?.id ? ` for prompt ${payload.cancellation.prompt.id}` : ""}`, data: payload }
}

async function resolveShellAgent(
  context: ShellContext,
  deps: ShellExecutorDeps,
  agentRef: string | undefined,
): Promise<{ ok: true; agent: AgentInstance } | { ok: false; message: string }> {
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }
  const reference = agentRef ?? context.agentId
  if (!reference) {
    return { ok: false, message: "usage: mcp|skill grants <agent-ref>" }
  }
  const response = await deps.client.send(listAgentsRequest(sessionId))
  const agents = expectVariant<{ agents: AgentInstance[] }>(response, "AgentsListed").agents
  const matches = agents.filter((agent) => agent.id === reference || agent.agent_ref === reference || agent.alias === reference)
  if (matches.length === 0) {
    return { ok: false, message: `unknown agent ${reference}` }
  }
  if (matches.length > 1) {
    return { ok: false, message: `agent reference ${reference} is ambiguous` }
  }
  return { ok: true, agent: matches[0]! }
}

async function tryResolveShellAgent(
  context: ShellContext,
  deps: ShellExecutorDeps,
  agentRef: string | undefined,
): Promise<{ ok: true; agent: AgentInstance } | { ok: false }> {
  const result = await resolveShellAgent(context, deps, agentRef)
  return result.ok ? result : { ok: false }
}

async function resolveShellAttachmentId(
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<{ ok: true; attachmentId: string } | { ok: false; message: string }> {
  if (context.attachmentId) {
    return { ok: true, attachmentId: context.attachmentId }
  }
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }
  const response = await deps.client.send(getSessionStateRequest(sessionId))
  const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
  const attachmentId = session.attachment_ids[0]
  if (!attachmentId) {
    return { ok: false, message: "current session has no attached client; stop/session-config commands require an attachment" }
  }
  return { ok: true, attachmentId }
}

async function attachShellSession(sessionId: string, deps: ShellExecutorDeps): Promise<string | undefined> {
  if (!deps.clientId) {
    return undefined
  }
  const response = await deps.client.send(attachToSessionRequest(sessionId, deps.clientId))
  const payload = expectVariant<{ attachment: { id: string } }>(response, "SessionAttached")
  return payload.attachment.id
}

function parseMcpInstallConfig(args: string[]): ArrobaMcpServerConfig | null {
  const name = args[1]
  if (!name) return null
  let command: string | null = null
  let url: string | null = null
  const mcpArgs: string[] = []
  const envVars: string[] = []
  let bearerTokenEnvVar: string | null = null
  for (let index = 2; index < args.length; index += 1) {
    const arg = args[index]
    const next = args[index + 1]
    if (arg === "--command" && next) {
      command = next
      index += 1
    } else if (arg === "--arg" && next) {
      mcpArgs.push(next)
      index += 1
    } else if (arg === "--env" && next) {
      envVars.push(next)
      index += 1
    } else if (arg === "--url" && next) {
      url = next
      index += 1
    } else if (arg === "--bearer-token-env-var" && next) {
      bearerTokenEnvVar = next
      index += 1
    } else {
      return null
    }
  }
  if (command && !url) {
    return {
      name,
      transport: { type: "stdio", command, args: mcpArgs, env: {}, env_vars: envVars },
      enabled: true,
      required: false,
    }
  }
  if (url && !command) {
    return {
      name,
      transport: {
        type: "streamable_http",
        url,
        bearer_token_env_var: bearerTokenEnvVar,
        http_headers: {},
        env_http_headers: {},
      },
      enabled: true,
      required: false,
    }
  }
  return null
}

function parseWorkflowPublicationCreateOptions(
  args: string[],
  currentWorkflowId?: string,
):
  | { ok: true; workflowRef: string; endpointRef: string; options: CreateWorkflowPublicationOptions }
  | { ok: false; message: string } {
  const positional: string[] = []
  const options: CreateWorkflowPublicationOptions = {}
  const methods: string[] = []
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (!arg) continue
    if (arg === "--route") {
      const value = args[++index]
      if (!value) return { ok: false, message: "usage: workflow publication create [workflow-ref] <endpoint-ref> [alias] --route <route>" }
      options.route = value
    } else if (arg === "--method") {
      const value = args[++index]
      if (!value) return { ok: false, message: "usage: workflow publication create ... --method <GET|POST|...>" }
      methods.push(value.toUpperCase())
    } else if (arg === "--transport-json") {
      const parsed = parseJsonOption(args[++index], "--transport-json")
      if (!parsed.ok) return parsed
      options.transport = parsed.value
    } else if (arg === "--auth-json") {
      const parsed = parseJsonOption(args[++index], "--auth-json")
      if (!parsed.ok) return parsed
      options.auth = parsed.value
    } else if (arg === "--parser-json") {
      const parsed = parseJsonOption(args[++index], "--parser-json")
      if (!parsed.ok) return parsed
      options.parser = parsed.value
    } else if (arg === "--input-schema-json") {
      const parsed = parseJsonOption(args[++index], "--input-schema-json")
      if (!parsed.ok) return parsed
      options.inputSchema = parsed.value
    } else if (arg === "--mode") {
      const value = args[++index]
      if (!value) return { ok: false, message: "usage: workflow publication create ... --mode <sync|async>" }
      options.mode = value
    } else if (arg.startsWith("--")) {
      return { ok: false, message: `unknown publication option: ${arg}` }
    } else {
      positional.push(arg)
    }
  }
  if (methods.length > 0) {
    options.methods = methods
  }
  const workflowRef = positional.length >= 3 ? positional[0] : currentWorkflowId
  const endpointRef = positional.length >= 3 ? positional[1] : positional[0]
  const alias = positional.length >= 3 ? positional[2] : positional[1]
  if (!workflowRef || !endpointRef) {
    return { ok: false, message: "usage: workflow publication create [workflow-ref] <endpoint-ref> [alias] [--route <route>] [--method POST] [--auth-json <json>] [--parser-json <json>] [--transport-json <json>] [--input-schema-json <json>] [--mode async]" }
  }
  if (positional.length > 3 || (!currentWorkflowId && positional.length < 2)) {
    return { ok: false, message: "usage: workflow publication create [workflow-ref] <endpoint-ref> [alias] ..." }
  }
  options.alias = alias ?? null
  return { ok: true, workflowRef, endpointRef, options }
}

function parseWorkflowPublicationExportOptions(
  args: string[],
): { ok: true; kernelUrl?: string | undefined } | { ok: false; message: string } {
  let kernelUrl: string | undefined
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (arg === "--kernel-url") {
      const value = args[++index]
      if (!value) return { ok: false, message: "usage: workflow publication export <publication-ref> <directory> [--kernel-url <url>]" }
      kernelUrl = value
    } else {
      return { ok: false, message: `unknown publication export option: ${arg ?? ""}` }
    }
  }
  return { ok: true, kernelUrl }
}

function parseNumericFlags(
  args: string[],
  allowedFlags: string[],
): { ok: true; values: Record<string, number> } | { ok: false; message: string } {
  const values: Record<string, number> = {}
  for (let index = 0; index < args.length; index += 1) {
    const flag = args[index]
    if (!flag || !allowedFlags.includes(flag)) {
      return { ok: false, message: `unsupported option ${flag ?? ""}`.trim() }
    }
    const raw = args[++index]
    const value = Number(raw)
    if (!raw || !Number.isFinite(value) || value < 0) {
      return { ok: false, message: `expected non-negative number after ${flag}` }
    }
    values[flag] = value
  }
  return { ok: true, values }
}

function parseJsonOption(
  value: string | undefined,
  option: string,
): { ok: true; value: unknown } | { ok: false; message: string } {
  if (!value) {
    return { ok: false, message: `${option} requires a JSON value` }
  }
  try {
    return { ok: true, value: JSON.parse(value) }
  } catch (error) {
    return { ok: false, message: `${option} is invalid JSON: ${error instanceof Error ? error.message : String(error)}` }
  }
}

function parseWatchdogIntervalSeconds(value: string | undefined): number | null {
  const match = value?.trim().match(/^(\d+)([smhd])$/i)
  if (!match) return null
  const amount = Number.parseInt(match[1]!, 10)
  if (!Number.isFinite(amount) || amount <= 0) return null
  const unit = match[2]!.toLowerCase()
  const multiplier = unit === "s" ? 1 : unit === "m" ? 60 : unit === "h" ? 3600 : 86400
  return amount * multiplier
}

function resourceResult(
  message: string,
  assignment: string | undefined,
  value: string,
  contextUpdates: ShellCommandResult["contextUpdates"],
  data: unknown,
): ShellCommandResult {
  return {
    ok: true,
    message,
    data,
    bindings: assignment ? { [assignment]: value } : undefined,
    contextUpdates,
  }
}

function extractSubmittedPrompt(payload: PromptSubmittedPayload, targetAgentId: string): PromptQueueItem | null {
  const variants = Object.values(payload.outcome ?? {})
  for (const variant of variants) {
    if (variant && typeof variant === "object" && "prompt" in variant) {
      const prompt = (variant as { prompt?: PromptQueueItem | null }).prompt
      if (prompt) return prompt
    }
  }
  const state = payload.session.prompt_states?.[targetAgentId]
  return state?.active_prompt
    ?? state?.queued_prompts?.[state.queued_prompts.length - 1]
    ?? (payload.session.active_prompt?.target_agent_id === targetAgentId ? payload.session.active_prompt : null)
    ?? [...payload.session.queued_prompts].reverse().find((prompt) => prompt.target_agent_id === targetAgentId)
    ?? null
}

async function waitForPromptCompletion(
  sessionId: string,
  attachmentId: string,
  agentId: string,
  promptId: string,
  deps: ShellExecutorDeps,
): Promise<RuntimeSession> {
  const deadline = Date.now() + 120_000
  let latest: RuntimeSession | null = null
  while (Date.now() < deadline) {
    await deps.client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => ({}))
    const response = await deps.client.send(getSessionStateRequest(sessionId))
    latest = expectSessionState(response)
    if (!sessionHasPrompt(latest, agentId, promptId)) {
      return latest
    }
    await sleep(250)
  }
  throw new Error(`timed out waiting for prompt ${promptId}`)
}

async function readPromptHistory(
  sessionId: string,
  agentId: string,
  promptText: string,
  deps: ShellExecutorDeps,
): Promise<SessionHistoryPageEntry[]> {
  const response = await deps.client.send(getSessionHistoryRequest(sessionId, 12, 120_000, null, agentId))
  const page = expectVariant<SessionHistoryPage>(response, "SessionHistory")
  const entries = [...page.entries].sort((left, right) => left.entry_index - right.entry_index)
  const promptIndex = findPromptHistoryIndex(entries, promptText)
  return promptIndex === null
    ? entries.filter((entry) => entry.entry.kind !== "user_prompt")
    : entries.filter((entry) => entry.entry_index > promptIndex && entry.entry.kind !== "user_prompt")
}

function findPromptHistoryIndex(entries: SessionHistoryPageEntry[], promptText: string): number | null {
  const normalizedPrompt = promptText.trim()
  const matches = entries.filter((entry) => entry.entry.kind === "user_prompt" && entry.entry.text.trim() === normalizedPrompt)
  const matched = matches[matches.length - 1]
  if (matched) return matched.entry_index
  const lastPrompt = [...entries].reverse().find((entry) => entry.entry.kind === "user_prompt")
  return lastPrompt?.entry_index ?? null
}

function sessionHasPrompt(session: RuntimeSession, agentId: string, promptId: string): boolean {
  const state = session.prompt_states?.[agentId]
  return state?.active_prompt?.id === promptId
    || Boolean(state?.queued_prompts?.some((prompt) => prompt.id === promptId))
    || session.active_prompt?.id === promptId
    || session.queued_prompts.some((prompt) => prompt.id === promptId)
}

function expectSessionState(response: Record<string, unknown>): RuntimeSession {
  if ("SessionState" in response) {
    return (response.SessionState as { session: RuntimeSession }).session
  }
  return expectVariant<{ session: RuntimeSession }>(response, "SessionStateLoaded").session
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function formatSlices(slices: SliceRecord[]): string {
  if (slices.length === 0) {
    return "no slices"
  }
  return slices.map(formatSlice).join("\n")
}

function formatSlice(slice: SliceRecord): string {
  const providers = (slice.providers ?? []).join(",") || "-"
  const display = slice.display_endpoint?.url ? ` display=${slice.display_endpoint.url}` : ""
  return `${formatSliceLabel(slice)} status=${slice.status} backend=${slice.backend} os=${slice.os} worker=${slice.worker_kernel_id ?? slice.worker_kernel_ref} providers=${providers}${display}`
}

function formatSliceLabel(slice: SliceRecord): string {
  return slice.name ? `${slice.name} id=${slice.id}` : slice.id
}

async function focusedAgentSliceRef(context: ShellContext, deps: ShellExecutorDeps): Promise<string> {
  const resolved = await resolveShellAgent(context, deps, undefined)
  if (!resolved.ok) {
    throw new Error("no slice specified and focused agent is not running in a slice")
  }
  const remote = resolved.agent.remote_execution
  if (!remote) {
    throw new Error("no slice specified and focused agent is not running in a slice")
  }
  const response = await deps.client.send(listSlicesRequest())
  const slices = expectVariant<{ slices: SliceRecord[] }>(response, "SlicesListed").slices
  const match = slices.find((slice) =>
    slice.worker_kernel_id === remote.worker_kernel_id
    || slice.worker_kernel_ref === remote.worker_kernel_id
    || slice.worker_machine_id === remote.worker_machine_id
  )
  if (!match) {
    throw new Error("no slice specified and focused agent is not running in a slice")
  }
  return match.id
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}

function parseExecutionMode(value: string | null | undefined): "build" | "plan" | null {
  if (!value) return null
  const normalized = value.trim().toLowerCase()
  return normalized === "build" || normalized === "plan" ? normalized : null
}

function parsePermissionLevel(value: string | null | undefined): "required" | "yolo" | null {
  if (!value) return null
  const normalized = value.trim().toLowerCase()
  return normalized === "required" || normalized === "yolo" ? normalized : null
}

function parseSubstitutionTimeoutMs(value: string | null | undefined): number | undefined {
  if (!value || value === "inherit" || value === "default") return undefined
  const normalized = value.trim().toLowerCase()
  const match = normalized.match(/^(\d+)(ms|s|m)?$/)
  if (!match) return undefined
  const amount = Number.parseInt(match[1] ?? "", 10)
  const unit = match[2] ?? "ms"
  if (!Number.isFinite(amount)) return undefined
  if (unit === "m") return amount * 60_000
  if (unit === "s") return amount * 1_000
  return amount
}
