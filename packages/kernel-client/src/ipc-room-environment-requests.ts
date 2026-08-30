export function getRoomEnvironmentStateRequest(sessionId: string) {
  return {
    GetRoomEnvironmentState: {
      session_id: sessionId,
    },
  }
}
