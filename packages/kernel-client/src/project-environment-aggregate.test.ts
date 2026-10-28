// MP-02 / MP-03 / MP-08 / MP-10: shared projection and Project-only protocol regressions.
import assert from "node:assert/strict"
import test from "node:test"
import { getProjectEnvironmentRequest, projectEnvironmentMinimumProtocolVersion, projectEnvironmentSections, projectEnvironmentLines } from "./project-environment-aggregate.js"
import type { ProjectEnvironment } from "./kernel-types-project-environment-aggregate.js"

test("Environment queries have no agent/session and use the current read protocol", () => {
  assert.equal(projectEnvironmentMinimumProtocolVersion, 660)
  assert.deepEqual(getProjectEnvironmentRequest("project"), { GetProjectEnvironment: { projectId: "project" } })
})

test("ENV P01 preserves both plain scopes, hides empty kinds, and never promotes legacy found", () => {
  const environment: ProjectEnvironment = {
    schema_version: 1, delivered_capabilities: { enabled_environment_operations: ["get"], supported_schema: 1 }, local_project_id: "project", lineage: { project_id: "lineage", environment_id: "env" }, revision: 0, content_digest: "digest", parent_revision_digest: null, evidence_digest: null,
    project_requirements: [], folders: [{ folder_id: "one", portable_folder_key: "one", local_workspace_binding: "/one", optional_git: null, label: "One", requirements: [{
      requirement_id: "secret", title: "PASSWORD", scope: { kind: "folder", folder_id: "one" }, origins: [{ kind: "migrated", source: "manifest", digest: "digest" }], spec: { kind: "secrets", name: "PASSWORD", vault: { service: "db", key: "password" } }, depends_on: [], platform_variants: [], required: false,
      legacy_entry: { name: "PASSWORD", workspace_id: "/one", kind: "variable", classification: "secret", uses: [{ path: "src/db.ts", line: 8 }], excluded: true, locator: { kind: "vault", service: "db", key: "password" }, status: "found" },
    }] }, { folder_id: "two", portable_folder_key: "two", local_workspace_binding: "/two", optional_git: null, label: "Two", requirements: [] }],
    proposals: [], reviewed_at_ms: null, reviewed_by: null, observations: [], operations: [], legacy_manifest: null, legacy_reviewed_manifest: null, legacy_review: null,
  }
  const sections = projectEnvironmentSections(environment)
  assert.deepEqual(sections.map(s => s.title), ["Project-wide", "One", "Two"])
  assert.deepEqual(sections[1]?.groups.map(g => g.kind), ["Secrets"])
  assert.equal(sections[1]?.groups[0]?.rows[0]?.status, "Not checked")
  assert.deepEqual(sections[1]?.groups[0]?.rows[0]?.details, ["Vault · db/password", "Excluded from setup"])
  assert.equal(sections[2]?.groups.length, 0)
})

test("Environment reads and operations share one protocol floor", async () => {
  const { requireProjectEnvironmentProtocol, projectEnvironmentOperationRequest, isProjectEnvironmentRequest } = await import("./project-environment-aggregate-requests.js")
  const request = projectEnvironmentOperationRequest("GetProjectEnvironment", { projectId: "project" })
  assert.equal(isProjectEnvironmentRequest(request), true)
  assert.throws(() => requireProjectEnvironmentProtocol(request, 659), /660/)
  assert.throws(() => requireProjectEnvironmentProtocol(request, undefined), /660/)
  assert.doesNotThrow(() => requireProjectEnvironmentProtocol(request, 660))
  const compact = { GetProjectEnvironment: { projectId: "project", view: { knownDigest: null, proposals: null } } }
  for (const read of [request, compact]) {
    for (const version of [487, 636, 659, NaN, 660.5, undefined]) assert.throws(() => requireProjectEnvironmentProtocol(read, version), /660/)
    assert.doesNotThrow(() => requireProjectEnvironmentProtocol(read, 660))
  }
  for (const kind of ["DetectProjectEnvironment", "PreviewEnvironmentDiff", "SaveProjectEnvironmentRevision", "PlanProjectEnvironment", "ApplyProjectEnvironment", "CheckProjectEnvironment", "GetEnvironmentOperation", "CancelEnvironmentOperation", "RetryEnvironmentOperation", "ExportProjectEnvironment", "PreviewEnvironmentImport", "CommitEnvironmentImport"]) {
    assert.equal(isProjectEnvironmentRequest({ [kind]: {} }), true)
    assert.throws(() => requireProjectEnvironmentProtocol({ [kind]: {} }, 659), /660/)
  }
  assert.doesNotThrow(() => requireProjectEnvironmentProtocol({ ListProjects: {} }, 435))
})

test("ENV P01 provider references use the official provider IDs", () => {
  const providers: import("./kernel-types-project-environment-aggregate.js").EnvironmentProvider[] = ["codex", "claude", "opencode"]
  assert.deepEqual(providers, ["codex", "claude", "opencode"])
})

