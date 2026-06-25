import { mkdir, readFile, writeFile } from "node:fs/promises"
import { dirname, resolve as resolvePath } from "node:path"

import type {
  WorkflowCodeArtifact,
  WorkflowCodeArtifactPackage,
  WorkflowCodeProviderRebinding,
  WorkflowCodeSourceExport,
  WorkflowCodeSourceExportFormat,
} from "./kernel-types.js"
import {
  applyWorkflowCodeArtifactRequest,
  applyWorkflowCodeRequest,
  createWorkflowCodeArtifactRequest,
  deleteWorkflowCodeArtifactRequest,
  exportWorkflowCodePackageRequest,
  exportWorkflowCodeSourceRequest,
  getWorkflowCodeArtifactRequest,
  importWorkflowCodePackageRequest,
  listWorkflowCodeArtifactsRequest,
  runWorkflowCodeArtifactRequest,
  runWorkflowCodeRequest,
  validateWorkflowCodeRequest,
} from "./ipc-requests.js"
import type { ShellCommandResult, ShellContext } from "./shell-core.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellWorkflowCodeCommandDeps = {
  client: ShellKernelClient
}

export async function executeWorkflowCodeCommand(
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }

  const [area, ...areaArgs] = args
  const [action, ...rest] = areaArgs
  if (area === "validate") return validateWorkflowCode(sessionId, areaArgs, context, deps)
  if (area === "apply") return applyWorkflowCode(sessionId, areaArgs, context, deps)
  if (area === "run") return runWorkflowCode(sessionId, areaArgs, context, deps)
  if (area === "save") return saveWorkflowCodeArtifact(sessionId, areaArgs, context, deps)
  if (area === "artifact") return executeWorkflowCodeArtifactCommand(sessionId, action, rest, context, deps)
  if (area === "package") {
    if (action === "export") return exportWorkflowCodePackage(sessionId, rest, context, deps)
    if (action === "import") return importWorkflowCodePackage(sessionId, rest, context, deps)
    return { ok: false, message: "usage: workflow code package export <artifact-name> <file> | workflow code package import <file> [name] [--overwrite]" }
  }
  if (area === "source") {
    if (action === "export") return exportWorkflowCodeSource(sessionId, rest, context, deps)
    if (action === "export-dir") return exportWorkflowCodeSourceDirectory(sessionId, rest, context, deps)
    return { ok: false, message: "usage: workflow code source export <artifact-or-workflow-ref> <file> [--workflow] | workflow code source export-dir <artifact-or-workflow-ref> <directory> [--workflow]" }
  }
  return { ok: false, message: "usage: workflow code validate|apply|run|save|artifact|package|source ..." }
}

async function validateWorkflowCode(
  sessionId: string,
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  const [inputFile, ...optionArgs] = args
  if (!inputFile) {
    return { ok: false, message: "usage: workflow code validate <file> [--provider-rebinding node=provider/model]" }
  }
  const parsed = parseWorkflowCodeCommonOptions(optionArgs)
  if (!parsed.ok) return { ok: false, message: parsed.message }
  const source = await readWorkflowCodeSource(context, inputFile)
  const response = await deps.client.send(validateWorkflowCodeRequest(sessionId, process.execPath, source, parsed.providerRebindings))
  const result = expectVariant<{ result: { validation?: { ok?: boolean; diagnostics?: unknown[] } } }>(response, "WorkflowCodeValidated").result
  const ok = result.validation?.ok === true
  return {
    ok,
    message: ok
      ? `workflow-code ${inputFile} is valid`
      : `workflow-code ${inputFile} is invalid (${result.validation?.diagnostics?.length ?? 0} diagnostics)`,
    data: result,
  }
}

