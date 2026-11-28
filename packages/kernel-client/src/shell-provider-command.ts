import type {
  ProviderAuthStatus,
  ProviderLoginStart,
  ProviderLoginStatus,
  ProviderProcessInfo,
} from "./kernel-types.js"
import {
  getProviderAuthStatusRequest,
  getProviderLoginStatusRequest,
  sendProviderLoginInputRequest,
  sendProviderLoginVaultEntryRequest,
  cancelProviderLoginRequest,
  listProviderProcessesRequest,
  logoutProviderRequest,
  setProviderAccountCredentialRequest,
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
  readSecret?: ((prompt: string) => Promise<string>) | undefined
}

export async function executeProviderCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellProviderCommandDeps,
): Promise<ShellCommandResult> {
  const [action, providerArg, ...rest] = parsed.args
  if (action === "setup-token") {
    const provider = providerArg?.trim()
    const accountProfile = rest.find((value) => !value.startsWith("--"))?.trim() || "default"
    const replace = rest.includes("--replace")
    const run = !rest.includes("--paste")
    if (provider !== "claude" || rest.filter((value) => !value.startsWith("--")).length > 1 || rest.some((value) => value.startsWith("--") && !["--replace", "--run", "--paste"].includes(value))) {
      return { ok: false, message: "usage: provider setup-token claude [account-profile] [--replace] (advanced: --paste)" }
    }
    if (run) {
      const response = await deps.client.send(setProviderAccountCredentialRequest(
        provider, accountProfile, "", replace,
        { ...(context.sessionId ? { sessionId: context.sessionId } : {}), ...(context.agentId ? { agentId: context.agentId } : {}) }, true,
      ))
      const login = expectVariant<{ login: ProviderLoginStart }>(response, "ProviderLoginStarted").login
      return { ok: true, message: formatProviderLoginStart(login, "setup-token"), data: { login } }
    }
    if (!deps.readSecret) {
      return {
        ok: false,
        message: "provider setup-token requires hidden input support; run it from interactive chariox-shell",
      }
    }
    const value = (await deps.readSecret(`Claude setup token for ${accountProfile}: `)).trim()
    if (!value) return { ok: false, message: "Claude setup token must not be empty" }
    const response = await deps.client.send(setProviderAccountCredentialRequest(
      provider,
      accountProfile,
      value,
      replace,
      {
        ...(context.sessionId ? { sessionId: context.sessionId } : {}),
        ...(context.agentId ? { agentId: context.agentId } : {}),
      },
    ))
    const stored = expectVariant<{
      provider: string
      account_profile: string
      credential_id: string
      replaced: boolean
    }>(response, "ProviderAccountCredentialStored")
    return {
      ok: true,
      message: `${stored.provider}/${stored.account_profile} setup token verified and ${stored.replaced ? "replaced" : "stored"} in Chariox Vault`,
      data: { provider: stored.provider, account_profile: stored.account_profile, replaced: stored.replaced },
    }
  }
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
      if ("ProviderLogoutStarted" in response) {
        const logout = (response.ProviderLogoutStarted as { logout: ProviderLoginStart }).logout
        return { ok: true, message: formatProviderLoginStart(logout, "logout"), data: { logout } }
      }
      const result = expectVariant<{ provider: string; local_only?: boolean }>(response, "ProviderLoggedOut")
      return { ok: true, message: `${result.provider} logged out${result.local_only ? " on this machine only; other machines keep their copy" : ""}`, data: result }
    }
    const options = parseLoginOptions(rest)
    if (typeof options === "string") return { ok: false, message: options }
    if (action === "reauth" && !provider.startsWith("claude")) {
      const logoutResponse = await deps.client.send(logoutProviderRequest(provider))
      if ("ProviderLogoutStarted" in logoutResponse) {
        const logout = (logoutResponse.ProviderLogoutStarted as { logout: ProviderLoginStart }).logout
        return { ok: true, message: `${formatProviderLoginStart(logout, "logout")}\nFinish logout before starting reauthentication.`, data: { logout } }
      }
    }
    const response = await deps.client.send(provider.startsWith("claude")
      ? setProviderAccountCredentialRequest("claude", options.accountProfile || "default", "", true,
        { ...(context.sessionId ? { sessionId: context.sessionId } : {}), ...(context.agentId ? { agentId: context.agentId } : {}) }, true)
      : startProviderLoginRequest(provider, options.accountProfile, options.method))
    const login = expectVariant<{ login: ProviderLoginStart }>(response, "ProviderLoginStarted").login
    const verb = action === "reauth" ? "reauth" : "login"
    if (options.vaultEntry && login.login_id) {
      await deps.client.send(sendProviderLoginVaultEntryRequest(login.login_id, options.vaultEntry))
      return await finishedVaultLogin(deps, login.login_id)
    }
    return { ok: true, message: formatProviderLoginStart(login, verb), data: { login } }
  }
  if (action === "login-status") {
    if (!providerArg) return { ok: false, message: "usage: provider login-status <login-id>" }
    const response = await deps.client.send(getProviderLoginStatusRequest(providerArg))
    const login = expectVariant<{ login: ProviderLoginStatus }>(response, "ProviderLoginStatus").login
    const output = Buffer.from(login.terminal_output_base64, "base64").toString("utf8").trimEnd()
    return {
      ok: login.state !== "failed",
      message: [output, `${login.provider}/${login.account_profile} login ${login.state}`].filter(Boolean).join("\n"),
      data: { login },
    }
  }
  if (action === "login-input") {
    if (!providerArg || !deps.readSecret) return { ok: false, message: "provider login-input requires a login ID and hidden input support" }
    const value = await deps.readSecret("provider login input: ")
    await deps.client.send(sendProviderLoginInputRequest(providerArg, Buffer.from(`${value}\r`, "utf8").toString("base64")))
    return { ok: true, message: "Hidden provider response sent" }
  }
  if (action === "login-cancel") {
    if (!providerArg) return { ok: false, message: "usage: provider login-cancel <login-id>" }
    const response = await deps.client.send(cancelProviderLoginRequest(providerArg))
    const login = expectVariant<{ login: ProviderLoginStatus }>(response, "ProviderLoginCancelled").login
    return { ok: true, message: `${login.provider}/${login.account_profile} login cancelled`, data: { login } }
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
  return { ok: false, message: "usage: provider status|login|setup-token|login-status|login-cancel|logout|reauth|processes [provider]|processes teardown <provider>" }
}

