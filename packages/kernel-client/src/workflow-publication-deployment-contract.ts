export const WORKFLOW_PUBLICATION_PACKAGE_VERSION = 3
export const WORKFLOW_PUBLICATION_DEPLOYMENT_CONTRACT_VERSION = 1

export interface WorkflowPublicationDeploymentNetworkDestination {
  readonly id: string
  readonly host: { readonly kind: "exact_dns"; readonly value: string }
  readonly ports: readonly [443]
  readonly protocols: readonly ["tls"]
  readonly credential_slot_ids: readonly string[]
}

export interface WorkflowPublicationDeploymentProviderAccess {
  readonly slot_id: string
  readonly bundle_kind: "platform_managed" | "development_stub" | "unsupported"
  readonly bundle_id: string
}

export type WorkflowPublicationDeploymentNetworkPolicy = {
  readonly kind: "enforced"
  readonly policy_version: 1
  readonly default_action: "deny"
  readonly destinations: readonly WorkflowPublicationDeploymentNetworkDestination[]
  readonly provider_access: readonly WorkflowPublicationDeploymentProviderAccess[]
}

export interface WorkflowPublicationPackageContractMetadata {
  readonly package_version?: number
  readonly publication_id?: string
  readonly source_session_id?: string
  readonly workflow_id?: string
  readonly source_workflow_revision?: number | null
  readonly source_snapshot_digest?: string | null
  readonly deployment_contract?: {
    readonly path?: string
    readonly schema_version?: number
  }
}

export interface WorkflowPublicationDeploymentContract {
  readonly schema_version: 1
  readonly package_id: string
  readonly artifact: {
    readonly content_digest: string
    readonly digest_algorithm: "sha256"
    readonly digest_scope: "package_files_excluding_deployment_contract"
  }
  readonly source: {
    readonly publication_id: string
    readonly session_id: string
    readonly workflow_id: string
    readonly endpoint_id: string
    readonly creator_user_id: string
    readonly captured_at_ms: number | null
    readonly workflow_revision?: number | null
    readonly snapshot_digest?: string | null
  }
  readonly compatibility: {
    readonly package_version: 3
    readonly minimum_kernel_version: string
    readonly minimum_local_daemon_protocol_version: number
  }
  readonly routes: readonly Record<string, unknown>[]
  readonly provider_requirements: readonly Record<string, unknown>[]
  readonly credential_slots: readonly Record<string, unknown>[]
  readonly configuration: readonly Record<string, unknown>[]
  readonly capabilities: Record<string, unknown>
  readonly resources: Record<string, unknown>
  readonly presentation: Record<string, unknown>
  readonly signatures: readonly Record<string, unknown>[]
}

export type WorkflowPublicationDeploymentContractResolution = {
  readonly kind: "native"
  readonly packageVersion: 3
  readonly contract: WorkflowPublicationDeploymentContract
}

export interface WorkflowPublicationDeploymentAdmissionContext {
  readonly targetLocalDaemonProtocolVersion: number
}

export function workflowPublicationDeploymentContractPath(
  publicationPackage: WorkflowPublicationPackageContractMetadata,
): string | null {
  const packageVersion = normalizedPackageVersion(publicationPackage.package_version)
  if (packageVersion !== WORKFLOW_PUBLICATION_PACKAGE_VERSION) {
    throw new Error(`unsupported publication package_version ${packageVersion}`)
  }
  const reference = publicationPackage.deployment_contract
  if (reference?.schema_version !== WORKFLOW_PUBLICATION_DEPLOYMENT_CONTRACT_VERSION) {
    throw new Error("publication package v3 requires deployment_contract schema_version 1")
  }
  const path = reference.path?.trim()
  if (!path || path.startsWith("/") || path.includes("\\") || path.split("/").some((part) => part === ".." || part === "." || !part)) {
    throw new Error("publication package v3 requires a safe relative deployment_contract path")
  }
  return path
}

