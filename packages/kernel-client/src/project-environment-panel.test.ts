// MP-08 / MP-10 / MP-11: Public panel stays value-free and uses kernel adjustment.
import assert from "node:assert/strict"
import test from "node:test"
import { adjustProjectEnvironmentRequest, projectEnvironmentPanelLines } from "./project-environment-panel.js"
import type { ProjectEnvironmentManifest } from "./kernel-types-project-environment.js"

test("MP-08 / MP-10 / MP-11 panel projects reasons, missing names and sealed sources only", () => {
  const manifest: ProjectEnvironmentManifest = {
    schema_version: 1, project_id: "project", evidence_digest: "a".repeat(64),
    entries: [{name: "TOKEN", workspace_id: "web", kind: "variable", classification: "secret", status: "missing", locator: {kind: "vault", service: "private-service", key: "private-key"}, uses: [{path: "app.ts", line: 7}]}],
    private_files: [{workspace_id: "web", path: "notes.md", bring: false, reason: "Personal notes, unused by the project"}],
    toolchain_hints: ["node"], package_hints: [], service_hints: [],
  }
  const lines = projectEnvironmentPanelLines({...manifest, value: "sentinel-private-value"} as ProjectEnvironmentManifest)
  assert.match(lines.join("\n"), /TOKEN · secret hidden — Project Vault; needs you; app.ts:7/)
  assert.match(lines.join("\n"), /Leave notes.md — Personal notes/)
  assert.doesNotMatch(lines.join("\n"), /sentinel-private-value|private-service|private-key/)
})

test("MP-08 / MP-10 / MP-11 Adjust binds the existing session and agent", () => {
  assert.deepEqual(adjustProjectEnvironmentRequest("session", "agent"), {AdjustProjectEnvironment: {sessionId: "session", agentId: "agent"}})
})
