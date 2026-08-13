import { createInterface } from "node:readline/promises"
import { readFile } from "node:fs/promises"
import { stdin as defaultStdin, stdout as defaultStdout, stderr as defaultStderr } from "node:process"
import { fileURLToPath } from "node:url"

import { LocalIpcClient } from "@chariox/kernel-client/ipc"
import { createDefaultShellContext, type ShellContext } from "@chariox/kernel-client/shell-core"
import { executeShellLine, executeShellScript, executeShellScriptLines } from "@chariox/kernel-client/shell-script"

export { executeShellLine, executeShellScript, executeShellScriptLines }

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
    "usage: chariox-shell [--kernel-url URL|--socket PATH] [--workspace PATH] [--worktree PATH] [--provider NAME] [--model MODEL] [--effort LEVEL] [--var NAME=VALUE]",
    "       chariox-shell run <file> [--kernel-url URL|--socket PATH] [--workspace PATH] [--worktree PATH] [--provider NAME] [--model MODEL] [--effort LEVEL] [--var NAME=VALUE] [--continue-on-error]",
    "",
    "Runs a Chariox command REPL. Commands do not use the TUI slash prefix:",
    "  @ session list",
    "  @ session new --dir ../qa as s",
    "  @ session use $s",
    "  @ agent spawn reviewer gpt-5.2 as reviewer",
    "  @ prompt $reviewer \"Summarize the repo\" --wait --show-summary",
    "  @ mcp list",
    "  @ skill list",
    "  @ machine list",
    "  @ slice list",
    "  @ relay status",
    "  @ credential set github-token",
    "  @ provider status codex",
    "  @ set provider codex",
    "  @ context",
    "  @ pwd",
    "  @ vars",
    "  @ source setup.chariox",
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
  const clientId = `chariox-shell-${process.pid}-${Date.now()}`
  const readline = createInterface({ input: io.input, output: io.output, terminal: Boolean((io.output as { isTTY?: boolean }).isTTY) })

  try {
    io.output.write("chariox-shell\n")
    for (;;) {
      const line = await readline.question("@ ")
      const result = await executeShellLine(
        line,
        context,
        {
          client,
          clientId,
          readSecret: async (prompt) => {
            readline.pause()
            try {
              return await readHiddenLine(prompt, io)
            } finally {
              readline.resume()
            }
          },
        },
        (text) => io.output.write(text),
      )
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

async function readHiddenLine(prompt: string, io: ShellIo): Promise<string> {
  const input = io.input as NodeJS.ReadStream & {
    isRaw?: boolean
    setRawMode?: (enabled: boolean) => NodeJS.ReadStream
  }
  const output = io.output as NodeJS.WritableStream & { isTTY?: boolean }
  if (!input.isTTY || !output.isTTY || typeof input.setRawMode !== "function") {
    throw new Error("hidden credential input requires an interactive TTY")
  }

  return await new Promise<string>((resolve, reject) => {
    const wasRaw = Boolean(input.isRaw)
    const chunks: string[] = []
    let settled = false

    const cleanup = () => {
      input.off("data", onData)
      input.setRawMode?.(wasRaw)
      output.write("\n")
    }
    const finish = (value: string) => {
      if (settled) return
      settled = true
      cleanup()
      resolve(value)
    }
    const fail = (error: Error) => {
      if (settled) return
      settled = true
      cleanup()
      reject(error)
    }
    const onData = (chunk: Buffer | string) => {
      const text = chunk.toString("utf8")
      for (const char of text) {
        if (char === "\u0003") {
          fail(new Error("credential input cancelled"))
          return
        }
        if (char === "\r" || char === "\n") {
          finish(chunks.join(""))
          return
        }
        if (char === "\u007f" || char === "\b") {
          chunks.pop()
          continue
        }
        if (char >= " ") {
          chunks.push(char)
        }
      }
    }

    output.write(prompt)
    input.setRawMode(true)
    input.resume()
    input.on("data", onData)
  })
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
  const clientId = `chariox-shell-script-${process.pid}-${Date.now()}`
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

export function defaultKernelEndpoint(): string {
  if (process.env.CHARIOX_KERNEL_URL) {
    return process.env.CHARIOX_KERNEL_URL
  }
  const host = process.env.CHARIOX_KERNEL_HOST ?? "127.0.0.1"
  const port = process.env.CHARIOX_KERNEL_PORT ?? "43118"
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
