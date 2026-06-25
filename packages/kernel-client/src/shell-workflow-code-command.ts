import { mkdir, readFile, writeFile } from "node:fs/promises"
import { dirname, resolve as resolvePath } from "node:path"

import type {
  WorkflowCodeArtifact,
  WorkflowCodeArtifactPackage,
  WorkflowCodeSourceExport,
  WorkflowCodeSourceExportFormat,
} from "./kernel-types.js"
import {
  exportWorkflowCodePackageRequest,
  exportWorkflowCodeSourceRequest,
  importWorkflowCodePackageRequest,
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

  const [area, action, ...rest] = args
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
  return { ok: false, message: "usage: workflow code package export|import | workflow code source export|export-dir" }
}

async function exportWorkflowCodePackage(
  sessionId: string,
  args: string[],
  context: ShellContext,
  deps: ShellWorkflowCodeCommandDeps,
): Promise<ShellCommandResult> {
  const [name, outputFile] = args
  if (!name || !outputFile) {
    return { ok: false, message: "usage: workflow code package export <artifact-name> <file>" }
  }
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
  const [inputFile, maybeName, ...rest] = args
  if (!inputFile) {
    return { ok: false, message: "usage: workflow code package import <file> [name] [--overwrite]" }
  }
  const overwrite = rest.includes("--overwrite") || maybeName === "--overwrite"
  const name = maybeName && maybeName !== "--overwrite" ? maybeName : undefined
  const resolved = resolveShellPath(context, inputFile)
  const payload = JSON.parse(await readFile(resolved, "utf8")) as WorkflowCodeArtifactPackage
  const response = await deps.client.send(importWorkflowCodePackageRequest(sessionId, payload, "node", {
    ...(name !== undefined ? { name } : {}),
    overwrite,
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
  const [name, outputPath, ...optionArgs] = args
  if (!name || !outputPath) {
    return { ok: false, message: "usage: workflow code source export <artifact-or-workflow-ref> <file|directory> [--workflow] [--format inline|directory]" }
  }
  const parsed = parseSourceExportOptions(optionArgs)
  if (!parsed.ok) return { ok: false, message: parsed.message }
  const response = await deps.client.send(exportWorkflowCodeSourceRequest(
    sessionId,
    parsed.target === "workflow" ? { kind: "workflow", workflow_ref: name } : { kind: "artifact", name },
    parsed.format,
  ))
  const exportResult = expectVariant<{ export: WorkflowCodeSourceExport }>(response, "WorkflowCodeSourceExported").export
  const resolved = resolveShellPath(context, outputPath)
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

function parseSourceExportOptions(args: string[]): { ok: true; format: WorkflowCodeSourceExportFormat; target: "artifact" | "workflow" } | { ok: false; message: string } {
  let format: WorkflowCodeSourceExportFormat = "inline"
  let target: "artifact" | "workflow" = "artifact"
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
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
        return { ok: false, message: "usage: workflow code source export ... [--workflow] [--format inline|directory]" }
      }
      format = value
      index += 1
      continue
    }
    return { ok: false, message: "usage: workflow code source export ... [--workflow] [--format inline|directory]" }
  }
  return { ok: true, format, target }
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
