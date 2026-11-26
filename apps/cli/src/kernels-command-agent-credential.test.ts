// Row 614: an agent's shell has its own credential for the kernel's own endpoint. A process consumes that credential
// once for one endpoint (a second one is refused by design), so these cells share one endpoint and live in a file of
// their own: each test file runs in its own process.
import assert from "node:assert/strict"
import type { AddressInfo } from "node:net"
import test, { after } from "node:test"
import { WebSocketServer } from "ws"
import { runKernelsCommand } from "./kernels-command.js"
import { runMachineCommand } from "./machine-command.js"
import { replyWith, startFakeKernel, type FakeKernelFrame } from "./fake-unix-kernel.test-support.js"

const refusal = (code: string) => ({ error: { code, message: `refused ${code}`, retryable: false } })
const TOKEN = "chx_act_run-1.signature"

// The grant socket answers like a kernel that has not approved this process.
const grantSocket = await startFakeKernel((socket, frame) => replyWith(socket, frame, refusal("kernel_access_not_covered")))
// The kernel's own endpoint, which admits the run's credential.
const authorizations: (string | undefined)[] = []
const seen: string[] = []
let answer: (frame: FakeKernelFrame) => unknown = () => ({})
const own = new WebSocketServer({ host: "127.0.0.1", port: 0 })
own.on("connection", (socket, request) => {
  authorizations.push(request.headers.authorization)
  socket.on("message", bytes => {
    const frame = JSON.parse(String(bytes)) as FakeKernelFrame
    seen.push(Object.keys(frame.request)[0]!)
    replyWith(socket, frame, { response: answer(frame) })
  })
})
await new Promise<void>(resolve => own.once("listening", () => resolve()))
process.env.CHARIOX_AGENT_CLI_TOKEN = TOKEN
process.env.CHARIOX_KERNEL_URL = `ws://127.0.0.1:${(own.address() as AddressInfo).port}`
process.env.CHARIOX_DAEMON_SOCKET = grantSocket.path
after(async () => {
  for (const client of own.clients) client.terminate()
  await new Promise<void>(resolve => own.close(() => resolve()))
  await grantSocket.close()
})

async function printing(run: () => Promise<unknown>): Promise<string[]> {
  seen.length = 0
  authorizations.length = 0
  grantSocket.frames.length = 0
  const printed: string[] = [], log = console.log, code = process.exitCode
  console.log = (line: string) => { printed.push(line) }
  try { await run() } finally { console.log = log; process.exitCode = code }
  return printed
}

test("kernels from an agent's shell uses the agent credential on the kernel's own endpoint, not the grant socket", async () => {
  answer = () => ({ GatewayKernelsListed: { kernels: [{ kernel_id: "k1", name: "this" }] } })
  const printed = await printing(() => runKernelsCommand(["kernels"]))
  assert.deepEqual(seen, ["ListGatewayKernels"])
  assert.equal(authorizations[0], `Bearer ${TOKEN}`)
  assert.equal(grantSocket.frames.length, 0, "nothing is sent over the grant socket")
  assert.deepEqual(JSON.parse(printed.at(-1)!), { kernels: [{ kernel_id: "k1", name: "this" }] })
})

test("machine status from an agent's shell uses the agent credential, with no access grant", async () => {
  answer = () => ({ MachineAcquisition: { result: { kind: "machine", allocation_ref: "alloc-1", state: "ready" } } })
  const printed = await printing(() => runMachineCommand(["machine", "status", "alloc-1"]))
  assert.deepEqual(seen, ["MachineAcquisition"])
  assert.equal(authorizations[0], `Bearer ${TOKEN}`)
  assert.equal(grantSocket.frames.length, 0, "nothing is sent over the grant socket")
  assert.equal(JSON.parse(printed.join("\n")).state, "ready")
})

test("an explicit --socket still wins over the agent credential", async () => {
  await printing(() => runKernelsCommand(["kernels", "--socket", grantSocket.path]))
  assert.ok(grantSocket.frames.length > 0, "the named socket was used")
  assert.deepEqual(seen, [], "the agent endpoint was not used")
})