async function applyWorkflowCode(
  sessionId: string,
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  const [inputFile, ...optionArgs] = args
  if (!inputFile) {
    return { ok: false, message: "usage: workflow code apply <file> [--provider-rebinding node=provider/model]" }
  }
  const parsed = parseWorkflowCodeCommonOptions(optionArgs)
  if (!parsed.ok) return { ok: false, message: parsed.message }
  const source = await readWorkflowCodeSource(context, inputFile)
  const response = await deps.client.send(applyWorkflowCodeRequest(sessionId, process.execPath, source, parsed.providerRebindings))
  const payload = expectVariant<{ result: { apply?: { workflow_id?: string } }; session: unknown }>(response, "WorkflowCodeApplied")
  return {
    ok: true,
    message: `applied workflow-code ${inputFile} as workflow ${payload.result.apply?.workflow_id ?? "(unknown)"}`,
    data: payload,
  }
}

async function runWorkflowCode(
  sessionId: string,
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  const [inputFile, ...optionArgs] = args
  if (!inputFile) {
    return { ok: false, message: "usage: workflow code run <file> --endpoint <handle> [--queue <handle>] --prompt \"...\" [--provider-rebinding node=provider/model]" }
  }
  const parsed = parseWorkflowCodeRunOptions(optionArgs)
  if (!parsed.ok) return { ok: false, message: parsed.message }
  const source = await readWorkflowCodeSource(context, inputFile)
  const runOptions: {
    providerRebindings: WorkflowCodeProviderRebinding[]
    endpoint: string
    queueRef?: string
  } = {
    providerRebindings: parsed.providerRebindings,
    endpoint: parsed.endpoint,
    ...(parsed.queue !== undefined ? { queueRef: parsed.queue } : {}),
  }
  const response = await deps.client.send(runWorkflowCodeRequest(sessionId, process.execPath, source, parsed.prompt, runOptions))
  const payload = expectVariant<{ result: { apply?: { apply?: { workflow_id?: string } }; invocation?: { kind?: string } }; session: unknown }>(response, "WorkflowCodeRun")
  return {
    ok: true,
    message: `ran workflow-code ${inputFile} as workflow ${payload.result.apply?.apply?.workflow_id ?? "(unknown)"} [${payload.result.invocation?.kind ?? "invoked"}]`,
    data: payload,
  }
}

async function saveWorkflowCodeArtifact(
  sessionId: string,
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  const [name, inputFile] = args
  if (!name || !inputFile) {
    return { ok: false, message: "usage: workflow code save <name> <file>" }
  }
  const source = await readWorkflowCodeSource(context, inputFile)
  const response = await deps.client.send(createWorkflowCodeArtifactRequest(sessionId, name, process.execPath, source))
  const artifact = expectVariant<{ artifact: WorkflowCodeArtifact }>(response, "WorkflowCodeArtifactCreated").artifact
  return {
    ok: true,
    message: `saved workflow-code artifact ${artifact.metadata.name}`,
    data: { artifact },
  }
}

async function executeWorkflowCodeArtifactCommand(
  sessionId: string,
  action: string | undefined,
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  if (action === "list" || action === "ls") return listWorkflowCodeArtifacts(sessionId, deps)
  if (action === "get") return getWorkflowCodeArtifact(sessionId, args, deps)
  if (action === "delete" || action === "rm") return deleteWorkflowCodeArtifact(sessionId, args, deps)
  if (action === "apply") return applyWorkflowCodeArtifact(sessionId, args, deps)
  if (action === "run") return runWorkflowCodeArtifact(sessionId, args, deps)
  return { ok: false, message: "usage: workflow code artifact list|get|delete|apply|run ..." }
}

async function listWorkflowCodeArtifacts(
  sessionId: string,
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  const response = await deps.client.send(listWorkflowCodeArtifactsRequest(sessionId))
  const artifacts = expectVariant<{ artifacts: Array<{ name: string; source_bytes?: number; validation?: { ok?: boolean } }> }>(response, "WorkflowCodeArtifactsListed").artifacts
  return {
    ok: true,
    message: artifacts.length === 0
      ? "no workflow-code artifacts"
      : artifacts.map((artifact) => `${artifact.name} validation=${artifact.validation?.ok === false ? "failed" : "ok"} bytes=${artifact.source_bytes ?? 0}`).join("\n"),
    data: { artifacts },
  }
}

