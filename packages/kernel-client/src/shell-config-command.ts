import type {
  ArrobaUserConfigPayload,
  ArrobaUserConfigSchemaPayload,
} from "./kernel-types.js"
import {
  deleteCredentialSecretRequest,
  getCredentialRequest,
  getUserConfigRequest,
  getUserConfigSchemaRequest,
  listCredentialsRequest,
  registerCredentialRequest,
  removeCredentialRequest,
  setCredentialSecretRequest,
  setUserConfigValueRequest,
  unsetUserConfigValueRequest,
  upsertCredentialRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult } from "./shell-core.js"
import {
  configMutationMessage,
  formatConfigSchemaKeys,
} from "./shell-config-format.js"

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
  if (!action || action === "show") {
    const response = await deps.client.send(getUserConfigRequest())
    const payload = expectVariant<ArrobaUserConfigPayload>(response, "UserConfig")
    return { ok: true, message: JSON.stringify(payload.config, null, 2), data: payload, format: "json" }
  }
  if (action === "path") {
    const response = await deps.client.send(getUserConfigRequest())
    const payload = expectVariant<ArrobaUserConfigPayload>(response, "UserConfig")
    return { ok: true, message: payload.path, data: payload }
  }
  if (action === "keys" || action === "list") {
    const response = await deps.client.send(getUserConfigSchemaRequest())
    const payload = expectVariant<ArrobaUserConfigSchemaPayload>(response, "UserConfigSchema")
    return { ok: true, message: formatConfigSchemaKeys(payload.entries), data: payload }
  }
  if (action === "schema") {
    const response = await deps.client.send(getUserConfigSchemaRequest())
    const payload = expectVariant<ArrobaUserConfigSchemaPayload>(response, "UserConfigSchema")
    return { ok: true, message: JSON.stringify(payload.entries, null, 2), data: payload, format: "json" }
  }
  if (action === "set") {
    const value = rest.join(" ").trim()
    if (!keyPath || !value) {
      return { ok: false, message: "usage: config set <path> <value>" }
    }
    const response = await deps.client.send(setUserConfigValueRequest(keyPath, value))
    const payload = expectVariant<ArrobaUserConfigPayload>(response, "UserConfigUpdated")
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
    const payload = expectVariant<ArrobaUserConfigPayload>(response, "UserConfigUpdated")
    return {
      ok: true,
      message: configMutationMessage(`config ${keyPath} unset`, payload),
      data: payload,
    }
  }
  if (action === "workspace-live-sync") {
    const mode = normalizeWorkspaceLiveSyncPolicy(keyPath ?? "off")
    if (rest.length > 0 || !mode) {
      return { ok: false, message: "usage: config workspace-live-sync off|managed|tracked" }
    }
    const response = await deps.client.send(setUserConfigValueRequest("providers.workspace_live_sync", mode))
    const payload = expectVariant<ArrobaUserConfigPayload>(response, "UserConfigUpdated")
    return {
      ok: true,
      message: configMutationMessage(`workspace live sync set to ${mode}`, payload),
      data: payload,
    }
  }
  return { ok: false, message: "usage: config show|path|keys|schema|set|unset|workspace-live-sync" }
}

function normalizeWorkspaceLiveSyncPolicy(value: string): "off" | "managed" | "tracked" | null {
  if (value === "off" || value === "managed" || value === "tracked") return value
  return null
}

export async function executeCredentialCommand(
  parsed: ParsedShellCommand,
  deps: ShellConfigCommandDeps,
): Promise<ShellCommandResult> {
  const [action, key, ...rest] = parsed.args
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
        message: "credential set requires hidden input support; run it from interactive arroba-shell",
      }
    }
    const value = await deps.readSecret(`credential ${key}: `)
    if (!value) {
      return { ok: false, message: "credential value must not be empty" }
    }
    await deps.client.send(setCredentialSecretRequest(key, value))
    return { ok: true, message: `credential ${key} stored in OS keychain` }
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
    await deps.client.send(deleteCredentialSecretRequest(key))
    return { ok: true, message: `credential ${key} deleted from OS keychain` }
  }
  return { ok: false, message: "usage: credential list|show|register|upsert-json|remove|set|delete" }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
