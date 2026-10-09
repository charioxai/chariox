import { executeAppCommand } from "@chariox/kernel-client/shell-app-command"
import { isTuiOnlyAppCommand } from "./app-command-catalog.js"
import { defaultKernelEndpoint, parseArgs } from "./cli-options.js"
import { isAppDeveloperCommand, runAppDeveloperCommand, type AppDeveloperDeps } from "./app-developer.js"
import { LocalIpcClient } from "./ipc.js"
import { AppFileInstaller, formatInstallOperation, terminalPhases } from "./app-install-file.js"

type AppCommandClient = {
  send(request: Record<string, unknown>): Promise<Record<string, unknown>>
  close(): Promise<void>
}

type ConnectionOptions = {
  relayAuthorizationIssuer?: { endpoint: string; daemonId: string }
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
  "--socket", "--kernel-url", "--relay-url", "--relay-token", "--relay-token-issuer",
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
  if (isTuiOnlyAppCommand(argv[1]!, argv[2])) throw new Error(`app ${argv[1]} runs in a Chariox terminal: use /app ${argv[1]} there`)
  const args: string[] = []
  const connectionArgs: string[] = []
  const seen = new Set<string>()
  for (let index = 1; index < argv.length; index += 1) {
    const arg = argv[index]!
    if (!connectionFlags.has(arg)) {
      args.push(arg)
      continue
    }
    const canonicalFlag = arg === "--pairing-link" ? "--terminal-pairing-link" : arg
    if (seen.has(canonicalFlag)) throw new Error(`duplicate connection option ${arg}`)
    seen.add(canonicalFlag)
    connectionArgs.push(arg)
    const valueCount = arg === "--relay-token-issuer" ? 2 : 1
    for (let valueIndex = 0; valueIndex < valueCount; valueIndex++) {
      const value = argv[++index]
      if (!value || value.startsWith("--")) throw new Error(`missing value for ${arg}`)
      connectionArgs.push(value)
    }
  }
  if (seen.has("--terminal-pairing-link") && seen.size > 1) {
    throw new Error("a terminal pairing link cannot be combined with other connection options")
  }
  const options = parseArgs(connectionArgs)
  if (!options.relayUrl && (options.relayToken || options.relayTokenIssuer || options.targetDaemonId || options.targetDaemonAlias)) {
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
        ...(options.relayTokenIssuer ? { relayAuthorizationIssuer: options.relayTokenIssuer } : {}),
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
    const result = await executeAppCommand(args, { send }, { appCommandPrefix: "chariox app" })
    if (!result.ok) throw new Error(result.message ?? "App command failed")
    if (result.message) deps.write(`${result.message}\n`)
  } finally {
    await client?.close()
  }
  return true
}

const installUsage = "usage: app install FILE.cxapp --session SESSION | app update INSTALLATION FILE.cxapp --session SESSION"

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
    value = await installer.follow(value, next => deps.write(`${formatInstallOperation(next)}\n`), {
      pollMs: deps.installPollMs ?? 2_000, until: Date.now() + (deps.installWaitMs ?? 15 * 60_000),
    })
    if (value.phase === "failed" || value.phase === "cancelled") throw new Error(formatInstallOperation(value))
    if (!terminalPhases.has(value.phase)) {
      // Only an operation awaiting approval needs the owner.
      const next = value.phase === "awaiting_approval"
        ? `Finish it in session ${session}'s terminal, then check \`chariox app list\`.`
        : value.phase === "queued"
          ? "It starts once an App worker slot is free; check `chariox app list` later."
          : "It continues on its own; check `chariox app list` later."
      throw new Error(`Still ${value.phase} after waiting; the kernel keeps operation ${value.request_id}. ${next}`)
    }
  } finally {
    await installer.dispose()
  }
}
