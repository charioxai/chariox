import { isDeepStrictEqual } from "node:util"

import {
  listConnectorsRequest,
  listCredentialsRequest,
  listMcpServersRequest,
  listScriptsRequest,
  listSkillsRequest,
} from "@chariox/kernel-client/ipc-requests"
import {
  validateWorkflowPublicationDeploymentExtensions,
  type WorkflowPublicationDeploymentExtensionRequirement,
} from "@chariox/kernel-client/workflow-publication-deployment-contract"

import type {
  KernelLookupClient,
  WorkflowPublicationRequirements,
} from "./publication-types.js"

export async function validatePublicationRequirements(
  requirements: WorkflowPublicationRequirements,
  client: KernelLookupClient,
  workspaceId?: string,
) {
  if (requirements.schema_version === 1) {
    return validateLegacyRequirements(requirements, client, workspaceId)
  }
  if (requirements.schema_version === 2) {
    return validateExactExtensionRequirements(requirements, client, workspaceId)
  }
  throw new Error(`unsupported publication requirements schema_version ${(requirements as { schema_version: unknown }).schema_version}`)
}

async function validateLegacyRequirements(
  requirements: Extract<WorkflowPublicationRequirements, { schema_version: 1 }>,
  client: KernelLookupClient,
  workspaceId?: string,
) {
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

async function validateExactExtensionRequirements(
  requirements: Extract<WorkflowPublicationRequirements, { schema_version: 2 }>,
  client: KernelLookupClient,
  workspaceId?: string,
) {
  const record = objectRecord(requirements, "publication requirements")
  requireExactKeys(
    record,
    ["schema_version", "extensions", "credential_slots", "network_destinations"],
    "publication requirements",
  )
  const extensions = validateWorkflowPublicationDeploymentExtensions(
    requireArray(record.extensions, "publication requirements extensions"),
  )
  const credentialSlots = requireArray(record.credential_slots, "publication requirements credential_slots")
  const expectedCredentialSlots = extensions
    .flatMap((extension) => extension.credential_slots)
    .sort(compareCredentialSlotIds)
  if (!isDeepStrictEqual([...credentialSlots].sort(compareCredentialSlotIds), expectedCredentialSlots)) {
    throw new Error("publication requirements credential_slots do not match the exact extension requirements")
  }
  const networkDestinations = requireArray(record.network_destinations, "publication requirements network_destinations")
  const expectedNetworkDestinations = extensions
    .flatMap((extension) => extension.network_destinations)
    .sort(compareRequirementIds)
  if (!isDeepStrictEqual([...networkDestinations].sort(compareRequirementIds), expectedNetworkDestinations)) {
    throw new Error("publication requirements network_destinations do not match the exact extension requirements")
  }

  const missing = [
    ...await missingExactExtensionKind("mcp", extensions, () => listKernelNames(client, listMcpServersRequest(workspaceId), "McpServersListed", "mcps")),
    ...await missingExactExtensionKind("skill", extensions, () => listKernelNames(client, listSkillsRequest(workspaceId), "SkillsListed", "skills")),
    ...await missingExactExtensionKind("script", extensions, () => listKernelNames(client, listScriptsRequest(workspaceId), "ScriptsListed", "scripts")),
    ...await missingExactExtensionKind("connector", extensions, () => listKernelNames(client, listConnectorsRequest(), "ConnectorsListed", "connectors")),
  ]
  if (missing.length > 0) {
    throw new Error(`publication requirements are missing: ${missing.join(", ")}`)
  }
}

async function missingExactExtensionKind(
  kind: WorkflowPublicationDeploymentExtensionRequirement["kind"],
  requirements: readonly WorkflowPublicationDeploymentExtensionRequirement[],
  loadAvailable: () => Promise<Set<string>>,
) {
  return missingNamedRequirements(
    kind,
    requirements.filter((requirement) => requirement.kind === kind),
    loadAvailable,
  )
}

function objectRecord(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be an object`)
  }
  return value as Record<string, unknown>
}

function requireArray(value: unknown, label: string): unknown[] {
  if (!Array.isArray(value)) throw new Error(`${label} must be an array`)
  return value
}

function requireExactKeys(record: Record<string, unknown>, keys: readonly string[], label: string) {
  const expected = new Set(keys)
  if (Object.keys(record).some((key) => !expected.has(key)) || Object.keys(record).length !== expected.size) {
    throw new Error(`${label} fields are invalid`)
  }
}

function compareRequirementIds(left: unknown, right: unknown) {
  return requirementId(left).localeCompare(requirementId(right))
}

function compareCredentialSlotIds(left: unknown, right: unknown) {
  return credentialSlotId(left).localeCompare(credentialSlotId(right))
}

function requirementId(value: unknown) {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return ""
  const id = (value as Record<string, unknown>).id
  return typeof id === "string" ? id : ""
}

function credentialSlotId(value: unknown) {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return ""
  const slotId = (value as Record<string, unknown>).slot_id
  return typeof slotId === "string" ? slotId : ""
}

async function missingNamedRequirements(
  kind: string,
  requirements: readonly { name?: unknown }[] | undefined,
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

function uniqueNames(requirements: readonly { name?: unknown }[] | undefined) {
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
