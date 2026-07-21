import assert from "node:assert/strict"
import test from "node:test"

import {
  applyWorkflowCodeArtifactRequest,
  applyWorkflowCodeRequest,
  bindWorkflowPublicationDeploymentRequest,
  createWorkflowCodeArtifactRequest,
  controlWorkflowPublicationRuntimeRequest,
  deleteWorkflowCodeArtifactRequest,
  exportWorkflowCodeArtifactRequest,
  exportWorkflowCodePackageRequest,
  exportWorkflowCodeSourceRequest,
  exportWorkflowPublicationPackageRequest,
  importWorkflowCodeArtifactRequest,
  listWorkflowCodeArtifactsRequest,
  pauseWorkflowRunRequest,
  runWorkflowCodeArtifactRequest,
  runWorkflowCodeRequest,
  runWorkflowRegistryEntryRequest,
  setWorkflowNodeWaitForAllInputsRequest,
} from "./ipc-workflow-requests.js"

test("pause workflow run request matches kernel shape", () => {
  assert.deepEqual(pauseWorkflowRunRequest("session-1", "run-1"), {
    PauseWorkflowRun: {
      session_id: "session-1",
      workflow_run_ref: "run-1",
    },
  })
})

test("export workflow publication package request matches kernel shape", () => {
  assert.deepEqual(exportWorkflowPublicationPackageRequest("session-1", "publication-1", {
    kernelUrl: "ws://127.0.0.1:43118",
    agentApp: {
      enabled: true,
      routes: [{ path: "/add/*" }],
    },
    agentAppAssetsDir: "/repo/dist",
  }), {
    ExportWorkflowPublicationPackage: {
      session_id: "session-1",
      publication_ref: "publication-1",
      kernel_url: "ws://127.0.0.1:43118",
      agent_app: {
        enabled: true,
        routes: [{ path: "/add/*" }],
      },
      agent_app_assets_dir: "/repo/dist",
    },
  })
})

test("workflow publication runtime control request matches kernel shape", () => {
  assert.deepEqual(controlWorkflowPublicationRuntimeRequest("session-1", "publication-1", "start", {
    host: "127.0.0.1",
    port: 3000,
    kernelUrl: "ws://127.0.0.1:43118",
  }), {
    ControlWorkflowPublicationRuntime: {
      session_id: "session-1",
      publication_ref: "publication-1",
      action: "start",
      host: "127.0.0.1",
      port: 3000,
      kernel_url: "ws://127.0.0.1:43118",
    },
  })
  assert.deepEqual(controlWorkflowPublicationRuntimeRequest("session-1", "publication-1", "stop"), {
    ControlWorkflowPublicationRuntime: {
      session_id: "session-1",
      publication_ref: "publication-1",
      action: "stop",
      host: null,
      port: null,
      kernel_url: null,
    },
  })
  assert.deepEqual(controlWorkflowPublicationRuntimeRequest("session-1", "publication-1", "inspect"), {
    ControlWorkflowPublicationRuntime: {
      session_id: "session-1",
      publication_ref: "publication-1",
      action: "inspect",
      host: null,
      port: null,
      kernel_url: null,
    },
  })
})

test("workflow publication deployment bind request matches kernel shape", () => {
  assert.deepEqual(bindWorkflowPublicationDeploymentRequest("session-1", "publication-1", {
    setupId: "setup-1",
    operationKey: "deployment-setup:setup-1:runtime",
    deploymentId: "deployment-1",
    releaseId: "release-1",
    packageDigest: `sha256:${"a".repeat(64)}`,
    desiredRevision: 7,
  }), {
    BindWorkflowPublicationDeployment: {
      session_id: "session-1",
      publication_ref: "publication-1",
      setup_id: "setup-1",
      operation_key: "deployment-setup:setup-1:runtime",
      deployment_id: "deployment-1",
      release_id: "release-1",
      package_digest: `sha256:${"a".repeat(64)}`,
      desired_revision: 7,
    },
  })
})

test("set workflow node wait-for-all-inputs request matches kernel shape", () => {
  assert.deepEqual(
    setWorkflowNodeWaitForAllInputsRequest("session-1", "workflow-1", "node-1", true),
    {
      SetWorkflowNodeWaitForAllInputs: {
        session_id: "session-1",
        workflow_ref: "workflow-1",
        node_id: "node-1",
        wait_for_all_inputs: true,
      },
    },
  )
})

test("workflow registry run request includes agent rebindings", () => {
  assert.deepEqual(
    runWorkflowRegistryEntryRequest("session-1", "loop-until-done", "Fix the app.", {
      endpoint: "entry",
      agentRebindings: [{ node: "worker", agent_ref: "agent-1" }],
    }),
    {
      RunWorkflowRegistryEntry: {
        session_id: "session-1",
        name: "loop-until-done",
        agent_rebindings: [{ node: "worker", agent_ref: "agent-1" }],
        endpoint: "entry",
        prompt: "Fix the app.",
      },
    },
  )
})

