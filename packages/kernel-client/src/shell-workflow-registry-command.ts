import { createHash } from "node:crypto"
import { readdir, readFile, stat } from "node:fs/promises"
import { basename, relative, resolve as resolvePath } from "node:path"

import type {
  WorkflowCodeProviderRebinding,
  WorkflowCodeSourceExportAgentMode,
  WorkflowCodeSourceExportFile,
  WorkflowRegistryEntryMetadata,
  WorkflowRegistrySourceInput,
  WorkflowRegistrySourceScope,
} from "./kernel-types.js"
import {
  addWorkflowRegistryEntryFromWorkflowRequest,
  addWorkflowRegistryEntryRequest,
  deleteWorkflowRegistryEntryRequest,
  getWorkflowRegistryEntryRequest,
  listWorkflowRegistryRequest,
  loadWorkflowRegistryEntryRequest,
  runWorkflowRegistryEntryRequest,
} from "./ipc-requests.js"
import type { ShellCommandResult, ShellContext } from "./shell-core.js"
import { sessionContextAgentId } from "./shell-session-context.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellWorkflowRegistryCommandDeps = {
  client: ShellKernelClient
}

export async function executeWorkflowRegistryCommand(
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowRegistryCommandDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }

  const [action, ...rest] = args
  if (action === "list" || action === "ls") return listWorkflowRegistry(sessionId, deps)
  if (action === "get" || action === "show") return getWorkflowRegistry(sessionId, rest, deps)
  if (action === "add" || action === "register") return addWorkflowRegistry(sessionId, rest, context, deps)
  if (action === "add-from-workflow" || action === "register-from-workflow") {
    return addWorkflowRegistryFromWorkflow(sessionId, rest, deps)
  }
  if (action === "delete" || action === "rm") return deleteWorkflowRegistry(sessionId, rest, deps)
  return { ok: false, message: "usage: workflow registry list|get|add|add-from-workflow|delete ..." }
}

export async function executeWorkflowRegistryLoadCommand(
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowRegistryCommandDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }

  const [name, ...optionArgs] = args
  if (!name) return { ok: false, message: "usage: workflow load <registry-name> [--input key=value] [--inputs-json '{...}'] [--provider-rebinding node=provider/model]" }
  const parsed = parseProviderRebindingOptions(optionArgs, "workflow load")
  if (!parsed.ok) return { ok: false, message: parsed.message }
  const response = await deps.client.send(loadWorkflowRegistryEntryRequest(sessionId, name, {
    parameters: parsed.parameters,
    providerRebindings: parsed.providerRebindings,
  }))
  const payload = expectVariant<{ entry: WorkflowRegistryEntryMetadata; result: { apply?: { workflow_id?: string } }; session: { id: string } & Record<string, unknown> }>(response, "WorkflowRegistryEntryLoaded")
  return {
    ok: true,
    message: `loaded workflow ${payload.entry.name} as workflow ${payload.result.apply?.workflow_id ?? "(unknown)"}`,
    data: payload,
    contextUpdates: { workflowId: payload.result.apply?.workflow_id, sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session as never) },
  }
}

export async function executeWorkflowRegistryRunCommand(
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowRegistryCommandDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }

  const [name, ...optionArgs] = args
  if (!name) {
    return { ok: false, message: "usage: workflow run <registry-name> --endpoint <handle> [--queue <handle>] --prompt \"...\" [--input key=value] [--inputs-json '{...}'] [--provider-rebinding node=provider/model]" }
  }
  const parsed = parseWorkflowRegistryRunOptions(optionArgs)
  if (!parsed.ok) return { ok: false, message: parsed.message }
  const response = await deps.client.send(runWorkflowRegistryEntryRequest(sessionId, name, parsed.prompt, {
    parameters: parsed.parameters,
    providerRebindings: parsed.providerRebindings,
    endpoint: parsed.endpoint,
    ...(parsed.queue !== undefined ? { queueRef: parsed.queue } : {}),
  }))
  const payload = expectVariant<{ entry: WorkflowRegistryEntryMetadata; result: { apply?: { apply?: { workflow_id?: string } }; invocation?: { kind?: string } }; session: { id: string } & Record<string, unknown> }>(response, "WorkflowRegistryEntryRun")
  return {
    ok: true,
    message: `ran workflow ${payload.entry.name} as workflow ${payload.result.apply?.apply?.workflow_id ?? "(unknown)"} [${payload.result.invocation?.kind ?? "invoked"}]`,
    data: payload,
    contextUpdates: { workflowId: payload.result.apply?.apply?.workflow_id, sessionId: payload.session.id, agentId: sessionContextAgentId(payload.session as never) },
  }
}

