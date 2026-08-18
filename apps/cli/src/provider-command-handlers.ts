import type {
  ProviderAuthStatus,
  ProviderAccountProfile,
  ProviderLoginStart,
  ProviderProcessInfo,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import { isBackendProviderId } from "./provider-catalog.js"

type FooterTone = "info" | "error"

export type ProviderCommandHandlerDeps = {
  currentProviderId: () => string
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  applyProviderSelection?: (value: string) => Promise<void>
  getProviderAuthStatus?: (provider: string, accountProfile?: string) => Promise<ProviderAuthStatus>
  startProviderLogin?: (provider: string, accountProfile?: string) => Promise<ProviderLoginStart>
  logoutProvider?: (provider: string, accountProfile?: string) => Promise<{ provider: string; account_profile?: string }>
  listProviderAccountProfiles?: (provider?: string | null) => Promise<ProviderAccountProfile[]>
  createProviderAccountProfile?: (provider: string, label: string) => Promise<ProviderAccountProfile>
  linkProviderAccountProfile?: (provider: string, label: string, path: string) => Promise<ProviderAccountProfile>
  renameProviderAccountProfile?: (provider: string, profile: string, label: string) => Promise<ProviderAccountProfile>
  setDefaultProviderAccountProfile?: (provider: string, profile: string) => Promise<ProviderAccountProfile>
  refreshProviderAccountProfile?: (provider: string, profile: string) => Promise<ProviderAccountProfile>
  removeProviderAccountProfile?: (provider: string, profile: string) => Promise<ProviderAccountProfile>
  deleteProviderAccountProfileData?: (provider: string, profile: string) => Promise<ProviderAccountProfile>
  listProviderProcesses?: (provider?: string | null) => Promise<ProviderProcessInfo[]>
  teardownProviderProcesses?: (provider?: string | null) => Promise<ProviderProcessInfo[]>
}

export async function handleProviderSlashCommand(
  deps: ProviderCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "provider" }>,
): Promise<void> {
  const { value } = command
  if (!value) {
    deps.flashFooter("usage: /provider <opencode|codex|claude-headless|claude-p|status|login|logout|reauth|processes>", "error")
    return
  }

  const parts = value.split(/\s+/).filter(Boolean)
  const [action, maybeProvider, maybeProfile] = parts
  if (action === "accounts") {
    await handleProviderAccountsCommand(deps, parts.slice(1))
    return
  }
  if (action === "status") {
    await showProviderStatus(deps, maybeProvider ?? deps.currentProviderId(), maybeProfile)
    return
  }
  if (action === "login") {
    await startProviderLogin(deps, maybeProvider ?? deps.currentProviderId(), maybeProfile)
    return
  }
  if (action === "logout") {
    await logoutProvider(deps, maybeProvider ?? deps.currentProviderId(), maybeProfile)
    return
  }
  if (action === "reauth") {
    await reauthProvider(deps, maybeProvider ?? deps.currentProviderId(), maybeProfile)
    return
  }
  if (action === "processes") {
    await handleProviderProcessesCommand(deps, parts)
    return
  }
  if (!isBackendProviderId(value)) {
    deps.flashFooter(`unknown provider: ${value}`, "error")
    return
  }
  if (deps.applyProviderSelection) {
    await deps.applyProviderSelection(value)
  } else {
    deps.flashFooter(`${value} selected`, "info")
  }
}

function formatProviderProcessNotice(process: ProviderProcessInfo): string {
  const lines = [
    `${process.process_id} provider=${process.provider} pid=${process.pid ?? "-"} rss=${formatBytes(process.resident_set_bytes ?? null)} status=${process.status} mode=${process.endpoint_mode} safe=${String(process.teardown_safe)}`,
    `  provider sessions: ${process.provider_session_ids.join(",") || "-"}`,
    `  owner runs: ${process.owner_provider_run_ids.join(",") || "-"}`,
    `  owner sessions: ${process.owner_session_ids.join(",") || "-"}`,
    `  attached sessions: ${process.attached_session_ids.join(",") || "-"}`,
    `  active workflow runs: ${process.active_workflow_run_ids.join(",") || "-"}`,
  ]
  if (process.teardown_blockers.length > 0) {
    lines.push(`  blockers: ${process.teardown_blockers.join("; ")}`)
  }
  lines.push(`  next: ${providerProcessNextAction(process)}`)
  return lines.join("\n")
}

