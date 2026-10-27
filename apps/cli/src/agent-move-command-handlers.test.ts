import assert from "node:assert/strict"
import test from "node:test"

import { handleAgentMoveCommand, type AgentMoveCommandHandlerDeps } from "./agent-move-command-handlers.js"

type Status = { state: "waiting_for_turn" | "moving" } | { state: "failed"; reason: string } | null

function harness(overrides: Partial<AgentMoveCommandHandlerDeps> = {}) {
  const flashed: { message: string; tone: string }[] = []
  const moved: (string | null)[] = []
  const cancelled: string[] = []
  const answers: { resolve: (agent: unknown) => void; refuse: (message: string) => void }[] = []
  const state = { status: null as Status, polls: 0 }
  const deps: AgentMoveCommandHandlerDeps = {
    flashFooter: (message, tone) => { flashed.push({ message, tone }) },
    formatError: error => (error instanceof Error ? error.message : String(error)),
    resolveSessionAgent: reference => ({ agent: { id: "agent-1", agent_ref: reference ?? "reviewer" } as never, error: null }),
    formatAgentLabel: agent => agent?.agent_ref ?? "",
    refreshSessionState: async () => { state.polls += 1; return { id: "room", agents: [{ id: "agent-1", agent_move: state.status }] } as never },
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    rebuildTranscript: () => {},
    sessionState: () => ({ id: "room" } as never),
    moveAgent: (_agent, destination) => new Promise((resolve, reject) => { moved.push(destination); answers.push({ resolve: resolve as never, refuse: message => reject(new Error(message)) }) }),
    cancelAgentMove: async agent => { cancelled.push(agent) },
    pause: async () => { await new Promise(resolve => setTimeout(resolve, 0)) },
    ...overrides,
  }
  return { deps, flashed, moved, cancelled, answers, state }
}
const until = async (condition: () => boolean) => {
  for (let i = 0; i < 200 && !condition(); i += 1) await new Promise(resolve => setTimeout(resolve, 1))
  assert.ok(condition(), "condition not reached")
}

test("/agent move announces a move the kernel holds for the turn, then reports the kernel's outcome", async () => {
  const h = harness()
  await handleAgentMoveCommand(h.deps, ["move", "kernel-b3"])
  assert.deepEqual(h.moved, ["kernel-b3"])
  h.state.status = { state: "waiting_for_turn" }
  h.answers[0]!.resolve({ id: "agent-1", agent_move: h.state.status })
  await until(() => h.flashed.at(-1)?.message.includes("after its current turn") === true)
  assert.equal(h.flashed.at(-1)?.message, "moving reviewer to kernel-b3 after its current turn; /agent move cancel stops it")
  // The request is long over: the outcome comes from the agent's published status.
  await until(() => h.state.polls >= 2)
  h.state.status = null
  await until(() => h.flashed.at(-1)?.message === "move of reviewer to kernel-b3 finished")
})

test("/agent move reports the reason a held move failed", async () => {
  const h = harness()
  await handleAgentMoveCommand(h.deps, ["move", "home"])
  h.state.status = { state: "failed", reason: "Project setup failed" }
  h.answers[0]!.resolve({ id: "agent-1", agent_move: { state: "waiting_for_turn" } })
  await until(() => h.flashed.at(-1)?.tone === "error")
  assert.equal(h.flashed.at(-1)?.message, "move of reviewer home failed: Project setup failed; /agent move cancel dismisses it")
})

test("/agent move reports an immediate move, a cancelled wait and a refusal", async () => {
  const h = harness()
  await handleAgentMoveCommand(h.deps, ["move", "kernel-b3"])
  h.answers[0]!.resolve({ id: "agent-1" })
  await until(() => h.flashed.at(-1)?.message === "moved reviewer to kernel-b3")
  await handleAgentMoveCommand(h.deps, ["move", "cancel", "reviewer"])
  assert.deepEqual(h.cancelled, ["agent-1"])
  assert.equal(h.flashed.at(-1)?.message, "cancelled the move of reviewer")
  await handleAgentMoveCommand(h.deps, ["move"])
  assert.equal(h.flashed.at(-1)?.tone, "error")
  await handleAgentMoveCommand(h.deps, ["move", "kernel-b3"])
  h.answers[1]!.refuse("Return this agent home before moving it to another kernel")
  await until(() => h.flashed.at(-1)?.tone === "error")
  assert.equal(h.flashed.at(-1)?.message, "Return this agent home before moving it to another kernel")
})