test("workflow-code artifact requests match kernel shape", () => {
  const workflowCodePackage = {
    package_version: 2,
    name: "toy-flow",
    language: "java_script" as const,
    source: "workflow.define({})",
    source_sha256: "sha256",
    source_bytes: 19,
    definition_sha256: "definition-sha256",
    definition: {
      schema_version: 1,
      workflow: {},
      nodes: [],
      endpoints: [],
    },
    validation: {
      ok: true,
      diagnostics: [],
    },
    exported_at_ms: 1_000,
  }
  assert.deepEqual(
    applyWorkflowCodeRequest("session-1", "/usr/local/bin/node", "workflow.define({})", [
      {
        node: "planner",
        provider: "opencode",
        model: "qwen3-coder",
      },
    ]),
    {
      ApplyWorkflowCode: {
        session_id: "session-1",
        node_path: "/usr/local/bin/node",
        source: "workflow.define({})",
        provider_rebindings: [
          {
            node: "planner",
            provider: "opencode",
            model: "qwen3-coder",
          },
        ],
      },
    },
  )
  assert.deepEqual(
    runWorkflowCodeRequest(
      "session-1",
      "/usr/local/bin/node",
      "workflow.define({})",
      "Run this workflow.",
      {
        endpoint: "entry",
        queueRef: "default",
        agentRebindings: [{ node: "worker", agent_ref: "agent-1" }],
        providerRebindings: [
          {
            node: "planner",
            provider: "opencode",
            model: "qwen3-coder",
            account_profile: "work",
          },
        ],
      },
    ),
    {
      RunWorkflowCode: {
        session_id: "session-1",
        node_path: "/usr/local/bin/node",
        source: "workflow.define({})",
        provider_rebindings: [
          {
            node: "planner",
            provider: "opencode",
            model: "qwen3-coder",
            account_profile: "work",
          },
        ],
        agent_rebindings: [
          {
            node: "worker",
            agent_ref: "agent-1",
          },
        ],
        endpoint: "entry",
        queue_ref: "default",
        prompt: "Run this workflow.",
      },
    },
  )
  assert.deepEqual(
    createWorkflowCodeArtifactRequest("session-1", "toy-flow", "/usr/local/bin/node", "workflow.define({})"),
    {
      CreateWorkflowCodeArtifact: {
        session_id: "session-1",
        name: "toy-flow",
        language: "java_script",
        node_path: "/usr/local/bin/node",
        source: "workflow.define({})",
      },
    },
  )
  assert.deepEqual(
    applyWorkflowCodeArtifactRequest("session-1", "toy-flow", [
      {
        node: "planner",
        provider: "opencode",
        model: "qwen3-coder",
      },
    ]),
    {
      ApplyWorkflowCodeArtifact: {
        session_id: "session-1",
        name: "toy-flow",
        provider_rebindings: [
          {
            node: "planner",
            provider: "opencode",
            model: "qwen3-coder",
          },
        ],
      },
    },
  )
  assert.deepEqual(
    runWorkflowCodeArtifactRequest("session-1", "toy-flow", "Run this saved workflow.", {
      endpoint: "entry",
      queueRef: "default",
      providerRebindings: [
        {
          node: "planner",
          provider: "opencode",
          model: "qwen3-coder",
          account_profile: "work",
        },
      ],
    }),
    {
      RunWorkflowCodeArtifact: {
        session_id: "session-1",
        name: "toy-flow",
        provider_rebindings: [
          {
            node: "planner",
            provider: "opencode",
            model: "qwen3-coder",
            account_profile: "work",
          },
        ],
        endpoint: "entry",
        queue_ref: "default",
        prompt: "Run this saved workflow.",
      },
    },
  )
  assert.deepEqual(listWorkflowCodeArtifactsRequest("session-1"), {
    ListWorkflowCodeArtifacts: {
      session_id: "session-1",
    },
  })
  assert.deepEqual(deleteWorkflowCodeArtifactRequest("session-1", "toy-flow"), {
    DeleteWorkflowCodeArtifact: {
      session_id: "session-1",
      name: "toy-flow",
    },
  })
  assert.deepEqual(
    exportWorkflowCodeSourceRequest(
      "session-1",
      { kind: "workflow", workflow_ref: "workflow-1" },
      "inline",
      "existing_agents",
    ),
    {
      ExportWorkflowCodeSource: {
        session_id: "session-1",
        target: { kind: "workflow", workflow_ref: "workflow-1" },
        format: "inline",
        agent_mode: "existing_agents",
      },
    },
  )
  assert.deepEqual(exportWorkflowCodeArtifactRequest("session-1", "toy-flow"), {
    ExportWorkflowCodeArtifact: {
      session_id: "session-1",
      name: "toy-flow",
    },
  })
  assert.deepEqual(exportWorkflowCodePackageRequest("session-1", "toy-flow"), {
    ExportWorkflowCodePackage: {
      session_id: "session-1",
      name: "toy-flow",
    },
  })
  assert.deepEqual(
    exportWorkflowCodePackageRequest(
      "session-1",
      "workflow-package",
      { kind: "workflow", workflow_ref: "workflow-1" },
      "portable_generated",
    ),
    {
      ExportWorkflowCodePackage: {
        session_id: "session-1",
        name: "workflow-package",
        target: { kind: "workflow", workflow_ref: "workflow-1" },
        agent_mode: "portable_generated",
      },
    },
  )
  assert.deepEqual(
    importWorkflowCodeArtifactRequest("session-1", workflowCodePackage, "/usr/local/bin/node", {
      name: "imported-toy-flow",
      overwrite: true,
    }),
    {
      ImportWorkflowCodeArtifact: {
        session_id: "session-1",
        package: workflowCodePackage,
        name: "imported-toy-flow",
        overwrite: true,
        node_path: "/usr/local/bin/node",
      },
    },
  )
})
