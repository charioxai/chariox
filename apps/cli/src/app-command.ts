import { executeAppCommand } from "@chariox/kernel-client/shell-app-command"
import { defaultKernelEndpoint, parseArgs } from "./cli-options.js"
import { isAppDeveloperCommand, runAppDeveloperCommand, type AppDeveloperDeps } from "./app-developer.js"
import { LocalIpcClient } from "./ipc.js"
import { AppFileInstaller, formatInstallOperation } from "./app-install-file.js"

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
  /** Test seams for following an install operation. */
  installWaitMs?: number
  installPollMs?: number
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
  const send = (request: Record<string, unknown>) => {
    client ??= deps.createClient(
      options.relayUrl ?? options.kernelUrl ?? options.socketPath ?? defaultKernelEndpoint(),
      {
        ...(options.relayToken ? { relayAuthToken: options.relayToken } : {}),
        ...(options.targetDaemonId ? { targetDaemonId: options.targetDaemonId } : {}),
        ...(options.targetDaemonAlias ? { targetDaemonAlias: options.targetDaemonAlias } : {}),
      },
    )
    return client.send(request)
  }
  try {
    if (args[0] === "install" || args[0] === "update") {
      await installFile(args, send, deps)
      return true
    }
    const result = await executeAppCommand(args, { send })
    if (!result.ok) throw new Error(result.message ?? "App command failed")
    if (result.message) deps.write(`${result.message}\n`)
  } finally {
    await client?.close()
  }
  return true
}

const installUsage = "usage: app install FILE.cxapp --session SESSION | app update INSTALLATION FILE.cxapp --session SESSION"
const terminal = new Set(["committed", "cancelled", "failed"])

/** Upload a local package and follow its operation until it ends; the owner
 * approves it in the named session's terminal. */
async function installFile(
  args: string[],
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>,
  deps: AppCommandDeps,
): Promise<void> {
  const at = args.indexOf("--session")
  const session = at > 0 ? args[at + 1] : undefined
  const rest = at > 0 ? [...args.slice(0, at), ...args.slice(at + 2)] : args
  const [action, ...targets] = rest
  if (!session || session.startsWith("--") || targets.length !== (action === "install" ? 1 : 2)
    || targets.some((value) => !value || value.startsWith("--"))) {
    throw new Error(installUsage)
  }
  const installer = new AppFileInstaller(send)
  try {
    let value = action === "install"
      ? await installer.install(targets[0]!, session)
      : await installer.update(targets[0]!, targets[1]!, session)
    deps.write(`${formatInstallOperation(value)}\n`)
    const until = Date.now() + (deps.installWaitMs ?? 15 * 60_000)
    while (!terminal.has(value.phase) && Date.now() < until) {
      await new Promise((resolve) => setTimeout(resolve, deps.installPollMs ?? 2_000))
      const next = await installer.status(value.request_id)
      if (next.phase !== value.phase) deps.write(`${formatInstallOperation(next)}\n`)
      value = next
    }
    if (value.phase === "failed" || value.phase === "cancelled") throw new Error(formatInstallOperation(value))
    if (!terminal.has(value.phase)) {
      // Queued needs nothing from the owner: it starts once a worker slot frees.
      const next = value.phase === "queued"
        ? "It starts once an App worker slot is free; check `chariox app list` later."
        : `Finish it in session ${session}'s terminal, then check \`chariox app list\`.`
      throw new Error(`Still ${value.phase} after waiting; the kernel keeps operation ${value.request_id}. ${next}`)
    }
  } finally {
    await installer.dispose()
  }
}
