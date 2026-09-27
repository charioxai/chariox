import assert from "node:assert/strict"
import test from "node:test"
import { parseSlashCommand } from "./commands.js"
import { handleRoomSlashCommand } from "./room-command-handler.js"

test("Room owner can explicitly bind a slice without starting or restarting it", async () => {
  const command = parseSlashCommand("/room bind desktop")
  assert.equal(command?.kind, "room")
  const requests: unknown[] = []
  const notices: string[] = []
  const errors: string[] = []
  await handleRoomSlashCommand({
    isAttached: () => true,
    sessionId: () => "room-1",
    send: async <T>(request: unknown) => {
      requests.push(request)
      return { RoomEnvironmentSlice: { binding: {
        session_id: "room-1", slice_id: "slice-7",
        owner_kernel_id: "home", worker_kernel_ref: "worker",
      } } } as T
    },
    appendNotice: value => notices.push(value),
    flashFooter: value => errors.push(value),
  }, command)
  assert.deepEqual(errors, [])
  assert.deepEqual(requests, [{ BindRoomEnvironmentSlice: { session_id: "room-1", slice_ref: "desktop" } }])
  assert.match(notices.join("\n"), /slice-7/)
  assert.match(notices.join("\n"), /restart/)
  assert.match(notices.join("\n"), /not started or restarted/)
})

test("Room binding rejects invalid arguments or a detached terminal before sending", async () => {
  for (const [input, attached] of [["/room bind", true], ["/room bind a b", true], ["/room bind desktop", false]] as const) {
    const command = parseSlashCommand(input)
    assert.equal(command?.kind, "room")
    const errors: string[] = []
    await handleRoomSlashCommand({
      isAttached: () => attached, sessionId: () => "room-1",
      send: async () => { assert.fail("invalid binding must not reach the kernel") },
      appendNotice: () => assert.fail("invalid binding must not report success"),
      flashFooter: value => errors.push(value),
    }, command)
    assert.equal(errors.length, 1)
  }
})

test("Room binding preserves kernel denial and rejects missing or wrong-Room results", async () => {
  const command = parseSlashCommand("/room bind desktop")
  assert.equal(command?.kind, "room")
  for (const response of [null, {}, { RoomEnvironmentSlice: { binding: null } }, {
    RoomEnvironmentSlice: { binding: { session_id: "other-room", slice_id: "slice-7" } },
  }]) {
    await assert.rejects(handleRoomSlashCommand({
      isAttached: () => true, sessionId: () => "room-1",
      send: async <T>() => response as T,
      appendNotice: () => assert.fail("bad response must not report success"),
      flashFooter: () => undefined,
    }, command), /binding response is malformed or belongs to another Room/)
  }
  const denial = new Error("Room owner required")
  await assert.rejects(handleRoomSlashCommand({
    isAttached: () => true, sessionId: () => "room-1",
    send: async () => { throw denial },
    appendNotice: () => assert.fail("denial must not report success"),
    flashFooter: () => undefined,
  }, command), error => error === denial)
})

test("Room binding reports the existing protocol minimum for old kernels", async () => {
  const command = parseSlashCommand("/room bind desktop")
  assert.equal(command?.kind, "room")
  await assert.rejects(handleRoomSlashCommand({
    isAttached: () => true, sessionId: () => "room-1",
    send: async () => { throw new Error("unknown variant `BindRoomEnvironmentSlice`") },
    appendNotice: () => assert.fail("unsupported kernel must not report success"),
    flashFooter: () => undefined,
  }, command), /Room slice binding requires kernel protocol 282 or newer/)
})