function shouldRunRegistryCommand(args: string[]): boolean {
  return args.includes("--endpoint") || args.includes("--prompt") || args.includes("--provider-rebinding") || args.includes("--provider-rebind")
}

export function workflowRunLooksLikeRegistryLoad(args: string[]): boolean {
  return shouldRunRegistryCommand(args)
}

async function listWorkflowRegistry(
  sessionId: string,
  deps: ShellWorkflowRegistryCommandDeps,
): Promise<ShellCommandResult> {
  const response = await deps.client.send(listWorkflowRegistryRequest(sessionId))
  const entries = expectVariant<{ entries: WorkflowRegistryEntryMetadata[] }>(response, "WorkflowRegistryListed").entries
  return {
    ok: true,
    message: entries.length === 0
      ? "no workflow registry entries"
      : entries.map((entry) => `${entry.name} scope=${entry.source_scope} kind=${entry.source_kind} validation=${entry.validation?.ok === false ? "failed" : "ok"}`).join("\n"),
    data: { entries },
  }
}

async function getWorkflowRegistry(
  sessionId: string,
  args: string[],
  deps: ShellWorkflowRegistryCommandDeps,
): Promise<ShellCommandResult> {
  const [name] = args
  if (!name) return { ok: false, message: "usage: workflow registry get <name>" }
  const response = await deps.client.send(getWorkflowRegistryEntryRequest(sessionId, name))
  const entry = expectVariant<{ entry: WorkflowRegistryEntryMetadata }>(response, "WorkflowRegistryEntry").entry
  return {
    ok: true,
    message: formatRegistryEntry(entry),
    data: { entry },
  }
}

async function addWorkflowRegistry(
  sessionId: string,
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowRegistryCommandDeps,
): Promise<ShellCommandResult> {
  const parsed = parseRegistryAddArgs(args)
  if (!parsed.ok) return { ok: false, message: parsed.message }
  const source = await readRegistrySource(context, parsed.path)
  const response = await deps.client.send(addWorkflowRegistryEntryRequest(sessionId, parsed.name, source, process.execPath, parsed.scope))
  const entry = expectVariant<{ entry: WorkflowRegistryEntryMetadata }>(response, "WorkflowRegistryEntryAdded").entry
  return {
    ok: true,
    message: `registered workflow ${entry.name} scope=${entry.source_scope} kind=${entry.source_kind}`,
    data: { entry },
  }
}

async function addWorkflowRegistryFromWorkflow(
  sessionId: string,
  args: string[],
  deps: ShellWorkflowRegistryCommandDeps,
): Promise<ShellCommandResult> {
  const parsed = parseRegistryAddFromWorkflowArgs(args)
  if (!parsed.ok) return { ok: false, message: parsed.message }
  const response = await deps.client.send(addWorkflowRegistryEntryFromWorkflowRequest(sessionId, parsed.name, parsed.workflowRef, {
    ...(parsed.scope !== undefined ? { scope: parsed.scope } : {}),
    agentMode: parsed.agentMode,
  }))
  const entry = expectVariant<{ entry: WorkflowRegistryEntryMetadata }>(response, "WorkflowRegistryEntryAdded").entry
  return {
    ok: true,
    message: `registered workflow ${entry.name} from ${parsed.workflowRef}`,
    data: { entry },
  }
}