export function resolveWorkflowPublicationDeploymentContract(
  publicationPackage: WorkflowPublicationPackageContractMetadata,
  value: unknown,
): WorkflowPublicationDeploymentContractResolution {
  const packageVersion = normalizedPackageVersion(publicationPackage.package_version)
  const path = workflowPublicationDeploymentContractPath(publicationPackage)
  if (!path) throw new Error("publication package v3 is missing deployment_contract")
  const contract = validateWorkflowPublicationDeploymentContract(value)
  if (contract.compatibility.package_version !== packageVersion) {
    throw new Error("deployment contract package version does not match publication package")
  }
  if (publicationPackage.publication_id && contract.source.publication_id !== publicationPackage.publication_id) {
    throw new Error("deployment contract publication_id does not match publication package")
  }
  if (publicationPackage.source_session_id && contract.source.session_id !== publicationPackage.source_session_id) {
    throw new Error("deployment contract session_id does not match publication package")
  }
  if (publicationPackage.workflow_id && contract.source.workflow_id !== publicationPackage.workflow_id) {
    throw new Error("deployment contract workflow_id does not match publication package")
  }
  if (
    publicationPackage.source_workflow_revision != null &&
    contract.source.workflow_revision !== publicationPackage.source_workflow_revision
  ) {
    throw new Error("deployment contract workflow_revision does not match publication package")
  }
  if (
    publicationPackage.source_snapshot_digest &&
    contract.source.snapshot_digest !== publicationPackage.source_snapshot_digest
  ) {
    throw new Error("deployment contract snapshot_digest does not match publication package")
  }
  return { kind: "native", packageVersion, contract }
}

export function validateWorkflowPublicationDeploymentContract(
  value: unknown,
): WorkflowPublicationDeploymentContract {
  const contract = objectRecord(value, "deployment contract")
  if (contract.schema_version !== WORKFLOW_PUBLICATION_DEPLOYMENT_CONTRACT_VERSION) {
    throw new Error(`unsupported deployment contract schema_version ${String(contract.schema_version)}`)
  }
  requireSha256(contract.package_id, "deployment contract package_id")
  const artifact = objectRecord(contract.artifact, "deployment contract artifact")
  requireSha256(artifact.content_digest, "deployment contract artifact content_digest")
  if (artifact.digest_algorithm !== "sha256" || artifact.digest_scope !== "package_files_excluding_deployment_contract") {
    throw new Error("deployment contract artifact digest metadata is invalid")
  }
  const source = objectRecord(contract.source, "deployment contract source")
  for (const key of ["publication_id", "session_id", "workflow_id", "endpoint_id", "creator_user_id"] as const) {
    requireString(source[key], `deployment contract source ${key}`)
  }
  if (source.captured_at_ms !== null && (!Number.isInteger(source.captured_at_ms) || Number(source.captured_at_ms) < 0)) {
    throw new Error("deployment contract source captured_at_ms must be a non-negative integer or null")
  }
  if (
    source.workflow_revision !== undefined &&
    source.workflow_revision !== null &&
    (!Number.isInteger(source.workflow_revision) || Number(source.workflow_revision) < 0)
  ) {
    throw new Error("deployment contract source workflow_revision must be a non-negative integer or null")
  }
  if (source.snapshot_digest !== undefined && source.snapshot_digest !== null) {
    requireSha256(source.snapshot_digest, "deployment contract source snapshot_digest")
  }
  const compatibility = objectRecord(contract.compatibility, "deployment contract compatibility")
  if (compatibility.package_version !== WORKFLOW_PUBLICATION_PACKAGE_VERSION) {
    throw new Error("deployment contract compatibility package_version must be 3")
  }
  requireString(compatibility.minimum_kernel_version, "deployment contract minimum_kernel_version")
  if (!Number.isInteger(compatibility.minimum_local_daemon_protocol_version)
    || Number(compatibility.minimum_local_daemon_protocol_version) < 1) {
    throw new Error("deployment contract minimum_local_daemon_protocol_version must be a positive integer")
  }
  requireArray(contract.routes, "deployment contract routes", true)
  requireArray(contract.provider_requirements, "deployment contract provider_requirements")
  const slots = requireArray(contract.credential_slots, "deployment contract credential_slots")
  const slotIds = slots.map((slot, index) => requireString(
    objectRecord(slot, `deployment contract credential_slots[${index}]`).slot_id,
    `deployment contract credential_slots[${index}].slot_id`,
  ))
  if (new Set(slotIds).size !== slotIds.length) {
    throw new Error("deployment contract credential slot IDs must be unique")
  }
  requireArray(contract.configuration, "deployment contract configuration")
  const capabilities = objectRecord(contract.capabilities, "deployment contract capabilities")
  const networkPolicy = validateDeploymentNetworkPolicy(capabilities.network)
  validateDeploymentNetworkBindings(networkPolicy, slots, contract.provider_requirements)
  objectRecord(contract.resources, "deployment contract resources")
  objectRecord(contract.presentation, "deployment contract presentation")
  requireArray(contract.signatures, "deployment contract signatures")
  assertNoSecretPayloadFields(contract)
  const validated = contract as unknown as WorkflowPublicationDeploymentContract
  validateWorkflowPublicationProviderPolicy(validated)
  return validated
}

