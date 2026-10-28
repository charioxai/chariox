// MP-08 / MP-10 / MP-11: concurrent draft rebase preserves both editors' distinct fields.
import assert from "node:assert/strict"
import test from "node:test"
import { environmentRevisionDraft, editEnvironmentRequirement, mergeEnvironmentDraft, saveEnvironmentRevisionRequest } from "./project-environment-edit.js"
import type { ProjectEnvironment, Requirement } from "./kernel-types-project-environment-aggregate.js"
const requirement: Requirement = { requirement_id: "node", title: "Node", scope: { kind: "project" }, origins: [], spec: { kind: "software", identity: "node", version_constraint: "22", platform: null, install_scope: "project", install_source: null, detect_only: true }, depends_on: [], platform_variants: [], required: true, legacy_entry: null }
const environment = { local_project_id: "project", revision: 2, content_digest: "current", project_requirements: [requirement], folders: [] } as unknown as ProjectEnvironment

test("explicit rebase overlays only our changed fields and preserves new kernel requirements", () => {
  const base = environmentRevisionDraft(environment)
  const ours = editEnvironmentRequirement(base, "node", { required: false })
  const theirs = editEnvironmentRequirement(base, "node", { title: "Editor A" })
  const latest = { ...theirs, project_requirements: [...theirs.project_requirements, { ...requirement, requirement_id: "python", title: "Python" }] }
  const merged = mergeEnvironmentDraft(base, ours, latest)
  assert.equal(merged.project_requirements[0]?.title, "Editor A")
  assert.equal(merged.project_requirements[0]?.required, false)
  assert.equal(merged.project_requirements[1]?.title, "Python")
  assert.deepEqual(saveEnvironmentRevisionRequest(environment, merged, ["accept"], ["exclude"]).SaveProjectEnvironmentRevision, { projectId: "project", expectedRevision: 2, expectedContentDigest: "current", draft: merged, acceptedProposalIds: ["accept"], excludedProposalIds: ["exclude"] })
})
test("a field changed by both editors remains an explicit choice; stale data does not mutate either snapshot", () => {
  const base = environmentRevisionDraft(environment)
  const ours = editEnvironmentRequirement(base, "node", { title: "Editor B" })
  const theirs = editEnvironmentRequirement(base, "node", { title: "Editor A" })
  assert.equal(mergeEnvironmentDraft(base, ours, theirs).project_requirements[0]?.title, "Editor B")
  assert.equal(base.project_requirements[0]?.title, "Node")
  assert.equal(theirs.project_requirements[0]?.title, "Editor A")
})

// MP-08 / MP-10 / MP-11: folder labels are editable specification fields.
test("folder rename survives rebase and preserves another editor's requirement change", () => {
  const folder = { folder_id: "api", portable_folder_key: "api", label: "api", optional_git: null, requirements: [{ ...requirement, scope: { kind: "folder" as const, folder_id: "api" } }] }
  const base = { project_requirements: [], folders: [folder] }
  const ours = { ...base, folders: [{ ...folder, label: "backend" }] }
  const latest = { ...base, folders: [{ ...folder, requirements: [{ ...folder.requirements[0]!, required: false }] }] }
  assert.equal(mergeEnvironmentDraft(base, ours, base).folders[0]?.label, "backend")
  const result = mergeEnvironmentDraft(base, ours, latest)
  assert.equal(result.folders[0]?.label, "backend")
  assert.equal(result.folders[0]?.requirements[0]?.required, false)
  assert.equal(mergeEnvironmentDraft(base, base, { ...latest, folders: [{ ...latest.folders[0]!, label: "server" }] }).folders[0]?.label, "server")
})

// MP-08 / MP-10 / MP-11: current Detect provenance remains authoritative during explicit edit rebase.
test("P03 edited proposal rebase adopts current provenance while retaining typed edits", async () => {
  const { rebaseEnvironmentProvenance } = await import("./project-environment-edit.js")
  const edited = { ...requirement, requirement_id: "proposal-node", origins: [{ kind: "detected_metadata" as const, source: "old", reference: "old" }], title: "Reviewed node" }
  const latest = { ...environment, project_requirements: [], proposals: [{ proposal_id: "new", requirement: { ...edited, title: "New evidence", origins: [{ kind: "detected_metadata" as const, source: "new", reference: "new" }] } }] }
  const result = rebaseEnvironmentProvenance({ project_requirements: [edited], folders: [] }, latest as any)
  assert.equal(result.project_requirements[0]?.title, "Reviewed node")
  assert.equal(result.project_requirements[0]?.origins[0]?.kind, "detected_metadata")
  assert.deepEqual(result.project_requirements[0]?.origins, latest.proposals[0]!.requirement.origins)
})

test("P03 explicit review retains a typed edit when another editor removed its row", () => {
  const base = environmentRevisionDraft(environment)
  const ours = editEnvironmentRequirement(base, "node", { title: "Reviewed removal" })
  assert.equal(mergeEnvironmentDraft(base, ours, { project_requirements: [], folders: [] }).project_requirements[0]?.title, "Reviewed removal")
  assert.deepEqual(mergeEnvironmentDraft(base, base, { project_requirements: [], folders: [] }).project_requirements, [])
})