function providerProcessNextAction(process: ProviderProcessInfo): string {
  const teardown = `run /provider processes teardown ${process.provider}`
  if (process.teardown_safe) {
    return `${teardown} to stop only safe daemon-tracked processes owned by you`
  }
  if (process.attached_session_ids.length > 0) {
    return `detach or finish attached sessions ${process.attached_session_ids.join(",")}; then ${teardown}`
  }
  if (process.active_workflow_run_ids.length > 0) {
    return `stop or finish workflow runs ${process.active_workflow_run_ids.join(",")}; then ${teardown}`
  }
  if (process.teardown_blockers.length > 0) {
    return `resolve blockers: ${process.teardown_blockers.join("; ")}; then ${teardown}`
  }
  return `inspect the owning session before teardown; then ${teardown}`
}

function formatBytes(bytes: number | null): string {
  if (typeof bytes !== "number" || !Number.isFinite(bytes) || bytes <= 0) {
    return "unknown"
  }
  const units = ["B", "KiB", "MiB", "GiB"]
  let value = bytes
  let unitIndex = 0
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024
    unitIndex += 1
  }
  const formatted = unitIndex === 0 ? `${Math.round(value)}` : value >= 10 ? value.toFixed(1) : value.toFixed(2)
  return `${formatted}${units[unitIndex]}`
}

async function showProviderStatus(
  deps: ProviderCommandHandlerDeps,
  provider: string,
  accountProfile?: string,
): Promise<void> {
  if (!deps.getProviderAuthStatus) {
    deps.flashFooter("provider status is not available in this daemon", "error")
    return
  }
  const status = await deps.getProviderAuthStatus(provider, accountProfile)
  const details = `${status.provider}/${status.account_profile}: ${status.auth_state}${status.identity_summary ? ` as ${status.identity_summary}` : ""}`
  deps.appendNotice(
    [
      details,
      status.detected_version ? `version ${status.detected_version}` : null,
      status.plan ? `plan ${status.plan}` : null,
      status.login_hint ?? null,
    ].filter(Boolean).join(" • "),
  )
  deps.flashFooter(details, "info")
}

async function startProviderLogin(
  deps: ProviderCommandHandlerDeps,
  provider: string,
  accountProfile?: string,
): Promise<void> {
  if (!deps.startProviderLogin) {
    deps.flashFooter("provider login is not available in this daemon", "error")
    return
  }
  const login = await deps.startProviderLogin(provider, accountProfile)
  const message = formatProviderLoginNotice(login, "login started")
  deps.appendNotice(message)
  deps.flashFooter(message, "info")
}

async function logoutProvider(
  deps: ProviderCommandHandlerDeps,
  provider: string,
  accountProfile?: string,
): Promise<void> {
  if (!deps.logoutProvider) {
    deps.flashFooter("provider logout is not available in this daemon", "error")
    return
  }
  const loggedOut = await deps.logoutProvider(provider, accountProfile)
  const message = `${loggedOut.provider}/${loggedOut.account_profile ?? accountProfile ?? "default"} logged out`
  deps.appendNotice(message)
  deps.flashFooter(message, "info")
}

async function reauthProvider(
  deps: ProviderCommandHandlerDeps,
  provider: string,
  accountProfile?: string,
): Promise<void> {
  if (!deps.logoutProvider || !deps.startProviderLogin) {
    deps.flashFooter("provider reauth is not available in this daemon", "error")
    return
  }
  await deps.logoutProvider(provider, accountProfile)
  const login = await deps.startProviderLogin(provider, accountProfile)
  const message = formatProviderLoginNotice(login, "reauth started")
  deps.appendNotice(message)
  deps.flashFooter(message, "info")
}

