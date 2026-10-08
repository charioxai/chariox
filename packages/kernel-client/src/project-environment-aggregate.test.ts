// MP-02 / MP-03 / MP-08 / MP-10: shared projection and Project-only protocol regressions.
import assert from "node:assert/strict"
import test from "node:test"
import { getProjectEnvironmentRequest, projectEnvironmentMinimumProtocolVersion, projectEnvironmentSections } from "./project-environment-aggregate.js"
import type { ProjectEnvironment } from "./kernel-types-project-environment-aggregate.js"

test("ENV P01 query has no agent/session and requires allocated protocol 471", () => {
  assert.equal(projectEnvironmentMinimumProtocolVersion, 471)
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

test("ENV VERSION(P01) gates the complete reserved operation family", async () => {
  const { requireProjectEnvironmentProtocol, projectEnvironmentOperationRequest, isProjectEnvironmentRequest } = await import("./project-environment-aggregate-requests.js")
  const request = projectEnvironmentOperationRequest("GetProjectEnvironment", { projectId: "project" })
  assert.equal(isProjectEnvironmentRequest(request), true)
  assert.throws(() => requireProjectEnvironmentProtocol(request, 470), /471/)
  assert.throws(() => requireProjectEnvironmentProtocol(request, undefined), /471/)
  assert.doesNotThrow(() => requireProjectEnvironmentProtocol(request, 471))
  for (const kind of ["DetectProjectEnvironment", "PreviewEnvironmentDiff", "SaveProjectEnvironmentRevision", "PlanProjectEnvironment", "ApplyProjectEnvironment", "CheckProjectEnvironment", "GetEnvironmentOperation", "CancelEnvironmentOperation", "RetryEnvironmentOperation", "ExportProjectEnvironment", "PreviewEnvironmentImport", "CommitEnvironmentImport"]) {
    assert.equal(isProjectEnvironmentRequest({ [kind]: {} }), true)
    assert.throws(() => requireProjectEnvironmentProtocol({ [kind]: {} }, 470), /471/)
  }
  assert.doesNotThrow(() => requireProjectEnvironmentProtocol({ ListProjects: {} }, 435))
})

test("ENV P01 provider references use the official provider IDs", () => {
  const providers: import("./kernel-types-project-environment-aggregate.js").EnvironmentProvider[] = ["codex", "claude", "opencode"]
  assert.deepEqual(providers, ["codex", "claude", "opencode"])
})
