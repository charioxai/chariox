import {
  assert,
  createCommandActionHandlers,
  makeAgent,
  makeCommandDeps,
  makeSession,
  test,
} from "../command-actions-agent.test-support.js"
import type { AgentInstance, RuntimeAttachment, RuntimeProviderRun, RuntimeSession } from "../command-actions-agent.test-support.js"

test("agent command usage advertises hierarchical spawn placement", async () => {
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    flashFooter: (message: string) => {
      flashedMessage = message
    },
  }))

  await handlers.handleAgentCommand({ kind: "agent", raw: "/agent", args: [] })

  assert.match(flashedMessage, /--machine <machine-ref>\|--kernel <kernel-ref>\|--slice off\|new:headless\|new:headed\|<slice-ref>/)
  assert.doesNotMatch(flashedMessage, /--slice-display/)
})

test("undo command sends optional agent refs to the kernel request path", async () => {
  const calls: Array<string | null> = []
  const notices: string[] = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    undoTurn: async (agentRef?: string | null) => {
      calls.push(agentRef ?? null)
      return {
        session_id: "session-1",
        agent_id: agentRef ?? "agent-1",
        turn_id: "turn-1",
        prompt_id: "prompt-1",
        provider_run_id: "provider-run-1",
        reverted_paths: ["src/lib.ts"],
        path_results: [{ path: "src/lib.ts", status: "applied", message: "restored" }],
      }
    },
    appendNotice: (message: string) => {
      notices.push(message)
    },
  }))

  await handlers.handleUndoCommand({ kind: "undo", raw: "/undo", args: [] })
  await handlers.handleUndoCommand({ kind: "undo", raw: "/undo qa", args: ["qa"] })

  assert.deepEqual(calls, [null, "qa"])
  assert.match(notices.at(0) ?? "", /undid turn turn-1 for agent-1/)
  assert.match(notices.at(1) ?? "", /undid turn turn-1 for qa/)
})

test("fork commands send optional refs and apply the forked provider run", async () => {
  const calls: Array<string | null> = []
  const providerRunIds: Array<string | null> = []
  const notices: string[] = []
  const forkedAgent = makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "fork" })
  const forkedSession = makeSession({ focused_agent_id: forkedAgent.id, agents: [makeAgent(), forkedAgent] })
  const handlers = createCommandActionHandlers(makeCommandDeps({
    forkAgent: async (sourceAgentRef?: string | null) => {
      calls.push(sourceAgentRef ?? null)
      return {
        source_agent_id: sourceAgentRef ?? "agent-1",
        agent: forkedAgent,
        provider_run: {
          id: "provider-run-2",
          session_id: "session-1",
          agent_instance_id: forkedAgent.id,
          adapter_key: "opencode",
          provider: "opencode",
          account_profile: "default",
          model: "openai/gpt-5",
          variant: "medium",
          usage_tokens_total: null,
          state: "running",
        },
        session: forkedSession,
      }
    },
    setProviderRunState: (run: RuntimeProviderRun | null) => {
      providerRunIds.push(run?.id ?? null)
    },
    appendNotice: (message: string) => {
      notices.push(message)
    },
  }))

  await handlers.handleForkCommand({ kind: "fork", raw: "/fork", args: [] })
  await handlers.handleAgentCommand({ kind: "agent", raw: "/agent fork qa", args: ["fork", "qa"] })

  assert.deepEqual(calls, [null, "qa"])
  assert.deepEqual(providerRunIds, ["provider-run-2", "provider-run-2"])
  assert.match(notices.at(0) ?? "", /forked agent-1 as agent-2/)
  assert.match(notices.at(1) ?? "", /forked qa as agent-2/)
})

test("agent task command updates focused Meta mode task", async () => {
  const metaagent = makeAgent({ meta_mode: { activated_at_ms: 1 }, alias: "planner" })
  const nextSession = makeSession({
    agents: [metaagent],
    metaagent_tasks: [{
      task_id: "task-1",
      metaagent_id: metaagent.id,
      status: "active",
      task_markdown: "Fix tests",
      plan_markdown: "",
      revision: 1,
      created_at_ms: 1,
      updated_at_ms: 2,
    }],
  })
  const calls: string[] = []
  let appliedSession: RuntimeSession | null = null
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    sessionState: () => makeSession({ agents: [metaagent], focused_agent_id: metaagent.id }),
    resolveSessionAgent: (reference?: string | null) => (
      !reference || reference === metaagent.id || reference === metaagent.agent_ref
        ? { agent: metaagent }
        : { agent: null, error: `agent '${reference}' not found` }
    ),
    updateMetaagentTask: async (_sessionId: string, metaagentId: string, updates: Record<string, unknown>) => {
      calls.push(`${metaagentId}:${updates.taskMarkdown}`)
      return nextSession
    },
    applySessionState: (session: RuntimeSession) => {
      appliedSession = session
    },
    flashFooter: (message: string) => {
      flashedMessage = message
    },
  }))

  await handlers.handleAgentCommand({ kind: "agent", raw: "/agent task edit Fix tests", args: ["task", "edit", "Fix", "tests"] })

  assert.deepEqual(calls, ["agent-1:Fix tests"])
  assert.equal(appliedSession, nextSession)
  assert.match(flashedMessage, /updated task for agent-1/)
})

test("agent task command rejects regular agents", async () => {
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    flashFooter: (message: string) => {
      flashedMessage = message
    },
  }))

  await handlers.handleAgentCommand({ kind: "agent", raw: "/agent task pause", args: ["task", "pause"] })

  assert.match(flashedMessage, /agent-1 is not in meta mode/)
})

