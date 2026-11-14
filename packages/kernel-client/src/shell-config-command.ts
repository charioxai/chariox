import { parseVaultUnlockDurationArgument, vaultUnlockDefaultFromStatus, vaultUnlockDurationUsage } from "./vault-unlock-duration.js"
import { visibleUserConfigEntries, workspaceLiveSyncUiEnabled } from "./workspace-live-sync-mode.js"
import type {
  CharioxUserConfigPayload,
  CharioxUserConfigSchemaPayload,
} from "./kernel-types.js"
import {
  createCredentialVaultRequest,
  deleteCredentialSecretRequest,
  getCredentialVaultStatusRequest,
  getCredentialRequest,
  getUserConfigRequest,
  getUserConfigSchemaRequest,
  lockCredentialVaultRequest,
  manageCredentialVaultRequest,
  listCredentialsRequest,
  registerCredentialRequest,
  removeCredentialRequest,
  setCredentialSecretRequest,
  unlockCredentialVaultRequest,
  setUserConfigValueRequest,
  unsetUserConfigValueRequest,
  upsertCredentialRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult } from "./shell-core.js"
import type { ShellContext } from "./shell-core.js"
import {
  configMutationMessage,
  formatConfigSchemaKeys,
} from "./shell-config-format.js"
import {
  formatWorkspaceLiveSyncDefaultModeChangeMessage,
} from "./workspace-live-sync-mode.js"

type ShellConfigCommandDeps = {
  client: {
    send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  }
  readSecret?: ((prompt: string) => Promise<string>) | undefined
}

export async function executeConfigCommand(
  parsed: ParsedShellCommand,
  deps: ShellConfigCommandDeps,
): Promise<ShellCommandResult> {
  const [action, keyPath, ...rest] = parsed.args
  if (action === "agents-offload") {
    if (rest.length || (keyPath && keyPath !== "on" && keyPath !== "off")) return { ok: false, message: "usage: config agents-offload [on|off]" }
    if (keyPath) {
      const changed = await deps.client.send(setUserConfigValueRequest("agents.offload_without_asking", keyPath === "on" ? "true" : "false"))
      expectVariant<CharioxUserConfigPayload>(changed, "UserConfigUpdated")
    }
    const response = await deps.client.send(getUserConfigRequest())
    const payload = expectVariant<CharioxUserConfigPayload>(response, "UserConfig")
    return { ok: true, message: `Agents may offload and acquire machines without asking: ${payload.config.agents?.offload_without_asking === true ? "On" : "Off"}`, data: payload }
  }
  if (!action || action === "show") {
    const response = await deps.client.send(getUserConfigRequest())
    const payload = expectVariant<CharioxUserConfigPayload>(response, "UserConfig")
    return { ok: true, message: JSON.stringify(payload.config, null, 2), data: payload, format: "json" }
  }
  if (action === "path") {
    const response = await deps.client.send(getUserConfigRequest())
    const payload = expectVariant<CharioxUserConfigPayload>(response, "UserConfig")
    return { ok: true, message: payload.path, data: payload }
  }
  if (action === "keys" || action === "list") {
    const response = await deps.client.send(getUserConfigSchemaRequest())
    const payload = expectVariant<CharioxUserConfigSchemaPayload>(response, "UserConfigSchema")
    return { ok: true, message: formatConfigSchemaKeys(visibleUserConfigEntries(payload.entries)), data: payload }
  }
  if (action === "schema") {
    const response = await deps.client.send(getUserConfigSchemaRequest())
    const payload = expectVariant<CharioxUserConfigSchemaPayload>(response, "UserConfigSchema")
    return { ok: true, message: JSON.stringify(visibleUserConfigEntries(payload.entries), null, 2), data: payload, format: "json" }
  }
  if (action === "set") {
    const value = rest.join(" ").trim()
    if (!keyPath || !value) {
      return { ok: false, message: "usage: config set <path> <value>" }
    }
    const response = await deps.client.send(setUserConfigValueRequest(keyPath, value))
    const payload = expectVariant<CharioxUserConfigPayload>(response, "UserConfigUpdated")
    return {
      ok: true,
      message: configMutationMessage(`config ${keyPath} set to ${value}`, payload),
      data: payload,
    }
  }
  if (action === "unset") {
    if (!keyPath) {
      return { ok: false, message: "usage: config unset <path>" }
    }
    const response = await deps.client.send(unsetUserConfigValueRequest(keyPath))
    const payload = expectVariant<CharioxUserConfigPayload>(response, "UserConfigUpdated")
    return {
      ok: true,
      message: configMutationMessage(`config ${keyPath} unset`, payload),
      data: payload,
    }
  }
  if (action === "workspace-live-sync") {
    if (!workspaceLiveSyncUiEnabled) return { ok: false, message: "This command is unavailable" } // MP-08 OWNER-20
    const mode = normalizeWorkspaceLiveSyncPolicy(keyPath ?? "off")
    if (rest.length > 0 || !mode) {
      return { ok: false, message: "usage: config workspace-live-sync off|managed|tracked" }
    }
    const response = await deps.client.send(setUserConfigValueRequest("providers.workspace_live_sync", mode))
    const payload = expectVariant<CharioxUserConfigPayload>(response, "UserConfigUpdated")
    return {
      ok: true,
      message: configMutationMessage(formatWorkspaceLiveSyncDefaultModeChangeMessage(mode), payload),
      data: payload,
    }
  }
  return { ok: false, message: "usage: config show|path|keys|schema|set|unset" }
}

