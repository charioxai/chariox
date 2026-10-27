import type {
  CharioxUserConfig,
  CharioxUserConfigPayload,
  CharioxUserConfigSchemaPayload,
  RuntimeSession,
  UserConfigMutationEffect,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  type CredentialVaultRequestContext,
  getCredentialVaultStatusRequest,
  getUserConfigRequest,
  getUserConfigSchemaRequest,
  lockCredentialVaultRequest,
  manageCredentialVaultRequest,
  setWorkspaceLiveSyncModeRequest,
  setUserConfigValueRequest,
  setCredentialSecretRequest,
  unsetUserConfigValueRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export async function getUserConfig(client: LocalIpcClient): Promise<CharioxUserConfigPayload> {
  const response = await client.send<Record<string, unknown>>(getUserConfigRequest())
  return expectVariant<{ path: string, config: CharioxUserConfig }>(response, "UserConfig")
}

export async function getUserConfigSchema(client: LocalIpcClient): Promise<CharioxUserConfigSchemaPayload> {
  const response = await client.send<Record<string, unknown>>(getUserConfigSchemaRequest())
  return expectVariant<CharioxUserConfigSchemaPayload>(response, "UserConfigSchema")
}

export async function setUserConfigValue(
  client: LocalIpcClient,
  path: string,
  value: string,
): Promise<CharioxUserConfigPayload> {
  const response = await client.send<Record<string, unknown>>(setUserConfigValueRequest(path, value))
  return expectVariant<CharioxUserConfigPayload>(response, "UserConfigUpdated")
}

export async function setWorkspaceLiveSyncMode(
  client: LocalIpcClient,
  sessionId: string,
  mode: "managed" | "tracked" | "unrestricted",
): Promise<{ session: RuntimeSession, effects?: UserConfigMutationEffect[] }> {
  const response = await client.send<Record<string, unknown>>(setWorkspaceLiveSyncModeRequest(sessionId, mode))
  return expectVariant<{ session: RuntimeSession, effects?: UserConfigMutationEffect[] }>(response, "WorkspaceLiveSyncModeUpdated")
}

export async function unsetUserConfigValue(
  client: LocalIpcClient,
  path: string,
): Promise<CharioxUserConfigPayload> {
  const response = await client.send<Record<string, unknown>>(unsetUserConfigValueRequest(path))
  return expectVariant<CharioxUserConfigPayload>(response, "UserConfigUpdated")
}

export async function setCredentialSecret(
  client: LocalIpcClient,
  key: string,
  value: string,
  context: CredentialVaultRequestContext = {},
): Promise<string> {
  const response = await client.send<Record<string, unknown>>(setCredentialSecretRequest(key, value, context))
  return expectVariant<{ key: string }>(response, "CredentialSecretStored").key
}

export async function getCredentialVaultStatus(
  client: LocalIpcClient,
): Promise<Record<string, unknown>> {
  const response = await client.send<Record<string, unknown>>(getCredentialVaultStatusRequest())
  return expectVariant<{ status: Record<string, unknown> }>(response, "CredentialVaultStatus").status
}

export async function lockCredentialVault(
  client: LocalIpcClient,
): Promise<Record<string, unknown>> {
  const response = await client.send<Record<string, unknown>>(lockCredentialVaultRequest())
  return expectVariant<{ status: Record<string, unknown> }>(response, "CredentialVaultLocked").status
}

export async function manageCredentialVault(
  client: LocalIpcClient,
  sessionId: string,
  agentId?: string | null,
): Promise<{ action: string; status: Record<string, unknown> }> {
  const response = await client.send<Record<string, unknown>>(manageCredentialVaultRequest(sessionId, agentId))
  return expectVariant<{ action: string; status: Record<string, unknown> }>(response, "CredentialVaultManaged")
}
