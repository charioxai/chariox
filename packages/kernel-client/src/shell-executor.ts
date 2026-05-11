import { execFile } from "node:child_process"
import { mkdir, stat, writeFile } from "node:fs/promises"
import { basename, dirname, resolve as resolvePath } from "node:path"
import { promisify } from "node:util"

import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  type ToolTranscriptUpdate,
} from "@arroba/tool-display"

import type {
  AgentInstance,
  ArrobaMcpServerConfig,
  ArrobaSkillMetadata,
  ArrobaUserConfigSchemaPayload,
  ArrobaUserConfigPayload,
  UserConfigSchemaEntry,
  McpImportOutcome,
  ProviderAuthStatus,
  ProviderLoginStart,
  ProviderProcessInfo,
  PromptQueueItem,
  PromptSubmittedPayload,
  PairedClientRecord,
  PairingInviteRecord,
  PairingJoinRecord,
  QueuedWorkflowLaunch,
  RelayKernelPresence,
  RelayStatus,
  RemoteMachineRecord,
  RuntimeSession,
  CloudCollaborator,
  CloudSessionMember,
  SessionInvite,
  SessionHistoryPage,
  SessionHistoryPageEntry,
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
  approveRemoteMachineRequest,
  acceptCloudSessionInviteRequest,
  cloudRelayStatusRequest,
  createCloudSessionInviteRequest,
  type CreateWorkflowPublicationOptions,
  createSessionInviteRequest,
  createPairingInviteRequest,
  createWorkspaceLinkRequest,
  createWorkflowEndpointRequest,
  createWorkflowPublicationPairCodeRequest,
  createWorkflowPublicationRequest,
  createWorkflowRequest,
  createWorkflowWatchdogRequest,
  createSessionRequest,
  deleteCredentialSecretRequest,
  deleteKernelRequest,
  focusAgentRequest,
  getMcpServerRequest,
  getProviderAuthStatusRequest,
  getSessionHistoryRequest,
  getSessionStateRequest,
  getSkillRequest,
  getWorkflowPublicationRequest,
  getWorkflowRunRequest,
  grantAgentCapabilityRequest,
  importMcpServersRequest,
  importSkillsRequest,
  invokeWorkflowEndpointRequest,
  installMcpServerRequest,
  installSkillRequest,
  joinPairingInviteRequest,
  joinSessionInviteRequest,
  listWorkflowPublicationSendersRequest,
  listCloudCollaboratorsRequest,
  listCloudSessionMembersRequest,
  getUserConfigRequest,
  getUserConfigSchemaRequest,
  detachWorkspaceLinkRequest,
  listAgentsRequest,
  listMcpServersRequest,
  listPairedClientsRequest,
  listProviderProcessesRequest,
  listQueuedWorkflowLaunchesRequest,
  listRemoteMachineKernelsRequest,
  listRemoteMachinesRequest,
  listSessionMembersRequest,
  listSessionsRequest,
  listSkillsRequest,
  listWorkspaceLinksRequest,
  listWorkflowWatchdogsRequest,
  listWorkflowPublicationsRequest,
  listWorkflowRunsRequest,
  listWorkflowsRequest,
  launchProviderRunRequest,
  logoutProviderRequest,
  pumpTerminalOutputRequest,
  relayStatusRequest,
  recordPairedClientRequest,
  removeQueuedWorkflowLaunchRequest,
  removeWorkflowEdgeRequest,
  removeWorkflowNodeRequest,
  removeWorkflowWatchdogRequest,
  forgetRemoteMachineRequest,
  renameRemoteMachineRequest,
  revokeAgentCapabilityRequest,
  redeemWorkflowPublicationPairCodeRequest,
  revokePairedClientRequest,
  revokeSessionInviteRequest,
  revokeWorkflowPublicationSenderRequest,
  disableWorkflowPublicationRequest,
  resolveWorkflowRequest,
  resumeWorkflowRunRequest,
  resolveSessionRequest,
  setWorkflowFlushContextRequest,
  setWorkflowIntermediateOutputSchemaRequest,
  setWorkflowLaunchPolicyRequest,
  setWorkflowNodeCanCompleteRunRequest,
  setWorkflowNodeCanEmitIntermediateOutputRequest,
  setWorkflowNodeIntermediateOutputSchemaRequest,
  setWorkflowNodeMaxTurnsRequest,
  setWorkflowRunOutputSchemaRequest,
  setWorkflowWatchdogEnabledRequest,
  setCredentialSecretRequest,
  setUserConfigValueRequest,
  showWorkspaceLinkRequest,
  spawnAgentRequest,
  startProviderLoginRequest,
  submitPromptRequest,
  cycleAgentFocusRequest,
  teardownProviderProcessesRequest,
  uninstallMcpServerRequest,
  uninstallSkillRequest,
  updateAgentConfigRequest,
  updateAgentProfileRequest,
  updateAgentSubstitutesRequest,
  updateSessionConfigRequest,
  unsetUserConfigValueRequest,
  updateMcpServerRequest,
  updateSkillRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"

const execFileAsync = promisify(execFile)

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

type LocalGitWorktreeOptions = {
  baseDirectory: string
  targetDirectory?: string | undefined
  branch?: string | undefined
  fromRef?: string | undefined
}

type PlacementOptions = {
  positional: string[]
  directory?: string | undefined
  gitWorktree?: string | undefined
  branch?: string | undefined
  fromRef?: string | undefined
  machineRef?: string | undefined
}

export type ShellExecutorDeps = {
  client: ShellKernelClient
  clientId?: string | undefined
  readSecret?: ((prompt: string) => Promise<string>) | undefined
  prepareLocalGitWorktree?: ((options: LocalGitWorktreeOptions) => Promise<string>) | undefined
  resolveExistingDirectory?: ((directory: string, baseDirectory: string, label: string) => Promise<string>) | undefined
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
          "relay status",
          "config show|path|keys|schema|set|unset|managed-io",
          "credential list|set|delete",
          "mcp list|show|install|update|uninstall|import|grant|revoke|grants",
          "skill list|show|install|update|uninstall|import|grant|revoke|grants",
          "workspace link create|list|show|attach|detach",
          "workflow list|new|show|run|runs|cancel|resume|node|edge|endpoint",
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
        return { ok: false, message: "usage: agent spawn [alias] [model] [--dir <directory>] [--worktree <directory> --branch <branch>] [--machine <machine-ref>]" }
      }
      const worktree = await resolveShellPlacement(parsedSpawn.options, context.worktree, "agent working directory", deps)
      const remotePlacement = parsedSpawn.options.machineRef && (parsedSpawn.options.gitWorktree || parsedSpawn.options.branch || parsedSpawn.options.fromRef)
        ? {
            target_directory: parsedSpawn.options.gitWorktree ?? null,
            branch: parsedSpawn.options.branch ?? null,
            from_ref: parsedSpawn.options.fromRef ?? null,
          }
        : undefined
      const response = await deps.client.send(spawnAgentRequest(
        sessionId,
        context.provider,
        alias,
        model ?? context.model,
        worktree,
        context.effort,
        undefined,
        undefined,
        parsedSpawn.options.machineRef,
        remotePlacement,
      ))
      const agent = expectVariant<{ agent: AgentInstance }>(response, "AgentSpawned").agent
      const placement = agent.remote_execution
        ? ` on ${parsedSpawn.options.machineRef ?? agent.remote_execution.worker_machine_id}`
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

async function executeClientCommand(
  parsed: ParsedShellCommand,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [action, clientId, publicKeyThumbprint, ...rest] = parsed.args
  switch (action) {
    case "invite": {
      if (clientId !== "create") {
        return { ok: false, message: "usage: client invite create [alias]" }
      }
      const alias = publicKeyThumbprint ? [publicKeyThumbprint, ...rest].join(" ") : null
      const response = await deps.client.send(createPairingInviteRequest("client", alias))
      const payload = expectVariant<{ invite: PairingInviteRecord }>(response, "PairingInviteCreated")
      return { ok: true, message: formatPairingInvite(payload.invite), data: payload }
    }
    case "join": {
      if (!clientId) {
        return { ok: false, message: "usage: client join <invite-token> [client-id] [alias]" }
      }
      const alias = rest.length > 0 ? rest.join(" ") : null
      const response = await deps.client.send(joinPairingInviteRequest(clientId, publicKeyThumbprint ?? null, null, alias))
      const payload = expectVariant<{ pairing: PairingJoinRecord }>(response, "PairingInviteJoined")
      return { ok: true, message: formatPairingJoin(payload.pairing), data: payload }
    }
    case "list":
    case "ls": {
      const response = await deps.client.send(listPairedClientsRequest())
      const clients = expectVariant<{ clients: PairedClientRecord[] }>(response, "PairedClientsListed").clients
      return { ok: true, message: formatPairedClients(clients), data: { clients } }
    }
    case "record": {
      if (!clientId || !publicKeyThumbprint) {
        return { ok: false, message: "usage: client record <client-id> <public-key-thumbprint> [alias]" }
      }
      const alias = rest.length > 0 ? rest.join(" ") : null
      const response = await deps.client.send(recordPairedClientRequest(clientId, publicKeyThumbprint, alias))
      const payload = expectVariant<{ client: PairedClientRecord }>(response, "PairedClientRecorded")
      return { ok: true, message: `paired client ${formatPairedClientLabel(payload.client)}`, data: payload }
    }
    case "revoke": {
      if (!clientId) {
        return { ok: false, message: "usage: client revoke <client-id>" }
      }
      const response = await deps.client.send(revokePairedClientRequest(clientId))
      const payload = expectVariant<{ client: PairedClientRecord }>(response, "PairedClientRevoked")
      return { ok: true, message: `revoked client ${formatPairedClientLabel(payload.client)}`, data: payload }
    }
    default:
      return { ok: false, message: "usage: client invite create|join|list|record|revoke" }
  }
}

async function executeMachineCommand(
  parsed: ParsedShellCommand,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [action, machineRef, ...rest] = parsed.args
  switch (action) {
    case "invite": {
      if (machineRef !== "create") {
        return { ok: false, message: "usage: machine invite create [alias]" }
      }
      const alias = rest.length > 0 ? rest.join(" ") : null
      const response = await deps.client.send(createPairingInviteRequest("machine", alias))
      const payload = expectVariant<{ invite: PairingInviteRecord }>(response, "PairingInviteCreated")
      return { ok: true, message: formatPairingInvite(payload.invite), data: payload }
    }
    case "join": {
      if (!machineRef) {
        return { ok: false, message: "usage: machine join <invite-token> [machine-id] [alias]" }
      }
      const subjectId = rest[0] ?? null
      const alias = rest.length > 1 ? rest.slice(1).join(" ") : null
      const response = await deps.client.send(joinPairingInviteRequest(machineRef, subjectId, null, alias))
      const payload = expectVariant<{ pairing: PairingJoinRecord }>(response, "PairingInviteJoined")
      return { ok: true, message: formatPairingJoin(payload.pairing), data: payload }
    }
    case "list":
    case "ls": {
      const response = await deps.client.send(listRemoteMachinesRequest())
      const machines = expectVariant<{ machines: RemoteMachineRecord[] }>(response, "RemoteMachinesListed").machines
      return { ok: true, message: formatRemoteMachines(machines), data: { machines } }
    }
    case "kernels": {
      if (!machineRef) {
        return { ok: false, message: "usage: machine kernels <machine-ref>" }
      }
      const response = await deps.client.send(listRemoteMachineKernelsRequest(machineRef))
      const payload = expectVariant<{ kernels: RelayKernelPresence[] }>(response, "RemoteMachineKernelsListed")
      return { ok: true, message: formatRemoteKernels(payload.kernels, machineRef), data: payload }
    }
    case "approve": {
      if (!machineRef) {
        return { ok: false, message: "usage: machine approve <machine-ref>" }
      }
      const response = await deps.client.send(approveRemoteMachineRequest(machineRef))
      const payload = expectVariant<{ machine: RemoteMachineRecord }>(response, "RemoteMachineApproved")
      return { ok: true, message: `approved machine ${formatRemoteMachineLabel(payload.machine)}`, data: payload }
    }
    case "rename": {
      if (!machineRef || rest.length === 0) {
        return { ok: false, message: "usage: machine rename <machine-ref> <alias>" }
      }
      const alias = rest.join(" ")
      const response = await deps.client.send(renameRemoteMachineRequest(machineRef, alias))
      const payload = expectVariant<{ machine: RemoteMachineRecord }>(response, "RemoteMachineRenamed")
      return { ok: true, message: `renamed machine ${formatRemoteMachineLabel(payload.machine)}`, data: payload }
    }
    case "forget":
    case "revoke": {
      if (!machineRef) {
        return { ok: false, message: "usage: machine revoke <machine-ref>" }
      }
      const response = await deps.client.send(forgetRemoteMachineRequest(machineRef))
      const payload = expectVariant<{ machine: RemoteMachineRecord }>(response, "RemoteMachineForgotten")
      return { ok: true, message: `revoked machine ${formatRemoteMachineLabel(payload.machine)}`, data: payload }
    }
    default:
      return { ok: false, message: "usage: machine invite create|join|list|kernels|approve|rename|revoke" }
  }
}

async function executeRelayCommand(
  parsed: ParsedShellCommand,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [action] = parsed.args
  if (action && action !== "status") {
    return { ok: false, message: "usage: relay status" }
  }
  const response = await deps.client.send(relayStatusRequest())
  const status = expectVariant<{ status: RelayStatus }>(response, "RelayStatus").status
  return { ok: true, message: formatRelayStatus(status), data: { status } }
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

async function executeConfigCommand(
  parsed: ParsedShellCommand,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [action, keyPath, ...rest] = parsed.args
  if (!action || action === "show") {
    const response = await deps.client.send(getUserConfigRequest())
    const payload = expectVariant<ArrobaUserConfigPayload>(response, "UserConfig")
    return { ok: true, message: JSON.stringify(payload.config, null, 2), data: payload, format: "json" }
  }
  if (action === "path") {
    const response = await deps.client.send(getUserConfigRequest())
    const payload = expectVariant<ArrobaUserConfigPayload>(response, "UserConfig")
    return { ok: true, message: payload.path, data: payload }
  }
  if (action === "keys" || action === "list") {
    const response = await deps.client.send(getUserConfigSchemaRequest())
    const payload = expectVariant<ArrobaUserConfigSchemaPayload>(response, "UserConfigSchema")
    return { ok: true, message: formatConfigSchemaKeys(payload.entries), data: payload }
  }
  if (action === "schema") {
    const response = await deps.client.send(getUserConfigSchemaRequest())
    const payload = expectVariant<ArrobaUserConfigSchemaPayload>(response, "UserConfigSchema")
    return { ok: true, message: JSON.stringify(payload.entries, null, 2), data: payload, format: "json" }
  }
  if (action === "set") {
    const value = rest.join(" ").trim()
    if (!keyPath || !value) {
      return { ok: false, message: "usage: config set <path> <value>" }
    }
    const response = await deps.client.send(setUserConfigValueRequest(keyPath, value))
    const payload = expectVariant<ArrobaUserConfigPayload>(response, "UserConfigUpdated")
    return {
      ok: true,
      message: configMutationMessage(`config ${keyPath} set to ${value}`, payload),
      data: payload,
    }
  }
  if (action === "unset") {
    if (!keyPath) {
      return { ok: false, message: "usage: config unset <path>" }
    }
    const response = await deps.client.send(unsetUserConfigValueRequest(keyPath))
    const payload = expectVariant<ArrobaUserConfigPayload>(response, "UserConfigUpdated")
    return {
      ok: true,
      message: configMutationMessage(`config ${keyPath} unset`, payload),
      data: payload,
    }
  }
  if (action === "managed-io") {
    const mode = keyPath ?? "required"
    if (rest.length > 0 || !["required", "unrestricted", "on", "off"].includes(mode)) {
      return { ok: false, message: "usage: config managed-io required|unrestricted|on|off" }
    }
    const normalizedMode = mode === "on" ? "required" : mode === "off" ? "unrestricted" : mode
    const configPath = "providers.managed_io"
    const response = await deps.client.send(setUserConfigValueRequest(configPath, normalizedMode))
    const payload = expectVariant<ArrobaUserConfigPayload>(response, "UserConfigUpdated")
    return {
      ok: true,
      message: configMutationMessage(`managed I/O set to ${normalizedMode}`, payload),
      data: payload,
    }
  }
  return { ok: false, message: "usage: config show|path|keys|schema|set|unset|managed-io" }
}

function formatConfigSchemaKeys(entries: UserConfigSchemaEntry[]): string {
  if (entries.length === 0) {
    return "(no config keys)"
  }
  return entries
    .filter((entry) => entry.settable)
    .map((entry) => {
      const values = entry.allowed_values && entry.allowed_values.length > 0
        ? ` values=${entry.allowed_values.join("|")}`
        : ""
      const unset = entry.unsettable ? " unset" : ""
      return `${entry.path} (${entry.value_type}; ${entry.status}; ${entry.effect}${unset}${values})`
    })
    .join("\n")
}

function configMutationMessage(prefix: string, payload: ArrobaUserConfigPayload): string {
  const effects = payload.effects ?? []
  if (effects.length === 0) {
    return prefix
  }
  return [prefix, ...effects.map((effect) => effect.message)].join("\n")
}

async function executeCredentialCommand(
  parsed: ParsedShellCommand,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [action, key, ...rest] = parsed.args
  if (!action || action === "list" || action === "ls") {
    const response = await deps.client.send(getUserConfigRequest())
    const payload = expectVariant<ArrobaUserConfigPayload>(response, "UserConfig")
    const credentials = Array.isArray(payload.config.credentials) ? payload.config.credentials : []
    if (credentials.length === 0) {
      return { ok: true, message: "no credential handles configured" }
    }
    return {
      ok: true,
      message: credentials
        .map((credential: Record<string, unknown>) => {
          const id = String(credential.id ?? "")
          const source = credential.source && typeof credential.source === "object"
            ? String((credential.source as Record<string, unknown>).type ?? "unknown")
            : "unknown"
          const uses = Array.isArray(credential.allowed_uses) ? credential.allowed_uses.join(",") : "any"
          return `${id}\t${source}\t${uses || "any"}`
        })
        .join("\n"),
      format: "table",
    }
  }
  if (action === "set") {
    if (!key || rest.length > 0) {
      return { ok: false, message: "usage: credential set <key>" }
    }
    if (!deps.readSecret) {
      return {
        ok: false,
        message: "credential set requires hidden input support; run it from interactive arroba-shell",
      }
    }
    const value = await deps.readSecret(`credential ${key}: `)
    if (!value) {
      return { ok: false, message: "credential value must not be empty" }
    }
    await deps.client.send(setCredentialSecretRequest(key, value))
    return { ok: true, message: `credential ${key} stored in OS keychain` }
  }
  if (action === "delete" || action === "remove" || action === "rm") {
    if (!key || rest.length > 0) {
      return { ok: false, message: "usage: credential delete <key>" }
    }
    await deps.client.send(deleteCredentialSecretRequest(key))
    return { ok: true, message: `credential ${key} deleted from OS keychain` }
  }
  return { ok: false, message: "usage: credential list|set|delete" }
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

async function executeProviderCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellExecutorDeps,
): Promise<ShellCommandResult> {
  const [action, providerArg, ...rest] = parsed.args
  if (action === "status") {
    const provider = providerArg ?? context.provider
    const response = await deps.client.send(getProviderAuthStatusRequest(provider))
    const status = expectVariant<{ status: ProviderAuthStatus }>(response, "ProviderAuthStatus").status
    return { ok: true, message: formatProviderAuthStatus(status), data: { status } }
  }
  if (action === "login" || action === "logout" || action === "reauth") {
    const provider = providerArg ?? context.provider
    if (action === "logout") {
      const response = await deps.client.send(logoutProviderRequest(provider))
      const result = expectVariant<{ provider: string }>(response, "ProviderLoggedOut")
      return { ok: true, message: `${result.provider} logged out`, data: result }
    }
    if (action === "reauth") {
      await deps.client.send(logoutProviderRequest(provider))
    }
    const response = await deps.client.send(startProviderLoginRequest(provider))
    const login = expectVariant<{ login: ProviderLoginStart }>(response, "ProviderLoginStarted").login
    const verb = action === "reauth" ? "reauth" : "login"
    return { ok: true, message: formatProviderLoginStart(login, verb), data: { login } }
  }
  if (action === "processes") {
    const subcommand = providerArg
    if (subcommand === "teardown") {
      const provider = rest[0] ?? null
      const response = await deps.client.send(teardownProviderProcessesRequest(provider))
      const processes = expectVariant<{ processes: ProviderProcessInfo[] }>(response, "ProviderProcessesTornDown").processes
      return { ok: true, message: processes.length === 0 ? "no safe provider processes to tear down" : `tore down ${processes.length} provider process(es)\n${formatProviderProcesses(processes)}`, data: { processes } }
    }
    const provider = providerArg ?? null
    const response = await deps.client.send(listProviderProcessesRequest(provider))
    const processes = expectVariant<{ processes: ProviderProcessInfo[] }>(response, "ProviderProcessesListed").processes
    return { ok: true, message: formatProviderProcesses(processes), data: { processes } }
  }
  return { ok: false, message: "usage: provider status|login|logout|reauth|processes" }
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

async function writeWorkflowPublicationExportPackage(
  publication: WorkflowPublicationDefinition,
  outputRoot: string,
  kernelUrl?: string,
) {
  await mkdir(outputRoot, { recursive: true })
  const config = workflowPublicationGatewayConfig(publication, kernelUrl)
  const files = {
    "publication.config.json": JSON.stringify(config, null, 2) + "\n",
    ".env.example": workflowPublicationEnvTemplate(publication, kernelUrl),
    "run.sh": workflowPublicationLauncherScript(),
    "README.md": workflowPublicationReadme(publication, config),
  }
  const paths: string[] = []
  for (const [name, content] of Object.entries(files)) {
    const filePath = resolvePath(outputRoot, name)
    await writeFile(filePath, content, name === "run.sh" ? { mode: 0o755 } : undefined)
    paths.push(filePath)
  }
  return paths
}

function workflowPublicationGatewayConfig(
  publication: WorkflowPublicationDefinition,
  kernelUrl?: string,
) {
  const config: Record<string, unknown> = {
    publication_id: publication.id,
    session_id: publication.session_id,
    workflow_ref: publication.workflow_id,
    endpoint_ref: publication.endpoint_id,
    route: publication.route ?? "/*",
    auth: publication.auth ?? { mode: "anonymous" },
    parser: publication.parser ?? { kind: "json" },
    mode: publication.mode === "async" ? "async" : "sync",
  }
  if (kernelUrl) config.kernel_endpoint = kernelUrl
  if (publication.methods?.length) config.methods = publication.methods
  if (publication.transport != null) config.transport = publication.transport
  if (publication.input_schema != null) config.input_schema = publication.input_schema
  return config
}

function workflowPublicationEnvTemplate(publication: WorkflowPublicationDefinition, kernelUrl?: string) {
  return [
    "# Copy this file to .env or export these variables before running run.sh.",
    "HOST=0.0.0.0",
    "PORT=3000",
    `ARROBA_KERNEL_URL=${kernelUrl ?? "ws://127.0.0.1:43118"}`,
    "ARROBA_PUBLICATION_CONFIG=./publication.config.json",
    `ARROBA_PUBLICATION_SESSION_ID=${publication.session_id}`,
    `ARROBA_PUBLICATION_ID=${publication.id}`,
    "",
    "# Optional HTTPS/TLS. When both files are set, the gateway serves HTTPS.",
    "# ARROBA_PUBLICATION_TLS_KEY_FILE=./tls.key",
    "# ARROBA_PUBLICATION_TLS_CERT_FILE=./tls.crt",
    "# ARROBA_PUBLICATION_TLS_ENABLED=true",
    "",
  ].join("\n")
}

function workflowPublicationLauncherScript() {
  return [
    "#!/usr/bin/env bash",
    "set -euo pipefail",
    "DIR=\"$(cd \"$(dirname \"${BASH_SOURCE[0]}\")\" && pwd)\"",
    "if [ -f \"$DIR/.env\" ]; then",
    "  set -a",
    "  . \"$DIR/.env\"",
    "  set +a",
    "fi",
    "export ARROBA_PUBLICATION_CONFIG=\"${ARROBA_PUBLICATION_CONFIG:-$DIR/publication.config.json}\"",
    "exec arroba-workflow-gateway",
    "",
  ].join("\n")
}

function workflowPublicationReadme(
  publication: WorkflowPublicationDefinition,
  config: Record<string, unknown>,
) {
  const route = String(config.route ?? "/*")
  const examplePath = route.includes("*") ? route.replace("*", "example") : route
  const methods = Array.isArray(config.methods) && config.methods.length ? config.methods.map(String) : ["GET", "POST"]
  const primaryMethod = methods[0] ?? "GET"
  const paired = isPairedSenderPublication(publication)
  const authHint = paired
    ? [
        "This publication uses paired sender auth.",
        "",
        "```bash",
        "PAIR_CODE=\"paste-code-here\"",
        "curl -sS -X POST \"$BASE_URL/.well-known/arroba/publication/pair\" \\",
        "  -H 'content-type: application/json' \\",
        "  -d \"{\\\"pair_code\\\":\\\"$PAIR_CODE\\\",\\\"display_name\\\":\\\"example sender\\\"}\"",
        "```",
        "",
        "Use the returned credential as `Authorization: Bearer <credential>`.",
      ].join("\n")
    : "This publication does not require paired sender auth unless its auth config says otherwise."
  const body = primaryMethod === "GET"
    ? ""
    : " \\\n  -H 'content-type: application/json' \\\n  -d '{\"input\":\"hello\"}'"
  const authHeader = paired ? " \\\n  -H \"authorization: Bearer $SENDER_CREDENTIAL\"" : ""
  return [
    `# Workflow Publication ${publication.alias ?? publication.id}`,
    "",
    "This directory is an Arroba workflow-gateway package. It runs only when an Arroba kernel is reachable.",
    "",
    "## Files",
    "",
    "- `publication.config.json`: gateway publication config",
    "- `.env.example`: environment template",
    "- `run.sh`: launcher for `arroba-workflow-gateway`",
    "",
    "## Run",
    "",
    "```bash",
    "cp .env.example .env",
    "./run.sh",
    "```",
    "",
    "## Invoke",
    "",
    "```bash",
    "BASE_URL=http://127.0.0.1:3000",
    `curl -sS -X ${primaryMethod} "$BASE_URL${examplePath}"${authHeader}${body}`,
    "```",
    "",
    "## WebSocket",
    "",
    "The gateway also accepts WebSocket clients at:",
    "",
    "```text",
    "ws://127.0.0.1:3000/.well-known/arroba/publication/ws",
    "wss://127.0.0.1:3000/.well-known/arroba/publication/ws",
    "```",
    "",
    "Send `{\"type\":\"invoke\",\"input\":{}}` to invoke the publication.",
    "",
    "## Local IPC",
    "",
    "Local scripts can invoke the publication without starting the HTTP gateway:",
    "",
    "```bash",
    "arroba-workflow-call --config ./publication.config.json --input '{\"input\":\"hello\"}'",
    "```",
    "",
    "## Auth",
    "",
    authHint,
    "",
  ].join("\n")
}

function isPairedSenderPublication(publication: WorkflowPublicationDefinition) {
  const auth = publication.auth as { mode?: string; paired_senders?: { enabled?: boolean } } | null | undefined
  return auth?.mode === "arroba" && auth.paired_senders?.enabled === true
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

function formatPromptSummary(history: SessionHistoryPageEntry[]): string {
  const summary = history
    .filter((entry) => entry.entry.kind === "provider_output")
    .map((entry) => entry.entry.text.trim())
    .filter(Boolean)
    .join("\n")
  return summary || "(no summary output)"
}

function formatPromptReply(history: SessionHistoryPageEntry[]): string {
  const tools = new Map<string, ToolTranscriptUpdate>()
  const reply = history
    .map((entry) => formatPromptHistoryEntry(entry, tools))
    .filter(Boolean)
    .join("\n")
  return reply || "(no reply output)"
}

function formatPromptHistoryEntry(
  entry: SessionHistoryPageEntry,
  tools: Map<string, ToolTranscriptUpdate>,
): string {
  const text = entry.entry.text.trim()
  if (!text) return ""
  if (entry.entry.kind === "provider_output") return text
  if (entry.entry.kind !== "provider_tool") return `[${entry.entry.kind}] ${text}`

  const parsed = parseToolTranscriptUpdate(entry.entry.text)
  if (!parsed) return `[${entry.entry.kind}] ${text}`

  const merged = mergeToolTranscriptUpdate(tools.get(parsed.id) ?? null, parsed)
  tools.set(parsed.id, merged)
  return formatToolTranscriptUpdate(merged)
}

function formatPromptBlob(promptId: string, title: string, content: string): string {
  const indent = "                        "
  const lines = content.split(/\r?\n/)
  return [`${promptId} ${title}`, ...lines.map((line) => `${indent}${line}`)].join("\n")
}

function formatAgentRef(agent: AgentInstance): string {
  return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function parsePlacementOptions(args: string[], allowMachine: boolean): { options: PlacementOptions; error?: string } {
  const options: PlacementOptions = { positional: [] }
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    const next = args[index + 1]
    if ((arg === "--dir" || arg === "--directory") && next) {
      options.directory = next
      index += 1
    } else if (arg === "--worktree" && next) {
      options.gitWorktree = next
      index += 1
    } else if (arg === "--branch" && next) {
      options.branch = next
      index += 1
    } else if (arg === "--from" && next) {
      options.fromRef = next
      index += 1
    } else if (arg === "--machine" && next && allowMachine) {
      options.machineRef = next
      index += 1
    } else if (arg?.startsWith("--")) {
      return { options, error: `unknown or incomplete option: ${arg}` }
    } else if (arg) {
      options.positional.push(arg)
    }
  }
  if (options.directory && options.gitWorktree) {
    return { options, error: "use either --dir or --worktree, not both" }
  }
  if ((options.branch || options.fromRef) && !options.gitWorktree) {
    return { options, error: "--branch/--from require --worktree" }
  }
  return { options }
}

async function resolveShellPlacement(
  options: PlacementOptions,
  baseDirectory: string,
  label: string,
  deps: ShellExecutorDeps,
): Promise<string | undefined> {
  if (options.machineRef) {
    return options.directory ?? options.gitWorktree ?? undefined
  }
  const positionalDirectory = options.positional.length === 1 && !options.directory && !options.gitWorktree
    ? options.positional[0]
    : undefined
  if (positionalDirectory || options.directory) {
    const directory = positionalDirectory ?? options.directory!
    const resolver = deps.resolveExistingDirectory ?? defaultResolveExistingDirectory
    return resolver(directory, baseDirectory, label)
  }
  if (options.gitWorktree) {
    const prepare = deps.prepareLocalGitWorktree ?? defaultPrepareLocalGitWorktree
    return prepare({
      baseDirectory,
      targetDirectory: options.gitWorktree,
      branch: options.branch,
      fromRef: options.fromRef,
    })
  }
  return undefined
}

async function defaultResolveExistingDirectory(directory: string, baseDirectory: string, label: string): Promise<string> {
  const resolved = resolvePath(baseDirectory, directory)
  const details = await stat(resolved)
  if (!details.isDirectory()) {
    throw new Error(`${label} is not a directory: ${resolved}`)
  }
  return resolved
}

async function defaultPrepareLocalGitWorktree(options: LocalGitWorktreeOptions): Promise<string> {
  const baseDirectory = resolvePath(options.baseDirectory)
  const repoRoot = (await runGit(baseDirectory, ["rev-parse", "--show-toplevel"])).trim()
  if (!repoRoot) {
    throw new Error(`git did not report a repository root for ${baseDirectory}`)
  }
  const fromRef = options.fromRef ?? "HEAD"
  const targetDirectory = options.targetDirectory
    ? resolvePath(baseDirectory, options.targetDirectory)
    : resolvePath(dirname(repoRoot), `${basename(repoRoot)}-${slugifyGitBranch(options.branch ?? fromRef)}`)
  const args = options.branch
    ? ["worktree", "add", "-b", options.branch, targetDirectory, fromRef]
    : ["worktree", "add", targetDirectory, fromRef]
  await runGit(repoRoot, args)
  const details = await stat(targetDirectory)
  if (!details.isDirectory()) {
    throw new Error(`created git worktree is not a directory: ${targetDirectory}`)
  }
  return targetDirectory
}

async function runGit(cwd: string, args: string[]): Promise<string> {
  try {
    const { stdout } = await execFileAsync("git", args, { cwd })
    return stdout
  } catch (error) {
    const detail = error && typeof error === "object" && "stderr" in error
      ? String((error as { stderr?: unknown }).stderr ?? "").trim()
      : ""
    const message = detail || (error instanceof Error ? error.message : String(error))
    throw new Error(`git ${args.join(" ")} failed in ${cwd}: ${message}`)
  }
}

function slugifyGitBranch(value: string): string {
  return value
    .trim()
    .replace(/[^A-Za-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    || "worktree"
}


function formatRemoteMachines(machines: RemoteMachineRecord[]): string {
  if (machines.length === 0) {
    return "no remote machines"
  }
  return machines.map((machine) => {
    const name = formatRemoteMachineLabel(machine)
    const providers = (machine.available_providers ?? []).join(",") || "-"
    const offline = machine.online ? "" : ",offline"
    return `${name} id=${machine.machine_id} status=${machine.trust_status}${offline} kernels=${machine.kernel_count} providers=${providers}`
  }).join("\n")
}

function formatRemoteMachineLabel(machine: RemoteMachineRecord): string {
  return machine.display_name || machine.machine_alias || machine.registry_alias || machine.machine_id
}

function formatPairedClients(clients: PairedClientRecord[]): string {
  if (clients.length === 0) {
    return "no paired clients"
  }
  return clients.map((client) => {
    const label = formatPairedClientLabel(client)
    const revoked = client.revoked ? " revoked=true" : ""
    return `${label} thumbprint=${client.public_key_thumbprint} paired_at_ms=${client.paired_at_ms}${revoked}`
  }).join("\n")
}

function formatPairedClientLabel(client: PairedClientRecord): string {
  return client.alias ? `${client.alias} id=${client.client_id}` : client.client_id
}

function formatPairingInvite(invite: PairingInviteRecord): string {
  return [
    `${invite.intent} invite ${invite.invite_id}`,
    `target=${invite.target_daemon_alias ?? invite.target_daemon_id}`,
    `relay=${invite.relay_url}`,
    `expires_at_ms=${invite.expires_at_ms}`,
    `token=${invite.invite_token}`,
  ].join("\n")
}

function formatPairingJoin(pairing: PairingJoinRecord): string {
  const alias = pairing.alias ? ` alias=${pairing.alias}` : ""
  return `joined ${pairing.intent} ${pairing.subject_id}${alias} target=${pairing.target_daemon_id} thumbprint=${pairing.public_key_thumbprint}`
}

function formatRemoteKernels(kernels: RelayKernelPresence[], machineRef: string): string {
  if (kernels.length === 0) {
    return `no live kernels found for machine ${machineRef}`
  }
  return kernels.map((kernel) => {
    const name = kernel.relay_alias ?? kernel.kernel_alias ?? kernel.kernel_id
    const providers = (kernel.available_providers ?? []).join(",") || "-"
    return `${name} id=${kernel.kernel_id} machine=${kernel.machine_alias ?? kernel.machine_id} providers=${providers} accepting_remote_leases=${String(kernel.accepting_remote_leases ?? false)} leased_agents=${kernel.leased_agent_count ?? 0} local_sessions=${kernel.local_session_count ?? 0}`
  }).join("\n")
}

function formatRelayStatus(status: RelayStatus): string {
  const state = !status.configured ? "not configured" : status.connected ? "connected" : "configured, disconnected"
  return [
    `relay ${state}`,
    `url=${status.relay_url ?? "-"}`,
    `token_configured=${String(status.relay_token_configured)}`,
    `daemon=${status.daemon_id}`,
    `machine=${status.machine_alias ?? status.machine_id}`,
  ].join("\n")
}

function formatMcpList(mcps: ArrobaMcpServerConfig[]): string {
  if (mcps.length === 0) {
    return "no MCP servers installed"
  }
  return mcps.map((mcp) => {
    const enabled = mcp.enabled === false ? "disabled" : "enabled"
    const transport = Object.keys(mcp.transport ?? {})[0] ?? "transport"
    return `${mcp.name} [${enabled}] ${transport}`
  }).join("\n")
}

function formatSkillList(skills: ArrobaSkillMetadata[]): string {
  if (skills.length === 0) {
    return "no skills installed"
  }
  return skills.map((skill) => {
    const description = skill.short_description || skill.description || skill.path
    return `${skill.name} - ${description}`
  }).join("\n")
}

function formatMcpImportOutcome(outcome: McpImportOutcome): string {
  const lines: string[] = []
  if (outcome.imported.length > 0) {
    lines.push(`Imported MCPs: ${outcome.imported.map((mcp) => mcp.name).join(", ")}`)
  }
  if (outcome.skipped.length > 0) {
    lines.push("Skipped MCPs:")
    lines.push(...outcome.skipped.map((skip) => `- ${skip.name}: ${skip.reason}`))
  }
  return lines.length === 0 ? "No MCPs imported." : lines.join("\n")
}

function formatSkillImportOutcome(outcome: SkillImportOutcome): string {
  const lines: string[] = []
  if (outcome.imported.length > 0) {
    lines.push(`Imported skills: ${outcome.imported.map((skill) => skill.name).join(", ")}`)
  }
  if (outcome.skipped.length > 0) {
    lines.push("Skipped skills:")
    lines.push(...outcome.skipped.map((skip) => `- ${skip.name}: ${skip.reason}`))
  }
  return lines.length === 0 ? "No skills imported." : lines.join("\n")
}

function formatAgentCapabilityGrants(agent: AgentInstance, kind: "mcp" | "skill"): string {
  const grants = kind === "mcp" ? (agent.mcp_grants ?? []) : (agent.skill_grants ?? [])
  const label = kind === "mcp" ? "MCP" : "skill"
  const agentLabel = `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
  if (grants.length === 0) {
    return `${agentLabel} has no ${label} grants.`
  }
  return `${agentLabel} ${label} grants:\n${grants.map((grant) => `- ${grant}`).join("\n")}`
}

function formatWorkflowLabel(workflow: WorkflowDefinition): string {
  return workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id
}

function formatWorkflowList(workflows: WorkflowDefinition[], currentWorkflowId?: string): string {
  if (workflows.length === 0) {
    return "no workflows in session"
  }
  return workflows.map((workflow) => {
    const current = workflow.id === currentWorkflowId ? " current" : ""
    return `${formatWorkflowLabel(workflow)} nodes=${workflow.nodes?.length ?? 0} edges=${workflow.edges?.length ?? 0} endpoints=${workflow.endpoints?.length ?? 0}${current}`
  }).join("\n")
}

function formatWorkflowDetails(workflow: WorkflowDefinition): string {
  return [
    `workflow ${formatWorkflowLabel(workflow)}`,
    `nodes=${workflow.nodes?.length ?? 0} edges=${workflow.edges?.length ?? 0} endpoints=${workflow.endpoints?.length ?? 0}`,
    `flush_context=${String(workflow.flush_agent_context_before_run ?? true)}`,
    workflow.run_output_schema_ref ? `run_output_schema=${workflow.run_output_schema_ref}` : null,
    workflow.intermediate_output_schema_ref ? `intermediate_output_schema=${workflow.intermediate_output_schema_ref}` : null,
  ].filter(Boolean).join("\n")
}

function formatWorkflowRunList(workflowRuns: WorkflowRun[], workflowRef: string | null): string {
  if (workflowRuns.length === 0) {
    return workflowRef ? `no workflow runs for ${workflowRef}` : "no workflow runs in session"
  }
  return workflowRuns.map((run) => {
    const failures = (run.failure_events?.length ?? 0) > 0 ? ` failures=${run.failure_events?.length ?? 0}` : ""
    return `${run.id} workflow=${run.workflow_id} endpoint=${run.endpoint_id} [${String(run.status).toLowerCase()}${failures}]`
  }).join("\n")
}

function formatWorkflowPublicationLabel(publication: WorkflowPublicationDefinition): string {
  return publication.alias ? `${publication.id} (${publication.alias})` : publication.id
}

function formatWorkflowPublications(publications: WorkflowPublicationDefinition[]): string {
  if (publications.length === 0) {
    return "no workflow publications configured"
  }
  return publications.map((publication) => {
    const route = publication.route ? ` route=${publication.route}` : ""
    const methods = publication.methods?.length ? ` methods=${publication.methods.join(",")}` : ""
    return `${formatWorkflowPublicationLabel(publication)} workflow=${publication.workflow_id} endpoint=${publication.endpoint_id} enabled=${String(publication.enabled)}${route}${methods}`
  }).join("\n")
}

function formatWorkflowPublicationSenders(senders: WorkflowPublicationTrustedSender[]): string {
  if (senders.length === 0) {
    return "no trusted senders configured"
  }
  return senders.map((sender) => {
    const name = sender.display_name ? ` (${sender.display_name})` : ""
    const transports = sender.allowed_transports?.length ? ` transports=${sender.allowed_transports.join(",")}` : ""
    const revoked = sender.revoked_at_ms ? " revoked=true" : ""
    return `${sender.sender_id}${name} publication=${sender.publication_id}${transports}${revoked}`
  }).join("\n")
}

function formatWorkflowWatchdogs(watchdogs: WorkflowWatchdogDefinition[]): string {
  if (watchdogs.length === 0) {
    return "no workflow watchdogs configured"
  }
  return watchdogs.map((watchdog) => (
    `${watchdog.id} workflow=${watchdog.workflow_id} endpoint=${watchdog.endpoint_id} every=${watchdog.interval_seconds}s policy=${watchdog.policy} enabled=${String(watchdog.enabled)} wakeups=${watchdog.wakeups_executed}/${watchdog.max_wakeups ?? "unbounded"}`
  )).join("\n")
}

function formatQueuedWorkflowLaunches(queuedLaunches: QueuedWorkflowLaunch[]): string {
  if (queuedLaunches.length === 0) {
    return "workflow queue is empty"
  }
  return queuedLaunches.map((queued) => (
    `${queued.id} workflow=${queued.workflow_id} endpoint=${queued.endpoint_id} source=${queued.source}`
  )).join("\n")
}

function formatProviderAuthStatus(status: ProviderAuthStatus): string {
  return [
    status.account_profile ? `${status.provider}: ${status.auth_state} as ${status.account_profile}` : `${status.provider}: ${status.auth_state}`,
    status.detected_version ? `version ${status.detected_version}` : null,
    status.login_hint ?? null,
  ].filter(Boolean).join(" • ")
}

function formatProviderLoginStart(login: ProviderLoginStart, verb: "login" | "reauth"): string {
  return [
    `${login.provider} ${verb} started`,
    login.user_code ? `code ${login.user_code}` : null,
    login.verification_url ?? login.auth_url ?? null,
  ].filter(Boolean).join(" • ")
}

function formatProviderProcesses(processes: ProviderProcessInfo[]): string {
  if (processes.length === 0) {
    return "no daemon-tracked provider processes"
  }
  return processes.map((process) => {
    const blockers = process.teardown_blockers.length > 0 ? ` blockers=${process.teardown_blockers.join(",")}` : ""
    return `${process.process_id} ${process.provider} ${process.process_label} status=${process.status} safe=${String(process.teardown_safe)} sessions=${process.owner_session_ids.join(",") || "-"}${blockers}`
  }).join("\n")
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}

function formatSessionList(sessions: RuntimeSession[], currentSessionId?: string): string {
  if (sessions.length === 0) {
    return "No sessions found."
  }
  return [
    "Sessions",
    ...sessions.map((session) => {
      const name = session.alias ? `\`${session.alias}\` (\`${session.id}\`)` : `\`${session.id}\``
      const location = basename(session.worktree_id) || session.worktree_id
      const attachments = `${session.attachment_ids.length} ${session.attachment_ids.length === 1 ? "CLI" : "CLIs"}`
      const current = session.id === currentSessionId ? " current" : ""
      return `- ${name} - ${session.status.toLowerCase()} - ${attachments} - ${location}${current}`
    }),
  ].join("\n")
}

function formatSessionMembers(members: SessionMember[], invites: SessionInvite[]): string {
  const lines = ["Session members"]
  if (members.length === 0) {
    lines.push("- none")
  } else {
    for (const member of members) {
      const inviter = member.invited_by_user_id ? ` invited_by=${member.invited_by_user_id}` : ""
      lines.push(`- ${member.user_id}${inviter}`)
    }
  }
  lines.push("Session invites")
  const activeInvites = invites.filter((invite) => !invite.revoked_at_ms)
  if (activeInvites.length === 0) {
    lines.push("- none")
  } else {
    for (const invite of activeInvites) {
      const maxUses = invite.max_uses ?? "unlimited"
      lines.push(`- ${invite.invite_id} uses=${invite.used_count}/${maxUses}`)
    }
  }
  return lines.join("\n")
}

function formatSessionInvite(invite: SessionInvite, inviteToken: string): string {
  const maxUses = invite.max_uses ?? "unlimited"
  const expires = invite.expires_at_ms ? ` expires_at=${invite.expires_at_ms}` : ""
  return `session invite ${invite.invite_id} uses=0/${maxUses}${expires}\n${inviteToken}`
}

function formatCloudMembers(members: CloudSessionMember[]): string {
  if (members.length === 0) {
    return "no cloud members in session"
  }
  return members.map((member) => (
    `${member.user_id} ${member.email}${member.display_name ? ` (${member.display_name})` : ""}`
  )).join("\n")
}

function formatCloudCollaborators(collaborators: CloudCollaborator[]): string {
  if (collaborators.length === 0) {
    return "no recent cloud collaborators"
  }
  return collaborators.map((collaborator) => (
    `${collaborator.user_id} ${collaborator.email} shared_sessions=${collaborator.shared_session_count}`
  )).join("\n")
}

function formatWorkspaceLinks(links: WorkspaceLinkDefinition[]): string {
  if (links.length === 0) {
    return "no workspace links in session"
  }
  return links.map((link) => (
    `${link.name} (${link.link_id}) attachments=${link.attachments?.length ?? 0}`
  )).join("\n")
}

function formatWorkspaceLinkDetails(link: WorkspaceLinkDefinition): string {
  const lines = [
    `workspace link ${link.name} (${link.link_id})`,
    `created_by=${link.created_by_user_id}`,
    `attachments=${link.attachments?.length ?? 0}`,
  ]
  for (const attachment of link.attachments ?? []) {
    const branch = attachment.branch ? ` branch=${attachment.branch}` : ""
    lines.push(`- ${attachment.user_id} ${attachment.repo_root}${branch}`)
  }
  return lines.join("\n")
}

function formatAgentListSummary(agents: AgentInstance[]): string {
  if (agents.length === 0) {
    return "no agents in session"
  }
  const agentList = agents
    .map((agent) => {
      const mode = agent.execution_mode_override ? ` mode=${agent.execution_mode_override}` : ""
      const permissions = agent.permission_level_override ? ` permissions=${agent.permission_level_override}` : ""
      return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""} [${agent.state}]${mode}${permissions}`
    })
    .join(", ")
  return `${agents.length} agent${agents.length === 1 ? "" : "s"}: ${agentList}`
}

function formatAgentSubstituteSummary(agent: AgentInstance): string {
  const substitutes = agent.substitutes ?? []
  if (substitutes.length === 0) {
    return `${formatAgentRef(agent)} has no substitutes`
  }
  const lines = substitutes.map((substitute, index) => {
    const marker = agent.active_substitute_index === index ? "*" : "-"
    const variant = substitute.variant ? `/${substitute.variant}` : ""
    return `${marker} ${index}: ${substitute.provider}/${substitute.model}${variant}`
  })
  const timeout = agent.substitution_timeout_ms == null ? "default" : `${agent.substitution_timeout_ms}ms`
  return `${formatAgentRef(agent)} substitutes (${substitutes.length}, timeout ${timeout}):\n${lines.join("\n")}`
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
