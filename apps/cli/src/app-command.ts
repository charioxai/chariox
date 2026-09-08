import { executeAppCommand } from "@chariox/kernel-client/shell-app-command"
import { defaultKernelEndpoint, parseArgs } from "./cli-options.js"
import { isAppDeveloperCommand, runAppDeveloperCommand, type AppDeveloperDeps } from "./app-developer.js"
import { LocalIpcClient } from "./ipc.js"

type AppCommandClient = {
  send(request: Record<string, unknown>): Promise<Record<string, unknown>>
  close(): Promise<void>
}

type ConnectionOptions = {
  relayAuthToken?: string
  targetDaemonId?: string
  targetDaemonAlias?: string
}

type AppCommandDeps = {
  createClient: (endpoint: string, options: ConnectionOptions) => AppCommandClient
  write: (message: string) => void
  developer?: AppDeveloperDeps
}

const connectionFlags = new Set([
  "--socket", "--kernel-url", "--relay-url", "--relay-token",
  "--target-daemon-id", "--target-daemon-alias", "--terminal-pairing-link", "--pairing-link",
])

export async function runAppCommand(
  argv: readonly string[],
  deps: AppCommandDeps = {
    createClient: (endpoint, options) => new LocalIpcClient(endpoint, options),
    write: (message) => { process.stdout.write(message) },
  },
): Promise<boolean> {
  if (argv[0] !== "app") return false
  if (isAppDeveloperCommand(argv[1])) return runAppDeveloperCommand(argv.slice(1), deps.developer)
  const args: string[] = []
  const connectionArgs: string[] = []
  const seen = new Set<string>()
  for (let index = 1; index < argv.length; index += 1) {
    const arg = argv[index]!
    if (!connectionFlags.has(arg)) {
      args.push(arg)
      continue
    }
    const value = argv[++index]
    if (!value || value.startsWith("--")) throw new Error(`missing value for ${arg}`)
    const canonicalFlag = arg === "--pairing-link" ? "--terminal-pairing-link" : arg
    if (seen.has(canonicalFlag)) throw new Error(`duplicate connection option ${arg}`)
    seen.add(canonicalFlag)
    connectionArgs.push(arg, value)
  }
  if (seen.has("--terminal-pairing-link") && seen.size > 1) {
    throw new Error("a terminal pairing link cannot be combined with other connection options")
  }
  const options = parseArgs(connectionArgs)
  if (!options.relayUrl && (options.relayToken || options.targetDaemonId || options.targetDaemonAlias)) {
    throw new Error("relay credentials and targets require --relay-url")
  }
  if (options.socketPath && options.kernelUrl) {
    throw new Error("--socket cannot be used together with --kernel-url")
  }
  let client: AppCommandClient | undefined
  try {
    const result = await executeAppCommand(args, {
      send: (request) => {
        client ??= deps.createClient(
          options.relayUrl ?? options.kernelUrl ?? options.socketPath ?? defaultKernelEndpoint(),
          {
            ...(options.relayToken ? { relayAuthToken: options.relayToken } : {}),
            ...(options.targetDaemonId ? { targetDaemonId: options.targetDaemonId } : {}),
            ...(options.targetDaemonAlias ? { targetDaemonAlias: options.targetDaemonAlias } : {}),
          },
        )
        return client.send(request)
      },
    })
    if (!result.ok) throw new Error(result.message ?? "App command failed")
    if (result.message) deps.write(`${result.message}\n`)
  } finally {
    await client?.close()
  }
  return true
}
