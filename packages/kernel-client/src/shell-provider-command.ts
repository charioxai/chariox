import type {
  ProviderAuthStatus,
  ProviderLoginStart,
  ProviderProcessInfo,
} from "./kernel-types.js"
import {
  getProviderAuthStatusRequest,
  listProviderProcessesRequest,
  logoutProviderRequest,
  startProviderLoginRequest,
  teardownProviderProcessesRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  formatProviderAuthStatus,
  formatProviderLoginStart,
  formatProviderProcesses,
} from "./shell-provider-format.js"

type ShellProviderCommandDeps = {
  client: {
    send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  }
}

export async function executeProviderCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellProviderCommandDeps,
): Promise<ShellCommandResult> {
  const [action, providerArg, ...rest] = parsed.args
  if (action === "status") {
    const provider = providerArg ?? context.provider
    const response = await deps.client.send(getProviderAuthStatusRequest(provider))
    const status = expectVariant<{ status: ProviderAuthStatus }>(response, "ProviderAuthStatus").status
    return { ok: true, message: formatProviderAuthStatus(status), data: { status } }
  }
  if (action === "login" || action === "logout" || action === "reauth") {
    const provider = providerArg ?? context.provider
    if (action === "logout") {
      const response = await deps.client.send(logoutProviderRequest(provider))
      const result = expectVariant<{ provider: string }>(response, "ProviderLoggedOut")
      return { ok: true, message: `${result.provider} logged out`, data: result }
    }
    if (action === "reauth") {
      await deps.client.send(logoutProviderRequest(provider))
    }
    const response = await deps.client.send(startProviderLoginRequest(provider))
    const login = expectVariant<{ login: ProviderLoginStart }>(response, "ProviderLoginStarted").login
    const verb = action === "reauth" ? "reauth" : "login"
    return { ok: true, message: formatProviderLoginStart(login, verb), data: { login } }
  }
  if (action === "processes") {
    const subcommand = providerArg
    if (subcommand === "teardown") {
      const provider = rest[0]?.trim() || null
      if (!provider) {
        return { ok: false, message: "usage: provider processes teardown <provider>" }
      }
      const response = await deps.client.send(teardownProviderProcessesRequest(provider))
      const processes = expectVariant<{ processes: ProviderProcessInfo[] }>(response, "ProviderProcessesTornDown").processes
      return { ok: true, message: processes.length === 0 ? "no safe provider processes to tear down" : `tore down ${processes.length} provider process(es)\n${formatProviderProcesses(processes)}`, data: { processes } }
    }
    const provider = providerArg ?? null
    const response = await deps.client.send(listProviderProcessesRequest(provider))
    const processes = expectVariant<{ processes: ProviderProcessInfo[] }>(response, "ProviderProcessesListed").processes
    return { ok: true, message: formatProviderProcesses(processes), data: { processes } }
  }
  return { ok: false, message: "usage: provider status|login|logout|reauth|processes" }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
