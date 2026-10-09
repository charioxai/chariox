// MP-08 / MP-10: Shared Web/TUI projection; no client-owned observations.
import type { ProjectEnvironment, Requirement, RequirementSpec, RequirementOrigin } from "./kernel-types-project-environment-aggregate.js"

export const projectEnvironmentMinimumProtocolVersion = 471
export function getProjectEnvironmentRequest(projectId: string) {
  return { GetProjectEnvironment: { projectId } } as const
}

const labels: Record<RequirementSpec["kind"], string> = {
  files: "Files", software: "Software", services: "Services", variables: "Variables", secrets: "Secrets",
  accounts: "Accounts", agent_tools: "Agent tools", chariox_apps: "Chariox Apps", setup_checks: "Setup & checks", machine_needs: "Machine needs",
}
export type EnvironmentViewRow = { readonly id: string; readonly title: string; readonly origins: readonly string[]; readonly details: readonly string[]; readonly status: string }
export type EnvironmentViewGroup = { readonly kind: string; readonly rows: readonly EnvironmentViewRow[] }
export type EnvironmentViewSection = { readonly id: string; readonly title: string; readonly groups: readonly EnvironmentViewGroup[] }

export function environmentOriginLabel(origin: RequirementOrigin): string {
  switch (origin.kind) {
    case "migrated": return `Migrated · ${origin.source}`
    case "detected": return `${origin.relative_path}${origin.line == null ? "" : `:${origin.line}`} · evidence ${origin.evidence_digest.slice(0, 8)}`
    case "detected_metadata": return `Proposal · ${origin.source}`
    case "user_added": return "Added by you"
  }
}
function details(requirement: Requirement): string[] {
  const entry = requirement.legacy_entry
  if (entry) {
    const locator = entry.locator
    const source = locator.kind === "vault" ? `Vault · ${locator.service}/${locator.key}`
      : locator.kind === "env_file" ? `${locator.path} · ${locator.key}`
      : locator.kind === "config_file" ? locator.path
      : locator.kind === "workspace_environment" ? `Workspace environment · ${locator.name}` : "No locator"
    return [source, ...(entry.excluded ? ["Excluded from setup"] : [])]
  }
  if (requirement.spec.kind === "setup_checks" && requirement.spec.legacy) {
    const setup = requirement.spec.legacy
    return [setup.source_path ?? setup.source, setup.target_platform, `${setup.setup_step_kinds.length} setup steps · ${setup.validation_command_digests.length} checks`, `Definition · ${setup.definition_digest}`]
  }
  if (requirement.spec.kind === "files") {
    return requirement.spec.entries.map(file => `${file.transfer_inclusion === "exclude" ? "Leave" : "Review"} ${file.relative_path}${file.reason ? ` · ${file.reason}` : ""}`)
  }
  if (requirement.spec.kind === "software") return [requirement.spec.version_constraint ?? "Version not specified", ...(requirement.spec.detect_only ? ["Detect/Check only"] : [])]
  if (requirement.spec.kind === "secrets") return ["Secret name only · choose Vault"]
  if (requirement.spec.kind === "agent_tools") return ["Proposal only · separate admission required"]
  if (requirement.spec.kind === "setup_checks") return [requirement.spec.source_path ?? "Reviewed source required", "Review only · no execution"]
  return []
}
export function environmentDetectionMessages(environment: ProjectEnvironment): string[] {
  return detectionResults(environment).filter(result => !isDetectionSkip(result)).map(result => result.safe_summary)
}
export function environmentDetectionSkips(environment: ProjectEnvironment): string[] {
  return detectionResults(environment).filter(isDetectionSkip).map(result => result.safe_summary)
}
function detectionResults(environment: ProjectEnvironment) {
  return environment.operations.filter(operation => operation.kind === "detect").flatMap(operation => operation.per_item_results)
}
function isDetectionSkip(result: ProjectEnvironment["operations"][number]["per_item_results"][number]): boolean {
  return result.reason_code === "skip_summary_limit" || !result.requirement_id.startsWith("detect:")
}
export function projectEnvironmentSections(environment: ProjectEnvironment): EnvironmentViewSection[] {
  const section = (id: string, title: string, requirements: readonly Requirement[]): EnvironmentViewSection => {
    const groups = Object.entries(labels).flatMap(([kind, label]) => {
      const rows = requirements.filter(r => r.spec.kind === kind).map(r => ({
        id: r.requirement_id, title: r.title, origins: r.origins.map(environmentOriginLabel), details: details(r),
        // P01 has no selected machine and no Check behaviour. Legacy readiness is not an observation.
        status: "Not checked",
      }))
      return rows.length ? [{ kind: label, rows }] : []
    })
    return { id, title, groups }
  }
  return [section("project", "Project-wide", environment.project_requirements),
    ...environment.folders.map(folder => section(folder.folder_id, folder.label, folder.requirements)),
    ...[...(environment.proposals.some(p => p.requirement.scope.kind === "project") ? [section("proposals", "Proposals · review required", environment.proposals.filter(p => p.requirement.scope.kind === "project").map(p => p.requirement))] : []),
      ...environment.folders.flatMap(folder => {
        const requirements = environment.proposals.filter(p => p.requirement.scope.kind === "folder" && p.requirement.scope.folder_id === folder.folder_id).map(p => p.requirement)
        return requirements.length ? [section(`proposals:${folder.folder_id}`, `${folder.label} · Proposals · review required`, requirements)] : []
      })],
  ]
}
export function projectEnvironmentLines(environment: ProjectEnvironment): string[] {
  const skips = environmentDetectionSkips(environment)
  return [...environmentDetectionMessages(environment), ...(skips.length <= 4 ? skips : [`${skips.length} skipped evidence items · complete list below proposals`]), ...projectEnvironmentSections(environment).flatMap(section => [section.title,
    ...(section.groups.length ? section.groups.flatMap(group => [`  ${group.kind}`, ...group.rows.flatMap(row => [
      `    ${row.title} · ${row.status}`, ...row.origins.map(origin => `      ${origin}`), ...row.details.map(detail => `      ${detail}`),
    ])]) : ["  No requirements · Not checked"]),
  ]), ...(skips.length > 4 ? ["Skipped evidence", ...skips] : [])]
}

// MP-08 / MP-10 / MP-11: disclosure comes from the kernel, never inferred from labels.
export function environmentFolderModelDisclosure(environment: ProjectEnvironment, folderId: string): "automatic" | "optional" | "unknown" {
  const operation = environment.operations.filter(operation => operation.kind === "detect").at(-1)
  const result = operation?.per_item_results.find(result => result.requirement_id === `detect:folder:${folderId}`)
  return result?.reason_code === "code_manifest" ? "automatic" : result?.reason_code === "no_code_manifest" ? "optional" : "unknown"
}

export { createEnvironmentViewCache } from "./project-environment-view-cache.js"