async function deleteWorkflowRegistry(
  sessionId: string,
  args: string[],
  deps: ShellWorkflowRegistryCommandDeps,
): Promise<ShellCommandResult> {
  const parsed = parseRegistryDeleteArgs(args)
  if (!parsed.ok) return { ok: false, message: parsed.message }
  const response = await deps.client.send(deleteWorkflowRegistryEntryRequest(sessionId, parsed.name, parsed.scope))
  const payload = expectVariant<{ name: string; path?: string }>(response, "WorkflowRegistryEntryDeleted")
  return {
    ok: true,
    message: `deleted workflow registry entry ${payload.name}`,
    data: payload,
  }
}

function parseRegistryAddArgs(args: string[]): { ok: true; name: string; path: string; scope?: WorkflowRegistrySourceScope } | { ok: false; message: string } {
  const usage = "usage: workflow registry add <name> <js-file-or-source-dir> [--user|--workspace]"
  const positional: string[] = []
  let scope: WorkflowRegistrySourceScope | undefined
  for (const arg of args) {
    if (arg === "--user") {
      scope = "user"
      continue
    }
    if (arg === "--workspace") {
      scope = "workspace"
      continue
    }
    if (arg.startsWith("--")) return { ok: false, message: usage }
    positional.push(arg)
  }
  const [name, path, extra] = positional
  if (!name || !path || extra) return { ok: false, message: usage }
  return { ok: true, name, path, ...(scope !== undefined ? { scope } : {}) }
}

function parseRegistryAddFromWorkflowArgs(args: string[]): { ok: true; name: string; workflowRef: string; scope?: WorkflowRegistrySourceScope; agentMode: WorkflowCodeSourceExportAgentMode } | { ok: false; message: string } {
  const usage = "usage: workflow registry add-from-workflow <name> <workflow-ref> [--existing-agents] [--user|--workspace]"
  const positional: string[] = []
  let scope: WorkflowRegistrySourceScope | undefined
  let agentMode: WorkflowCodeSourceExportAgentMode = "portable_generated"
  for (const arg of args) {
    if (arg === "--user") {
      scope = "user"
      continue
    }
    if (arg === "--workspace") {
      scope = "workspace"
      continue
    }
    if (arg === "--existing-agents") {
      agentMode = "existing_agents"
      continue
    }
    if (arg.startsWith("--")) return { ok: false, message: usage }
    positional.push(arg)
  }
  const [name, workflowRef, extra] = positional
  if (!name || !workflowRef || extra) return { ok: false, message: usage }
  return { ok: true, name, workflowRef, ...(scope !== undefined ? { scope } : {}), agentMode }
}

function parseRegistryDeleteArgs(args: string[]): { ok: true; name: string; scope?: WorkflowRegistrySourceScope } | { ok: false; message: string } {
  const usage = "usage: workflow registry delete <name> [--user|--workspace]"
  let name: string | undefined
  let scope: WorkflowRegistrySourceScope | undefined
  for (const arg of args) {
    if (arg === "--user") {
      scope = "user"
      continue
    }
    if (arg === "--workspace") {
      scope = "workspace"
      continue
    }
    if (arg.startsWith("--")) return { ok: false, message: usage }
    if (name) return { ok: false, message: usage }
    name = arg
  }
  if (!name) return { ok: false, message: usage }
  return { ok: true, name, ...(scope !== undefined ? { scope } : {}) }
}

