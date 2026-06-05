#!/usr/bin/env node
import { readFile } from "node:fs/promises"
import process from "node:process"

import {
  invokePublicationInput,
  loadGatewayPublicationConfig,
  loadPublicationConfig,
  loadPublicationConfigFromKernel,
  loadPublicationPackageConfig,
  type WorkflowPublicationConfig,
} from "./index.js"

type CallOptions = {
  configPath?: string
  packagePath?: string
  hookId?: string
  kernelUrl?: string
  sessionId?: string
  publicationId?: string
  inputJson?: string
  inputFile?: string
  mode?: "sync" | "async"
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const publication = await loadPublication(options)
  const input = await loadInput(options)
  const invocationOptions = {
    input,
    requestIdPrefix: "ipc",
    ...(options.mode ? { mode: options.mode } : {}),
  }
  const result = await invokePublicationInput(publication, {
    ...invocationOptions,
  })
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`)
}

function parseArgs(args: string[]): CallOptions {
  const options: CallOptions = {}
  const positionals: string[] = []
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (arg === "--config") {
      options.configPath = requireValue(args[++index], arg)
    } else if (arg === "--package") {
      options.packagePath = requireValue(args[++index], arg)
    } else if (arg === "--hook") {
      options.hookId = requireValue(args[++index], arg)
    } else if (arg === "--kernel-url") {
      options.kernelUrl = requireValue(args[++index], arg)
    } else if (arg === "--session-id") {
      options.sessionId = requireValue(args[++index], arg)
    } else if (arg === "--publication-id" || arg === "--publication") {
      options.publicationId = requireValue(args[++index], arg)
    } else if (arg === "--input") {
      options.inputJson = requireValue(args[++index], arg)
    } else if (arg === "--input-file") {
      options.inputFile = requireValue(args[++index], arg)
    } else if (arg === "--mode") {
      const mode = requireValue(args[++index], arg)
      if (mode !== "sync" && mode !== "async") throw new Error("--mode must be sync or async")
      options.mode = mode
    } else if (arg === "--help" || arg === "-h") {
      printHelp()
      process.exit(0)
    } else if (arg?.startsWith("-")) {
      throw new Error(`unknown option: ${arg}`)
    } else if (arg) {
      positionals.push(arg)
    }
  }
  if (!options.publicationId && positionals[0]) options.publicationId = positionals[0]
  return options
}

function requireValue(value: string | undefined, option: string) {
  if (!value) throw new Error(`${option} requires a value`)
  return value
}

async function loadPublication(options: CallOptions): Promise<WorkflowPublicationConfig> {
  if (options.configPath) return loadPublicationConfig(options.configPath)
  if (options.packagePath) {
    const packageOptions: { kernelEndpoint?: string; hookId?: string } = {}
    if (options.kernelUrl) packageOptions.kernelEndpoint = options.kernelUrl
    if (options.hookId) packageOptions.hookId = options.hookId
    return loadPublicationPackageConfig(options.packagePath, packageOptions)
  }
  if (options.sessionId && options.publicationId) {
    return loadPublicationConfigFromKernel(options.sessionId, options.publicationId, options.kernelUrl ?? defaultKernelEndpoint())
  }
  const fromEnv = await loadGatewayPublicationConfig()
  if (fromEnv) return fromEnv
  throw new Error("publication not specified; use --config or --session-id plus --publication-id")
}

async function loadInput(options: CallOptions): Promise<unknown> {
  if (options.inputJson !== undefined) return JSON.parse(options.inputJson)
  if (options.inputFile) {
    const text = options.inputFile === "-"
      ? await readStdin()
      : await readFile(options.inputFile, "utf8")
    return text.trim() ? JSON.parse(text) : {}
  }
  if (!process.stdin.isTTY) {
    const text = await readStdin()
    return text.trim() ? JSON.parse(text) : {}
  }
  return {}
}

async function readStdin() {
  let text = ""
  for await (const chunk of process.stdin) {
    text += chunk.toString()
  }
  return text
}

function defaultKernelEndpoint() {
  return process.env.ARROBA_KERNEL_URL ?? `ws://${process.env.ARROBA_KERNEL_HOST ?? "127.0.0.1"}:${process.env.ARROBA_KERNEL_PORT ?? "43118"}`
}

function printHelp() {
  process.stdout.write([
    "usage: arroba-workflow-call [publication-id] [options]",
    "",
    "Options:",
    "  --config <path>             Load exported publication.config.json",
    "  --package <path>            Load exported publication package directory or publication.json",
    "  --hook <id>                 Select a hook from the publication package",
    "  --session-id <id>           Kernel session id for kernel-owned publication lookup",
    "  --publication-id <id>       Kernel-owned publication id/ref",
    "  --kernel-url <url>          Kernel WebSocket URL",
    "  --input <json>              JSON input payload",
    "  --input-file <path|->       JSON input file or stdin",
    "  --mode <sync|async>         Override invocation mode",
    "",
  ].join("\n"))
}

main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
  process.exit(1)
})
