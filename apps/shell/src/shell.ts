import { createInterface } from "node:readline/promises"
import { readFile } from "node:fs/promises"
import { isAbsolute, resolve as resolvePath } from "node:path"
import { stdin as defaultStdin, stdout as defaultStdout, stderr as defaultStderr } from "node:process"
import { fileURLToPath } from "node:url"

import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import { applyShellCommandResult, createDefaultShellContext, parseShellCommand, renderShellCommandResult, type ShellContext } from "@arroba/kernel-client/shell-core"
import { executeShellCommand, type ShellExecutorDeps } from "@arroba/kernel-client/shell-executor"

export type ShellCliOptions = {
  kernelUrl?: string | undefined
  socketPath?: string | undefined
  workspace?: string | undefined
  worktree?: string | undefined
  provider?: string | undefined
  model?: string | undefined
  effort?: string | undefined
  mode?: "repl" | "run" | undefined
  scriptPath?: string | undefined
  continueOnError?: boolean | undefined
  variables?: Record<string, string> | undefined
}

export type ShellIo = {
  input: NodeJS.ReadableStream
  output: NodeJS.WritableStream
  error: NodeJS.WritableStream
}

type ShellScriptOptions = {
  continueOnError?: boolean | undefined
  loadScript?: ((path: string, context: ShellContext) => Promise<string>) | undefined
  maxSourceDepth?: number | undefined
  sourceDepth?: number | undefined
}

export function parseShellCliArgs(argv: string[]): ShellCliOptions {
  const options: ShellCliOptions = {}
  let args = argv
  if (args[0] === "run") {
    const scriptPath = args[1]
    if (!scriptPath || scriptPath.startsWith("--")) {
      throw new Error("run requires a script file")
    }
    options.mode = "run"
    options.scriptPath = scriptPath
    args = args.slice(2)
  }
  const next = (index: number, flag: string) => {
    const value = args[index + 1]
    if (!value || value.startsWith("--")) {
      throw new Error(`${flag} requires a value`)
    }
    return value
  }
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    switch (arg) {
      case "--kernel-url":
        options.kernelUrl = next(index, arg)
        index += 1
        break
      case "--socket":
        options.socketPath = next(index, arg)
        index += 1
        break
      case "--workspace":
        options.workspace = next(index, arg)
        index += 1
        break
      case "--worktree":
        options.worktree = next(index, arg)
        index += 1
        break
      case "--provider":
        options.provider = next(index, arg)
        index += 1
        break
      case "--model":
        options.model = next(index, arg)
        index += 1
        break
      case "--effort":
        options.effort = next(index, arg)
        index += 1
        break
      case "--var": {
        const value = next(index, arg)
        const equalsIndex = value.indexOf("=")
        const name = equalsIndex < 0 ? value : value.slice(0, equalsIndex)
        const variableValue = equalsIndex < 0 ? "" : value.slice(equalsIndex + 1)
        if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) {
          throw new Error("--var requires NAME=VALUE with an identifier name")
        }
        options.variables = { ...(options.variables ?? {}), [name]: variableValue }
        index += 1
        break
      }
      case "--continue-on-error":
        options.continueOnError = true
        break
      case "--help":
      case "-h":
        throw new ShellHelpRequested()
      default:
        throw new Error(`unknown option: ${arg}`)
    }
  }
  if (options.kernelUrl && options.socketPath) {
    throw new Error("--kernel-url cannot be used together with --socket")
  }
  return options
}

export function createInitialShellContext(options: ShellCliOptions): ShellContext {
  const workspace = options.workspace ?? process.cwd()
  const contextOptions: Partial<ShellContext> = {
    workspace,
    worktree: options.worktree ?? workspace,
  }
  if (options.provider) contextOptions.provider = options.provider
  if (options.model) contextOptions.model = options.model
  if (options.effort) contextOptions.effort = options.effort
  if (options.variables) contextOptions.variables = options.variables
  return createDefaultShellContext(contextOptions)
}

