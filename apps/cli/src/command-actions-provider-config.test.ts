import assert from "node:assert/strict"
import { mkdtemp, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { createCommandActionHandlers, formatAgentCapabilityGrants, formatAgentListSummary, parseMcpInstallConfig, parseRequestedViewLayout } from "./command-actions.js"
import type { AgentInstance, ProviderProcessInfo, WorkflowQueuedPrompt, RuntimeAttachment, RuntimeProviderRun, RuntimeSession, WorkflowDefinition, WorkflowRun } from "./cli-types.js"
import { makeAgent, makeCommandDeps, makeSession, runGit } from "./command-actions-test-support.js"

test("provider command can switch backends and manage codex auth", async () => {
  const events: string[] = []
  let flashedMessage = ""
  let notice = ""

  const handlers = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => makeSession(),
    attachmentState: (): RuntimeAttachment => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: () => null,
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    focusedAgentId: () => "agent-1",
    multiAgentResponseLayout: () => "split",
    maxAgentsPerScreen: () => 3,
    flashFooter: (message) => { flashedMessage = message },
    appendNotice: (message) => { notice = message },
    formatError: (error) => String(error),
    createSession: async () => ({ id: "session-1", alias: null }),
    attachBinding: async () => {},
    resolveSession: async () => ({ id: "session-1", alias: null }),
    listSessions: async () => [],
    deleteSessionByRef: async () => ({ id: "session-1", alias: null }),
    transitionToNoSession: () => {},
    applyProviderSelection: async (value) => { events.push(`provider:${value}`) },
    applyModelSelection: async () => {},
    applyVariantSelection: async () => {},
    getProviderAuthStatus: async () => ({
      provider: "codex",
      auth_state: "authenticated",
      account_profile: "user@example.com",
      login_hint: null,
      detected_version: "codex-cli 0.118.0",
    }),
    startProviderLogin: async () => ({
      provider: "codex",
      login_kind: "chatgptDeviceCode",
      login_id: "login-1",
      auth_url: null,
      verification_url: "https://auth.openai.com/codex/device",
      user_code: "ABCD-1234",
    }),
    logoutProvider: async (provider) => ({ provider }),
    setMultiAgentResponseLayout: () => {},
    applyResponseLayout: () => {},
    updateSessionResponseLayout: async () => ({ session: makeSession(), config: makeSession().config_state }),
    updateSessionConfig: async () => ({ session: makeSession(), config: makeSession().config_state }),
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: makeSession() }),
    launchAgentProviderRun: async () => { throw new Error("unused") },
    setProviderRunState: () => {},
    refreshSessionState: async () => makeSession(),
    spawnAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    destroyAgent: async () => makeSession(),
    focusAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    resolveSessionAgent: () => ({ agent: makeAgent() }),
    workflowScreenActive: () => false,
    showWorkflowScreen: () => {},
    selectWorkflowCanvas: () => {},
    replaceWorkflowDefinitions: () => {},
    upsertWorkflowDefinition: () => {},
    createWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    listWorkflows: async () => [],
    resolveWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null } }),
    assignWorkflowAlias: async () => null,
    createWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    assignWorkflowEndpointAlias: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    bindWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
  })

  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider codex", value: "codex" })
  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider status", value: "status" })
  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider login", value: "login" })
  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider logout", value: "logout" })
  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider reauth", value: "reauth" })

  assert.deepEqual(events, ["provider:codex"])
  assert.equal(flashedMessage, "codex reauth started • code ABCD-1234 • https://auth.openai.com/codex/device")
  assert.equal(notice, "codex reauth started • code ABCD-1234 • https://auth.openai.com/codex/device")
})