async function getWorkflowCodeArtifact(
  sessionId: string,
  args: string[],
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  const [name] = args
  if (!name) return { ok: false, message: "usage: workflow code artifact get <name>" }
  const response = await deps.client.send(getWorkflowCodeArtifactRequest(sessionId, name))
  const artifact = expectVariant<{ artifact: WorkflowCodeArtifact }>(response, "WorkflowCodeArtifact").artifact
  return {
    ok: true,
    message: `workflow-code artifact ${artifact.metadata.name} validation=${artifact.metadata.validation?.ok === false ? "failed" : "ok"}`,
    data: { artifact },
  }
}

async function deleteWorkflowCodeArtifact(
  sessionId: string,
  args: string[],
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  const [name] = args
  if (!name) return { ok: false, message: "usage: workflow code artifact delete <name>" }
  const response = await deps.client.send(deleteWorkflowCodeArtifactRequest(sessionId, name))
  const payload = expectVariant<{ name: string; path?: string }>(response, "WorkflowCodeArtifactDeleted")
  return {
    ok: true,
    message: `deleted workflow-code artifact ${payload.name}`,
    data: payload,
  }
}

async function applyWorkflowCodeArtifact(
  sessionId: string,
  args: string[],
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  const [name, ...optionArgs] = args
  if (!name) return { ok: false, message: "usage: workflow code artifact apply <name> [--provider-rebinding node=provider/model]" }
  const parsed = parseWorkflowCodeCommonOptions(optionArgs)
  if (!parsed.ok) return { ok: false, message: parsed.message }
  const response = await deps.client.send(applyWorkflowCodeArtifactRequest(sessionId, name, parsed.providerRebindings))
  const payload = expectVariant<{ result: { apply?: { workflow_id?: string } }; session: unknown }>(response, "WorkflowCodeApplied")
  return {
    ok: true,
    message: `applied workflow-code artifact ${name} as workflow ${payload.result.apply?.workflow_id ?? "(unknown)"}`,
    data: payload,
  }
}

async function runWorkflowCodeArtifact(
  sessionId: string,
  args: string[],
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  const [name, ...optionArgs] = args
  if (!name) return { ok: false, message: "usage: workflow code artifact run <name> --endpoint <handle> [--queue <handle>] --prompt \"...\" [--provider-rebinding node=provider/model]" }
  const parsed = parseWorkflowCodeRunOptions(optionArgs)
  if (!parsed.ok) return { ok: false, message: parsed.message }
  const runOptions: {
    providerRebindings: WorkflowCodeProviderRebinding[]
    endpoint: string
    queueRef?: string
  } = {
    providerRebindings: parsed.providerRebindings,
    endpoint: parsed.endpoint,
    ...(parsed.queue !== undefined ? { queueRef: parsed.queue } : {}),
  }
  const response = await deps.client.send(runWorkflowCodeArtifactRequest(sessionId, name, parsed.prompt, runOptions))
  const payload = expectVariant<{ result: { apply?: { apply?: { workflow_id?: string } }; invocation?: { kind?: string } }; session: unknown }>(response, "WorkflowCodeRun")
  return {
    ok: true,
    message: `ran workflow-code artifact ${name} as workflow ${payload.result.apply?.apply?.workflow_id ?? "(unknown)"} [${payload.result.invocation?.kind ?? "invoked"}]`,
    data: payload,
  }
}