function parseProviderRebindingOptions(args: string[], command: string): { ok: true; parameters: Record<string, unknown>; providerRebindings: WorkflowCodeProviderRebinding[] } | { ok: false; message: string } {
  const parameters: Record<string, unknown> = {}
  const providerRebindings: WorkflowCodeProviderRebinding[] = []
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (arg === "--input") {
      const value = args[index + 1]
      if (!value) return { ok: false, message: `usage: ${command} <name> --input key=value` }
      const parsed = parseWorkflowInput(value)
      if (!parsed.ok) return parsed
      parameters[parsed.key] = parsed.value
      index += 1
      continue
    }
    if (arg === "--inputs-json") {
      const value = args[index + 1]
      if (!value) return { ok: false, message: `usage: ${command} <name> --inputs-json '{...}'` }
      const parsed = parseWorkflowInputsJson(value)
      if (!parsed.ok) return parsed
      Object.assign(parameters, parsed.parameters)
      index += 1
      continue
    }
    if (arg === "--provider-rebinding" || arg === "--provider-rebind") {
      const value = args[index + 1]
      if (!value) return { ok: false, message: `usage: ${command} <name> --provider-rebinding node=provider/model` }
      const parsed = parseProviderRebinding(value)
      if (!parsed.ok) return parsed
      providerRebindings.push(parsed.rebinding)
      index += 1
      continue
    }
    return { ok: false, message: `unknown ${command} option: ${arg}` }
  }
  return { ok: true, parameters, providerRebindings }
}

function parseWorkflowRegistryRunOptions(args: string[]): { ok: true; parameters: Record<string, unknown>; providerRebindings: WorkflowCodeProviderRebinding[]; endpoint: string; queue?: string; prompt: string } | { ok: false; message: string } {
  const parameters: Record<string, unknown> = {}
  const providerRebindings: WorkflowCodeProviderRebinding[] = []
  let endpoint: string | undefined
  let queue: string | undefined
  let prompt: string | undefined
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (arg === "--input") {
      const value = args[index + 1]
      if (!value) return { ok: false, message: "usage: --input key=value" }
      const parsed = parseWorkflowInput(value)
      if (!parsed.ok) return parsed
      parameters[parsed.key] = parsed.value
      index += 1
      continue
    }
    if (arg === "--inputs-json") {
      const value = args[index + 1]
      if (!value) return { ok: false, message: "usage: --inputs-json '{...}'" }
      const parsed = parseWorkflowInputsJson(value)
      if (!parsed.ok) return parsed
      Object.assign(parameters, parsed.parameters)
      index += 1
      continue
    }
    if (arg === "--provider-rebinding" || arg === "--provider-rebind") {
      const value = args[index + 1]
      if (!value) return { ok: false, message: "usage: --provider-rebinding node=provider/model" }
      const parsed = parseProviderRebinding(value)
      if (!parsed.ok) return parsed
      providerRebindings.push(parsed.rebinding)
      index += 1
      continue
    }
    if (arg === "--endpoint") {
      endpoint = args[index + 1]
      if (!endpoint) return { ok: false, message: "usage: --endpoint <handle>" }
      index += 1
      continue
    }
    if (arg === "--queue") {
      queue = args[index + 1]
      if (!queue) return { ok: false, message: "usage: --queue <handle>" }
      index += 1
      continue
    }
    if (arg === "--prompt") {
      prompt = args[index + 1] ?? ""
      if (!prompt) return { ok: false, message: "usage: --prompt <text>" }
      index += 1
      continue
    }
    return { ok: false, message: `unknown workflow registry run option: ${arg}` }
  }
  if (!endpoint) return { ok: false, message: "usage: --endpoint <handle>" }
  if (prompt === undefined) return { ok: false, message: "usage: --prompt <text>" }
  return { ok: true, parameters, providerRebindings, endpoint, ...(queue !== undefined ? { queue } : {}), prompt }
}

function parseWorkflowInput(value: string): { ok: true; key: string; value: unknown } | { ok: false; message: string } {
  const separator = value.indexOf("=")
  if (separator <= 0) return { ok: false, message: "usage: --input key=value" }
  const key = value.slice(0, separator)
  const rawValue = value.slice(separator + 1)
  if (!key) return { ok: false, message: "usage: --input key=value" }
  const parsed = parseWorkflowInputValue(rawValue)
  if (!parsed.ok) return parsed
  return { ok: true, key, value: parsed.value }
}

function parseWorkflowInputsJson(value: string): { ok: true; parameters: Record<string, unknown> } | { ok: false; message: string } {
  let parsed: unknown
  try {
    parsed = JSON.parse(value)
  } catch (error) {
    return { ok: false, message: `invalid --inputs-json: ${error instanceof Error ? error.message : String(error)}` }
  }
  if (!isPlainObject(parsed)) return { ok: false, message: "--inputs-json must be a JSON object" }
  return { ok: true, parameters: parsed }
}

