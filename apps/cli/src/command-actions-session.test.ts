import assert from "node:assert/strict"
import { mkdtemp } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { createCommandActionHandlers, formatAgentCapabilityGrants, formatAgentListSummary, parseMcpInstallConfig, parseRequestedViewLayout } from "./command-actions.js"
import type { AgentInstance, ProviderProcessInfo, WorkflowQueuedPrompt, RuntimeAttachment, RuntimeProviderRun, RuntimeSession, WorkflowDefinition, WorkflowRun } from "./cli-types.js"
import { makeAgent, makeCommandDeps, makeSession } from "./command-actions-test-support.js"

test("session new can attach a new session in an existing directory", async () => {
  const sessionDir = await mkdtemp(join(tmpdir(), "arroba-session-dir-"))
  const createCalls: Array<{ workspace: string; worktree: string; alias: string | undefined }> = []
  let attachedSession: Pick<RuntimeSession, "id"> | null = null
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    createSession: async (workspace: string, worktree: string, alias?: string) => {
      createCalls.push({ workspace, worktree, alias })
      return { id: "session-dir", alias: null }
    },
    attachBinding: async (session: Pick<RuntimeSession, "id">) => {
      attachedSession = session
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleSessionCommand({
    kind: "session",
    raw: `/session new ${sessionDir}`,
    action: "new",
    args: [sessionDir],
    value: sessionDir,
  })

  assert.deepEqual(createCalls, [{ workspace: process.cwd(), worktree: sessionDir, alias: undefined }])
  assert.deepEqual(attachedSession, { id: "session-dir", alias: null })
  assert.equal(flashedMessage, `attached to session session-dir in ${sessionDir}`)
})

test("session new forwards local git worktree placement to the kernel", async () => {
  const createCalls: Array<{ worktree: string; alias: string | undefined; placement: unknown }> = []
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    createSession: async (_workspace: string, worktree: string, alias?: string, _agentDefaults?: RuntimeSession["agent_defaults"], placement?: unknown) => {
      createCalls.push({ worktree, alias, placement })
      return { id: "session-worktree", alias: null }
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleSessionCommand({
    kind: "session",
    raw: "/session new --worktree ../feature --branch feature/session --from main",
    action: "new",
    args: ["--worktree", "../feature", "--branch", "feature/session", "--from", "main"],
    value: "--worktree ../feature --branch feature/session --from main",
  })

  assert.deepEqual(createCalls, [{
    worktree: process.cwd(),
    alias: undefined,
    placement: {
      target_directory: "../feature",
      branch: "feature/session",
      from_ref: "main",
    },
  }])
  assert.equal(flashedMessage, "attached to session session-worktree")
})

  test("session command aliases the current session", async () => {
    let flashedMessage = ""
    let aliasedPayload: { sessionId: string; alias: string } | null = null
    let appliedSession: Pick<RuntimeSession, "alias"> | null = null
    const currentSession = makeSession()
    const handlers = createCommandActionHandlers({
      workspace: "workspace-1",
      worktree: "worktree-1",
      accountProfile: "default",
      isAttached: () => true,
      sessionState: () => currentSession,
      attachmentState: () => ({ id: "attachment-1", session_id: "session-1" }),
      providerRunState: () => null,
      currentModelId: () => "openai/gpt-5",
      currentVariantId: () => "medium",
      currentProviderId: () => "opencode",
      focusedAgentId: () => currentSession.focused_agent_id,
      multiAgentResponseLayout: () => "split",
      maxAgentsPerScreen: () => 3,
      flashFooter: (message) => { flashedMessage = message },
      appendNotice: () => {},
      formatError: (error) => String(error),
      createSession: async () => ({ id: "session-1", alias: null }),
      attachBinding: async () => {},
      resolveSession: async () => ({ id: "session-1", alias: null }),
      listSessions: async () => [],
      deleteSessionByRef: async () => ({ id: "session-1", alias: null }),
      transitionToNoSession: () => {},
      applyModelSelection: async () => {},
      applyVariantSelection: async () => {},
      setMultiAgentResponseLayout: () => {},
      applyResponseLayout: () => {},
      updateSessionResponseLayout: async () => ({
        session: currentSession,
        config: currentSession.config_state,
      }),
      updateSessionConfig: async () => ({ session: currentSession, config: currentSession.config_state }),
      assignSessionAlias: async (sessionId, alias) => {
        aliasedPayload = { sessionId, alias }
        return { ...currentSession, alias }
      },
      applySessionState: (session) => {
        appliedSession = session
      },
      refreshAgentPanes: async () => {},
      saveUiPreferences: async () => {},
      rebuildTranscript: () => {},
      requestRender: () => {},
      cycleAgentFocus: async () => ({ agent: null, session: currentSession }),
      launchAgentProviderRun: async () => {
        throw new Error("unused")
      },
      setProviderRunState: () => {},
      refreshSessionState: async () => currentSession,
      spawnAgent: async () => ({ agent: makeAgent(), session: currentSession }),
      destroyAgent: async () => currentSession,
      focusAgent: async () => ({ agent: makeAgent(), session: currentSession }),
      resolveSessionAgent: () => ({ agent: null }),
      workflowScreenActive: () => false,
      showWorkflowScreen: () => {},
      selectWorkflowCanvas: () => {},
      replaceWorkflowDefinitions: () => {},
      upsertWorkflowDefinition: () => {},
      createWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null }, session: currentSession }),
      listWorkflows: async () => [],
      resolveWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null } }),
      assignWorkflowAlias: async () => null,
      createWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
      assignWorkflowEndpointAlias: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
      bindWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
      addWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
      removeWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
      addWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
      removeWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
      formatAgentLabel: (agent) => agent?.agent_ref ?? "",
      refreshSplitPaneFocusRepaint: () => {},
      formatSessionList: () => "",
    })

  await handlers.handleSessionCommand({
    kind: "session",
    raw: "/session work-session",
    action: "work-session",
    args: [],
    value: "work-session",
  })

  assert.deepEqual(aliasedPayload, { sessionId: "session-1", alias: "work-session" })
  assert.equal(
    (appliedSession as (Pick<RuntimeSession, "alias"> | null))?.alias,
    "work-session",
  )
  assert.equal(flashedMessage, "session session-1 aliased as work-session")
})
