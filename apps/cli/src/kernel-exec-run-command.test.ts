import assert from "node:assert/strict"
import { EventEmitter } from "node:events"
import { CommandFailure } from "./agent-command-table.js"
import { Writable } from "node:stream"
import test from "node:test"
import { runKernelExec, type KernelExecOptions } from "./kernel-exec.js"
import type { SendRequest } from "./kernel-files.js"

async function exercise(options: KernelExecOptions) {
  const requests: { target: unknown; operation: Record<string, unknown> }[] = []
  const written: Buffer[] = []
  let endInput!: () => void
  const inputEnded = new Promise<void>(resolve => { endInput = resolve })
  const output: Buffer[] = []
  const stdout = new Writable({ write(bytes: Buffer, _encoding, done) { output.push(Buffer.from(bytes)); done() } })
  const send: SendRequest = async body => {
    assert.ok("RunCommand" in body, "the kernel CLI entry uses only the unified command family")
    const request = body.RunCommand as { target: unknown; operation: Record<string, unknown> }
    requests.push(request)
    const operation = request.operation
    let result: unknown
    switch (operation.operation) {
      case "exec_open": result = { result: "exec_opened", exec_id: "exec-1" }; break
      case "exec_write": {
        const bytes = Buffer.from(operation.data_base64 as string, "base64")
        written.push(bytes)
        result = { result: "exec_input", next_offset: Number(operation.offset) + bytes.length }
        break
      }
      case "exec_stdin_close":
        endInput()
        result = { result: "exec_input", next_offset: operation.offset }
        break
      case "exec_read":
        await inputEnded
        result = { result: "exec_output", output: { chunks: [{ cursor: 1, stream: "stdout", data_base64: Buffer.from("complete").toString("base64") }], next_cursor: 1, outcome: { kind: "exited", code: 7 } } }
        break
      case "exec_close": result = { result: "exec_closed" }; break
      default: assert.fail(`unexpected command operation ${String(operation.operation)}`)
    }
    return { RunCommand: { result } }
  }
  assert.equal(await runKernelExec(options, send, stdout), 7)
  assert.equal(Buffer.concat(output).toString(), "complete")
  assert.ok(requests.every(request => JSON.stringify(request.target) === JSON.stringify({ kind: "kernel" })))
  return { opening: requests.find(request => request.operation.operation === "exec_open")!.operation, written: Buffer.concat(written) }
}

test("the own-kernel CLI command uses RunCommand with its directory and deadline", async () => {
  const { opening, written } = await exercise({ command: ["printf", "complete"], directory: "/workspace", timeoutSeconds: 12 })
  assert.equal(opening.directory, "/workspace")
  assert.equal(opening.timeout_ms, 12_000)
  assert.equal(written.length, 0)
})

test("the own-kernel CLI keeps validated inline stdin as binary command writes", async () => {
  const bytes = Buffer.from([0, 255, 13, 10, 65])
  const { opening, written } = await exercise({ command: ["cat"], timeoutSeconds: 30, stdin: { kind: "inline", data_base64: bytes.toString("base64") } })
  assert.equal(opening.stdin_file, undefined)
  assert.deepEqual(written, bytes)
})

test("the own-kernel CLI names a relative stdin file on the target without reading local stdin", async () => {
  const { opening, written } = await exercise({ command: ["cat"], directory: "/workspace", timeoutSeconds: 30, stdin: { kind: "file", path: "scripts/input.bin" } })
  assert.equal(opening.stdin_file, "scripts/input.bin")
  assert.equal(opening.directory, "/workspace")
  assert.equal(written.length, 0)
})


test("the kernel CLI detaches every signal listener after an admission refusal", async () => {
  const signals = new EventEmitter() as NodeJS.Process
  const stdout = new Writable({ write(_bytes, _encoding, done) { done() } })
  const names = ["SIGINT", "SIGTERM", "SIGHUP"]
  const before = names.map(name => signals.listenerCount(name))
  await assert.rejects(runKernelExec({ command: ["bad-command"], timeoutSeconds: 1, signals }, async body => {
    assert.ok("RunCommand" in body)
    throw new CommandFailure("exec_invalid_command", "command is invalid")
  }, stdout), (failure: unknown) => failure instanceof CommandFailure && failure.code === "exec_invalid_command")
  assert.deepEqual(names.map(name => signals.listenerCount(name)), before)
})
