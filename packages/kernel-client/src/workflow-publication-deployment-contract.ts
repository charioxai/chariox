export const WORKFLOW_PUBLICATION_PACKAGE_VERSION = 3
export const WORKFLOW_PUBLICATION_DEPLOYMENT_CONTRACT_VERSION = 1

export interface WorkflowPublicationPackageContractMetadata {
  readonly package_version?: number
  readonly publication_id?: string
  readonly source_session_id?: string
  readonly workflow_id?: string
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

export type WorkflowPublicationDeploymentContractResolution =
  | {
    readonly kind: "native"
    readonly packageVersion: 3
    readonly contract: WorkflowPublicationDeploymentContract
  }
  | {
    readonly kind: "legacy_adapter"
    readonly packageVersion: 1 | 2
    readonly contract: null
  }

export function workflowPublicationDeploymentContractPath(
  publicationPackage: WorkflowPublicationPackageContractMetadata,
): string | null {
  const packageVersion = normalizedPackageVersion(publicationPackage.package_version)
  if (packageVersion <= 2) return null
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
  value?: unknown,
): WorkflowPublicationDeploymentContractResolution {
  const packageVersion = normalizedPackageVersion(publicationPackage.package_version)
  const path = workflowPublicationDeploymentContractPath(publicationPackage)
  if (packageVersion <= 2) {
    return { kind: "legacy_adapter", packageVersion: packageVersion === 2 ? 2 : 1, contract: null }
  }
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
  objectRecord(contract.capabilities, "deployment contract capabilities")
  objectRecord(contract.resources, "deployment contract resources")
  objectRecord(contract.presentation, "deployment contract presentation")
  requireArray(contract.signatures, "deployment contract signatures")
  assertNoSecretPayloadFields(contract)
  return contract as unknown as WorkflowPublicationDeploymentContract
}

function normalizedPackageVersion(value: unknown): number {
  if (value === undefined || value === null) return 1
  if (!Number.isInteger(value) || Number(value) < 1) {
    throw new Error("publication package_version must be a positive integer")
  }
  return Number(value)
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