export function workflowPublicationAllowedProviders(
  contract: WorkflowPublicationDeploymentContract,
  agentId: string,
  capturedProvider?: string,
): readonly string[] {
  const matches = contract.configuration.filter((candidate) => {
    const field = objectRecord(candidate, "deployment contract configuration field")
    return field.kind === "provider_profile" && field.agent_id === agentId
  })
  if (matches.length !== 1) {
    throw new Error(`deployment contract must declare exactly one provider profile for agent ${agentId}`)
  }
  const field = objectRecord(matches[0], `deployment contract provider profile ${agentId}`)
  const captured = objectRecord(field.captured, `deployment contract captured provider profile ${agentId}`)
  const capturedValue = requireString(captured.provider, `deployment contract captured provider ${agentId}`).trim()
  if (capturedProvider !== undefined && capturedValue !== capturedProvider.trim()) {
    throw new Error(`deployment contract captured provider does not match bindings for agent ${agentId}`)
  }
  const declared = field.allowed_providers === undefined
    ? [capturedValue]
    : requireArray(field.allowed_providers, `deployment contract allowed providers ${agentId}`)
      .map((provider) => requireString(provider, `deployment contract allowed provider ${agentId}`).trim())
  if (declared.length === 0 || new Set(declared).size !== declared.length || !declared.includes(capturedValue)) {
    throw new Error(`deployment contract allowed providers are invalid for agent ${agentId}`)
  }
  const packagedFamilies = new Set(contract.provider_requirements.map((candidate, index) => {
    const requirement = objectRecord(candidate, `deployment contract provider requirement ${index}`)
    return providerFamily(requireString(requirement.provider, `deployment contract provider requirement ${index}`).trim())
  }))
  if (declared.some((provider) => !packagedFamilies.has(providerFamily(provider)))) {
    throw new Error(`deployment contract allowed providers exceed packaged requirements for agent ${agentId}`)
  }
  return declared
}

function validateWorkflowPublicationProviderPolicy(contract: WorkflowPublicationDeploymentContract): void {
  const agentIds = contract.configuration.map((candidate, index) => {
    const field = objectRecord(candidate, `deployment contract configuration field ${index}`)
    if (field.kind !== "provider_profile") {
      throw new Error(`deployment contract configuration field ${index} is unsupported`)
    }
    return requireString(field.agent_id, `deployment contract configuration agent ${index}`).trim()
  })
  if (new Set(agentIds).size !== agentIds.length) {
    throw new Error("deployment contract provider profile agents must be unique")
  }
  for (const agentId of agentIds) workflowPublicationAllowedProviders(contract, agentId)
}

function providerFamily(provider: string): string {
  const value = provider.trim().toLowerCase()
  if (value === "default") return "opencode"
  if (value === "claude-headless" || value === "claude-p") return "claude"
  return value
}

export function workflowPublicationDeploymentNetworkPolicy(
  contract: WorkflowPublicationDeploymentContract,
): WorkflowPublicationDeploymentNetworkPolicy {
  return validateDeploymentNetworkPolicy(objectRecord(contract.capabilities, "deployment contract capabilities").network)
}

export function assertWorkflowPublicationDeploymentRuntimeCompatibility(
  contract: WorkflowPublicationDeploymentContract,
  admissionContext: WorkflowPublicationDeploymentAdmissionContext,
): void {
  const targetLocalDaemonProtocolVersion = admissionContext?.targetLocalDaemonProtocolVersion
  if (!Number.isInteger(targetLocalDaemonProtocolVersion) || Number(targetLocalDaemonProtocolVersion) < 1) {
    throw new Error("deployment contract admission requires a positive target local daemon protocol version")
  }
  const minimumLocalDaemonProtocolVersion = contract.compatibility.minimum_local_daemon_protocol_version
  if (minimumLocalDaemonProtocolVersion > Number(targetLocalDaemonProtocolVersion)) {
    throw new Error(
      `deployment contract requires local daemon protocol version ${minimumLocalDaemonProtocolVersion}, but target runtime supports ${targetLocalDaemonProtocolVersion}`,
    )
  }
}

function normalizedPackageVersion(value: unknown): number {
  if (!Number.isInteger(value)) {
    throw new Error(`publication package_version must be ${WORKFLOW_PUBLICATION_PACKAGE_VERSION}`)
  }
  const packageVersion = Number(value)
  if (packageVersion !== WORKFLOW_PUBLICATION_PACKAGE_VERSION) {
    throw new Error(`unsupported publication package_version ${packageVersion}`)
  }
  return packageVersion
}

