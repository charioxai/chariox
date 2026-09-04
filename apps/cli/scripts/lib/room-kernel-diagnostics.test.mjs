import assert from "node:assert/strict"
import { mkdtemp, writeFile, rm, symlink } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { captureRoomKernelDiagnostics } from "./room-kernel-diagnostics.mjs"

test("retains private relay connection stages without log payloads or credentials", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-kernel-evidence-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const privateUrl = "ws://127.0.0.1:45200"
  await writeFile(path.join(root, "100-daemon-42.ndjson"), [
    { component: "daemon.slice_private_relay", message: "home connector thread starting", timestamp_ms: 100, relay_url: privateUrl, slice_id: "private-room-name" },
    { component: "daemon.relay_client", message: "attempting relay connection", timestamp_ms: 101, relay_url: privateUrl, auth_token: "TOKEN-MUST-NOT-LEAK" },
    { component: "daemon.relay_client", message: "relay socket disconnected", timestamp_ms: 102, error: "Connection refused: TOKEN-MUST-NOT-LEAK", output: "PROMPT-MUST-NOT-LEAK" },
    { component: "provider", message: "attempting relay connection", output: "PROMPT-MUST-NOT-LEAK" },
  ].map(JSON.stringify).join("\n") + "\n")
  const result = await captureRoomKernelDiagnostics(root, { private: privateUrl })
  assert.equal(result.status, "captured")
  assert.deepEqual(result.events, [
    { component: "daemon.slice_private_relay", event: "home connector thread starting", timestampMs: 100, relay: "private" },
    { component: "daemon.relay_client", event: "attempting relay connection", timestampMs: 101, relay: "private" },
    { component: "daemon.relay_client", event: "relay socket disconnected", timestampMs: 102, errorClass: "connection-refused" },
  ])
  assert.doesNotMatch(JSON.stringify(result), /TOKEN|PROMPT|private-room-name|127\.0\.0\.1/)
})

test("classifies the kernel's actual disconnect reason field without retaining its text", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-kernel-evidence-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  await writeFile(path.join(root, "100-daemon-42.ndjson"), JSON.stringify({
    component: "daemon.relay_client", message: "relay socket disconnected",
    reason: "relay token does not allow requested action: SECRET",
  }) + "\n")
  const result = await captureRoomKernelDiagnostics(root)
  assert.equal(result.events[0].errorClass, "action-not-allowed")
  assert.doesNotMatch(JSON.stringify(result), /SECRET/)
})

test("bounds large logs and ignores symlinks, malformed records and partial writes", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-kernel-evidence-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const record = JSON.stringify({ component: "daemon.relay_client", message: "relay register sent", timestamp_ms: 1 }) + "\n"
  await writeFile(path.join(root, "100-daemon-42.ndjson"), "PRIVATE".repeat(12000) + "\ninvalid\n" + record.repeat(200) + '{"partial":')
  await writeFile(path.join(root, "other.log"), record)
  await symlink(path.join(root, "other.log"), path.join(root, "101-daemon-43.ndjson"))
  const result = await captureRoomKernelDiagnostics(root)
  assert.equal(result.filesRead, 1)
  assert.equal(result.bytesRead, 65536)
  assert.equal(result.truncated, true)
  assert.equal(result.events.length, 128)
  assert.ok(result.events.every((event) => event.event === "relay register sent"))
  assert.doesNotMatch(JSON.stringify(result), /PRIVATE|partial|invalid/)
})

test("bounds total reads across rotated daemon logs", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-kernel-evidence-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  for (let index = 0; index < 10; index++) {
    await writeFile(path.join(root, `100-daemon-42-${index}.ndjson`), "x".repeat(70000))
  }
  const result = await captureRoomKernelDiagnostics(root)
  assert.equal(result.filesRead, 4)
  assert.equal(result.bytesRead, 262144)
  assert.equal(result.truncated, true)
  assert.deepEqual(result.events, [])
})

test("reports missing log directories without retaining their private path", async () => {
  const result = await captureRoomKernelDiagnostics("/nonexistent/PRIVATE-ROOM-NAME/logs")
  assert.equal(result.status, "missing")
  assert.doesNotMatch(JSON.stringify(result), /PRIVATE/)
})

test("retains the latest failure when retries exceed the event budget", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-kernel-evidence-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const records = Array.from({ length: 200 }, (_, index) => ({
    component: "daemon.relay_client", message: "attempting relay connection", timestamp_ms: index,
  }))
  records.push({ component: "daemon.relay_client", message: "relay socket connect failed", timestamp_ms: 200, error: "Connection refused" })
  await writeFile(path.join(root, "100-daemon-42.ndjson"), records.map(JSON.stringify).join("\n") + "\n")
  const result = await captureRoomKernelDiagnostics(root)
  assert.equal(result.events.length, 128)
  assert.equal(result.events[0].timestampMs, 73)
  assert.equal(result.events.at(-1).errorClass, "connection-refused")
  assert.equal(result.truncated, true)
})
