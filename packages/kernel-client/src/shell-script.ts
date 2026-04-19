import { readFile } from "node:fs/promises"
import { isAbsolute, resolve as resolvePath } from "node:path"

import {
  applyShellCommandResult,
  parseShellCommand,
  renderShellCommandResult,
  type ShellContext,
} from "./shell-core.js"
import { executeShellCommand, type ShellExecutorDeps } from "./shell-executor.js"

export type ShellScriptOptions = {
  continueOnError?: boolean | undefined
  loadScript?: ((path: string, context: ShellContext) => Promise<string>) | undefined
  maxSourceDepth?: number | undefined
  sourceDepth?: number | undefined
}

export async function executeShellScriptLines(
  lines: string[],
  initialContext: ShellContext,
  deps: ShellExecutorDeps,
  write: (text: string) => void = () => {},
  options: ShellScriptOptions = {},
): Promise<number> {
  return (await executeShellScript(lines, initialContext, deps, write, options)).code
}

export async function executeShellScript(
  lines: string[],
  initialContext: ShellContext,
  deps: ShellExecutorDeps,
  write: (text: string) => void = () => {},
  options: ShellScriptOptions = {},
): Promise<{ code: number; context: ShellContext }> {
  let context = initialContext
  let failed = false
  for (let index = 0; index < lines.length; index += 1) {
    const sourceLine = lines[index] ?? ""
    const line = sourceLine.trim()
    if (!line || line.startsWith("#")) {
      continue
    }
    write(`@ ${line}\n`)
    try {
      const result = await executeShellLine(line, context, deps, write, options)
      context = result.context
      if (!result.ok) {
        failed = true
        if (options.continueOnError) {
          write(`line ${index + 1} failed; continuing\n`)
          continue
        }
        write(`stopped at line ${index + 1}\n`)
        return { code: 1, context }
      }
      if (result.exit) {
        return { code: 0, context }
      }
    } catch (error) {
      failed = true
      if (options.continueOnError) {
        write(`${formatShellScriptError(error)}\n`)
        write(`line ${index + 1} failed; continuing\n`)
        continue
      }
      write(`${formatShellScriptError(error)}\n`)
      write(`stopped at line ${index + 1}\n`)
      return { code: 1, context }
    }
  }
  return { code: failed ? 1 : 0, context }
}

export async function executeShellLine(
  line: string,
  context: ShellContext,
  deps: ShellExecutorDeps,
  write: (text: string) => void = () => {},
  options: ShellScriptOptions = {},
): Promise<{ ok: boolean; context: ShellContext; exit?: boolean | undefined }> {
  const parsed = parseShellCommand(line, context)
  if (parsed.command === "source" || parsed.command === "run") {
    if (parsed.args.length !== 1) {
      write(`usage: ${parsed.command} <file>\n`)
      return { ok: false, context }
    }
    const sourceDepth = options.sourceDepth ?? 0
    const maxSourceDepth = options.maxSourceDepth ?? 16
    if (sourceDepth >= maxSourceDepth) {
      write(`maximum source depth ${maxSourceDepth} exceeded\n`)
      return { ok: false, context }
    }
    const scriptPath = resolveShellScriptPath(parsed.args[0]!, context)
    const loadScript = options.loadScript ?? ((path) => readFile(path, "utf8"))
    const source = await loadScript(scriptPath, context)
    const nested = await executeShellScript(source.split(/\r?\n/), context, deps, write, {
      ...options,
      sourceDepth: sourceDepth + 1,
    })
    return { ok: nested.code === 0, context: nested.context }
  }
  const result = await executeShellCommand(parsed, context, deps)
  const rendered = renderShellCommandResult(result)
  if (rendered) {
    write(`${rendered}\n`)
  }
  return {
    ok: result.ok,
    context: applyShellCommandResult(context, result),
    exit: Boolean(result.data && typeof result.data === "object" && "exit" in result.data),
  }
}

export function resolveShellScriptPath(scriptPath: string, context: ShellContext): string {
  return isAbsolute(scriptPath) ? scriptPath : resolvePath(context.worktree, scriptPath)
}

function formatShellScriptError(error: unknown): string {
  return error instanceof Error ? `error: ${error.message}` : `error: ${String(error)}`
}