test("config command renders kernel mutation effects", async () => {
  const notices: string[] = []
  const flashes: string[] = []
  const updates: Array<{ path: string; value: string }> = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    appendNotice: (message: string) => { notices.push(message) },
    flashFooter: (message: string) => { flashes.push(message) },
    getUserConfigSchema: async () => ({
      entries: [
        {
          path: "providers.workspace_live_sync",
          value_type: "enum",
          allowed_values: ["required", "unrestricted"],
          settable: true,
          unsettable: true,
          effect: "provider_reload",
          status: "live",
          description: "Global workspace live sync policy.",
        },
      ],
    }),
    setUserConfigValue: async (path: string, value: string) => {
      updates.push({ path, value })
      return {
        path: "/home/.arroba/config.toml",
        config: { version: 1, providers: { workspace_live_sync: value as "required" | "unrestricted" } },
        effects: [
          {
            kind: "provider_reload",
            path,
            message: "workspace live sync policy updated; provider reloads: 1 reloaded, 0 deferred, 0 unaffected",
            provider_reload: { reloaded: 1, deferred: 0, unaffected: 0 },
          },
        ],
      }
    },
  }))

  await handlers.handleConfigCommand({
    kind: "config",
    raw: "/config keys",
    args: ["keys"],
  })
  await handlers.handleConfigCommand({
    kind: "config",
    raw: "/config workspace-live-sync off",
    args: ["workspace-live-sync", "off"],
  })

  assert.deepEqual(updates, [{ path: "providers.workspace_live_sync", value: "unrestricted" }])
  assert.deepEqual(notices, [
    "providers.workspace_live_sync (enum; live; provider_reload unset values=required|unrestricted)",
    "workspace live sync policy updated; provider reloads: 1 reloaded, 0 deferred, 0 unaffected",
  ])
  assert.deepEqual(flashes, ["listed 1 config key", "workspace live sync set to unrestricted"])
})

test("provider processes command lists and tears down safe daemon-tracked processes", async () => {
  let flashedMessage = ""
  let notice = ""
  let listedProvider: string | null | undefined = undefined
  let tornDownProvider: string | null | undefined = undefined

  const handlers = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => makeSession(),
    attachmentState: (): RuntimeAttachment => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: () => null,
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    focusedAgentId: () => "agent-1",
    multiAgentResponseLayout: () => "split",
    maxAgentsPerScreen: () => 3,
    flashFooter: (message) => { flashedMessage = message },
    appendNotice: (message) => { notice = message },
    formatError: (error) => String(error),
    createSession: async () => ({ id: "session-1", alias: null }),
    attachBinding: async () => {},
    resolveSession: async () => ({ id: "session-1", alias: null }),
    listSessions: async () => [],
    deleteSessionByRef: async () => ({ id: "session-1", alias: null }),
    transitionToNoSession: () => {},
    applyProviderSelection: async () => {},
    applyModelSelection: async () => {},
    applyVariantSelection: async () => {},
    getProviderAuthStatus: async () => ({
      provider: "codex",
      auth_state: "authenticated",
      account_profile: "user@example.com",
      login_hint: null,
      detected_version: "codex-cli 0.118.0",
    }),
    startProviderLogin: async () => ({
      provider: "codex",
      login_kind: "chatgptDeviceCode",
      login_id: "login-1",
      auth_url: null,
      verification_url: "https://auth.openai.com/codex/device",
      user_code: "ABCD-1234",
    }),
    logoutProvider: async (provider) => ({ provider }),
    listProviderProcesses: async (provider) => {
      listedProvider = provider
      return [
        {
          process_id: "codex:shared-token",
          provider: "codex",
          process_label: "codex:gpt-5.4",
          pid: 4321,
          endpoint_mode: "managed",
          status: "idle",
          started_at_ms: 1,
          last_activity_at_ms: 2,
          provider_session_ids: ["thread-1"],
          owner_session_ids: ["session-1"],
          owner_provider_run_ids: ["provider-run-1"],
          attached_session_ids: [],
          active_workflow_run_ids: [],
          teardown_safe: true,
          teardown_blockers: [],
        },
      ]
    },
    teardownProviderProcesses: async (provider) => {
      tornDownProvider = provider
      return [
        {
          process_id: "codex:shared-token",
          provider: "codex",
          process_label: "codex:gpt-5.4",
          pid: 4321,
          endpoint_mode: "managed",
          status: "idle",
          started_at_ms: 1,
          last_activity_at_ms: 2,
          provider_session_ids: ["thread-1"],
          owner_session_ids: ["session-1"],
          owner_provider_run_ids: ["provider-run-1"],
          attached_session_ids: [],
          active_workflow_run_ids: [],
          teardown_safe: true,
          teardown_blockers: [],
        },
      ]
    },
    setMultiAgentResponseLayout: () => {},
    applyResponseLayout: () => {},
    updateSessionResponseLayout: async () => ({ session: makeSession(), config: makeSession().config_state }),
    updateSessionConfig: async () => ({ session: makeSession(), config: makeSession().config_state }),
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: makeSession() }),
    launchAgentProviderRun: async () => { throw new Error("unused") },
    setProviderRunState: () => {},
    refreshSessionState: async () => makeSession(),
    spawnAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    destroyAgent: async () => makeSession(),
    focusAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    resolveSessionAgent: () => ({ agent: makeAgent() }),
    workflowScreenActive: () => false,
    showWorkflowScreen: () => {},
    selectWorkflowCanvas: () => {},
    replaceWorkflowDefinitions: () => {},
    upsertWorkflowDefinition: () => {},
    createWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    listWorkflows: async () => [],
    resolveWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null } }),
    assignWorkflowAlias: async () => null,
    createWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    assignWorkflowEndpointAlias: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    bindWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
  })

  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider processes codex", value: "processes codex" })
  assert.equal(listedProvider, "codex")
  assert.equal(flashedMessage, "listed 1 provider process(es)")
  assert.match(notice, /codex:shared-token provider=codex pid=4321 status=idle mode=managed safe=true/)
  assert.match(notice, /provider sessions: thread-1/)
  assert.match(notice, /owner runs: provider-run-1/)

  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider processes teardown codex", value: "processes teardown codex" })
  assert.equal(tornDownProvider, "codex")
  assert.equal(flashedMessage, "tore down 1 provider process(es)")
  assert.match(notice, /codex:shared-token provider=codex pid=4321 status=idle mode=managed safe=true/)
})

