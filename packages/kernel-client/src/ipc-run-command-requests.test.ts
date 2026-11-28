import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { readFile } from "node:fs/promises"
import test from "node:test"
import { runCommandRequest } from "./ipc-run-command-requests.js"
import type { RunCommandRequest, RunCommandResult } from "./kernel-types.js"

function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical)
  if (!value || typeof value !== "object") return value
  return Object.fromEntries(Object.entries(value).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([k, v]) => [k, canonical(v)]))
}

test("binary command requests and results share the kernel's golden byte contract", async () => {
  const fixture = JSON.parse(await readFile(new URL("../../../fixtures/room-exec-protocol.json", import.meta.url), "utf8")) as {
    shapes: ({ kind: "request"; wire: { RunCommand: RunCommandRequest }; sha256: string }
      | { kind: "response"; wire: { RunCommand: { result: RunCommandResult } }; sha256: string })[]
  }
  for (const shape of fixture.shapes) {
    const hash = createHash("sha256").update(JSON.stringify(canonical(shape.wire))).digest("hex")
    assert.equal(hash, shape.sha256)
    if (shape.kind === "request") {
      const value = shape.wire.RunCommand
      assert.deepEqual(runCommandRequest(value.target, value.operation, value.acting_agent), shape.wire)
    } else {
      assert.ok(shape.wire.RunCommand.result.result.startsWith("exec_"))
    }
  }
})