async function exportWorkflowCodePackage(
  sessionId: string,
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  const parsed = parseNameAndOutput(args, "usage: workflow code package export <artifact-name> --out <file.workflow-code.json>")
  if (!parsed.ok) return { ok: false, message: parsed.message }
  const { name, outputPath: outputFile } = parsed
  const response = await deps.client.send(exportWorkflowCodePackageRequest(sessionId, name))
  const workflowCodePackage = expectVariant<{ package: WorkflowCodeArtifactPackage }>(response, "WorkflowCodePackageExported").package
  const resolved = resolveShellPath(context, outputFile)
  await writeJsonFile(resolved, workflowCodePackage)
  return {
    ok: true,
    message: `exported workflow-code package ${workflowCodePackage.name} to ${resolved}`,
    data: { package: workflowCodePackage, path: resolved },
  }
}

async function importWorkflowCodePackage(
  sessionId: string,
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  const parsed = parsePackageImportArgs(args)
  if (!parsed.ok) return { ok: false, message: parsed.message }
  const resolved = resolveShellPath(context, parsed.inputFile)
  const payload = JSON.parse(await readFile(resolved, "utf8")) as WorkflowCodeArtifactPackage
  const response = await deps.client.send(importWorkflowCodePackageRequest(sessionId, payload, "node", {
    ...(parsed.name !== undefined ? { name: parsed.name } : {}),
    overwrite: parsed.overwrite,
  }))
  const artifact = expectVariant<{ artifact: WorkflowCodeArtifact }>(response, "WorkflowCodePackageImported").artifact
  return {
    ok: true,
    message: `imported workflow-code package ${artifact.metadata.name} from ${resolved}`,
    data: { artifact, path: resolved },
  }
}

async function exportWorkflowCodeSource(
  sessionId: string,
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  const parsed = parseSourceExportOptions(args)
  if (!parsed.ok) return { ok: false, message: parsed.message }
  const response = await deps.client.send(exportWorkflowCodeSourceRequest(
    sessionId,
    parsed.target === "workflow" ? { kind: "workflow", workflow_ref: parsed.name } : { kind: "artifact", name: parsed.name },
    parsed.format,
  ))
  const exportResult = expectVariant<{ export: WorkflowCodeSourceExport }>(response, "WorkflowCodeSourceExported").export
  const resolved = resolveShellPath(context, parsed.outputPath)
  if (exportResult.format === "directory") {
    for (const file of exportResult.files ?? []) {
      await writeTextFile(resolvePath(resolved, file.path), file.contents)
    }
  } else {
    await writeTextFile(resolved, exportResult.source)
  }
  return {
    ok: true,
    message: `exported workflow-code source ${exportResult.name} to ${resolved}`,
    data: { export: exportResult, path: resolved },
  }
}

async function exportWorkflowCodeSourceDirectory(
  sessionId: string,
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  return exportWorkflowCodeSource(sessionId, [...args, "--format", "directory"], context, deps)
}

function parseSourceExportOptions(args: string[]): { ok: true; name: string; outputPath: string; format: WorkflowCodeSourceExportFormat; target: "artifact" | "workflow" } | { ok: false; message: string } {
  const usage = "usage: workflow code source export <artifact-or-workflow-ref> --out <file|directory> [--workflow] [--format inline|directory]"
  const positionals: string[] = []
  let format: WorkflowCodeSourceExportFormat = "inline"
  let target: "artifact" | "workflow" = "artifact"
  let outputPath: string | undefined
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (arg === undefined) continue
    if (arg === "--out") {
      outputPath = args[index + 1]
      if (!outputPath) return { ok: false, message: usage }
      index += 1
      continue
    }
    if (arg === "--directory" || arg === "--dir") {
      format = "directory"
      continue
    }
    if (arg === "--workflow") {
      target = "workflow"
      continue
    }
    if (arg === "--format") {
      const value = args[index + 1]
      if (value !== "inline" && value !== "directory") {
        return { ok: false, message: usage }
      }
      format = value
      index += 1
      continue
    }
    if (arg.startsWith("--")) return { ok: false, message: usage }
    positionals.push(arg)
  }
  const [name, positionalOutput, ...extra] = positionals
  if (!name || extra.length > 0) return { ok: false, message: usage }
  outputPath ??= positionalOutput
  if (!outputPath) return { ok: false, message: usage }
  return { ok: true, name, outputPath, format, target }
}