test("provider processes teardown reports blocked daemon-tracked processes", async () => {
  let flashedMessage = ""
  let notices: string[] = []

  const blockedProcess: ProviderProcessInfo = {
    process_id: "codex:blocked",
    provider: "codex",
    process_label: "codex:gpt-5.4",
    pid: 5555,
    endpoint_mode: "managed",
    status: "active",
    started_at_ms: 1,
    last_activity_at_ms: 2,
    provider_session_ids: ["thread-blocked"],
    owner_session_ids: ["session-1"],
    owner_provider_run_ids: ["provider-run-blocked"],
    attached_session_ids: ["session-1"],
    active_workflow_run_ids: [],
    teardown_safe: false,
    teardown_blockers: ["attached sessions: session-1"],
  }

  const handlers = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => makeSession(),
    attachmentState: (): RuntimeAttachment => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: () => null,
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    focusedAgentId: () => "agent-1",
    multiAgentResponseLayout: () => "split",
    maxAgentsPerScreen: () => 3,
    flashFooter: (message) => { flashedMessage = message },
    appendNotice: (message) => { notices.push(message) },
    formatError: (error) => String(error),
    createSession: async () => ({ id: "session-1", alias: null }),
    attachBinding: async () => {},
    resolveSession: async () => ({ id: "session-1", alias: null }),
    listSessions: async () => [],
    deleteSessionByRef: async () => ({ id: "session-1", alias: null }),
    transitionToNoSession: () => {},
    applyProviderSelection: async () => {},
    applyModelSelection: async () => {},
    applyVariantSelection: async () => {},
    getProviderAuthStatus: async () => ({ provider: "codex", auth_state: "authenticated", account_profile: null, login_hint: null, detected_version: null }),
    startProviderLogin: async () => ({ provider: "codex", login_kind: "chatgptDeviceCode", login_id: "login-1", auth_url: null, verification_url: null, user_code: null }),
    logoutProvider: async (provider) => ({ provider }),
    listProviderProcesses: async () => [blockedProcess],
    teardownProviderProcesses: async () => [],
    setMultiAgentResponseLayout: () => {},
    applyResponseLayout: () => {},
    updateSessionResponseLayout: async () => ({ session: makeSession(), config: makeSession().config_state }),
    updateSessionConfig: async () => ({ session: makeSession(), config: makeSession().config_state }),
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: makeSession() }),
    launchAgentProviderRun: async () => { throw new Error("unused") },
    setProviderRunState: () => {},
    refreshSessionState: async () => makeSession(),
    spawnAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    destroyAgent: async () => makeSession(),
    focusAgent: async () => ({ agent: makeAgent(), session: makeSession() }),
    resolveSessionAgent: () => ({ agent: makeAgent() }),
    workflowScreenActive: () => false,
    showWorkflowScreen: () => {},
    selectWorkflowCanvas: () => {},
    replaceWorkflowDefinitions: () => {},
    upsertWorkflowDefinition: () => {},
    createWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    listWorkflows: async () => [],
    resolveWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null } }),
    assignWorkflowAlias: async () => null,
    createWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    assignWorkflowEndpointAlias: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    bindWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
  })

  await handlers.handleProviderCommand({ kind: "provider", raw: "/provider processes teardown codex", value: "processes teardown codex" })
  assert.equal(flashedMessage, "no safe provider processes to tear down")
  assert.match(notices[0]!, /blocked provider processes:/)
  assert.match(notices[0]!, /blockers: attached sessions: session-1/)
})
