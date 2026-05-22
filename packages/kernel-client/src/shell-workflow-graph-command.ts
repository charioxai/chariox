import { randomUUID } from "node:crypto"
import { readFile } from "node:fs/promises"
import { isAbsolute, resolve as resolvePath } from "node:path"

import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEdgeDefinition,
  WorkflowEndpointDefinition,
  WorkflowNodeDefinition,
} from "./kernel-types.js"
import {
  addWorkflowEdgeRequest,
  addWorkflowNodeRequest,
  aliasWorkflowEndpointRequest,
  bindWorkflowEndpointRequest,
  createWorkflowEndpointRequest,
  removeWorkflowEdgeRequest,
  removeWorkflowNodeRequest,
  setWorkflowNodeCanCompleteRunRequest,
  setWorkflowNodeCanEmitIntermediateOutputRequest,
  setWorkflowNodeIntermediateOutputSchemaRequest,
  setWorkflowNodeMaxTurnsRequest,
  applyWorkflowDesignOpRequest,
  getSessionStateRequest,
  grantAgentExtensionRequest,
  resolveWorkflowRequest,
  revokeAgentExtensionRequest,
  updateWorkflowNodeInstructionsRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import { resolveShellAgent } from "./shell-agent-resolver.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellWorkflowGraphCommandDeps = {
  client: ShellKernelClient
  clientId?: string | undefined
}

