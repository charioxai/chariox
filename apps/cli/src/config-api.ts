import type {
  ArrobaUserConfig,
  ArrobaUserConfigPayload,
  ArrobaUserConfigSchemaPayload,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  getUserConfigRequest,
  getUserConfigSchemaRequest,
  setUserConfigValueRequest,
  setCredentialSecretRequest,
  unsetUserConfigValueRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export async function getUserConfig(client: LocalIpcClient): Promise<ArrobaUserConfigPayload> {
  const response = await client.send<Record<string, unknown>>(getUserConfigRequest())
  return expectVariant<{ path: string, config: ArrobaUserConfig }>(response, "UserConfig")
}

export async function getUserConfigSchema(client: LocalIpcClient): Promise<ArrobaUserConfigSchemaPayload> {
  const response = await client.send<Record<string, unknown>>(getUserConfigSchemaRequest())
  return expectVariant<ArrobaUserConfigSchemaPayload>(response, "UserConfigSchema")
}

export async function setUserConfigValue(
  client: LocalIpcClient,
  path: string,
  value: string,
): Promise<ArrobaUserConfigPayload> {
  const response = await client.send<Record<string, unknown>>(setUserConfigValueRequest(path, value))
  return expectVariant<ArrobaUserConfigPayload>(response, "UserConfigUpdated")
}

export async function unsetUserConfigValue(
  client: LocalIpcClient,
  path: string,
): Promise<ArrobaUserConfigPayload> {
  const response = await client.send<Record<string, unknown>>(unsetUserConfigValueRequest(path))
  return expectVariant<ArrobaUserConfigPayload>(response, "UserConfigUpdated")
}

export async function setCredentialSecret(
  client: LocalIpcClient,
  key: string,
  value: string,
): Promise<string> {
  const response = await client.send<Record<string, unknown>>(setCredentialSecretRequest(key, value))
  return expectVariant<{ key: string }>(response, "CredentialSecretStored").key
}
