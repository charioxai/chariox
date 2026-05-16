import assert from "node:assert/strict"
import { mkdtemp } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { createConnection, type Socket } from "node:net"
import test from "node:test"

import {
  automationSnapshotMatches,
  startCliAutomationServer,
  stopCliAutomationServer,
} from "./cli-automation.js"

test("automationSnapshotMatches applies wait-for filters", () => {
  const snapshot = {
    screen: "workflow",
    daemonDisconnected: false,
    statusLine: "ready",
    session: { id: "session-1" },
    selectedWorkflow: { alias: "release" },
    workflows: [{ alias: "release" }, { alias: "audit" }],
    shell: { entries: [{ id: 1 }, { id: 2 }] },
  }

  assert.equal(automationSnapshotMatches(snapshot, {
    screen: "workflow",
    daemonDisconnected: false,
    sessionId: "session-1",
    statusLine: "ready",
    selectedWorkflowAlias: "release",
    workflowAlias: "audit",
    shellEntryCount: 2,
  }), true)
  assert.equal(automationSnapshotMatches(snapshot, { workflowAlias: "missing" }), false)
  assert.equal(automationSnapshotMatches(snapshot, { shellEntryCount: 3 }), false)
})

test("startCliAutomationServer frames json-line requests", async () => {
  const dir = await mkdtemp(join(tmpdir(), "arroba-cli-automation-"))
  const socketPath = join(dir, "automation.sock")
  const server = await startCliAutomationServer({
    socketPath,
    formatError: (error) => error instanceof Error ? error.message : String(error),
    handleRequest: async (request) => {
      if (request.action === "fail") {
        throw new Error("boom")
      }
      return { action: request.action ?? null, prompt: request.prompt ?? null }
    },
  })
  const socket = await connect(socketPath)
  try {
    socket.write("not-json\n")
    const invalid = await readAutomationLine(socket)
    assert.equal(invalid.ok, false)
    assert.match(String(invalid.error), /invalid JSON automation request/)

    socket.write(`${JSON.stringify({ id: "request-1", action: "echo", prompt: "hello" })}\n`)
    const success = await readAutomationLine(socket)
    assert.deepEqual(success, {
      id: "request-1",
      ok: true,
      data: { action: "echo", prompt: "hello" },
    })

    socket.write(`${JSON.stringify({ id: 2, action: "fail" })}\n`)
    const failure = await readAutomationLine(socket)
    assert.deepEqual(failure, {
      id: 2,
      ok: false,
      error: "boom",
    })
  } finally {
    socket.destroy()
    stopCliAutomationServer(server, socketPath)
  }
})

async function connect(socketPath: string): Promise<Socket> {
  const socket = createConnection(socketPath)
  socket.setEncoding("utf8")
  await new Promise<void>((resolve, reject) => {
    socket.once("connect", resolve)
    socket.once("error", reject)
  })
  return socket
}

function readAutomationLine(socket: Socket): Promise<Record<string, unknown>> {
  return new Promise((resolve, reject) => {
    let buffer = ""
    const onData = (chunk: string | Buffer) => {
      buffer += chunk.toString()
      const newlineIndex = buffer.indexOf("\n")
      if (newlineIndex === -1) {
        return
      }
      socket.off("data", onData)
      socket.off("error", onError)
      try {
        resolve(JSON.parse(buffer.slice(0, newlineIndex)) as Record<string, unknown>)
      } catch (error) {
        reject(error)
      }
    }
    const onError = (error: Error) => {
      socket.off("data", onData)
      reject(error)
    }
    socket.on("data", onData)
    socket.once("error", onError)
  })
}