function parseWorkflowInputValue(value: string): { ok: true; value: unknown } | { ok: false; message: string } {
  const trimmed = value.trim()
  if (trimmed === "true" || trimmed === "false" || trimmed === "null") {
    return { ok: true, value: JSON.parse(trimmed) }
  }
  if (/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(trimmed)) {
    return { ok: true, value: Number(trimmed) }
  }
  if (/^[\[{"]/.test(trimmed)) {
    try {
      return { ok: true, value: JSON.parse(trimmed) }
    } catch (error) {
      return { ok: false, message: `invalid --input JSON value: ${error instanceof Error ? error.message : String(error)}` }
    }
  }
  return { ok: true, value }
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

function parseProviderRebinding(value: string): { ok: true; rebinding: WorkflowCodeProviderRebinding } | { ok: false; message: string } {
  const [node, providerAndModel] = value.split("=", 2)
  if (!node || !providerAndModel) {
    return { ok: false, message: "usage: --provider-rebinding node=provider/model" }
  }
  const slash = providerAndModel.indexOf("/")
  const provider = slash >= 0 ? providerAndModel.slice(0, slash) : providerAndModel
  const model = slash >= 0 ? providerAndModel.slice(slash + 1) : undefined
  if (!provider || model === "") {
    return { ok: false, message: "usage: --provider-rebinding node=provider/model" }
  }
  return {
    ok: true,
    rebinding: {
      node,
      provider,
      ...(model !== undefined ? { model } : {}),
    },
  }
}

async function readRegistrySource(context: ShellContext, inputPath: string): Promise<WorkflowRegistrySourceInput> {
  const resolved = resolveShellPath(context, inputPath)
  const entryStat = await stat(resolved)
  if (entryStat.isDirectory()) {
    const files = await readRegistryDirectoryFiles(resolved)
    return { kind: "source_directory", files }
  }
  const source = await readFile(resolved, "utf8")
  return {
    kind: "single_file",
    source,
    source_path: basename(inputPath),
  }
}

async function readRegistryDirectoryFiles(root: string): Promise<WorkflowCodeSourceExportFile[]> {
  const files: WorkflowCodeSourceExportFile[] = []
  await readRegistryDirectoryFilesInto(root, root, files)
  files.sort((left, right) => left.path.localeCompare(right.path))
  return files
}

async function readRegistryDirectoryFilesInto(root: string, directory: string, files: WorkflowCodeSourceExportFile[]) {
  const entries = await readdir(directory, { withFileTypes: true })
  for (const entry of entries) {
    const fullPath = resolvePath(directory, entry.name)
    if (entry.isDirectory()) {
      await readRegistryDirectoryFilesInto(root, fullPath, files)
      continue
    }
    if (!entry.isFile()) continue
    const contents = await readFile(fullPath, "utf8")
    files.push({
      path: relative(root, fullPath).split(/[\\/]/g).join("/"),
      contents,
      sha256: sha256(contents),
    })
  }
}

function resolveShellPath(context: ShellContext, path: string): string {
  return resolvePath(context.worktree ?? context.workspace ?? process.cwd(), path)
}

function sha256(contents: string): string {
  return createHash("sha256").update(contents, "utf8").digest("hex")
}

function formatRegistryEntry(entry: WorkflowRegistryEntryMetadata): string {
  const lines = [
    `workflow registry entry ${entry.name}`,
    `scope=${entry.source_scope}`,
    `kind=${entry.source_kind}`,
    `source=${entry.source_path}`,
    `sha256=${entry.source_sha256}`,
    `validation=${entry.validation?.ok === false ? "failed" : "ok"}`,
  ]
  if (entry.parameters_schema !== undefined && entry.parameters_schema !== null) {
    lines.push(`parameters_schema=${JSON.stringify(entry.parameters_schema, null, 2)}`)
  }
  return lines.join(" ")
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
