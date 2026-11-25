import assert from "node:assert/strict"
import test from "node:test"
import { runKernelsCommand } from "./kernels-command.js"
import { replyWith, startFakeKernel } from "./fake-unix-kernel.test-support.js"

const relayStatus = (capabilities?: string[]) => ({ response: { RelayStatus: { status: { daemon_id: "d", machine_id: "m", ...(capabilities ? { capabilities } : {}) } } } })

// Row 551: `chariox kernels restart --kernel X` restarts the kernel (a hot reload); `--browser` keeps restarting only its browser.
test("kernels restart --kernel X sends the kernel restart request to that kernel", async () => {
  const kernel = await startFakeKernel((socket, frame) => {
    if ("RelayStatus" in frame.request) return replyWith(socket, frame, relayStatus(["kernel_gateway_v1"]))
    replyWith(socket, frame, { response: { KernelRestarting: { hot_reload: true } } })
  })
  const printed: string[] = [], log = console.log, written = process.stderr.write.bind(process.stderr), errors: string[] = []
  console.log = (line: string) => { printed.push(line) }
  process.stderr.write = ((chunk: string | Uint8Array) => { errors.push(String(chunk)); return true }) as typeof process.stderr.write
  const exitCode = process.exitCode
  try { await runKernelsCommand(["kernels", "restart", "--kernel", "b3", "--socket", kernel.path]) }
  finally { console.log = log; process.stderr.write = written; process.exitCode = exitCode; await kernel.close() }
  const sent = kernel.frames.map(frame => frame.request).filter(request => !("RelayStatus" in request))
  assert.deepEqual(sent, [{ RestartKernel: {} }], `the restart request is sent as is (stderr: ${errors.join("")})`)
  assert.deepEqual(JSON.parse(printed.at(-1)!), { KernelRestarting: { hot_reload: true } })
})
