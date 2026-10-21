import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import test from "node:test"

import {
  closeRoomTerminalRequest,
  expectRoomTerminalResult,
  listRoomTerminalsRequest,
  openRoomTerminalRequest,
  readRoomTerminalOutputRequest,
  resizeRoomTerminalRequest,
  roomTerminalInputRequest,
  roomTerminalMinimumProtocolVersion,
  roomTerminalRequest,
  roomTerminalResult,
  roomTerminalSnapshotRequest,
} from "./ipc-room-terminal-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

// The kernel pins these exact wire values and hashes in
// apps/kernel/src/local/api/tests/protocol_shapes/room_terminal.rs; sorted keys match serde_json.
function sorted(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sorted)
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).sort(([a], [b]) => (a < b ? -1 : 1)).map(([k, v]) => [k, sorted(v)]))
  }
  return value
}
const sha256 = (value: unknown) => createHash("sha256").update(JSON.stringify(sorted(value))).digest("hex")

test("room terminal requests match the kernel's pinned open, input and wait shapes", () => {
  const room = { sessionId: "room-a" }
  const open = openRoomTerminalRequest(room, { cols: 100, rows: 30 })
  assert.deepEqual(open, { RoomTerminal: { session_id: "room-a", operation: { operation: "open", cols: 100, rows: 30 } } })
  assert.equal(sha256(open), "fda70d2f8b97d306b0f93f048b302f9316b8b8fda89be2be913667c525f6ece7")

  const input = roomTerminalInputRequest({ ...room, actingAgent: "agent-1" }, "term-1", new TextEncoder().encode("ls\r\n"))
  assert.equal(sha256(input), "f63a2067883cd391ac1a88656965d7c2500a2204300c45dabe785c071672cce7")

  const wait = roomTerminalRequest(room, {
    operation: "wait",
    terminal_id: "term-1",
    after_offset: 42,
    watches: [{ kind: "finished" }, { kind: "output_contains", text: "ready" }, { kind: "waiting_for_input" }],
    timeout_ms: 240000,
  })
  assert.equal(sha256(wait), "b04036b0df3810f15b001625952526db578c86a3486365e901a3567e58f765a0")
})

test("room terminal builders send only what was asked for", () => {
  const worker = { sessionId: "room-a", hostAgent: "worker-agent" }
  assert.deepEqual(listRoomTerminalsRequest(worker), {
    RoomTerminal: { session_id: "room-a", host_agent: "worker-agent", operation: { operation: "list" } },
  })
  assert.deepEqual(roomTerminalSnapshotRequest(worker, "t").RoomTerminal.operation, { operation: "snapshot", terminal_id: "t" })
  assert.deepEqual(resizeRoomTerminalRequest(worker, "t", { cols: 80, rows: 24 }).RoomTerminal.operation, {
    operation: "resize", terminal_id: "t", cols: 80, rows: 24,
  })
  assert.deepEqual(closeRoomTerminalRequest(worker, "t").RoomTerminal.operation, { operation: "close", terminal_id: "t" })
  assert.deepEqual(readRoomTerminalOutputRequest(worker, "t", 7).RoomTerminal.operation, {
    operation: "read_output", terminal_id: "t", after_offset: 7,
  })
  assert.deepEqual(readRoomTerminalOutputRequest(worker, "t", 7, 2000).RoomTerminal.operation, {
    operation: "read_output", terminal_id: "t", after_offset: 7, wait_ms: 2000,
  })
  assert.deepEqual(
    openRoomTerminalRequest(worker, { cols: 80, rows: 24 }, { title: "build", command: ["make", "-j4"] }).RoomTerminal.operation,
    { operation: "open", cols: 80, rows: 24, title: "build", command: ["make", "-j4"] },
  )
})

test("input bytes survive base64 including binary and large pastes", () => {
  const bytes = Uint8Array.from({ length: 70_000 }, (_, i) => i % 256)
  const request = roomTerminalInputRequest({ sessionId: "r" }, "t", bytes)
  const operation = request.RoomTerminal.operation
  assert.equal(operation.operation, "input")
  if (operation.operation !== "input") return
  assert.deepEqual(new Uint8Array(Buffer.from(operation.data_base64, "base64")), bytes)
})

test("room terminal results are read from the kernel reply or explain the refusal", () => {
  const refused = { RoomTerminal: { result: { result: "input", input: {
    accepted: false,
    driver: { state: "busy", driven_by: { kind: "agent", agent_id: "agent-1" } },
    refusal: "busy (driven by agent-1)",
  } } } }
  assert.equal(sha256(refused), "909020e37bf1bb285c875761906626455d18750481536875ae14b21c0b8a8542")
  assert.equal(expectRoomTerminalResult(refused, "input").input.refusal, "busy (driven by agent-1)")
  assert.throws(() => expectRoomTerminalResult(refused, "closed"), /Expected a closed terminal result, got input/)
  assert.throws(() => roomTerminalResult({ Error: { message: "terminal is not yours" } }), /terminal is not yours/)
  assert.throws(() => roomTerminalResult({}), /no terminal result/)
})

test("the Room terminal service is available from its protocol version", () => {
  assert.ok(LOCAL_DAEMON_PROTOCOL_VERSION >= roomTerminalMinimumProtocolVersion)
})
