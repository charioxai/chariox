// MP-08: Protocol shapes exist before behaviour; only Get is delivered in P01.
import type * as Environment from "./kernel-types-project-environment-aggregate.js"
import type { RuntimeProject } from "./kernel-types-session.js"

export type EnvironmentRequestPayloads = {
  GetProjectEnvironment: { readonly projectId: string }
  DetectProjectEnvironment: { readonly projectId: string; readonly operationId: string; readonly folderIds: readonly string[]; readonly target: Environment.EnvironmentTargetBinding; readonly provider: Environment.EnvironmentProvider | null; readonly allowModelFolders: readonly string[] }
  PreviewEnvironmentDiff: { readonly projectId: string; readonly expectedRevision: number; readonly draft: Environment.EnvironmentRevisionDraft }
  SaveProjectEnvironmentRevision: { readonly projectId: string; readonly expectedRevision: number; readonly expectedContentDigest: string; readonly draft: Environment.EnvironmentRevisionDraft; readonly acceptedProposalIds: readonly string[]; readonly excludedProposalIds: readonly string[] }
  PlanProjectEnvironment: { readonly projectId: string; readonly expectedRevision: number; readonly revisionDigest: string; readonly target: Environment.EnvironmentTargetBinding; readonly selectedItems: readonly string[] }
  ApplyProjectEnvironment: { readonly projectId: string; readonly operationId: string; readonly planId: string; readonly expectedRevision: number; readonly revisionDigest: string; readonly target: Environment.EnvironmentTargetBinding; readonly selectedItems: readonly string[]; readonly perItemOptIns: readonly Environment.EnvironmentItemOptIn[] }
  CheckProjectEnvironment: { readonly projectId: string; readonly operationId: string; readonly revisionDigest: string; readonly target: Environment.EnvironmentTargetBinding; readonly selectedItems: readonly string[] }
  GetEnvironmentOperation: { readonly projectId: string; readonly operationId: string }
  CancelEnvironmentOperation: { readonly projectId: string; readonly operationId: string }
  RetryEnvironmentOperation: { readonly projectId: string; readonly operationId: string; readonly expectedAttempt: number }
  ExportProjectEnvironment: { readonly projectId: string; readonly operationId: string; readonly expectedRevision: number; readonly revisionDigest: string; readonly selectedItems: readonly string[]; readonly selectedFiles: readonly Environment.EnvironmentFile[]; readonly destination: { readonly kind: "file"; readonly path: string } | { readonly kind: "kernel"; readonly kernel_id: string; readonly target_key_thumbprint: string } }
  PreviewEnvironmentImport: { readonly artifactId: string; readonly artifactDigest: string; readonly choice: Environment.EnvironmentImportChoice; readonly folderMap: readonly Environment.EnvironmentFolderBinding[] }
  CommitEnvironmentImport: { readonly operationId: string; readonly previewId: string; readonly previewDigest: string; readonly choice: Environment.EnvironmentImportChoice }
}
export type EnvironmentProtocolResponse =
  | { readonly ProjectEnvironment: { readonly environment: Environment.ProjectEnvironment } }
  | { readonly EnvironmentUnsupportedFeature: { readonly feature: Environment.EnvironmentCapability; readonly supported_schema: number } }
  | { readonly ProjectEnvironmentDiff: { readonly diff: Environment.EnvironmentRevisionDiff } }
  | { readonly ProjectEnvironmentSaved: { readonly environment: Environment.ProjectEnvironment; readonly diff: Environment.EnvironmentRevisionDiff } }
  | { readonly ProjectEnvironmentPlan: { readonly plan: Environment.EnvironmentApplyPlan } }
  | { readonly EnvironmentOperation: { readonly operation: Environment.EnvironmentOperation } }
  | { readonly ProjectEnvironmentExport: { readonly operation: Environment.EnvironmentOperation; readonly envelope: Environment.EnvironmentExportEnvelope | null; readonly artifact_id: string | null } }
  | { readonly EnvironmentImportPreview: { readonly preview: Environment.EnvironmentImportPreview } }
  | { readonly EnvironmentImportCommitted: { readonly project: RuntimeProject; readonly environment: Environment.ProjectEnvironment; readonly operation: Environment.EnvironmentOperation } }
  | { readonly EnvironmentProviderAuthorizationLink: { readonly link: Environment.EnvironmentProviderAuthorizationLink } }

export function projectEnvironmentOperationRequest<K extends keyof EnvironmentRequestPayloads>(kind: K, input: EnvironmentRequestPayloads[K]): { readonly [P in K]: EnvironmentRequestPayloads[P] } {
  return { [kind]: input } as { readonly [P in K]: EnvironmentRequestPayloads[P] }
}
const names: readonly (keyof EnvironmentRequestPayloads)[] = ["GetProjectEnvironment", "DetectProjectEnvironment", "PreviewEnvironmentDiff", "SaveProjectEnvironmentRevision", "PlanProjectEnvironment", "ApplyProjectEnvironment", "CheckProjectEnvironment", "GetEnvironmentOperation", "CancelEnvironmentOperation", "RetryEnvironmentOperation", "ExportProjectEnvironment", "PreviewEnvironmentImport", "CommitEnvironmentImport"]
export function isProjectEnvironmentRequest(request: unknown): boolean {
  return request !== null && typeof request === "object" && names.some(name => name in request)
}
export function requireProjectEnvironmentProtocol(request: unknown, version: number | null | undefined): void {
  if (isProjectEnvironmentRequest(request) && (version == null || !Number.isInteger(version) || version < 471)) throw new Error(`Project Environment requires protocol 471 or newer (this kernel: ${version ?? "unknown"}). Upgrade the kernel and reconnect.`)
}