function parseNameAndOutput(args: string[], usage: string): { ok: true; name: string; outputPath: string } | { ok: false; message: string } {
  const positionals: string[] = []
  let outputPath: string | undefined
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (arg === undefined) continue
    if (arg === "--out") {
      outputPath = args[index + 1]
      if (!outputPath) return { ok: false, message: usage }
      index += 1
      continue
    }
    if (arg.startsWith("--")) return { ok: false, message: usage }
    positionals.push(arg)
  }
  const [name, positionalOutput, ...extra] = positionals
  if (!name || extra.length > 0) return { ok: false, message: usage }
  outputPath ??= positionalOutput
  if (!outputPath) return { ok: false, message: usage }
  return { ok: true, name, outputPath }
}

function parsePackageImportArgs(args: string[]): { ok: true; inputFile: string; name?: string; overwrite: boolean } | { ok: false; message: string } {
  const usage = "usage: workflow code package import <file.workflow-code.json> [--name <name>] [--overwrite]"
  let inputFile: string | undefined
  let name: string | undefined
  let overwrite = false
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (arg === undefined) continue
    if (arg === "--name") {
      name = args[index + 1]
      if (!name) return { ok: false, message: usage }
      index += 1
      continue
    }
    if (arg === "--overwrite") {
      overwrite = true
      continue
    }
    if (arg.startsWith("--")) return { ok: false, message: usage }
    if (!inputFile) {
      inputFile = arg
      continue
    }
    if (!name) {
      name = arg
      continue
    }
    return { ok: false, message: usage }
  }
  if (!inputFile) return { ok: false, message: usage }
  return { ok: true, inputFile, ...(name !== undefined ? { name } : {}), overwrite }
}

function parseWorkflowCodeCommonOptions(args: string[]): { ok: true; providerRebindings: WorkflowCodeProviderRebinding[] } | { ok: false; message: string } {
  const providerRebindings: WorkflowCodeProviderRebinding[] = []
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (arg === "--provider-rebinding" || arg === "--provider-rebind") {
      const value = args[index + 1]
      if (!value) {
        return { ok: false, message: "usage: --provider-rebinding node=provider/model" }
      }
      const parsed = parseProviderRebinding(value)
      if (!parsed.ok) return parsed
      providerRebindings.push(parsed.rebinding)
      index += 1
      continue
    }
    return { ok: false, message: `unknown workflow-code option: ${arg}` }
  }
  return { ok: true, providerRebindings }
}

function parseWorkflowCodeRunOptions(args: string[]): { ok: true; providerRebindings: WorkflowCodeProviderRebinding[]; endpoint: string; queue?: string; prompt: string } | { ok: false; message: string } {
  const providerRebindings: WorkflowCodeProviderRebinding[] = []
  let endpoint: string | undefined
  let queue: string | undefined
  let prompt: string | undefined
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
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
    return { ok: false, message: `unknown workflow-code run option: ${arg}` }
  }
  if (!endpoint) return { ok: false, message: "usage: --endpoint <handle>" }
  if (prompt === undefined) return { ok: false, message: "usage: --prompt <text>" }
  return { ok: true, providerRebindings, endpoint, ...(queue !== undefined ? { queue } : {}), prompt }
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

async function readWorkflowCodeSource(context: ShellContext, inputFile: string): Promise<string> {
  return readFile(resolveShellPath(context, inputFile), "utf8")
}

function resolveShellPath(context: ShellContext, path: string): string {
  return resolvePath(context.worktree ?? context.workspace ?? process.cwd(), path)
}

async function writeJsonFile(path: string, value: unknown) {
  await writeTextFile(path, `${JSON.stringify(value, null, 2)}\n`)
}

async function writeTextFile(path: string, contents: string) {
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, contents, "utf8")
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
