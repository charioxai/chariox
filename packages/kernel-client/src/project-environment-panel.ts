// MP-08 / MP-10 / MP-11: Display only kernel-selected metadata, never values.
import type { ProjectEnvironmentManifest } from "./kernel-types-project-environment.js"

export const projectEnvironmentAdjustmentMinimumProtocolVersion = 372

export function adjustProjectEnvironmentRequest(sessionId: string, agentId: string) {
  return { AdjustProjectEnvironment: { sessionId, agentId } } as const
}

export function projectEnvironmentPanelLines(manifest: ProjectEnvironmentManifest): string[] {
  const selected = manifest.entries.filter(entry => !entry.excluded)
  const files = manifest.private_files ?? []
  const lines = [
    `Environment: ${selected.length} inputs, ${selected.filter(entry => entry.classification === "secret").length} secrets hidden`,
    `Files: bringing ${files.filter(file => file.bring).length} / leaving ${files.filter(file => !file.bring).length}`,
    `Setup: ${[...manifest.toolchain_hints, ...manifest.package_hints, ...manifest.service_hints].join(", ") || "No additional tools"}`,
  ]
  for (const entry of manifest.entries) {
    const locator = entry.locator
    const source = entry.excluded ? "left behind"
      : locator.kind === "env_file" || locator.kind === "config_file" ? locator.path
      : locator.kind === "vault" ? "Project Vault"
      : locator.kind === "workspace_environment" ? "workspace shell" : "not found"
    lines.push(`${entry.name}${entry.classification === "secret" ? " · secret hidden" : ""} — ${source}${!entry.excluded && entry.status !== "found" ? "; needs you" : ""}; ${entry.uses.map(use => `${use.path}:${use.line}`).join(", ")}`)
  }
  for (const file of files) lines.push(`${file.bring ? "Bring" : "Leave"} ${file.path} — ${file.reason}`)
  return lines
}
