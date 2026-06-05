import { resolve as resolvePath } from "node:path"

import type {
  RuntimeSession,
  WorkflowPublicationDefinition,
} from "./kernel-types.js"
import {
  type CreateWorkflowPublicationOptions,
  createWorkflowPublicationRequest,
  disableWorkflowPublicationRequest,
  getWorkflowPublicationRequest,
  listWorkflowPublicationsRequest,
  getSessionStateRequest,
} from "./ipc-requests.js"
import type { ShellCommandResult, ShellContext } from "./shell-core.js"
import { sessionContextAgentId } from "./shell-session-context.js"
import { writeWorkflowPublicationExportPackage } from "./shell-workflow-publication-export.js"
import {
  formatWorkflowPublicationLabel,
  formatWorkflowPublications,
} from "./shell-workflow-format.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellWorkflowPublicationCommandDeps = {
  client: ShellKernelClient
}

export async function executeWorkflowPublicationCommand(
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowPublicationCommandDeps,
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
    const sessionResponse = await deps.client.send(getSessionStateRequest(sessionId))
    const session = expectVariant<{ session: RuntimeSession }>(sessionResponse, "SessionState").session
    const outputRoot = resolvePath(context.worktree ?? context.workspace ?? process.cwd(), outputDirectory)
    const packageFiles = await writeWorkflowPublicationExportPackage(publication, session, outputRoot, options.kernelUrl)
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
      contextUpdates: { sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session) },
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
      { sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session), workflowId: payload.publication.workflow_id },
      payload,
    )
  }

  return { ok: false, message: "usage: workflow publication list|create|show|export|disable" }
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
    return { ok: false, message: "usage: workflow publication create [workflow-ref] <endpoint-ref> [alias] [--route <route>] [--method POST] [--parser-json <json>] [--transport-json <json>] [--input-schema-json <json>] [--mode async]" }
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

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
