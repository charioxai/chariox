import assert from "node:assert/strict"
import test from "node:test"

import { listRoomListSessionsPage } from "./room-list-sessions-page.js"

test("the full session browser pages the Room list with the kernel's Room list wire names", async () => {
  const sent: unknown[] = []
  const page = { sessions: [], next_cursor: "next" }
  const client = {
    send: async (request: unknown) => {
      sent.push(request)
      return { RoomListSessionsListed: { page } }
    },
  }
  assert.deepEqual(await listRoomListSessionsPage(client as never, "cursor-1", "inventory-7"), page)
  assert.deepEqual(sent, [{ ListRoomListSessions: { cursor: "cursor-1", limit: 500, inventory_version: "inventory-7" } }])
})
