import assert from "node:assert/strict"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"
import {
  makeSession,
  makeWorkflow,
  makeWorkflowRun,
} from "./shell-executor.test-support.js"

test("executeShellCommand exports and imports workflow-code packages and source", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-workflow-code-shell-"))
  try {
    const workflowCodePackage = {
      package_version: 2,
      name: "toy-flow",
      language: "JavaScript",
      source: "workflow.define({ alias: \"toy\" })\n",
      source_sha256: "source-sha256",
      source_bytes: 34,
      definition_sha256: "definition-sha256",
      definition: {
        schema_version: 1,
        workflow: { alias: "toy" },
      },
      validation: { ok: true },
      exported_at_ms: 1_000,
    }
    const artifact = {
      metadata: {
        name: "imported-toy",
        language: "JavaScript",
        path: "/repo/.chariox/workflow-code/imported-toy.json",
        source_sha256: "source-sha256",
        source_bytes: 34,
        validation: { ok: true },
        provenance: { created_by: { user_id: "user-1" }, updated_by: { user_id: "user-1" } },
        created_at_ms: 1_000,
        updated_at_ms: 1_000,
      },
      source: workflowCodePackage.source,
      definition: workflowCodePackage.definition,
    }
    const requests: Record<string, unknown>[] = []
    const fake = {
      client: {
        send: async (request: Record<string, unknown>) => {
          requests.push(request)
          if ("ExportWorkflowCodePackage" in request) {
            return { WorkflowCodePackageExported: { package: workflowCodePackage } }
          }
          if ("ImportWorkflowCodePackage" in request) {
            return { WorkflowCodePackageImported: { artifact } }
          }
          const payload = request.ExportWorkflowCodeSource as { format?: string }
          if (payload.format === "directory") {
            return {
              WorkflowCodeSourceExported: {
                export: {
                  name: "toy-flow",
                  language: "JavaScript",
                  format: "directory",
                  source_path: "workflow.js",
                  source: "async function defineWorkflow(workflow) {}\n",
                  source_sha256: "dir-source-sha256",
                  source_bytes: 43,
                  definition_sha256: "definition-sha256",
                  files: [
                    { path: "workflow.js", contents: "async function defineWorkflow(workflow) {}\n", sha256: "dir-source-sha256" },
                    { path: "schemas/final.json", contents: "{\n  \"type\": \"object\"\n}\n", sha256: "schema-sha256" },
                    { path: "manifest.json", contents: "{\n  \"manifest_version\": 1\n}\n", sha256: "manifest-sha256" },
                  ],
                },
              },
            }
          }
          return {
            WorkflowCodeSourceExported: {
              export: {
                name: "toy-flow",
                language: "JavaScript",
                format: "inline",
                source_path: "workflow.js",
                source: workflowCodePackage.source,
                source_sha256: "source-sha256",
                source_bytes: 34,
                definition_sha256: "definition-sha256",
                files: [],
              },
            },
          }
        },
      },
    }
    const context = createDefaultShellContext({ workspace: root, worktree: root, sessionId: "session-1" })
    const packageExport = await executeShellCommand(parseShellCommand("workflow code package export toy-flow --out exports/toy.workflow-code.json"), context, { client: fake.client })
    const packageImport = await executeShellCommand(parseShellCommand("workflow code package import exports/toy.workflow-code.json --name imported-toy --overwrite"), context, { client: fake.client })
    const sourceInline = await executeShellCommand(parseShellCommand("workflow code source export toy-flow --out exports/toy.js"), context, { client: fake.client })
    const sourceDirectory = await executeShellCommand(parseShellCommand("workflow code source export toy-flow --out exports/toy-source --format directory"), context, { client: fake.client })
    const sourceDirectoryPrimary = await executeShellCommand(parseShellCommand("workflow code source export-directory toy-flow --out exports/toy-source-primary"), context, { client: fake.client })
    const sourceDirectoryAlias = await executeShellCommand(parseShellCommand("workflow code source export-dir toy-flow --out exports/toy-source-alias"), context, { client: fake.client })
    const workflowSource = await executeShellCommand(parseShellCommand("workflow code source export workflow-1 --out exports/workflow.js --workflow --existing-agents"), context, { client: fake.client })

    assert.equal(packageExport.ok, true)
    assert.equal(packageImport.ok, true)
    assert.equal(sourceInline.ok, true)
    assert.equal(sourceDirectory.ok, true)
    assert.equal(sourceDirectoryPrimary.ok, true)
    assert.equal(sourceDirectoryAlias.ok, true)
    assert.equal(workflowSource.ok, true)
    assert.equal(JSON.parse(await readFile(join(root, "exports/toy.workflow-code.json"), "utf8")).name, "toy-flow")
    assert.equal(await readFile(join(root, "exports/toy.js"), "utf8"), workflowCodePackage.source)
    assert.equal(await readFile(join(root, "exports/toy-source/workflow.js"), "utf8"), "async function defineWorkflow(workflow) {}\n")
    assert.equal(await readFile(join(root, "exports/toy-source/schemas/final.json"), "utf8"), "{\n  \"type\": \"object\"\n}\n")
    assert.equal(await readFile(join(root, "exports/toy-source-primary/manifest.json"), "utf8"), "{\n  \"manifest_version\": 1\n}\n")
    assert.equal(await readFile(join(root, "exports/toy-source-alias/manifest.json"), "utf8"), "{\n  \"manifest_version\": 1\n}\n")
    assert.deepEqual(requests, [
      { ExportWorkflowCodePackage: { session_id: "session-1", name: "toy-flow" } },
      {
        ImportWorkflowCodePackage: {
          session_id: "session-1",
          package: workflowCodePackage,
          name: "imported-toy",
          overwrite: true,
          node_path: "node",
        },
      },
      {
        ExportWorkflowCodeSource: {
          session_id: "session-1",
          target: { kind: "artifact", name: "toy-flow" },
          format: "inline",
          agent_mode: "portable_generated",
        },
      },
      {
        ExportWorkflowCodeSource: {
          session_id: "session-1",
          target: { kind: "artifact", name: "toy-flow" },
          format: "directory",
          agent_mode: "portable_generated",
        },
      },
      {
        ExportWorkflowCodeSource: {
          session_id: "session-1",
          target: { kind: "artifact", name: "toy-flow" },
          format: "directory",
          agent_mode: "portable_generated",
        },
      },
      {
        ExportWorkflowCodeSource: {
          session_id: "session-1",
          target: { kind: "artifact", name: "toy-flow" },
          format: "directory",
          agent_mode: "portable_generated",
        },
      },
      {
        ExportWorkflowCodeSource: {
          session_id: "session-1",
          target: { kind: "workflow", workflow_ref: "workflow-1" },
          format: "inline",
          agent_mode: "existing_agents",
        },
      },
    ])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("executeShellCommand validates, saves, applies, and runs workflow-code artifacts", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-workflow-code-shell-"))
  try {
    const source = "workflow.define({ alias: \"toy\" })\n"
    await writeFile(join(root, "toy.workflow.js"), source, "utf8")
    const definition = {
      schema_version: 1,
      workflow: { alias: "toy" },
    }
    const compile = {
      definition,
      validation: { ok: true, diagnostics: [] },
      logs: "",
    }
    const artifact = {
      metadata: {
        name: "toy-flow",
        language: "java_script",
        path: "/repo/.chariox/workflow-code/toy-flow.json",
        source_sha256: "source-sha256",
        source_bytes: source.length,
        validation: { ok: true, diagnostics: [] },
        provenance: { created_by: { user_id: "user-1" }, updated_by: { user_id: "user-1" } },
        created_at_ms: 1_000,
        updated_at_ms: 1_000,
      },
      source,
      definition,
    }
    const session = makeSession()
    const requests: Record<string, unknown>[] = []
    const fake = {
      client: {
        send: async (request: Record<string, unknown>) => {
          requests.push(request)
          if ("ValidateWorkflowCode" in request) {
            return { WorkflowCodeValidated: { result: compile } }
          }
          if ("CreateWorkflowCodeArtifact" in request) {
            return { WorkflowCodeArtifactCreated: { artifact } }
          }
          if ("ListWorkflowCodeArtifacts" in request) {
            return { WorkflowCodeArtifactsListed: { artifacts: [artifact.metadata] } }
          }
          if ("GetWorkflowCodeArtifact" in request) {
            return { WorkflowCodeArtifact: { artifact } }
          }
          if ("ApplyWorkflowCode" in request || "ApplyWorkflowCodeArtifact" in request) {
            return { WorkflowCodeApplied: { result: { compile, apply: { workflow_id: "workflow-1", canvas_layout_applied: true } }, session } }
          }
          if ("RunWorkflowCode" in request || "RunWorkflowCodeArtifact" in request) {
            return {
              WorkflowCodeRun: {
                result: {
                  apply: { compile, apply: { workflow_id: "workflow-2", canvas_layout_applied: true } },
                  invocation: { kind: "started", workflow_run: makeWorkflowRun(), workflow: makeWorkflow(), endpoint: makeWorkflow().endpoints![0] },
                },
                session,
              },
            }
          }
          return { WorkflowCodeArtifactDeleted: { name: "toy-flow", path: "/repo/.chariox/workflow-code/toy-flow.json" } }
        },
      },
    }
    const context = createDefaultShellContext({ workspace: root, worktree: root, sessionId: "session-1" })
    const validate = await executeShellCommand(parseShellCommand("workflow code validate toy.workflow.js --provider-rebinding planner=dev-stub/default"), context, { client: fake.client })
    const save = await executeShellCommand(parseShellCommand("workflow code save toy-flow toy.workflow.js"), context, { client: fake.client })
    const list = await executeShellCommand(parseShellCommand("workflow code artifact list"), context, { client: fake.client })
    const get = await executeShellCommand(parseShellCommand("workflow code artifact get toy-flow"), context, { client: fake.client })
    const apply = await executeShellCommand(parseShellCommand("workflow code apply toy.workflow.js --provider-rebinding planner=dev-stub/default"), context, { client: fake.client })
    const run = await executeShellCommand(parseShellCommand("workflow code run toy.workflow.js --endpoint entry --queue urgent --prompt \"Run it\" --provider-rebinding planner=dev-stub/default"), context, { client: fake.client })
    const artifactApply = await executeShellCommand(parseShellCommand("workflow code artifact apply toy-flow --provider-rebinding planner=dev-stub/default"), context, { client: fake.client })
    const artifactRun = await executeShellCommand(parseShellCommand("workflow code artifact run toy-flow --endpoint entry --prompt \"Run saved\""), context, { client: fake.client })
    const deleted = await executeShellCommand(parseShellCommand("workflow code artifact delete toy-flow"), context, { client: fake.client })

    assert.equal(validate.ok, true)
    assert.match(validate.message ?? "", /is valid/)
    assert.equal(save.ok, true)
    assert.equal(list.ok, true)
    assert.match(list.message ?? "", /toy-flow validation=ok/)
    assert.equal(get.ok, true)
    assert.equal(apply.ok, true)
    assert.match(apply.message ?? "", /workflow-1/)
    assert.equal(run.ok, true)
    assert.match(run.message ?? "", /workflow-2/)
    assert.equal(artifactApply.ok, true)
    assert.equal(artifactRun.ok, true)
    assert.equal(deleted.ok, true)
    assert.deepEqual(requests, [
      {
        ValidateWorkflowCode: {
          session_id: "session-1",
          node_path: process.execPath,
          source,
          provider_rebindings: [{ node: "planner", provider: "dev-stub", model: "default" }],
        },
      },
      {
        CreateWorkflowCodeArtifact: {
          session_id: "session-1",
          name: "toy-flow",
          language: "java_script",
          node_path: process.execPath,
          source,
        },
      },
      { ListWorkflowCodeArtifacts: { session_id: "session-1" } },
      { GetWorkflowCodeArtifact: { session_id: "session-1", name: "toy-flow" } },
      {
        ApplyWorkflowCode: {
          session_id: "session-1",
          node_path: process.execPath,
          source,
          provider_rebindings: [{ node: "planner", provider: "dev-stub", model: "default" }],
        },
      },
      {
        RunWorkflowCode: {
          session_id: "session-1",
          node_path: process.execPath,
          source,
          provider_rebindings: [{ node: "planner", provider: "dev-stub", model: "default" }],
          endpoint: "entry",
          queue_ref: "urgent",
          prompt: "Run it",
        },
      },
      {
        ApplyWorkflowCodeArtifact: {
          session_id: "session-1",
          name: "toy-flow",
          provider_rebindings: [{ node: "planner", provider: "dev-stub", model: "default" }],
        },
      },
      {
        RunWorkflowCodeArtifact: {
          session_id: "session-1",
          name: "toy-flow",
          endpoint: "entry",
          prompt: "Run saved",
        },
      },
      { DeleteWorkflowCodeArtifact: { session_id: "session-1", name: "toy-flow" } },
    ])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
