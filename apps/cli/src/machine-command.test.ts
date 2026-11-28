// The owner, or an agent acting for them, acquires and releases a machine from a shell, through the
// kernel's own acquisition request: the same options, choices and refusals as chariox.machines.*.
import assert from "node:assert/strict"
import { test } from "node:test"
import { runMachineCommand } from "./machine-command.js"
import { replyWith, startFakeKernel, type FakeKernelFrame } from "./fake-unix-kernel.test-support.js"
import type { WebSocket } from "ws"

const machine = { kind: "machine", allocation_ref: "alloc-1", revision: 1, state: "creating", available_actions: [], kernel_target: null, access_state: "pending", access_reason: null }
const rented = { allocation: machine, option: { class_id: "agent-small", region: "fsn1", available: true }, estimate: { kind: "unknown", reason: "x" } }
const card = { session_id: undefined, interaction_id: "machine-acquisition-abc", title: "t", message: "Acquire agent-small?", choices: [{ id: "approve", label: "Approve" }] }

type Handler = (socket: WebSocket, frame: FakeKernelFrame) => void

async function run(argv: string[], handler: Handler) {
  const kernel = await startFakeKernel((socket, frame) => handler(socket, frame))
  const previous = { out: console.log, error: console.error, exit: process.exitCode }
  let out = ""
  let err = ""
  const write = process.stderr.write
  console.log = (line: string) => { out += line }
  console.error = (line: string) => { err += line }
  process.stderr.write = ((chunk: string) => { err += chunk; return true }) as typeof process.stderr.write
  try {
    await runMachineCommand([...argv, "--socket", kernel.path])
    return { out, err, exit: process.exitCode as number | undefined, requests: kernel.frames.map(frame => frame.request) }
  } finally {
    console.log = previous.out
    console.error = previous.error
    process.stderr.write = write
    process.exitCode = previous.exit
    await kernel.close()
  }
}

const answer = (result: unknown): Handler => (socket, frame) => replyWith(socket, frame, { response: { MachineAcquisition: { result } } })

test("machine options asks for the options and prints them", async () => {
  const options = { options: [{ class_id: "agent-small", region: "fsn1", available: true }], default: null }
  const result = await run(["machine", "options"], answer(options))
  assert.deepEqual(result.requests, [{ MachineAcquisition: { action: "options" } }])
  assert.deepEqual(JSON.parse(result.out), options)
})

test("machine acquire sends the class, region and request id and prints the allocation", async () => {
  const result = await run(["machine", "acquire", "agent-small", "--region", "fsn1", "--request-id", "try-1"], answer(rented))
  assert.deepEqual(result.requests, [{ MachineAcquisition: { action: "acquire", class_id: "agent-small", region: "fsn1", request_id: "try-1" } }])
  assert.equal(JSON.parse(result.out).allocation.allocation_ref, "alloc-1")
  assert.equal(result.exit, undefined)
})

test("machine acquire without a region or request id leaves them to the kernel", async () => {
  const result = await run(["machine", "acquire", "agent-small"], answer(rented))
  assert.deepEqual(result.requests, [{ MachineAcquisition: { action: "acquire", class_id: "agent-small" } }])
})

test("machine acquire says it waits for the owner's card, then prints the allocation", async () => {
  let rentFrame: FakeKernelFrame | undefined
  const result = await run(["machine", "acquire", "agent-small"], (socket, frame) => {
    if ("ListOwnerInteractions" in frame.request) {
      replyWith(socket, frame, { response: { OwnerInteractionsListed: { interactions: [card] } } })
      // The owner answers once the CLI has shown the card.
      setTimeout(() => rentFrame && replyWith(socket, rentFrame, { response: { MachineAcquisition: { result: rented } } }), 100)
    } else rentFrame = frame
  })
  assert.match(result.err, /Waiting for the owner to approve this machine \(card machine-acquisition-abc\)/)
  assert.equal(JSON.parse(result.out).allocation.allocation_ref, "alloc-1")
})

test("machine status and release name the allocation", async () => {
  const status = await run(["machine", "status", "alloc-1"], answer(machine))
  assert.deepEqual(status.requests, [{ MachineAcquisition: { action: "status", allocation_ref: "alloc-1" } }])
  const release = await run(["machine", "release", "alloc-1"], answer({ ...machine, state: "deleting" }))
  assert.deepEqual(release.requests, [{ MachineAcquisition: { action: "release", allocation_ref: "alloc-1" } }])
  assert.equal(JSON.parse(release.out).state, "deleting")
})

test("a refusal prints its plain sentence and a refused denial exits 3", async () => {
  const refused = (code: string, message: string): Handler => (socket, frame) => replyWith(socket, frame, { error: { code, message, retryable: false } })
  const stock = await run(["machine", "acquire", "agent-small", "--region", "fsn1"], refused("capacity_unavailable", "agent-small in fsn1 is out of stock. In stock now: agent-small in hel1."))
  assert.match(stock.err, /agent-small in fsn1 is out of stock\. In stock now: agent-small in hel1\./)
  assert.equal(stock.exit, 1)
  const denied = await run(["machine", "acquire", "agent-small"], refused("acquisition_denied", "The owner denied this request, so it was not sent to the service"))
  assert.equal(denied.exit, 3)
})

test("machine needs a known verb and a class", async () => {
  const none = await run(["machine"], answer({}))
  assert.equal(none.exit, 2)
  const noClass = await run(["machine", "acquire"], answer({}))
  assert.equal(noClass.exit, 2)
  assert.deepEqual(noClass.requests, [])
})

test("machine status without an allocation ref asks for the owner's allocations with no allocation_ref and prints them", async () => {
  const listed = { kind: "machines", machines: [{ allocation_ref: "alloc-1", name: "Cloud 1", state: "ready", revision: 1, since: "2026-11-26T00:00:00Z", kernel_id: null, machine_id: null, available_actions: ["delete"], class_id: "agent-small", region: "fsn1", created_at: "2026-11-26T00:00:00Z" }], saved_states: [] }
  const result = await run(["machine", "status"], answer(listed))
  assert.deepEqual(result.requests, [{ MachineAcquisition: { action: "status" } }])
  assert.equal(JSON.parse(result.out).machines[0].class_id, "agent-small")
})
