import type { RoomListSessionSummary } from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"

export type RoomListSessionsPage = { sessions: RoomListSessionSummary[]; next_cursor: string | null }

export async function listRoomListSessionsPage(client: Pick<LocalIpcClient, "send">, cursor: string | null, inventoryVersion: string): Promise<RoomListSessionsPage> {
  const response = await client.send({ ListRoomListSessions: { cursor, limit: 500, inventory_version: inventoryVersion } }) as { RoomListSessionsListed: { page: RoomListSessionsPage } }
  return response.RoomListSessionsListed.page
}
