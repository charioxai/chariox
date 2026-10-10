// MP-08 / MP-10 / MP-11: value-free draft edits and explicit three-way conflict review.
import type { ProjectEnvironment, EnvironmentRevisionDraft, Requirement } from "./kernel-types-project-environment-aggregate.js"
export function environmentRevisionDraft(e: ProjectEnvironment): EnvironmentRevisionDraft {
  return { project_requirements: e.project_requirements, folders: e.folders.map(({ local_workspace_binding: _binding, ...folder }) => folder) }
}
export function editEnvironmentRequirement(draft: EnvironmentRevisionDraft, id: string, patch: Partial<Pick<Requirement, "title" | "required" | "spec">>): EnvironmentRevisionDraft {
  const edit = (r: Requirement) => r.requirement_id === id ? { ...r, ...patch } : r
  return { project_requirements: draft.project_requirements.map(edit), folders: draft.folders.map(f => ({ ...f, requirements: f.requirements.map(edit) })) }
}
export function mergeEnvironmentDraft(base: EnvironmentRevisionDraft, draft: EnvironmentRevisionDraft, latest: EnvironmentRevisionDraft): EnvironmentRevisionDraft {
  // Only fields actually changed by this editor overlay the latest kernel specification.
  const all = (d: EnvironmentRevisionDraft) => [...d.project_requirements, ...d.folders.flatMap(f => f.requirements)]
  const original = new Map(all(base).map(r => [r.requirement_id, r]))
  const edited = new Map(all(draft).map(r => [r.requirement_id, r]))
  const fields = ["title", "scope", "spec", "required", "depends_on", "platform_variants"] as const
  const merge = (r: Requirement): Requirement | null => {
    const old = original.get(r.requirement_id), ours = edited.get(r.requirement_id)
    if (!old) return r
    if (!ours) return null
    const patch: Record<string, unknown> = {}
    for (const field of fields) {
      if (JSON.stringify(old[field]) !== JSON.stringify(ours[field])) patch[field] = ours[field]
    }
    return { ...r, ...patch }
  }
  const merged = all(latest).flatMap(r => { const result = merge(r); return result ? [result] : [] })
  for (const r of all(draft)) {
    const old = original.get(r.requirement_id)
    const changed = !old || fields.some(field => JSON.stringify(old[field]) !== JSON.stringify(r[field]))
    if (changed && !merged.some(current => current.requirement_id === r.requirement_id)) merged.push(r)
  }
  return { project_requirements: merged.filter(r => r.scope.kind === "project"), folders: latest.folders.map(f => {
    const old = base.folders.find(folder => folder.folder_id === f.folder_id)
    const ours = draft.folders.find(folder => folder.folder_id === f.folder_id)
    return { ...f, label: old && ours && old.label !== ours.label ? ours.label : f.label, requirements: merged.filter(r => r.scope.kind === "folder" && r.scope.folder_id === f.folder_id) }
  }) }
}
export function saveEnvironmentRevisionRequest(environment: ProjectEnvironment, draft: EnvironmentRevisionDraft, accepted: readonly string[] = [], excluded: readonly string[] = []) {
  return { SaveProjectEnvironmentRevision: { projectId: environment.local_project_id, expectedRevision: environment.revision, expectedContentDigest: environment.content_digest, draft, acceptedProposalIds: accepted, excludedProposalIds: excluded } } as const
}
