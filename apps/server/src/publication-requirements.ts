import {
  listConnectorsRequest,
  listCredentialsRequest,
  listMcpServersRequest,
  listScriptsRequest,
  listSkillsRequest,
} from "@arroba/kernel-client/ipc-requests"

import type {
  KernelLookupClient,
  WorkflowPublicationRequirements,
} from "./publication-types.js"

export async function validatePublicationRequirements(
  requirements: WorkflowPublicationRequirements,
  client: KernelLookupClient,
  workspaceId?: string,
) {
  if (requirements.schema_version !== 1) {
    throw new Error(`unsupported publication requirements schema_version ${requirements.schema_version}`)
  }
  const missing = [
    ...await missingNamedRequirements("mcp", requirements.mcps, () => listKernelNames(client, listMcpServersRequest(workspaceId), "McpServersListed", "mcps")),
    ...await missingNamedRequirements("skill", requirements.skills, () => listKernelNames(client, listSkillsRequest(workspaceId), "SkillsListed", "skills")),
    ...await missingNamedRequirements("script", requirements.scripts, () => listKernelNames(client, listScriptsRequest(workspaceId), "ScriptsListed", "scripts")),
    ...await missingNamedRequirements("connector", requirements.connectors, () => listKernelNames(client, listConnectorsRequest(), "ConnectorsListed", "connectors")),
    ...await missingNamedRequirements("credential", requirements.credentials, () => listKernelNames(client, listCredentialsRequest(), "CredentialsListed", "credentials")),
  ]
  if (missing.length > 0) {
    throw new Error(`publication requirements are missing: ${missing.join(", ")}`)
  }
}

async function missingNamedRequirements(
  kind: string,
  requirements: Array<{ name?: unknown }> | undefined,
  loadAvailable: () => Promise<Set<string>>,
) {
  const required = uniqueNames(requirements)
  if (required.length === 0) return []
  const available = await loadAvailable()
  return required
    .filter((name) => !available.has(name))
    .map((name) => `${kind}:${name}`)
}

async function listKernelNames(
  client: KernelLookupClient,
  request: Record<string, unknown>,
  variant: string,
  field: string,
) {
  const response = await client.send(request)
  const payload = response[variant] as Record<string, unknown> | undefined
  const items = Array.isArray(payload?.[field]) ? payload[field] as unknown[] : []
  return new Set(items.flatMap((item) => itemNames(item)))
}

function itemNames(item: unknown) {
  if (!item || typeof item !== "object" || Array.isArray(item)) return []
  const record = item as Record<string, unknown>
  return [record.name, record.id]
    .filter((value): value is string => typeof value === "string" && value.trim().length > 0)
    .map((value) => value.trim())
}

function uniqueNames(requirements: Array<{ name?: unknown }> | undefined) {
  const seen = new Set<string>()
  const names: string[] = []
  for (const requirement of requirements ?? []) {
    const name = typeof requirement.name === "string" ? requirement.name.trim() : ""
    if (!name || seen.has(name)) continue
    seen.add(name)
    names.push(name)
  }
  return names
}
