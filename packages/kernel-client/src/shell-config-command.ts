import type {
  ArrobaUserConfigPayload,
  ArrobaUserConfigSchemaPayload,
} from "./kernel-types.js"
import {
  deleteCredentialSecretRequest,
  getUserConfigRequest,
  getUserConfigSchemaRequest,
  setCredentialSecretRequest,
  setUserConfigValueRequest,
  unsetUserConfigValueRequest,
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
  if (action === "managed-io") {
    const mode = keyPath ?? "required"
    if (rest.length > 0 || !["required", "unrestricted", "on", "off"].includes(mode)) {
      return { ok: false, message: "usage: config managed-io required|unrestricted|on|off" }
    }
    const normalizedMode = mode === "on" ? "required" : mode === "off" ? "unrestricted" : mode
    const configPath = "providers.managed_io"
    const response = await deps.client.send(setUserConfigValueRequest(configPath, normalizedMode))
    const payload = expectVariant<ArrobaUserConfigPayload>(response, "UserConfigUpdated")
    return {
      ok: true,
      message: configMutationMessage(`managed I/O set to ${normalizedMode}`, payload),
      data: payload,
    }
  }
  return { ok: false, message: "usage: config show|path|keys|schema|set|unset|managed-io" }
}

export async function executeCredentialCommand(
  parsed: ParsedShellCommand,
  deps: ShellConfigCommandDeps,
): Promise<ShellCommandResult> {
  const [action, key, ...rest] = parsed.args
  if (!action || action === "list" || action === "ls") {
    const response = await deps.client.send(getUserConfigRequest())
    const payload = expectVariant<ArrobaUserConfigPayload>(response, "UserConfig")
    const credentials = Array.isArray(payload.config.credentials) ? payload.config.credentials : []
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
    await deps.client.send(deleteCredentialSecretRequest(key))
    return { ok: true, message: `credential ${key} deleted from OS keychain` }
  }
  return { ok: false, message: "usage: credential list|set|delete" }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