type LoginOptions = { accountProfile?: string; method?: string; vaultEntry?: string }

/** `[account-profile] [--method <method>] [--vault <credential-id>]`; text is the usage error. */
function parseLoginOptions(args: string[]): LoginOptions | string {
  const usage = "usage: provider login <provider> [account-profile] [--method <method>] [--vault <credential-id>]"
  const options: LoginOptions = {}
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index] as string
    if (arg === "--method" || arg === "--vault") {
      const value = args[index + 1]?.trim()
      if (!value || value.startsWith("--")) return usage
      if (arg === "--method") options.method = value
      else options.vaultEntry = value
      index += 1
    } else if (arg.startsWith("--") || options.accountProfile) {
      return usage
    } else {
      options.accountProfile = arg.trim()
    }
  }
  if (options.vaultEntry && !options.method) {
    return "provider login --vault needs --method <method> of an API key sign-in, such as opencode_go_api_key"
  }
  return options
}

const VAULT_LOGIN_POLL_MS = 500
const VAULT_LOGIN_POLL_LIMIT = 40

/** The kernel types the entry once the login asks for its key; the answer is the login's own end state. */
async function finishedVaultLogin(deps: ShellProviderCommandDeps, loginId: string): Promise<ShellCommandResult> {
  for (let attempt = 0; attempt < VAULT_LOGIN_POLL_LIMIT; attempt += 1) {
    const response = await deps.client.send(getProviderLoginStatusRequest(loginId))
    const login = expectVariant<{ login: ProviderLoginStatus }>(response, "ProviderLoginStatus").login
    if (login.state !== "running") {
      return {
        ok: login.state === "succeeded",
        message: `${login.provider}/${login.account_profile} login ${login.state} with a Vault entry`,
        data: { login },
      }
    }
    await new Promise((resolve) => setTimeout(resolve, VAULT_LOGIN_POLL_MS))
  }
  return { ok: false, message: `login ${loginId} is still running; check it with provider login-status ${loginId}` }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