export async function executeWorkflowNodeCommand(
  args: string[],
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellWorkflowGraphCommandDeps,
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
  if (action === "instructions") {
    const instructionsAction = args[1]
    if (instructionsAction === "show") {
      const explicitWorkflowRef = args.length >= 4 ? args[2] : null
      const workflowRef = explicitWorkflowRef ?? context.workflowId
      const nodeId = explicitWorkflowRef ? args[3] : args[2]
      if (!workflowRef || !nodeId) {
        return { ok: false, message: "usage: workflow node instructions show [workflow-ref] <node-id>" }
      }
      const response = await deps.client.send(resolveWorkflowRequest(sessionId, workflowRef))
      const payload = expectVariant<{ workflow: WorkflowDefinition }>(response, "WorkflowResolved")
      const node = payload.workflow.nodes?.find((entry) => entry.id === nodeId)
      if (!node) {
        return { ok: false, message: `workflow node ${nodeId} not found` }
      }
      return {
        ok: true,
        message: node.instructions?.trim() ? node.instructions : "(no instructions)",
        data: { workflow: payload.workflow, node },
        contextUpdates: { workflowId: payload.workflow.id },
      }
    }
    if (instructionsAction === "set") {
      const explicitWorkflowRef = args.length >= 5 ? args[2] : null
      const workflowRef = explicitWorkflowRef ?? context.workflowId
      const nodeId = explicitWorkflowRef ? args[3] : args[2]
      const fileRef = explicitWorkflowRef ? args[4] : args[3]
      if (!workflowRef || !nodeId || !fileRef) {
        return { ok: false, message: "usage: workflow node instructions set [workflow-ref] <node-id> <file>" }
      }
      const instructionsPath = isAbsolute(fileRef) ? fileRef : resolvePath(context.worktree, fileRef)
      const instructions = await readFile(instructionsPath, "utf8")
      if (deps.clientId) {
        const resolved = await deps.client.send(resolveWorkflowRequest(sessionId, workflowRef))
        const resolvedPayload = expectVariant<{ workflow: WorkflowDefinition }>(resolved, "WorkflowResolved")
        const node = resolvedPayload.workflow.nodes?.find((entry) => entry.id === nodeId)
        if (!node) {
          return { ok: false, message: `workflow node ${nodeId} not found` }
        }
        const response = await deps.client.send(applyWorkflowDesignOpRequest(
          sessionId,
          deps.clientId,
          `shell-${randomUUID()}`,
          { kind: "node_update", workflow_id: resolvedPayload.workflow.id, node_id: nodeId, patch: { instructions } },
        ))
        const payload = expectVariant<{ event: { op?: unknown }; session: RuntimeSession }>(response, "WorkflowDesignOpAccepted")
        return {
          ok: true,
          message: `updated workflow node ${nodeId} instructions`,
          data: payload,
          contextUpdates: { workflowId: resolvedPayload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
        }
      }
      const response = await deps.client.send(updateWorkflowNodeInstructionsRequest(sessionId, workflowRef, nodeId, instructions))
      const payload = expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(response, "WorkflowNodeInstructionsUpdated")
      return { ok: true, message: `updated workflow node ${payload.node.id} instructions`, data: payload, contextUpdates: { workflowId: payload.workflow.id, sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined } }
    }
    return { ok: false, message: "usage: workflow node instructions show|set [workflow-ref] <node-id> [file]" }
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
  if (action === "extensions") {
    const explicitWorkflowRef = args.length >= 3 ? args[1] : null
    const workflowRef = explicitWorkflowRef ?? context.workflowId
    const nodeId = explicitWorkflowRef ? args[2] : args[1]
    if (!workflowRef || !nodeId) {
      return { ok: false, message: "usage: workflow node extensions [workflow-ref] <node-id>" }
    }
    const resolved = await resolveWorkflowNodeAgent(deps, sessionId, workflowRef, nodeId)
    if (!resolved.ok) return resolved
    const sessionResponse = await deps.client.send(getSessionStateRequest(sessionId))
    const session = expectAnyVariant<{ session: RuntimeSession }>(sessionResponse, ["SessionState", "SessionStateLoaded"]).session
    const agent = session.agents.find((candidate) => candidate.id === resolved.node.agent_id)
    const grants = agent?.extension_grants ?? []
    return {
      ok: true,
      message: grants.length
        ? grants.map((grant) => `${grant.kind}:${grant.name}${grant.environment ? `@${grant.environment}` : ""}${grant.max_safety ? ` allow=${grant.max_safety}` : ""}`).join("\n")
        : `node ${resolved.node.id} agent ${resolved.agentRef} has no extensions`,
      data: { workflow: resolved.workflow, node: resolved.node, agent },
      contextUpdates: { workflowId: resolved.workflow.id, sessionId },
    }
  }
  if (action === "extension") {
    const extensionAction = args[1]
    const hasExplicitWorkflow = args.length >= 6
    const workflowRef = (hasExplicitWorkflow ? args[2] : context.workflowId) ?? null
    const nodeId = hasExplicitWorkflow ? args[3] : args[2]
    const kind = hasExplicitWorkflow ? args[4] : args[3]
    const name = hasExplicitWorkflow ? args[5] : args[4]
    if ((extensionAction !== "grant" && extensionAction !== "revoke") || !workflowRef || !nodeId || !isExtensionKind(kind) || !name) {
      return { ok: false, message: "usage: workflow node extension grant|revoke [workflow-ref] <node-id> <mcp|skill|script|connector> <name> [--environment <name>] [--credential <id>] [--allow read|write|destructive]" }
    }
    const resolved = await resolveWorkflowNodeAgent(deps, sessionId, workflowRef, nodeId)
    if (!resolved.ok) return resolved
    const response = extensionAction === "grant"
      ? await deps.client.send(grantAgentExtensionRequest(
        context.workspace,
        resolved.agentRef,
        kind,
        name,
        readOption(args, "--environment"),
        { credential: readOption(args, "--credential"), maxSafety: readOption(args, "--allow") },
      ))
      : await deps.client.send(revokeAgentExtensionRequest(resolved.agentRef, kind, name))
    const payload = expectVariant<{ agent: unknown }>(response, extensionAction === "grant" ? "AgentExtensionGranted" : "AgentExtensionRevoked")
    return {
      ok: true,
      message: `${extensionAction === "grant" ? "granted" : "revoked"} ${kind} ${name} ${extensionAction === "grant" ? "to" : "from"} workflow node ${resolved.node.id}`,
      data: payload,
      contextUpdates: { workflowId: resolved.workflow.id, sessionId },
    }
  }
  return { ok: false, message: "usage: workflow node add [workflow-ref] <agent-ref> | remove [workflow-ref] <node-id> | instructions show|set ... | can-complete-run|can-emit-intermediate-output|intermediate-output-schema|max-turns|extensions|extension ..." }
}

export async function executeWorkflowEdgeCommand(
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowGraphCommandDeps,
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

export async function executeWorkflowEndpointCommand(
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowGraphCommandDeps,
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

async function resolveWorkflowNodeAgent(
  deps: ShellWorkflowGraphCommandDeps,
  sessionId: string,
  workflowRef: string,
  nodeId: string,
): Promise<
  | { ok: true; workflow: WorkflowDefinition; node: WorkflowNodeDefinition; agentRef: string }
  | { ok: false; message: string }
> {
  const response = await deps.client.send(resolveWorkflowRequest(sessionId, workflowRef))
  const payload = expectVariant<{ workflow: WorkflowDefinition }>(response, "WorkflowResolved")
  const node = payload.workflow.nodes?.find((entry) => entry.id === nodeId)
  if (!node) return { ok: false, message: `workflow node ${nodeId} not found` }
  return { ok: true, workflow: payload.workflow, node, agentRef: node.agent_id }
}

function readOption(args: readonly string[], name: string): string | null {
  const index = args.indexOf(name)
  return index >= 0 ? args[index + 1] ?? null : null
}

function isExtensionKind(value: unknown): value is "mcp" | "skill" | "script" | "connector" {
  return value === "mcp" || value === "skill" || value === "script" || value === "connector"
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}

function expectAnyVariant<T>(response: Record<string, unknown>, variants: readonly string[]): T {
  for (const variant of variants) {
    if (variant in response) return response[variant] as T
  }
  throw new Error(`unexpected response variant: expected ${variants.join(" or ")}`)
}
