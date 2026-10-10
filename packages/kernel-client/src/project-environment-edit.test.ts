// MP-08 / MP-10 / MP-11: concurrent draft rebase preserves both editors' distinct fields.
import assert from "node:assert/strict"
import test from "node:test"
import { environmentRevisionDraft, editEnvironmentRequirement, mergeEnvironmentDraft, saveEnvironmentRevisionRequest } from "./project-environment-edit.js"
import type { ProjectEnvironment, Requirement } from "./kernel-types-project-environment-aggregate.js"
const requirement: Requirement = { requirement_id:"node",title:"Node",scope:{kind:"project"},origins:[],spec:{kind:"software",identity:"node",version_constraint:"22",platform:null,install_scope:"project",install_source:null,detect_only:true},depends_on:[],platform_variants:[],required:true,legacy_entry:null }
const environment = { local_project_id:"project",revision:2,content_digest:"current",project_requirements:[requirement],folders:[] } as unknown as ProjectEnvironment

test("explicit rebase overlays only our changed fields and preserves new kernel requirements", () => {
  const base=environmentRevisionDraft(environment)
  const ours=editEnvironmentRequirement(base,"node",{required:false})
  const theirs=editEnvironmentRequirement(base,"node",{title:"Editor A"})
  const latest={...theirs,project_requirements:[...theirs.project_requirements,{...requirement,requirement_id:"python",title:"Python"}]}
  const merged=mergeEnvironmentDraft(base,ours,latest)
  assert.equal(merged.project_requirements[0]?.title,"Editor A")
  assert.equal(merged.project_requirements[0]?.required,false)
  assert.equal(merged.project_requirements[1]?.title,"Python")
  assert.deepEqual(saveEnvironmentRevisionRequest(environment,merged,["accept"],["exclude"]).SaveProjectEnvironmentRevision,{projectId:"project",expectedRevision:2,expectedContentDigest:"current",draft:merged,acceptedProposalIds:["accept"],excludedProposalIds:["exclude"]})
})
test("a field changed by both editors remains an explicit choice; stale data does not mutate either snapshot", () => {
  const base=environmentRevisionDraft(environment)
  const ours=editEnvironmentRequirement(base,"node",{title:"Editor B"})
  const theirs=editEnvironmentRequirement(base,"node",{title:"Editor A"})
  assert.equal(mergeEnvironmentDraft(base,ours,theirs).project_requirements[0]?.title,"Editor B")
  assert.equal(base.project_requirements[0]?.title,"Node")
  assert.equal(theirs.project_requirements[0]?.title,"Editor A")
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

// MP-08 / MP-10 / MP-11: a modify/delete conflict must retain an explicit restoration.
for (const scope of [{ kind: "project" as const }, { kind: "folder" as const, folder_id: "api" }]) {
  for (const field of ["title", "version"] as const) test(`${scope.kind} ${field} edit survives concurrent deletion; unchanged deletion stays deleted`, () => {
    const saved = { ...requirement, scope, origins: [{ kind: "user_added" as const, user_id: "editor", revision: 2 }] }
    const folder = { folder_id: "api", portable_folder_key: "api", label: "api", optional_git: null, requirements: scope.kind === "folder" ? [saved] : [] }
    const base = { project_requirements: scope.kind === "project" ? [saved] : [], folders: [folder] }
    const latest = { project_requirements: [], folders: [{ ...folder, requirements: [] }] }
    const patch = field === "title" ? { title: "Edited Node" } : { spec: { ...saved.spec, version_constraint: "24" } as Requirement["spec"] }
    const changed = editEnvironmentRequirement(base, "node", patch)
    const merged = mergeEnvironmentDraft(base, changed, latest)
    const restored = scope.kind === "project" ? merged.project_requirements : merged.folders[0]!.requirements
    assert.deepEqual(restored, [{ ...saved, ...patch }])
    const unchanged = mergeEnvironmentDraft(base, base, latest)
    assert.deepEqual(unchanged.project_requirements, [])
    assert.deepEqual(unchanged.folders[0]!.requirements, [])
    assert.equal(saved.title, "Node")
    assert.deepEqual(latest.folders[0]!.requirements, [])
  })
}
