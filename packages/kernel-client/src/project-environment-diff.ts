// MP-08 / MP-10 / MP-11: readable reference-only revision review, shared with the TUI.
import type { Requirement, EnvironmentRevisionDiff } from "./kernel-types-project-environment-aggregate.js"
export function environmentRequirementSummary(r: Requirement): string[] {
  const s = r.spec
  const lines = [`Title: ${r.title}`, `Scope: ${r.scope.kind === "project" ? "Project-wide" : r.scope.folder_id}`, `Required: ${r.required ? "Yes" : "No"}`, `Dependencies: ${r.depends_on.join(", ") || "None"}`]
  switch (s.kind) {
    case "software": lines.push(`Software: ${s.identity}`, `Version constraint: ${s.version_constraint ?? "Not specified"}`, `Install scope: ${s.install_scope}`, `Source: ${s.install_source ?? "Not specified"}`, `Detect/Check only: ${s.detect_only ? "Yes" : "No"}`); break
    case "files": lines.push(...s.entries.map(e => `File: ${e.relative_path} · ${e.transfer_inclusion} · ${e.content_digest ?? "No digest"} · ${e.credential_filter_verdict}`)); break
    case "variables": lines.push(`Variable: ${s.name}`, `Source: ${s.locator.kind}`, ...(s.locator.kind === "env_file" ? [`Path: ${s.locator.path}`, `Key: ${s.locator.key}`] : s.locator.kind === "config_file" ? [`Path: ${s.locator.path}`] : s.locator.kind === "workspace_environment" ? [`Environment name: ${s.locator.name}`] : [])); break
    case "secrets": lines.push(`Secret: ${s.name}`, `Vault: ${s.vault ? `${s.vault.service}/${s.vault.key}` : "Needs your input"}`); break
    case "accounts": lines.push(`Provider: ${s.provider}`, `Profile: ${s.linked_profile_ref ?? "Not linked"}`, `Capabilities: ${s.required_capabilities.join(", ")}`); break
    case "agent_tools": lines.push(`Agent tool: ${s.tool_kind} · ${s.registry_ref}`, `Digest: ${s.package_digest ?? "Not specified"}`, `Runtimes: ${s.runtime_requirements.join(", ")}`, `Vault references: ${s.vault_refs.map(v => `${v.service}/${v.key}`).join(", ")}`, `Providers: ${s.compatible_providers.join(", ")}`); break
    case "chariox_apps": lines.push(`App: ${s.app_id} · ${s.version ?? "Not specified"}`, `Digest: ${s.content_digest ?? "Not specified"}`, `Grants: ${s.required_grants.join(", ")}`); break
    case "services": lines.push(`Service: ${s.identity}`); if (s.probe) lines.push(...probeLines(s.probe)); break
    case "setup_checks": lines.push(`Boundary: ${s.boundary}`, `Source: ${s.source_path ?? "Not specified"} · ${s.source_digest ?? "No digest"}`, ...s.commands.map(c => `Command: ${c.executable} ${c.arguments.join(" ")} · ${c.timeout_ms} ms · ${c.output_limit_bytes} bytes`), ...s.probes.flatMap(probeLines)); break
    case "machine_needs": lines.push(`CPUs: ${s.minimum_cpus ?? "Not specified"}`, `Memory: ${s.minimum_memory_bytes ?? "Not specified"}`, `Free disk: ${s.minimum_free_disk_bytes ?? "Not specified"}`, `GPU: ${s.gpu ?? "Not specified"}`); break
  }
  if ("platform" in s && s.platform) lines.push(`Platform: ${s.platform.os}/${s.platform.architecture}`)
  if ("platforms" in s) lines.push(...s.platforms.map(p => `Platform: ${p.os}/${p.architecture}`))
  return lines
}
function probeLines(p: import("./kernel-types-project-environment-aggregate.js").EnvironmentProbe): string[] { return [`Probe: ${p.kind} · ${p.kind === "tcp" ? `${p.host}:${p.port}` : p.kind === "http" ? `${p.url} → ${p.expected_status}` : `${p.name} ${p.arguments.join(" ")}`} · ${p.timeout_ms} ms`] }
export function environmentDiffLines(diff: EnvironmentRevisionDiff): string[] { return [`Revision ${diff.expected_revision} · ${diff.source_digest} → ${diff.target_digest}`, ...diff.requirements.filter(r => r.kind !== "unchanged").flatMap(r => [`${r.kind} · ${r.after?.title ?? r.before?.title ?? r.requirement_id}`, "Before", ...(r.before ? environmentRequirementSummary(r.before) : ["Absent"]), "After", ...(r.after ? environmentRequirementSummary(r.after) : ["Removed"])])] }