// MP-08 / MP-10 / MP-11: Real MDN has hundreds of binary skips; proposals must remain reachable.
test("ENV P02a many evidence skips do not bury the first proposal", () => {
  const environment = {
    project_requirements: [], folders: [],
    proposals: [{ proposal_id: "proposal", requirement: { requirement_id: "node", title: "Node", scope: { kind: "project" }, origins: [], spec: { kind: "software", identity: "Node", version_constraint: null, detect_only: false } } }],
    operations: [{ kind: "detect", per_item_results: [
      ...Array.from({ length: 100 }, (_, i) => ({ requirement_id: `image-${i}`, reason_code: "unsupported_encoding", safe_summary: `Skipped image-${i}.png · unsupported encoding` })),
      { requirement_id: "detect:model", reason_code: "utility_completed", safe_summary: "Official provider metadata utility completed · proposals need review" },
    ] }],
  } as unknown as ProjectEnvironment
  const lines = projectEnvironmentLines(environment)
  assert(lines.findIndex(line => line.includes("Node · Not checked")) < 12)
  assert.equal(lines.filter(line => line.startsWith("Skipped image-")).length, 100)
})

// MP-08 / MP-10 / MP-11: review931 messages use stable result identities, not English prose.
test("review931_7 evidence labels stay short and exclusions use structured identities", async () => {
  const { environmentOriginLabel, environmentDetectionMessages, environmentDetectionSkips } = await import("./project-environment-aggregate.js")
  const digest = "a".repeat(64)
  const label = environmentOriginLabel({ kind: "detected", relative_path: "src/main.ts", folder_id: "folder", line: 3, evidence_digest: digest })
  assert(label.length < 60)
  const environment = { operations: [{ kind: "detect", per_item_results: [
    { requirement_id: "excluded-path", reason_code: "file_too_large", safe_summary: "Excluded video" },
    { requirement_id: "detect:model", reason_code: "utility_completed", safe_summary: "Skipped no required metadata" },
  ] }] } as unknown as ProjectEnvironment
  assert.deepEqual(environmentDetectionSkips(environment), ["Excluded video"])
  assert.deepEqual(environmentDetectionMessages(environment), ["Skipped no required metadata"])
})

// MP-08 / MP-10 / MP-11: rendering memory cannot cross a selected kernel or grow without bound.
test("Environment view memory is bounded and isolates kernel contexts", async () => {
  const { createEnvironmentViewCache } = await import("./project-environment-view-cache.js")
  const cache = createEnvironmentViewCache()
  for (let n = 0; n < 9; n++) cache.remember("one", { schema_version: 1, local_project_id: `project-${n}` } as ProjectEnvironment)
  assert.equal(cache.peek("one", "project-0"), null)
  assert.equal(cache.peek("one", "project-8")?.local_project_id, "project-8")
  assert.equal(cache.peek("two", "project-8"), null)
  assert.equal(cache.peek("one", "project-8"), null, "switching back does not resurrect a prior kernel view")
})

// MP-08 / MP-10: ENV-09 observations supersede legacy hints and stale Ready.
test("P04 projects five measured statuses and suppresses stale Ready", () => {
  const requirement = { requirement_id: "node", title: "Node", scope: {kind:"project"}, origins:[], spec:{kind:"software", identity:"node", version_constraint:null, platform:null, install_scope:"system", install_source:null, detect_only:false}, depends_on:[], platform_variants:[], required:true, legacy_entry:null }
  const base = {schema_version:1, local_project_id:"project", lineage:{project_id:"p",environment_id:"env"}, revision:1, content_digest:"digest", project_requirements:[requirement], folders:[], proposals:[], observations:[], operations:[]} as unknown as ProjectEnvironment
  const row = {environment_id:"env",requirement_id:"node", revision_digest:"digest",status:"ready" as const,checked_at_ms:Date.now(),stale:false,safe_summary:"Measured",reason_code:"probe",target:{machine_id:"m",target_instance_generation:"g",slice_ref:null},measured_facts:{platform:null,version:null,content_digest:null,present:null,cpus:null,memory_bytes:null,free_disk_bytes:null,gpu:null},check_operation_id:"check"}
  for (const [status,label] of [["ready","Ready"],["missing","Missing"],["outdated","Outdated"],["needs_your_input","Needs your input"],["conflict","Conflict"]] as const) {
    const environment={...base,observations:[{...row,status}]}
    assert.equal(projectEnvironmentSections(environment)[0]?.groups[0]?.rows[0]?.status,label)
  }
  for(const changed of [{stale:true},{revision_digest:"old"},{checked_at_ms:null}]) {
    const environment={...base,observations:[{...row,...changed}]}
    assert.doesNotMatch(projectEnvironmentSections(environment)[0]!.groups[0]!.rows[0]!.status,/^Ready/)
  }
})