export function shellUsage(): string {
  return [
    "usage: arroba-shell [--kernel-url URL|--socket PATH] [--workspace PATH] [--worktree PATH] [--provider NAME] [--model MODEL] [--effort LEVEL] [--var NAME=VALUE]",
    "       arroba-shell run <file> [--kernel-url URL|--socket PATH] [--workspace PATH] [--worktree PATH] [--provider NAME] [--model MODEL] [--effort LEVEL] [--var NAME=VALUE] [--continue-on-error]",
    "",
    "Runs an Arroba command REPL. Commands do not use the TUI slash prefix:",
    "  @ session list",
    "  @ session new --dir ../qa as s",
    "  @ session use $s",
    "  @ agent spawn reviewer gpt-5.2 as reviewer",
    "  @ mcp list",
    "  @ skill list",
    "  @ machine list",
    "  @ relay status",
    "  @ provider status codex",
    "  @ set provider codex",
    "  @ vars",
    "  @ source setup.arroba",
    "  @ exit",
  ].join("\n")
}

export async function runShellRepl(options: ShellCliOptions, io: ShellIo = {
  input: defaultStdin,
  output: defaultStdout,
  error: defaultStderr,
}): Promise<number> {
  let context = createInitialShellContext(options)
  const client = new LocalIpcClient(options.socketPath ?? options.kernelUrl ?? defaultKernelEndpoint())
  const clientId = `arroba-shell-${process.pid}-${Date.now()}`
  const readline = createInterface({ input: io.input, output: io.output, terminal: Boolean((io.output as { isTTY?: boolean }).isTTY) })

  try {
    io.output.write("arroba-shell\n")
    for (;;) {
      const line = await readline.question("@ ")
      const result = await executeShellLine(line, context, { client, clientId }, (text) => io.output.write(text))
      context = result.context
      if (result.exit) {
        return 0
      }
    }
  } catch (error) {
    if ((error as { code?: string }).code === "ERR_USE_AFTER_CLOSE") {
      return 0
    }
    io.error.write(`${formatShellError(error)}\n`)
    return 1
  } finally {
    readline.close()
    await client.close()
  }
}

export async function runShellScript(options: ShellCliOptions, io: ShellIo = {
  input: defaultStdin,
  output: defaultStdout,
  error: defaultStderr,
}): Promise<number> {
  if (!options.scriptPath) {
    io.error.write("run requires a script file\n")
    return 1
  }
  const context = createInitialShellContext(options)
  const client = new LocalIpcClient(options.socketPath ?? options.kernelUrl ?? defaultKernelEndpoint())
  const clientId = `arroba-shell-script-${process.pid}-${Date.now()}`
  try {
    const source = await readFile(options.scriptPath, "utf8")
    const result = await executeShellScript(source.split(/\r?\n/), context, { client, clientId }, (line) => io.output.write(line), {
      continueOnError: options.continueOnError,
    })
    return result.code
  } catch (error) {
    io.error.write(`${formatShellError(error)}\n`)
    return 1
  } finally {
    await client.close()
  }
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
        write(`${formatShellError(error)}\n`)
        write(`line ${index + 1} failed; continuing\n`)
        continue
      }
      write(`${formatShellError(error)}\n`)
      write(`stopped at line ${index + 1}\n`)
      return { code: 1, context }
    }
  }
  return { code: failed ? 1 : 0, context }
}

async function executeShellLine(
  line: string,
  context: ShellContext,
  deps: ShellExecutorDeps,
  write: (text: string) => void,
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
    const scriptPath = resolveScriptPath(parsed.args[0]!, context)
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

function resolveScriptPath(scriptPath: string, context: ShellContext): string {
  return isAbsolute(scriptPath) ? scriptPath : resolvePath(context.worktree, scriptPath)
}

export function defaultKernelEndpoint(): string {
  if (process.env.ARROBA_KERNEL_URL) {
    return process.env.ARROBA_KERNEL_URL
  }
  const host = process.env.ARROBA_KERNEL_HOST ?? "127.0.0.1"
  const port = process.env.ARROBA_KERNEL_PORT ?? "43118"
  return `ws://${host}:${port}/kernel`
}

function formatShellError(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

class ShellHelpRequested extends Error {}

async function main() {
  try {
    const options = parseShellCliArgs(process.argv.slice(2))
    process.exitCode = options.mode === "run" ? await runShellScript(options) : await runShellRepl(options)
  } catch (error) {
    if (error instanceof ShellHelpRequested) {
      console.log(shellUsage())
      process.exitCode = 0
      return
    }
    console.error(formatShellError(error))
    console.error(shellUsage())
    process.exitCode = 1
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  void main()
}