function normalizeWorkspaceLiveSyncPolicy(value: string): "off" | "managed" | "tracked" | null {
  if (value === "off" || value === "managed" || value === "tracked") return value
  return null
}

export async function executeCredentialCommand(
  parsed: ParsedShellCommand,
  deps: ShellConfigCommandDeps,
  context: ShellContext = {
    workspace: "",
    worktree: "",
    provider: "",
    model: "",
    effort: "",
    variables: {},
  },
): Promise<ShellCommandResult> {
  const [action, key, ...rest] = parsed.args
  if (action === "vault") {
    const vaultAction = key ?? "status"
    if (vaultAction === "status") {
      const response = await deps.client.send(getCredentialVaultStatusRequest())
      const status = expectVariant<{ status: Record<string, unknown> }>(response, "CredentialVaultStatus").status
      return {
        ok: true,
        message: credentialVaultStatusMessage(status),
        data: { status },
        format: "json",
      }
    }
    if (vaultAction === "lock") {
      const response = await deps.client.send(lockCredentialVaultRequest())
      const status = expectVariant<{ status: Record<string, unknown> }>(response, "CredentialVaultLocked").status
      return { ok: true, message: "vault locked", data: { status }, format: "json" }
    }
    if (vaultAction === "create") {
      const input = await readNewVaultPassphrase(deps.readSecret)
      if (!input.ok) return input
      const response = await deps.client.send(createCredentialVaultRequest(input.passphrase))
      const payload = expectVariant<{ action: string; status: Record<string, unknown> }>(response, "CredentialVaultUnlocked")
      return { ok: true, message: `vault ${payload.action}`, data: payload, format: "json" }
    }
    if (vaultAction === "unlock") {
      if (!deps.readSecret) {
        return {
          ok: false,
          message: "credential vault unlock requires hidden input support; run it from interactive chariox-shell",
        }
      }
      const chosen = rest[0] === undefined ? undefined : parseVaultUnlockDurationArgument(rest[0])
      if (chosen === null) return { ok: false, message: `unknown unlock length "${rest[0]}". ${vaultUnlockDurationUsage}` }
      const passphrase = await deps.readSecret("Vault passphrase: ")
      if (!passphrase) return { ok: false, message: "the Vault passphrase must not be empty" }
      const duration = chosen ?? vaultUnlockDefaultFromStatus(await deps.client.send(getCredentialVaultStatusRequest()))
      const response = await deps.client.send(unlockCredentialVaultRequest(passphrase, duration, "terminal"))
      const payload = expectVariant<{ action: string; status: Record<string, unknown> }>(response, "CredentialVaultUnlocked")
      return { ok: true, message: `vault ${payload.action}`, data: payload, format: "json" }
    }
    if (vaultAction === "manage") {
      if (!context.sessionId) {
        return { ok: false, message: "credential vault manage requires an attached session" }
      }
      const response = await deps.client.send(manageCredentialVaultRequest(context.sessionId, context.agentId))
      const payload = expectVariant<{ action: string; status: Record<string, unknown> }>(response, "CredentialVaultManaged")
      return { ok: true, message: `vault ${payload.action}`, data: payload, format: "json" }
    }
    return { ok: false, message: "usage: credential vault status|create|lock|unlock [once|30m|2h|kernel]|manage" }
  }
  if (!action || action === "list" || action === "ls") {
    const response = await deps.client.send(listCredentialsRequest())
    const credentials = expectVariant<{ credentials: Array<Record<string, unknown>> }>(response, "CredentialsListed").credentials
    if (credentials.length === 0) {
      return { ok: true, message: "no credential handles configured" }
    }
    return {
      ok: true,
      message: credentials
        .map((credential: Record<string, unknown>) => {
          const id = String(credential.id ?? "")
          const source = credential.source && typeof credential.source === "object"
            ? String((credential.source as Record<string, unknown>).type ?? "unknown")
            : "unknown"
          const uses = Array.isArray(credential.allowed_uses) ? credential.allowed_uses.join(",") : "any"
          return `${id}\t${source}\t${uses || "any"}`
        })
        .join("\n"),
      format: "table",
    }
  }
  if (action === "show") {
    if (!key || rest.length > 0) {
      return { ok: false, message: "usage: credential show <id>" }
    }
    const response = await deps.client.send(getCredentialRequest(key))
    const credential = expectVariant<{ credential: Record<string, unknown> }>(response, "Credential").credential
    return { ok: true, message: JSON.stringify(credential, null, 2), data: { credential }, format: "json" }
  }
  if (action === "register") {
    if (!key || rest.length > 0) {
      return { ok: false, message: "usage: credential register <file.yaml>" }
    }
    const response = await deps.client.send(registerCredentialRequest(key))
    const credential = expectVariant<{ credential: { id: string } }>(response, "CredentialRegistered").credential
    return { ok: true, message: `registered credential ${credential.id}`, data: { credential } }
  }
  if (action === "upsert-json") {
    const json = [key, ...rest].filter(Boolean).join(" ").trim()
    if (!json) {
      return { ok: false, message: "usage: credential upsert-json <credential-json>" }
    }
    let credential: Record<string, unknown>
    try {
      credential = JSON.parse(json) as Record<string, unknown>
    } catch (error) {
      return { ok: false, message: `invalid credential json: ${error instanceof Error ? error.message : String(error)}` }
    }
    const response = await deps.client.send(upsertCredentialRequest(credential))
    const payload = expectVariant<{ credential: { id: string } }>(response, "CredentialUpserted")
    return { ok: true, message: `upserted credential ${payload.credential.id}`, data: payload }
  }
  if (action === "set") {
    if (!key || rest.length > 0) {
      return { ok: false, message: "usage: credential set <key>" }
    }
    if (!deps.readSecret) {
      return {
        ok: false,
        message: "credential set requires hidden input support; run it from interactive chariox-shell",
      }
    }
    const value = await deps.readSecret(`credential ${key}: `)
    if (!value) {
      return { ok: false, message: "credential value must not be empty" }
    }
    await deps.client.send(setCredentialSecretRequest(key, value, credentialVaultContext(context)))
    return { ok: true, message: `credential ${key} stored in Chariox Vault` }
  }
  if (action === "delete" || action === "remove" || action === "rm") {
    if (!key || rest.length > 0) {
      return { ok: false, message: "usage: credential delete <key>" }
    }
    if (action === "remove") {
      const response = await deps.client.send(removeCredentialRequest(key))
      const credential = expectVariant<{ credential: { id: string } }>(response, "CredentialRemoved").credential
      return { ok: true, message: `removed credential ${credential.id}`, data: { credential } }
    }
    await deps.client.send(deleteCredentialSecretRequest(key, credentialVaultContext(context)))
    return { ok: true, message: `credential ${key} deleted from Chariox Vault` }
  }
  return { ok: false, message: "usage: credential list|show|register|upsert-json|remove|set|delete|vault" }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}

function credentialVaultContext(context: ShellContext) {
  return {
    ...(context.sessionId ? { sessionId: context.sessionId } : {}),
    ...(context.agentId ? { agentId: context.agentId } : {}),
  }
}

/** Status reads never start a secret prompt, including on a fresh kernel. */
export function credentialVaultStatusMessage(status: Record<string, unknown>): string {
  if (status.awaiting_passphrase === true) return "vault needs a passphrase; run /credential vault create"
  return status.unlocked === true ? "vault unlocked" : "vault locked"
}

export async function readNewVaultPassphrase(readSecret?: (prompt: string) => Promise<string>): Promise<
  { ok: true; passphrase: string } | { ok: false; message: string }
> {
  if (!readSecret) return { ok: false, message: "credential vault create requires hidden input support; run it from an interactive terminal" }
  const passphrase = await readSecret("New Vault passphrase: ")
  if (!passphrase) return { ok: false, message: "the Vault passphrase must not be empty" }
  if (passphrase !== await readSecret("Confirm Vault passphrase: ")) return { ok: false, message: "the Vault passphrases do not match; try again" }
  return { ok: true, passphrase }
}
