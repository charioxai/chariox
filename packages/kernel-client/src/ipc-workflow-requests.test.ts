import assert from "node:assert/strict"
import test from "node:test"

import {
  createWorkflowCodeArtifactRequest,
  deleteWorkflowCodeArtifactRequest,
  exportWorkflowPublicationPackageRequest,
  listWorkflowCodeArtifactsRequest,
  setWorkflowNodeWaitForAllInputsRequest,
} from "./ipc-workflow-requests.js"

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

test("workflow-code artifact requests match kernel shape", () => {
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
})
