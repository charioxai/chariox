import assert from "node:assert/strict"
import test from "node:test"

import { handleAgentMoveCommand, type AgentMoveCommandHandlerDeps } from "./agent-move-command-handlers.js"

function harness(overrides: Partial<AgentMoveCommandHandlerDeps> = {}) {
  const flashed: { message: string; tone: string }[] = []
  const moved: (string | null)[] = []
  const cancelled: string[] = []
  const settle: { finish: () => void; refuse: (message: string) => void }[] = []
  const deps: AgentMoveCommandHandlerDeps = {
    flashFooter: (message, tone) => { flashed.push({ message, tone }) },
    formatError: error => (error instanceof Error ? error.message : String(error)),
    resolveSessionAgent: reference => ({ agent: { id: "agent-1", agent_ref: reference ?? "reviewer" } as never, error: null }),
    formatAgentLabel: agent => agent?.agent_ref ?? "",
    refreshSessionState: async () => ({ id: "room" } as never),
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    rebuildTranscript: () => {},
    sessionState: () => ({ id: "room" } as never),
    moveAgent: (_agent, destination) => new Promise<void>((resolve, reject) => { moved.push(destination); settle.push({ finish: resolve, refuse: message => reject(new Error(message)) }) }),
    cancelAgentMove: async agent => { cancelled.push(agent) },
    ...overrides,
  }
  return { deps, flashed, moved, cancelled, settle }
}
const tick = () => new Promise(resolve => setTimeout(resolve, 0))

test("/agent move announces a waiting move at once, then reports it moved", async () => {
  const h = harness()
  await handleAgentMoveCommand(h.deps, ["move", "kernel-b3"])
  assert.deepEqual(h.moved, ["kernel-b3"])
  assert.deepEqual(h.flashed, [{ message: "moving reviewer to kernel-b3 after its current turn; /agent move cancel stops it", tone: "info" }])
  h.settle[0]!.finish()
  await tick()
  assert.equal(h.flashed.at(-1)?.message, "moved reviewer to kernel-b3")
})

test("/agent move home returns the agent and a cancelled wait is not an error", async () => {
  const h = harness()
  await handleAgentMoveCommand(h.deps, ["move", "home", "reviewer"])
  assert.deepEqual(h.moved, [null])
  await handleAgentMoveCommand(h.deps, ["move", "cancel", "reviewer"])
  assert.deepEqual(h.cancelled, ["agent-1"])
  assert.equal(h.flashed.at(-1)?.message, "cancelled the move of reviewer")
  h.settle[0]!.refuse("Move cancelled before the agent's turn ended")
  await tick()
  assert.deepEqual(h.flashed.at(-1), { message: "move of reviewer cancelled", tone: "info" })
})

test("/agent move reports a refusal and a missing destination", async () => {
  const h = harness()
  await handleAgentMoveCommand(h.deps, ["move"])
  assert.equal(h.flashed.at(-1)?.tone, "error")
  await handleAgentMoveCommand(h.deps, ["move", "kernel-b3"])
  h.settle[0]!.refuse("Return this agent home before moving it to another kernel")
  await tick()
  assert.deepEqual(h.flashed.at(-1), { message: "Return this agent home before moving it to another kernel", tone: "error" })
})