async function handleProviderAccountsCommand(
  deps: ProviderCommandHandlerDeps,
  args: string[],
): Promise<void> {
  const [action = "list", provider, profile, ...rest] = args
  if (action === "list") {
    if (!deps.listProviderAccountProfiles) {
      deps.flashFooter("provider account management is unavailable", "error")
      return
    }
    const profiles = await deps.listProviderAccountProfiles(provider ?? null)
    const lines = profiles.map((entry) => {
      const usage = entry.usage.meters?.[0]
      const usageText = usage?.used_percent != null
        ? `${Math.round(usage.used_percent)}% used`
        : usage?.remaining != null
          ? `${usage.remaining}${usage.unit ? ` ${usage.unit}` : ""} remaining`
          : entry.usage.availability
      return `${entry.provider}/${entry.profile_id} "${entry.label}"${entry.is_default ? " [default]" : ""} · ${entry.auth_state}${entry.identity_summary ? ` · ${entry.identity_summary}` : ""}${entry.plan ? ` · ${entry.plan}` : ""} · ${usageText}`
    })
    deps.appendNotice(lines.length > 0 ? lines.join("\n") : "No provider accounts registered")
    deps.flashFooter(`${profiles.length} provider account profile${profiles.length === 1 ? "" : "s"}`, "info")
    return
  }
  if (!provider) {
    deps.flashFooter("usage: /provider accounts <add|link|refresh|rename|default|remove|delete> <provider> ...", "error")
    return
  }
  let result: ProviderAccountProfile
  if (action === "add" && profile && deps.createProviderAccountProfile) {
    result = await deps.createProviderAccountProfile(provider, [profile, ...rest].join(" "))
  } else if (action === "link" && profile && rest.length >= 1 && deps.linkProviderAccountProfile) {
    const path = rest.at(-1)!
    result = await deps.linkProviderAccountProfile(provider, [profile, ...rest.slice(0, -1)].join(" "), path)
  } else if (action === "refresh" && profile && deps.refreshProviderAccountProfile) {
    result = await deps.refreshProviderAccountProfile(provider, profile)
  } else if (action === "rename" && profile && rest.length > 0 && deps.renameProviderAccountProfile) {
    result = await deps.renameProviderAccountProfile(provider, profile, rest.join(" "))
  } else if (action === "default" && profile && deps.setDefaultProviderAccountProfile) {
    result = await deps.setDefaultProviderAccountProfile(provider, profile)
  } else if (action === "remove" && profile && deps.removeProviderAccountProfile) {
    result = await deps.removeProviderAccountProfile(provider, profile)
  } else if (action === "delete" && profile && rest[0] === profile && deps.deleteProviderAccountProfileData) {
    result = await deps.deleteProviderAccountProfileData(provider, profile)
  } else {
    deps.flashFooter("invalid or unavailable Provider Accounts action; destructive delete requires repeating the profile ID", "error")
    return
  }
  deps.appendNotice(`${action}: ${result.provider}/${result.profile_id} "${result.label}"`)
  deps.flashFooter(`provider account ${action} complete`, "info")
}

function formatProviderLoginNotice(
  login: ProviderLoginStart,
  action: string,
): string {
  return [
    `${login.provider} ${action}`,
    login.user_code ? `code ${login.user_code}` : null,
    login.verification_url ?? login.auth_url ?? null,
  ].filter(Boolean).join(" • ")
}

async function handleProviderProcessesCommand(
  deps: ProviderCommandHandlerDeps,
  parts: string[],
): Promise<void> {
  if (parts[1] === "teardown") {
    const provider = parts[2]?.trim() || null
    if (!provider) {
      deps.flashFooter("usage: /provider processes teardown <provider>", "error")
      return
    }
    await teardownProviderProcesses(deps, provider)
    return
  }
  await listProviderProcesses(deps, parts[1] ?? null)
}

async function teardownProviderProcesses(
  deps: ProviderCommandHandlerDeps,
  provider: string | null,
): Promise<void> {
  if (!deps.teardownProviderProcesses) {
    deps.flashFooter("provider process teardown is not available in this daemon", "error")
    return
  }
  const blocked = deps.listProviderProcesses
    ? (await deps.listProviderProcesses(provider)).filter((process) => !process.teardown_safe)
    : []
  const tornDown = await deps.teardownProviderProcesses(provider)
  if (tornDown.length === 0) {
    if (blocked.length > 0) {
      deps.appendNotice(`blocked provider processes:\n${blocked.map((process) => formatProviderProcessNotice(process)).join("\n")}`)
    }
    deps.flashFooter("no safe provider processes to tear down", "info")
    return
  }
  deps.appendNotice(
    tornDown.map((process) => formatProviderProcessNotice(process)).join("\n"),
  )
  if (blocked.length > 0) {
    deps.appendNotice(`skipped blocked provider processes:\n${blocked.map((process) => formatProviderProcessNotice(process)).join("\n")}`)
  }
  deps.flashFooter(`tore down ${tornDown.length} provider process(es)`, "info")
}

async function listProviderProcesses(
  deps: ProviderCommandHandlerDeps,
  provider: string | null,
): Promise<void> {
  if (!deps.listProviderProcesses) {
    deps.flashFooter("provider process inspection is not available in this daemon", "error")
    return
  }
  const processes = await deps.listProviderProcesses(provider)
  if (processes.length === 0) {
    deps.flashFooter("no daemon-tracked provider processes", "info")
    return
  }
  deps.appendNotice(
    processes.map((process) => formatProviderProcessNotice(process)).join("\n"),
  )
  deps.flashFooter(`listed ${processes.length} provider process(es)`, "info")
}
