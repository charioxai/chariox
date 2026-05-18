import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { mkdtemp, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { createCommandActionHandlers, formatAgentCapabilityGrants, formatAgentListSummary, parseMcpInstallConfig, parseRequestedViewLayout } from "./command-actions.js"
import type { AgentInstance, ProviderProcessInfo, QueuedWorkflowLaunch, RuntimeAttachment, RuntimeProviderRun, RuntimeSession, WorkflowDefinition, WorkflowRun } from "./cli-types.js"
import { makeAgent, makeCommandDeps, makeSession, runGit } from "./command-actions-test-support.js"

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

test("session new can create a local git worktree before attaching", async () => {
  const preparedWorktree = "/tmp/arroba-session-feature"
  const prepareCalls: Array<{ targetDirectory?: string; branch?: string; fromRef?: string }> = []
  const createCalls: Array<{ worktree: string; alias: string | undefined }> = []
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    prepareLocalGitWorktree: async (options: { targetDirectory?: string; branch?: string; fromRef?: string }) => {
      prepareCalls.push(options)
      return preparedWorktree
    },
    createSession: async (_workspace: string, worktree: string, alias?: string) => {
      createCalls.push({ worktree, alias })
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

  assert.equal(prepareCalls.length, 1)
  assert.equal(prepareCalls[0]?.targetDirectory, "../feature")
  assert.equal(prepareCalls[0]?.branch, "feature/session")
  assert.equal(prepareCalls[0]?.fromRef, "main")
  assert.deepEqual(createCalls, [{ worktree: preparedWorktree, alias: undefined }])
  assert.equal(flashedMessage, `attached to session session-worktree in ${preparedWorktree}`)
})

test("session new materializes a real local git worktree", async () => {
  const repo = await mkdtemp(join(tmpdir(), "arroba-local-worktree-repo-"))
  const target = await mkdtemp(join(tmpdir(), "arroba-local-worktree-parent-"))
  const targetWorktree = join(target, "feature-local")
  runGit(repo, ["init", "-b", "main"])
  runGit(repo, ["config", "user.email", "arroba@example.test"])
  runGit(repo, ["config", "user.name", "Arroba Test"])
  await writeFile(join(repo, "README.md"), "local worktree\n", "utf8")
  runGit(repo, ["add", "README.md"])
  runGit(repo, ["commit", "-m", "init"])

  const createCalls: Array<{ worktree: string }> = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    workspace: repo,
    worktree: repo,
    createSession: async (_workspace: string, worktree: string) => {
      createCalls.push({ worktree })
      return { id: "session-worktree", alias: null }
    },
  }))

  await handlers.handleSessionCommand({
    kind: "session",
    raw: `/session new --worktree ${targetWorktree} --branch feature/local-drill --from main`,
    action: "new",
    args: ["--worktree", targetWorktree, "--branch", "feature/local-drill", "--from", "main"],
    value: `--worktree ${targetWorktree} --branch feature/local-drill --from main`,
  })

  assert.deepEqual(createCalls, [{ worktree: targetWorktree }])
  assert.equal(execFileSync("git", ["branch", "--show-current"], { cwd: targetWorktree, encoding: "utf8" }).trim(), "feature/local-drill")
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
