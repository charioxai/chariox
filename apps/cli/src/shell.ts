import { createInterface } from "node:readline/promises"
import { stdin as defaultStdin, stdout as defaultStdout, stderr as defaultStderr } from "node:process"
import { fileURLToPath } from "node:url"

import { LocalIpcClient } from "./ipc.js"
import { applyShellCommandResult, createDefaultShellContext, parseShellCommand, renderShellCommandResult, type ShellContext } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"

export type ShellCliOptions = {
  kernelUrl?: string | undefined
  socketPath?: string | undefined
  workspace?: string | undefined
  worktree?: string | undefined
  provider?: string | undefined
  model?: string | undefined
  effort?: string | undefined
}

export type ShellIo = {
  input: NodeJS.ReadableStream
  output: NodeJS.WritableStream
  error: NodeJS.WritableStream
}

export function parseShellCliArgs(argv: string[]): ShellCliOptions {
  const options: ShellCliOptions = {}
  const next = (index: number, flag: string) => {
    const value = argv[index + 1]
    if (!value || value.startsWith("--")) {
      throw new Error(`${flag} requires a value`)
    }
    return value
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
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
  return createDefaultShellContext(contextOptions)
}

export function shellUsage(): string {
  return [
    "usage: arroba-shell [--kernel-url URL|--socket PATH] [--workspace PATH] [--worktree PATH] [--provider NAME] [--model MODEL] [--effort LEVEL]",
    "",
    "Runs an Arroba command REPL. Commands do not use the TUI slash prefix:",
    "  @ session list",
    "  @ session new --dir ../qa as s",
    "  @ session use $s",
    "  @ agent spawn reviewer gpt-5.2 as reviewer",
    "  @ set provider codex",
    "  @ vars",
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
  const readline = createInterface({ input: io.input, output: io.output, terminal: Boolean((io.output as { isTTY?: boolean }).isTTY) })

  try {
    io.output.write("arroba-shell\n")
    for (;;) {
      const line = await readline.question("@ ")
      const parsed = parseShellCommand(line, context)
      const result = await executeShellCommand(parsed, context, { client })
      const rendered = renderShellCommandResult(result)
      if (rendered) {
        io.output.write(`${rendered}\n`)
      }
      context = applyShellCommandResult(context, result)
      if (result.data && typeof result.data === "object" && "exit" in result.data) {
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
    process.exitCode = await runShellRepl(options)
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