function objectRecord(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be an object`)
  }
  return value as Record<string, unknown>
}

function requireArray(value: unknown, label: string, nonEmpty = false): unknown[] {
  if (!Array.isArray(value) || (nonEmpty && value.length === 0)) {
    throw new Error(`${label} must be ${nonEmpty ? "a non-empty" : "an"} array`)
  }
  return value
}

function requireString(value: unknown, label: string): string {
  if (typeof value !== "string" || !value.trim()) throw new Error(`${label} must be a non-empty string`)
  return value
}

function requireSha256(value: unknown, label: string): void {
  if (typeof value !== "string" || !/^sha256:[a-f0-9]{64}$/.test(value)) {
    throw new Error(`${label} must be a sha256 digest`)
  }
}

function validateDeploymentNetworkPolicy(value: unknown): WorkflowPublicationDeploymentNetworkPolicy {
  const policy = objectRecord(value, "deployment contract network policy")
  requireExactKeys(
    policy,
    ["policy_version", "default_action", "destinations", "provider_access"],
    "deployment contract network policy",
  )
  if (policy.policy_version !== 1 || policy.default_action !== "deny") {
    throw new Error("deployment contract network policy must use version 1 and deny by default")
  }
  const destinations = requireArray(policy.destinations, "deployment contract network destinations")
    .map((candidate, index) => validateNetworkDestination(candidate, index))
  if (destinations.length > 256) throw new Error("deployment contract network destinations exceed 256 entries")
  const destinationIds = destinations.map((destination) => destination.id)
  const authorities = destinations.map((destination) => `${destination.host.value}:443`)
  if (new Set(destinationIds).size !== destinationIds.length || new Set(authorities).size !== authorities.length) {
    throw new Error("deployment contract network destination IDs and authorities must be unique")
  }
  const providerAccess = requireArray(policy.provider_access, "deployment contract provider access")
    .map((candidate, index) => validateProviderAccess(candidate, index))
  if (new Set(providerAccess.map((access) => access.slot_id)).size !== providerAccess.length) {
    throw new Error("deployment contract provider access slot IDs must be unique")
  }
  return {
    kind: "enforced",
    policy_version: 1,
    default_action: "deny",
    destinations,
    provider_access: providerAccess,
  }
}

function validateNetworkDestination(value: unknown, index: number): WorkflowPublicationDeploymentNetworkDestination {
  const destination = objectRecord(value, `deployment contract network destinations[${index}]`)
  requireExactKeys(
    destination,
    ["id", "host", "ports", "protocols", "credential_slot_ids"],
    `deployment contract network destinations[${index}]`,
  )
  const id = requireString(destination.id, `deployment contract network destinations[${index}].id`)
  if (!/^[a-z][a-z0-9:_-]{0,127}$/.test(id)) throw new Error("deployment contract network destination id is invalid")
  const host = objectRecord(destination.host, `deployment contract network destinations[${index}].host`)
  requireExactKeys(host, ["kind", "value"], `deployment contract network destinations[${index}].host`)
  if (host.kind !== "exact_dns" || typeof host.value !== "string" || !isCanonicalDnsName(host.value)) {
    throw new Error("deployment contract network destination host must be an exact canonical DNS name")
  }
  if (!Array.isArray(destination.ports) || destination.ports.length !== 1 || destination.ports[0] !== 443) {
    throw new Error("deployment contract network destination must permit TLS port 443 only")
  }
  if (!Array.isArray(destination.protocols) || destination.protocols.length !== 1 || destination.protocols[0] !== "tls") {
    throw new Error("deployment contract network destination must use TLS only")
  }
  const credentialSlotIds = requireArray(
    destination.credential_slot_ids,
    `deployment contract network destinations[${index}].credential_slot_ids`,
  ).map((slotId, slotIndex) => requireCredentialSlotId(
    slotId,
    `deployment contract network destinations[${index}].credential_slot_ids[${slotIndex}]`,
  ))
  if (new Set(credentialSlotIds).size !== credentialSlotIds.length) {
    throw new Error("deployment contract network destination credential slot IDs must be unique")
  }
  return {
    id,
    host: { kind: "exact_dns", value: host.value },
    ports: [443],
    protocols: ["tls"],
    credential_slot_ids: credentialSlotIds,
  }
}

function validateProviderAccess(value: unknown, index: number): WorkflowPublicationDeploymentProviderAccess {
  const access = objectRecord(value, `deployment contract provider_access[${index}]`)
  requireExactKeys(access, ["slot_id", "bundle_kind", "bundle_id"], `deployment contract provider_access[${index}]`)
  const slotId = requireCredentialSlotId(access.slot_id, `deployment contract provider_access[${index}].slot_id`)
  if (!slotId.startsWith("provider:")) throw new Error("deployment contract provider access requires a provider slot")
  if (!new Set(["platform_managed", "development_stub", "unsupported"]).has(String(access.bundle_kind))) {
    throw new Error("deployment contract provider access bundle kind is invalid")
  }
  const bundleId = requireString(access.bundle_id, `deployment contract provider_access[${index}].bundle_id`)
  if (!/^[a-z0-9][a-z0-9-]*$/.test(bundleId)) throw new Error("deployment contract provider access bundle id is invalid")
  return {
    slot_id: slotId,
    bundle_kind: access.bundle_kind as WorkflowPublicationDeploymentProviderAccess["bundle_kind"],
    bundle_id: bundleId,
  }
}

function validateDeploymentNetworkBindings(
  policy: WorkflowPublicationDeploymentNetworkPolicy,
  slots: unknown[],
  providerRequirementsValue: unknown,
): void {
  const slotRecords = slots.map((slot, index) => objectRecord(slot, `deployment contract credential_slots[${index}]`))
  const slotIds = new Set(slotRecords.map((slot, index) => requireCredentialSlotId(
    slot.slot_id,
    `deployment contract credential_slots[${index}].slot_id`,
  )))
  const destinationIds = new Set(policy.destinations.map((destination) => destination.id))
  for (const destination of policy.destinations) {
    for (const slotId of destination.credential_slot_ids) {
      if (!slotIds.has(slotId)) throw new Error("deployment contract network destination references an unknown credential slot")
    }
  }
  for (const [index, slot] of slotRecords.entries()) {
    if (slot.allowed_destination_ids === undefined) continue
    const allowed = requireArray(
      slot.allowed_destination_ids,
      `deployment contract credential_slots[${index}].allowed_destination_ids`,
    ).map((id, destinationIndex) => requireString(
      id,
      `deployment contract credential_slots[${index}].allowed_destination_ids[${destinationIndex}]`,
    ))
    if (new Set(allowed).size !== allowed.length || allowed.some((id) => !destinationIds.has(id))) {
      throw new Error("deployment contract credential slot allowed destination IDs are invalid")
    }
    const slotId = String(slot.slot_id)
    const expected = policy.destinations
      .filter((destination) => destination.credential_slot_ids.includes(slotId))
      .map((destination) => destination.id)
    if (allowed.length !== expected.length || allowed.some((id, allowedIndex) => id !== expected[allowedIndex])) {
      throw new Error("deployment contract credential slot destination ceiling is inconsistent")
    }
  }
  const providerRequirementSlots = new Set(requireArray(
    providerRequirementsValue,
    "deployment contract provider_requirements",
  ).map((requirement, index) => requireCredentialSlotId(
    objectRecord(requirement, `deployment contract provider_requirements[${index}]`).slot_id,
    `deployment contract provider_requirements[${index}].slot_id`,
  )))
  for (const access of policy.provider_access) {
    if (!slotIds.has(access.slot_id) || !providerRequirementSlots.has(access.slot_id)) {
      throw new Error("deployment contract provider access references an unknown provider slot")
    }
  }
}

function requireCredentialSlotId(value: unknown, label: string): string {
  const slotId = requireString(value, label)
  if (!/^(provider|integration):[a-z0-9-]+$/.test(slotId)) throw new Error(`${label} is invalid`)
  return slotId
}

function isCanonicalDnsName(value: string): boolean {
  if (value.length > 253 || value !== value.toLowerCase() || value.endsWith(".") || value.includes("*") || /^\d+(?:\.\d+){3}$/.test(value)) {
    return false
  }
  const labels = value.split(".")
  return labels.length >= 2 && labels.every((label) => /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/.test(label))
}

function requireExactKeys(record: Record<string, unknown>, keys: readonly string[], label: string): void {
  const expected = new Set(keys)
  if (Object.keys(record).some((key) => !expected.has(key)) || keys.some((key) => !(key in record))) {
    throw new Error(`${label} fields are invalid`)
  }
}

function assertNoSecretPayloadFields(value: unknown, path = "deployment contract"): void {
  if (Array.isArray(value)) {
    value.forEach((item, index) => assertNoSecretPayloadFields(item, `${path}[${index}]`))
    return
  }
  if (!value || typeof value !== "object") return
  const forbidden = new Set(["authorization", "account_profile", "credential_payload", "password", "secret", "token"])
  for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
    if (forbidden.has(key.toLowerCase())) {
      throw new Error(`${path} contains forbidden secret payload field ${key}`)
    }
    assertNoSecretPayloadFields(child, `${path}.${key}`)
  }
}
